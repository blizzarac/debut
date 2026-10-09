//! Command log and undo (TL-11, NFR-05, COL-05, COL-06, PLT-03).
//!
//! All project mutation goes through [`Command`]s appended to a log. Undo pops the
//! log; the log is persisted continuously, so a crash loses at most the last few
//! seconds (NFR-05) and collaboration syncs operations, not files (COL).

pub mod command;
pub mod history;
pub mod journal;

pub use command::Command;
pub use history::History;
