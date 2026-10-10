//! Op ordering and rebase (COL-02, COL-06).
//!
//! The [`Server`] owns the authoritative project and gives every accepted
//! edit the next version number. An edit that no longer applies on the
//! server's project (someone deleted the clip first) is rejected back to its
//! author.
//!
//! A [`Client`] applies its own edits immediately and keeps them as
//! *pending* until the server echoes them back. When someone else's edit
//! arrives first, the client undoes its pending edits, applies the remote
//! one, and replays its pending edits on top; one that no longer applies is
//! dropped and reported. When the server confirms an edit it stops being
//! pending. After a disconnect the client rebuilds from a fresh snapshot and
//! replays whatever it edited offline.

use debut_command::Command;
use debut_core::{Error, Result};
use debut_project::Project;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// One accepted edit in its final order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub version: u64,
    pub author: u64,
    /// The author's id for the edit, to match it against their pending list.
    pub id: u64,
    pub cmd: Command,
}

#[derive(Debug)]
pub struct Server {
    pub project: Project,
    version: u64,
}

impl Server {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            version: 0,
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// Apply `cmd` from `author` and give it the next version, or say why not.
    pub fn submit(&mut self, author: u64, id: u64, cmd: Command) -> Result<LogEntry> {
        cmd.apply(&mut self.project)?;
        self.version += 1;
        Ok(LogEntry {
            version: self.version,
            author,
            id,
            cmd,
        })
    }
}

/// An edit made locally that the server has not confirmed yet.
#[derive(Clone, Debug)]
struct Pending {
    id: u64,
    cmd: Command,
    /// Undoes `cmd` on the project as it was when `cmd` last applied.
    inverse: Command,
}

/// What happened to local edits while integrating remote ones.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A pending edit no longer applied after a remote edit and was dropped.
    Dropped { id: u64, reason: String },
}

