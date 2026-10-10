//! The collaboration server (COL-02): accepts editors over TCP, gives every
//! accepted edit its place in one order, broadcasts it, and relays presence
//! and locks. Messages are `debut_collab::Message`s, one JSON object per line.
//!
//! Each connection gets a reader thread and a writer fed by a channel, so a
//! slow client never holds up the others.

use debut_collab::{access, Locks, Message, Presence, Role, Server};
use debut_project::Project;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
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

fn serve_client(hub: Arc<Mutex<Hub>>, stream: TcpStream) {
    let Ok(mut write) = stream.try_clone() else {
        return;
    };
    let mut lines = BufReader::new(stream).lines();
    // The first message must be Hello.
    let Some(Ok(first)) = lines.next() else {
        return;
    };
    let Some(Message::Hello { name, role }) = Message::from_line(&first) else {
        return;
    };
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
    for line in lines {
        let Ok(line) = line else { break };
        if let Some(msg) = Message::from_line(&line) {
            hub.lock().unwrap().handle(id, msg);
        }
    }
    hub.lock().unwrap().leave(id);
}

/// Serve `project` on `listener` in the background. `on_change` is called
/// with the project after every accepted edit (to save it).
pub fn serve(
    listener: TcpListener,
    project: Project,
    on_change: Option<OnChange>,
) -> std::io::Result<Handle> {
    let addr = listener.local_addr()?;
    let hub = Arc::new(Mutex::new(Hub {
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
