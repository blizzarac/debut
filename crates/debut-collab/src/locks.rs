//! Track locks (COL-01): a collaborator can lock tracks so nobody else's
//! edits touch them while they work. Which tracks a command touches is read
//! from its `Target`s (sequence + track) wherever they sit in the command.

use debut_command::Command;
use std::collections::{BTreeSet, HashMap};

/// Track ids (as decimal strings) a command changes: every `Target`
/// (`{"sequence":…,"track":…}`) in its JSON. The text is scanned rather than
/// parsed into a `serde_json::Value`, which cannot hold 128-bit ids.
pub fn tracks_touched(cmd: &Command) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(text) = serde_json::to_string(cmd) else {
        return out;
    };
    let needle = "\"track\":";
    let mut rest = text.as_str();
    while let Some(i) = rest.find(needle) {
        let before = &rest[..i];
        rest = &rest[i + needle.len()..];
        // Only a Target's track: the field right after its sequence.
        if !before
            .trim_end_matches(',')
            .rsplit(['{', ','])
            .next()
            .unwrap_or("")
            .starts_with("\"sequence\":")
        {
            continue;
        }
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            out.insert(digits);
        }
    }
    out
}

/// Who holds which track.
#[derive(Clone, Debug, Default)]
pub struct Locks {
    owner: HashMap<String, u64>,
}

impl Locks {
    /// Lock `track` for `client`; false when someone else holds it.
    pub fn lock(&mut self, track: &str, client: u64) -> bool {
        match self.owner.get(track) {
            Some(o) if *o != client => false,
            _ => {
                self.owner.insert(track.to_string(), client);
                true
            }
        }
    }

    pub fn unlock(&mut self, track: &str, client: u64) {
        if self.owner.get(track) == Some(&client) {
            self.owner.remove(track);
        }
    }

    /// Drop every lock a departed client held.
    pub fn release_all(&mut self, client: u64) {
        self.owner.retain(|_, o| *o != client);
    }

    /// The first track `cmd` touches that someone other than `client` holds.
    pub fn blocked(&self, cmd: &Command, client: u64) -> Option<String> {
        tracks_touched(cmd)
            .into_iter()
            .find(|t| self.owner.get(t).is_some_and(|o| *o != client))
    }

    pub fn all(&self) -> Vec<(String, u64)> {
        let mut v: Vec<_> = self.owner.iter().map(|(t, o)| (t.clone(), *o)).collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_command::Target;
    use debut_core::{ClipId, Rational, SequenceId, TrackId};

    #[test]
    fn finds_targets_and_enforces_locks() {
        let t = |n: u128| Target {
            sequence: SequenceId(u128::MAX),
            track: TrackId(n),
        };
        let trim = |track: u128| Command::TrimTail {
            target: t(track),
            clip: ClipId(5),
            delta: Rational::ONE,
        };
        let big = u128::MAX - 7;
        let group = Command::Group(vec![trim(big), trim(3)]);
        assert_eq!(
            tracks_touched(&group).into_iter().collect::<Vec<_>>(),
            vec!["3".to_string(), big.to_string()]
        );
        let mut locks = Locks::default();
        assert!(locks.lock(&big.to_string(), 1));
        assert!(!locks.lock(&big.to_string(), 2), "held by 1");
        assert_eq!(locks.blocked(&group, 2), Some(big.to_string()));
        assert_eq!(locks.blocked(&group, 1), None);
        locks.release_all(1);
        assert_eq!(locks.blocked(&group, 2), None);
    }
}