#[derive(Debug, Default)]
pub struct Client {
    pub id: u64,
    /// Last server version integrated.
    pub version: u64,
    pending: VecDeque<Pending>,
    next_id: u64,
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Record `cmd`, which the caller has just applied to `project` (its
    /// inverse computed beforehand), and return its edit id for submission.
    pub fn record(&mut self, cmd: Command, inverse: Command) -> u64 {
        self.next_id += 1;
        self.pending.push_back(Pending {
            id: self.next_id,
            cmd,
            inverse,
        });
        self.next_id
    }

    /// The pending edits to (re)send, oldest first.
    pub fn outgoing(&self) -> Vec<(u64, Command)> {
        self.pending.iter().map(|p| (p.id, p.cmd.clone())).collect()
    }

    /// Undo every pending edit on `project` (newest first).
    fn unwind(&self, project: &mut Project) -> Result<()> {
        for p in self.pending.iter().rev() {
            p.inverse.apply(project)?;
        }
        Ok(())
    }

    /// Replay pending edits on `project`, dropping those that no longer apply.
    fn replay(&mut self, project: &mut Project) -> Vec<Event> {
        let mut events = Vec::new();
        let mut kept = VecDeque::new();
        for p in self.pending.drain(..) {
            let applied = p
                .cmd
                .invert(project)
                .and_then(|inv| p.cmd.apply(project).map(|_| inv));
            match applied {
                Ok(inverse) => kept.push_back(Pending { inverse, ..p }),
                Err(e) => events.push(Event::Dropped {
                    id: p.id,
                    reason: e.to_string(),
                }),
            }
        }
        self.pending = kept;
        events
    }

    /// Integrate an edit from the server into `project`.
    pub fn receive(&mut self, project: &mut Project, entry: &LogEntry) -> Result<Vec<Event>> {
        if entry.version <= self.version {
            return Ok(Vec::new()); // already have it
        }
        if entry.version != self.version + 1 {
            return Err(Error::InvalidArgument(format!(
                "missed edits: at {} but got {}",
                self.version, entry.version
            )));
        }
        self.version = entry.version;
        // Our own oldest pending edit, confirmed in place: nothing to redo.
        if entry.author == self.id && self.pending.front().is_some_and(|p| p.id == entry.id) {
            self.pending.pop_front();
            return Ok(Vec::new());
        }
        self.unwind(project)?;
        entry.cmd.apply(project)?;
        Ok(self.replay(project))
    }

    /// The server rejected pending edit `id`: take it out, keep the rest.
    pub fn rejected(&mut self, project: &mut Project, id: u64) -> Result<Vec<Event>> {
        self.unwind(project)?;
        self.pending.retain(|p| p.id != id);
        Ok(self.replay(project))
    }

    /// (Re)joined: `snapshot` at `version` replaces the project; edits made
    /// meanwhile (offline included) are replayed on it and stay pending, to
    /// be sent again with [`Client::outgoing`].
    pub fn rejoin(
        &mut self,
        project: &mut Project,
        id: u64,
        version: u64,
        snapshot: Project,
    ) -> Vec<Event> {
        self.id = id;
        self.version = version;
        *project = snapshot;
        self.replay(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_command::Target;
    use debut_core::{ClipId, FrameRate, IdGen, Rational};
    use debut_project::{Clip, ClipSource, Sequence, Track, TrackKind};

    fn sec(n: i64) -> Rational {
        Rational::from_int(n)
    }

    /// One sequence with V1 holding three 10 s clips.
    fn base(ids: &mut IdGen) -> (Project, Target, Vec<ClipId>) {
        let mut p = Project::new(ids.fresh(), "p");
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 16, 9);
        let mut t = Track::new(ids.fresh(), TrackKind::Video);
        let m = ids.fresh();
        for i in 0..3 {
            t.clips.push(Clip::new(
                ids.fresh(),
                ClipSource::Media(m),
                sec(i * 10),
                sec(10),
                sec(0),
            ));
        }
        let target = Target {
            sequence: seq.id,
            track: t.id,
        };
        let clips = t.clips.iter().map(|c| c.id).collect();
        seq.tracks.push(t);
        p.sequences.push(seq);
        (p, target, clips)
    }

    /// A peer: its own project copy plus a client.
    struct Peer {
        project: Project,
        client: Client,
    }

    impl Peer {
        fn edit(&mut self, cmd: Command) -> (u64, Command) {
            let inv = cmd.invert(&self.project).unwrap();
            cmd.apply(&mut self.project).unwrap();
            (self.client.record(cmd.clone(), inv), cmd)
        }
    }

    fn json(p: &Project) -> String {
        debut_project::schema::to_json(p).unwrap()
    }

    #[test]
    fn concurrent_edits_converge() {
        let mut ids = IdGen::new(5);
        let (p, target, clips) = base(&mut ids);
        let mut server = Server::new(p.clone());
        let mut a = Peer {
            project: p.clone(),
            client: Client {
                id: 1,
                ..Client::new()
            },
        };
        let mut b = Peer {
            project: p,
            client: Client {
                id: 2,
                ..Client::new()
            },
        };
        // Both edit at once: A moves clip 0, B trims clip 2.
        let (ia, ca) = a.edit(Command::Move {
            target,
            clip: clips[0],
            delta: sec(-0),
        });
        let (ib, cb) = b.edit(Command::TrimTail {
            target,
            clip: clips[2],
            delta: sec(-4),
        });
        // The server takes B's first.
        let eb = server.submit(2, ib, cb).unwrap();
        let ea = server.submit(1, ia, ca).unwrap();
        for e in [&eb, &ea] {
            assert!(a.client.receive(&mut a.project, e).unwrap().is_empty());
            assert!(b.client.receive(&mut b.project, e).unwrap().is_empty());
        }
        assert_eq!(json(&a.project), json(&server.project));
        assert_eq!(json(&b.project), json(&server.project));
        assert_eq!((a.client.pending(), b.client.pending()), (0, 0));
    }

    #[test]
    fn conflicting_edit_is_rejected_and_dropped_locally() {
        let mut ids = IdGen::new(6);
        let (p, target, clips) = base(&mut ids);
        let mut server = Server::new(p.clone());
        let mut a = Peer {
            project: p.clone(),
            client: Client {
                id: 1,
                ..Client::new()
            },
        };
        let mut b = Peer {
            project: p,
            client: Client {
                id: 2,
                ..Client::new()
            },
        };
        // A removes clip 1 (lift its span); B trims the same clip meanwhile.
        let lift = Command::lift(target, sec(10), sec(20), &mut ids);
        let (ia, ca) = a.edit(lift);
        let (ib, cb) = b.edit(Command::TrimTail {
            target,
            clip: clips[1],
            delta: sec(-2),
        });
        let ea = server.submit(1, ia, ca).unwrap();
        // B's trim now targets a missing clip: the server refuses it.
        assert!(server.submit(2, ib, cb.clone()).is_err());
        // B integrates A's lift: its pending trim no longer applies and is dropped.
        let events = b.client.receive(&mut b.project, &ea).unwrap();
        assert!(matches!(events.as_slice(), [Event::Dropped { id, .. }] if *id == ib));
        assert!(b.client.rejected(&mut b.project, ib).unwrap().is_empty());
        a.client.receive(&mut a.project, &ea).unwrap();
        assert_eq!(json(&a.project), json(&server.project));
        assert_eq!(json(&b.project), json(&server.project));
    }

    #[test]
    fn offline_edits_replay_on_a_fresh_snapshot() {
        let mut ids = IdGen::new(7);
        let (p, target, clips) = base(&mut ids);
        let mut server = Server::new(p.clone());
        let mut a = Peer {
            project: p.clone(),
            client: Client {
                id: 1,
                ..Client::new()
            },
        };
        // While A is offline, someone else trims clip 0 on the server…
        server
            .submit(
                9,
                1,
                Command::TrimTail {
                    target,
                    clip: clips[0],
                    delta: sec(-5),
                },
            )
            .unwrap();
        // …and A trims clip 2 locally.
        let (_, _) = a.edit(Command::TrimTail {
            target,
            clip: clips[2],
            delta: sec(-3),
        });
        // Reconnect: snapshot + replay keeps A's edit pending and on top.
        let events = a
            .client
            .rejoin(&mut a.project, 4, server.version(), server.project.clone());
        assert!(events.is_empty());
        let out = a.client.outgoing();
        assert_eq!(out.len(), 1);
        let e = server.submit(4, out[0].0, out[0].1.clone()).unwrap();
        a.client.receive(&mut a.project, &e).unwrap();
        assert_eq!(a.client.pending(), 0);
        assert_eq!(json(&a.project), json(&server.project));
        let clips_now: Vec<Rational> = server.project.sequences[0].tracks[0]
            .clips
            .iter()
            .map(|c| c.duration)
            .collect();
        assert_eq!(clips_now, vec![sec(5), sec(10), sec(7)]);
    }
}
