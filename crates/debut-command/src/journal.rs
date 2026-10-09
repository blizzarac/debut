//! Append-only on-disk journal of commands; replayed on crash recovery (NFR-05) and
//! shipped as the sync unit for collaboration (COL-05, COL-06).

use crate::Command;
use debut_core::Result;

pub trait Journal {
    fn append(&mut self, cmd: &Command) -> Result<()>;
    fn replay(&self) -> Result<Vec<Command>>;
}
