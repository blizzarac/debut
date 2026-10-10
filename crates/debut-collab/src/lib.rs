//! Collaboration (COL-01 .. COL-08). Built on the command log: peers exchange
//! commands, not project files. A server orders every edit and keeps the
//! authoritative project; clients apply their own edits at once and rebase
//! them over everyone else's, so nobody waits on the network to edit.

pub mod access; // COL-08 roles; COL-03/04 reviewers comment through markers
pub mod auth; // NFR-13 invite codes, challenge-response
pub mod locks; // COL-01 track locks
pub mod presence; // COL-02 who is where
pub mod protocol; // wire messages (JSON lines)
pub mod review; // COL-03, COL-04 review comments are markers
pub mod sync; // COL-02, COL-06 op ordering, rebase, offline reconciliation

pub use access::Role;
pub use locks::Locks;
pub use presence::Presence;
pub use protocol::Message;
pub use sync::{Client, LogEntry, Server};
