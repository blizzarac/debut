//! Append-only journal of commands. Replayed on crash recovery (NFR-05) and shipped
//! as the sync unit for collaboration (COL-05, COL-06). The on-disk form is one JSON
//! document per line, so a truncated tail after a crash loses only the last entry.

use crate::Command;
use debut_core::{Error, Result};

pub trait Journal {
    fn append(&mut self, cmd: &Command) -> Result<()>;
    fn replay(&self) -> Result<Vec<Command>>;
}

/// In-memory journal: tests, and the staging buffer in front of a file-backed one.
#[derive(Default)]
pub struct MemoryJournal {
    entries: Vec<Command>,
}

impl MemoryJournal {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Journal for MemoryJournal {
    fn append(&mut self, cmd: &Command) -> Result<()> {
        self.entries.push(cmd.clone());
        Ok(())
    }

    fn replay(&self) -> Result<Vec<Command>> {
        Ok(self.entries.clone())
    }
}

/// Encode one command as a single JSON line (no embedded newlines).
pub fn encode_line(cmd: &Command) -> Result<String> {
    let mut s = serde_json::to_string(cmd).map_err(|e| Error::Other(e.to_string()))?;
    s.push('\n');
    Ok(s)
}

/// Decode a journal file. A malformed final line (torn write) is dropped; a
/// malformed line anywhere else is an error.
pub fn decode_lines(text: &str) -> Result<Vec<Command>> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        match serde_json::from_str::<Command>(line) {
            Ok(c) => out.push(c),
            Err(_) if i + 1 == lines.len() => break,
            Err(e) => return Err(Error::Other(format!("journal line {}: {e}", i + 1))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torn_last_line_is_dropped_but_earlier_corruption_is_an_error() {
        let a = encode_line(&Command::Noop).unwrap();
        let text = format!("{a}{a}{{\"Noo");
        assert_eq!(decode_lines(&text).unwrap().len(), 2);
        let text = format!("{a}{{\"Noo\n{a}");
        assert!(decode_lines(&text).is_err());
    }
}
