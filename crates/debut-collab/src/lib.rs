//! Collaboration (COL-01 .. COL-08). Built on the command log: peers exchange
//! commands, not project files, and reconcile on reconnect.

pub mod sync;      // COL-02, COL-06 op exchange and conflict resolution
pub mod locks;     // COL-01 bin / timeline locks
pub mod presence;  // COL-02
pub mod review;    // COL-03, COL-04 comments <-> markers
pub mod access;    // COL-08 roles
