//! The collaboration server (COL-02): accepts editors over TCP, gives every
//! accepted edit its place in one order, broadcasts it, and relays presence
//! and locks. Messages are `debut_collab::Message`s, one JSON object per line.
//!
//! Each connection gets a reader thread and a writer fed by a channel, so a
//! slow client never holds up the others.
//!
//! Joining takes an invite code (NFR-13): the server sends a random nonce,
//! the client answers with HMAC(code, nonce), and the role is the one whose
//! code matches. Lines from clients are capped, so a peer cannot make the
//! server buffer without limit.

use debut_collab::auth::Invite;
use debut_collab::{access, Locks, Message, Presence, Role, Server};
use debut_project::Project;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

/// Called with the project after every accepted edit (to save it).
pub type OnChange = Box<dyn Fn(&Project) + Send>;

struct Peer {
    tx: Sender<String>,
    role: Role,
    presence: Presence,
}

struct Hub {
    invite: Invite,
    server: Server,
    locks: Locks,
    peers: HashMap<u64, Peer>,
    next: u64,
    on_change: Option<OnChange>,
}

impl Hub {
    fn send(&self, to: u64, msg: &Message) {
        if let Some(p) = self.peers.get(&to) {
            let _ = p.tx.send(msg.to_line());
        }
    }

    fn broadcast(&self, msg: &Message, except: Option<u64>) {
        let line = msg.to_line();
        for (id, p) in &self.peers {
            if Some(*id) != except {
                let _ = p.tx.send(line.clone());
            }
        }
    }

    fn handle(&mut self, from: u64, msg: Message) {
        match msg {
            Message::Submit { id, cmd, .. } => {
                let role = self
                    .peers
                    .get(&from)
                    .map(|p| p.role)
                    .unwrap_or(Role::Reviewer);
                let refused = if !access::allowed(role, &cmd) {
                    Some("reviewers can only add or change comments (markers)".to_string())
                } else {
                    self.locks
                        .blocked(&cmd, from)
                        .map(|t| format!("track {t} is locked by someone else"))
                };
                if let Some(reason) = refused {
                    self.send(from, &Message::Reject { id, reason });
                    return;
                }
                match self.server.submit(from, id, cmd) {
                    Ok(entry) => {
                        self.broadcast(&Message::Op(entry), None);
                        if let Some(f) = &self.on_change {
                            f(&self.server.project);
                        }
                    }
                    Err(e) => self.send(
                        from,
                        &Message::Reject {
                            id,
                            reason: e.to_string(),
                        },
                    ),
                }
            }
            Message::Presence(mut p) => {
                p.client = from;
                if let Some(peer) = self.peers.get_mut(&from) {
                    p.name = peer.presence.name.clone();
                    p.role = peer.role;
                    peer.presence = p.clone();
                }
                self.broadcast(&Message::Presence(p), Some(from));
            }
            Message::Lock { track, on } => {
                if on {
                    self.locks.lock(&track, from);
                } else {
                    self.locks.unlock(&track, from);
                }
                self.broadcast(
                    &Message::Locks {
                        locks: self.locks.all(),
                    },
                    None,
                );
            }
            _ => {}
        }
    }

    fn leave(&mut self, client: u64) {
        self.peers.remove(&client);
        self.locks.release_all(client);
        self.broadcast(&Message::Left { client }, None);
        self.broadcast(
            &Message::Locks {
                locks: self.locks.all(),
            },
            None,
        );
    }
}

/// A running server.
pub struct Handle {
    pub addr: std::net::SocketAddr,
    hub: Arc<Mutex<Hub>>,
}

impl Handle {
    /// The authoritative project right now.
    pub fn project(&self) -> Project {
        self.hub.lock().unwrap().server.project.clone()
    }

    pub fn version(&self) -> u64 {
        self.hub.lock().unwrap().server.version()
    }
}

/// Longest line a client may send (an edit is far smaller).
pub const MAX_LINE: usize = 1 << 20;

/// Random bytes from the OS.
pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("OS random numbers");
    b
}

/// Fresh invite codes for a session.
pub fn new_invite() -> Invite {
    Invite::from_random(random(), random())
}

/// One line of at most `max` bytes (without the newline); `None` at end of
/// stream, on an error or when the line is longer.
fn read_line(reader: &mut impl BufRead, max: usize) -> Option<String> {
    let mut buf = Vec::new();
    let n = reader
        .by_ref()
        .take(max as u64 + 1)
        .read_until(b'\n', &mut buf)
        .ok()?;
    if n == 0 || (buf.last() != Some(&b'\n') && buf.len() > max) {
        return None;
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    String::from_utf8(buf).ok()
}

fn serve_client(hub: Arc<Mutex<Hub>>, stream: TcpStream) {
    let Ok(mut write) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    // Challenge, then the first message must be a Hello that answers it.
    let nonce: String = random::<16>().iter().map(|b| format!("{b:02x}")).collect();
    if write
        .write_all(
            Message::Challenge {
                nonce: nonce.clone(),
            }
            .to_line()
            .as_bytes(),
        )
        .is_err()
    {
        return;
    }
    let Some(first) = read_line(&mut reader, MAX_LINE) else {
        return;
    };
    let Some(Message::Hello { name, proof }) = Message::from_line(&first) else {
        return;
    };
    let role = hub.lock().unwrap().invite.role_for(&nonce, &proof);
    let Some(role) = role else {
        let denied = Message::Denied {
            reason: "the invite code is not valid for this session".into(),
        };
        let _ = write.write_all(denied.to_line().as_bytes());
        return;
    };
    let name: String = name.chars().take(64).collect();
    let (tx, rx) = channel::<String>();
    std::thread::spawn(move || {
        for line in rx {
            if write.write_all(line.as_bytes()).is_err() {
                break;
            }
        }
    });
    let id = {
        let mut h = hub.lock().unwrap();
        h.next += 1;
        let id = h.next;
        let presence = Presence {
            client: id,
            name,
            role,
            sequence: None,
            playhead: 0.0,
            clip: None,
        };
        let welcome = Message::Welcome {
            client: id,
            role,
            version: h.server.version(),
            project: debut_project::schema::to_json(&h.server.project).unwrap_or_default(),
            locks: h.locks.all(),
            peers: h.peers.values().map(|p| p.presence.clone()).collect(),
        };
        let _ = tx.send(welcome.to_line());
        h.broadcast(&Message::Presence(presence.clone()), None);
        h.peers.insert(id, Peer { tx, role, presence });
        id
    };
    while let Some(line) = read_line(&mut reader, MAX_LINE) {
        if let Some(msg) = Message::from_line(&line) {
            hub.lock().unwrap().handle(id, msg);
        }
    }
    hub.lock().unwrap().leave(id);
}

/// Serve `project` on `listener` in the background to holders of `invite`'s
/// codes. `on_change` is called with the project after every accepted edit
/// (to save it).
pub fn serve(
    listener: TcpListener,
    project: Project,
    on_change: Option<OnChange>,
    invite: Invite,
) -> std::io::Result<Handle> {
    let addr = listener.local_addr()?;
    let hub = Arc::new(Mutex::new(Hub {
        invite,
        server: Server::new(project),
        locks: Locks::default(),
        peers: HashMap::new(),
        next: 0,
        on_change,
    }));
    let h2 = Arc::clone(&hub);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let hub = Arc::clone(&h2);
            std::thread::spawn(move || serve_client(hub, stream));
        }
    });
    Ok(Handle { addr, hub })
}
