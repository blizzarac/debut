//! Collaborative editing (COL-01 .. COL-08): join a shared session, send
//! every local edit, fold in everyone else's, and see where the others are.
//!
//! Local edits apply at once and are sent with their base version; edits
//! from others arrive in server order and are rebased under ours by
//! `debut_collab::Client`. Edits the server refuses (a locked track, a
//! reviewer touching the cut, a clip someone else deleted) are rolled back
//! and reported as notes. Polling is explicit (`collab_poll`) so the UI
//! decides how often the network is read.

use super::*;
use debut_collab::{access, sync::Event, Client, Message, Presence, Role};
use debut_platform::Connection;

pub(crate) struct Collab {
    conn: Box<dyn Connection>,
    client: Client,
    address: String,
    name: String,
    role: Role,
    joined: bool,
    peers: std::collections::HashMap<u64, Presence>,
    locks: Vec<(String, u64)>,
    notes: Vec<String>,
    last_presence: Option<(Option<String>, i64, Option<String>)>,
}

impl Collab {
    fn send(&mut self, msg: &Message) {
        if self.conn.send(&msg.to_line()).is_err() {
            self.notes
                .push("connection lost; edits are kept and sent on reconnect".into());
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct PeerDto {
    pub client: u64,
    pub name: String,
    pub role: String,
    pub sequence: Option<String>,
    pub playhead: f64,
    pub clip: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct LockDto {
    pub track: String,
    pub client: u64,
    /// "you" or the holder's name.
    pub owner: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CollabDto {
    pub connected: bool,
    pub joined: bool,
    pub address: String,
    pub name: String,
    pub role: String,
    pub client: u64,
    pub version: u64,
    /// Local edits the server has not confirmed yet.
    pub pending: usize,
    pub peers: Vec<PeerDto>,
    pub locks: Vec<LockDto>,
    /// Refused or dropped edits and connection notices, newest last.
    pub notes: Vec<String>,
}

fn role_name(r: Role) -> String {
    match r {
        Role::Editor => "editor".into(),
        Role::Reviewer => "reviewer".into(),
    }
}

impl Session {
    /// Join (or rejoin) the session at `addr` as `name` with `role`
    /// ("editor" or "reviewer"). The shared project replaces the open one
    /// once the server answers (`collab_poll`); edits made while away are
    /// replayed on it.
    pub fn collab_join(&mut self, addr: &str, name: String, role: &str) -> Result<(), String> {
        let role = match role {
            "editor" => Role::Editor,
            "reviewer" => Role::Reviewer,
            other => return Err(format!("unknown role {other}")),
        };
        self.project().ok_or("no project open")?;
        let mut conn = self.platform.connect(addr).map_err(|e| e.to_string())?;
        conn.send(
            &Message::Hello {
                name: name.clone(),
                role,
            }
            .to_line(),
        )
        .map_err(|e| e.to_string())?;
        // Keep the client (and its pending edits) across reconnects.
        let client = self.collab.take().map(|c| c.client).unwrap_or_default();
        self.collab = Some(Collab {
            conn,
            client,
            address: addr.to_string(),
            name,
            role,
            joined: false,
            peers: Default::default(),
            locks: Vec::new(),
            notes: Vec::new(),
            last_presence: None,
        });
        Ok(())
    }

    /// A copy of the open project, to host a session with.
    pub fn project_snapshot(&self) -> Option<Project> {
        self.project().cloned()
    }

    pub fn collab_leave(&mut self) {
        self.collab = None;
    }

    /// Before a local edit: refuse what this role may not do. Returns the
    /// inverse to record when collaborating.
    pub(crate) fn collab_before(&self, cmd: &Command) -> Result<Option<Command>, String> {
        let Some(c) = &self.collab else {
            return Ok(None);
        };
        if !access::allowed(c.role, cmd) {
            return Err("reviewers can only add or change comments (markers)".into());
        }
        let project = self.project().ok_or("no project open")?;
        cmd.invert(project).map(Some).map_err(|e| e.to_string())
    }

    /// After a local edit applied: queue it and send it.
    pub(crate) fn collab_after(&mut self, cmd: Command, inverse: Command) {
        if let Some(c) = &mut self.collab {
            let id = c.client.record(cmd.clone(), inverse);
            if c.joined {
                let base = c.client.version;
                c.send(&Message::Submit { id, base, cmd });
            }
        }
    }

    /// Read the network: apply others' edits, confirmations and refusals,
    /// presence and locks; send our presence if it changed. `clip` is the
    /// clip selected in the UI. Returns whether the project changed.
    pub fn collab_poll(&mut self, clip: Option<String>) -> Result<bool, String> {
        let Some(mut c) = self.collab.take() else {
            return Ok(false);
        };
        let lines = c.conn.poll();
        let mut changed = false;
        let note = |c: &mut Collab, events: Vec<Event>| {
            for Event::Dropped { reason, .. } in events {
                c.notes.push(format!("your edit was undone: {reason}"));
            }
        };
        for line in lines {
            let Some(msg) = Message::from_line(&line) else {
                continue;
            };
            match msg {
                Message::Welcome {
                    client,
                    version,
                    project,
                    locks,
                    peers,
                } => {
                    let snapshot = schema::from_json(&project).map_err(|e| e.to_string())?;
                    let ws = self.workspace.as_mut().ok_or("no project open")?;
                    let mut p = ws.project.clone();
                    let events = c.client.rejoin(&mut p, client, version, snapshot);
                    ws.replace_project(p);
                    note(&mut c, events);
                    c.joined = true;
                    c.locks = locks;
                    c.peers = peers.into_iter().map(|p| (p.client, p)).collect();
                    for (id, cmd) in c.client.outgoing() {
                        let base = c.client.version;
                        c.send(&Message::Submit { id, base, cmd });
                    }
                    self.active = None;
                    changed = true;
                }
                Message::Op(entry) => {
                    let ws = self.workspace.as_mut().ok_or("no project open")?;
                    match c.client.receive(&mut ws.project, &entry) {
                        Ok(events) => note(&mut c, events),
                        Err(e) => c.notes.push(format!("out of step ({e}); rejoin to resync")),
                    }
                    ws.touch();
                    changed = true;
                }
                Message::Reject { id, reason } => {
                    let ws = self.workspace.as_mut().ok_or("no project open")?;
                    let events = c
                        .client
                        .rejected(&mut ws.project, id)
                        .map_err(|e| e.to_string())?;
                    note(&mut c, events);
                    c.notes.push(format!("edit refused: {reason}"));
                    changed = true;
                }
                Message::Presence(p) => {
                    if p.client != c.client.id {
                        c.peers.insert(p.client, p);
                    }
                }
                Message::Locks { locks } => c.locks = locks,
                Message::Left { client } => {
                    c.peers.remove(&client);
                }
                _ => {}
            }
        }
        // Our own presence, when it moved.
        if c.joined {
            let seq = self.first_sequence().ok().map(|s| id_str(s.id.0));
            let playhead = self.playhead().as_f64();
            let key = (seq.clone(), (playhead * 25.0).round() as i64, clip.clone());
            if c.last_presence.as_ref() != Some(&key) {
                c.last_presence = Some(key);
                let me = Presence {
                    client: c.client.id,
                    name: c.name.clone(),
                    role: c.role,
                    sequence: seq,
                    playhead,
                    clip,
                };
                c.send(&Message::Presence(me));
            }
        }
        if !c.conn.is_open() && c.joined {
            c.joined = false;
            c.notes
                .push("disconnected; keep editing, rejoin to send your edits".into());
        }
        self.collab = Some(c);
        if changed {
            self.sync_player()?;
        }
        Ok(changed)
    }

    /// Lock or unlock a track for yourself (others' edits to it are refused).
    pub fn collab_lock(&mut self, track: &str, on: bool) -> Result<(), String> {
        let c = self.collab.as_mut().ok_or("not in a shared session")?;
        c.send(&Message::Lock {
            track: track.to_string(),
            on,
        });
        Ok(())
    }

    pub fn collab_status(&self) -> Option<CollabDto> {
        let c = self.collab.as_ref()?;
        let name_of = |id: u64| {
            if id == c.client.id {
                "you".to_string()
            } else {
                c.peers
                    .get(&id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| format!("#{id}"))
            }
        };
        let mut peers: Vec<PeerDto> = c
            .peers
            .values()
            .map(|p| PeerDto {
                client: p.client,
                name: p.name.clone(),
                role: role_name(p.role),
                sequence: p.sequence.clone(),
                playhead: p.playhead,
                clip: p.clip.clone(),
            })
            .collect();
        peers.sort_by_key(|p| p.client);
        Some(CollabDto {
            connected: c.conn.is_open(),
            joined: c.joined,
            address: c.address.clone(),
            name: c.name.clone(),
            role: role_name(c.role),
            client: c.client.id,
            version: c.client.version,
            pending: c.client.pending(),
            peers,
            locks: c
                .locks
                .iter()
                .map(|(t, id)| LockDto {
                    track: t.clone(),
                    client: *id,
                    owner: name_of(*id),
                })
                .collect(),
            notes: c.notes.iter().rev().take(20).rev().cloned().collect(),
        })
    }
}
