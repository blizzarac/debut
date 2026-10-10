//! What travels between clients and the server: one JSON object per line.

use crate::{LogEntry, Presence, Role};
use debut_command::Command;
use serde::{Deserialize, Serialize};

/// Externally tagged (`{"submit": {...}}`): an internally tagged enum would
/// buffer its fields through serde's `Content`, which cannot hold the
/// 128-bit ids commands carry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Message {
    /// Server → client, first: answer with HMAC(invite code, nonce).
    Challenge { nonce: String },
    /// Client → server: join with a name and the answer to the challenge;
    /// the role follows from which invite code produced it (NFR-13).
    Hello { name: String, proof: String },
    /// Server → client: the answer matched no invite code; the connection
    /// closes.
    Denied { reason: String },
    /// Server → client: your id and role, the project at `version` (JSON)
    /// and the current locks and peers.
    Welcome {
        client: u64,
        role: Role,
        version: u64,
        project: String,
        locks: Vec<(String, u64)>,
        peers: Vec<Presence>,
    },
    /// Client → server: an edit made on top of `base`.
    Submit { id: u64, base: u64, cmd: Command },
    /// Server → everyone: an edit in its final order.
    Op(LogEntry),
    /// Server → author: the edit `id` could not be applied.
    Reject { id: u64, reason: String },
    /// Both ways: where a collaborator is.
    Presence(Presence),
    /// Client → server: lock or unlock a track.
    Lock { track: String, on: bool },
    /// Server → everyone: the locks now held.
    Locks { locks: Vec<(String, u64)> },
    /// Server → everyone: a collaborator left.
    Left { client: u64 },
}

impl Message {
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }

    pub fn from_line(line: &str) -> Option<Message> {
        serde_json::from_str(line.trim()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_command::Target;
    use debut_core::{ClipId, Rational, SequenceId, TrackId};

    #[test]
    fn edits_with_128_bit_ids_round_trip() {
        let cmd = Command::TrimTail {
            target: Target {
                sequence: SequenceId(u128::MAX - 1),
                track: TrackId(1 << 100),
            },
            clip: ClipId(u128::MAX),
            delta: Rational::new(-2, 5),
        };
        for msg in [
            Message::Submit {
                id: 7,
                base: 3,
                cmd: cmd.clone(),
            },
            Message::Op(LogEntry {
                version: 4,
                author: 2,
                id: 7,
                cmd,
            }),
            Message::Hello {
                name: "Ana".into(),
                proof: "ab".repeat(32),
            },
            Message::Challenge { nonce: "n".into() },
        ] {
            let line = msg.to_line();
            assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
            assert_eq!(Message::from_line(&line), Some(msg));
        }
    }
}
