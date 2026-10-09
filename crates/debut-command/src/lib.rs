//! Command log and undo (TL-11, NFR-05, COL-05, COL-06, PLT-03).
//!
//! All project mutation goes through [`Command`]s. A command is serializable,
//! applies deterministically (it carries any IDs it creates), and can compute its
//! own inverse against the project state it is about to change. [`History`] pairs
//! each applied command with that inverse for unlimited undo/redo; a [`Journal`]
//! persists the stream, so a crash loses at most the unflushed tail (NFR-05) and
//! collaboration syncs operations, not files (COL).

pub mod command;
pub mod history;
pub mod journal;

pub use command::{Command, MarkerTarget, Target};
pub use history::History;
pub use journal::{Journal, MemoryJournal};
