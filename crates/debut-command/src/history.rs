//! Unlimited undo/redo backed by the command log (TL-11).

use crate::{Command, Journal};
use debut_core::Result;
use debut_project::Project;

struct Entry {
    forward: Command,
    inverse: Command,
}

/// Applied commands with their inverses. Every mutation of a project in the app
/// goes through [`History::execute`].
#[derive(Default)]
pub struct History {
    done: Vec<Entry>,
    undone: Vec<Entry>,
}

impl History {
    /// Apply `cmd`, remember how to undo it, and journal it. Redo history is cleared.
    pub fn execute(
        &mut self,
        project: &mut Project,
        cmd: Command,
        journal: &mut dyn Journal,
    ) -> Result<()> {
        let inverse = cmd.invert(project)?;
        cmd.apply(project)?;
        journal.append(&cmd)?;
        self.done.push(Entry {
            forward: cmd,
            inverse,
        });
        self.undone.clear();
        Ok(())
    }

    /// Undo the last step. Returns `false` if there was nothing to undo.
    pub fn undo(&mut self, project: &mut Project, journal: &mut dyn Journal) -> Result<bool> {
        let Some(entry) = self.done.pop() else {
            return Ok(false);
        };
        entry.inverse.apply(project)?;
        journal.append(&entry.inverse)?;
        self.undone.push(entry);
        Ok(true)
    }

    /// Redo the last undone step. Returns `false` if there was nothing to redo.
    pub fn redo(&mut self, project: &mut Project, journal: &mut dyn Journal) -> Result<bool> {
        let Some(entry) = self.undone.pop() else {
            return Ok(false);
        };
        entry.forward.apply(project)?;
        journal.append(&entry.forward)?;
        self.done.push(entry);
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// What `undo` would apply, and what would redo it (collaboration sends
    /// an undo to others as an ordinary edit).
    pub fn peek_undo(&self) -> Option<(&Command, &Command)> {
        self.done.last().map(|e| (&e.inverse, &e.forward))
    }

    /// What `redo` would apply, and what would undo it.
    pub fn peek_redo(&self) -> Option<(&Command, &Command)> {
        self.undone.last().map(|e| (&e.forward, &e.inverse))
    }

    /// Forget all entries (the project was replaced from elsewhere).
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }

    /// Fold every step done after the first `mark` into one, so a single
    /// undo reverts them all (a script run, NFR-11). The project is not
    /// touched. Returns how many steps were folded.
    pub fn squash_since(&mut self, mark: usize) -> usize {
        if self.done.len() <= mark + 1 {
            return self.done.len().saturating_sub(mark);
        }
        let tail: Vec<Entry> = self.done.drain(mark..).collect();
        let n = tail.len();
        let (forward, mut inverse): (Vec<Command>, Vec<Command>) =
            tail.into_iter().map(|e| (e.forward, e.inverse)).unzip();
        inverse.reverse();
        self.done.push(Entry {
            forward: Command::Group(forward),
            inverse: Command::Group(inverse),
        });
        n
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn len(&self) -> usize {
        self.done.len()
    }

    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryJournal, Target};
    use debut_core::{FrameRate, IdGen, Rational};
    use debut_project::{Clip, ClipSource, Sequence, Track, TrackKind};

    fn setup() -> (Project, Target, IdGen, Clip) {
        let mut ids = IdGen::new(9);
        let mut project = Project::new(ids.fresh(), "h");
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 1920, 1080);
        let track = Track::new(ids.fresh(), TrackKind::Video);
        let target = Target {
            sequence: seq.id,
            track: track.id,
        };
        seq.tracks.push(track);
        project.sequences.push(seq);
        let clip = Clip::new(
            ids.fresh(),
            ClipSource::Media(ids.fresh()),
            Rational::ZERO,
            Rational::from_int(5),
            Rational::ZERO,
        );
        (project, target, ids, clip)
    }

    #[test]
    fn undo_redo_walk_the_log_and_replay_matches() {
        let (mut project, target, mut ids, clip) = setup();
        let empty = project.clone();
        let mut journal = MemoryJournal::default();
        let mut history = History::default();

        let mut second = clip.clone();
        second.id = ids.fresh();
        history
            .execute(
                &mut project,
                Command::insert(target, Rational::ZERO, vec![clip], &mut ids),
                &mut journal,
            )
            .unwrap();
        history
            .execute(
                &mut project,
                Command::insert(target, Rational::from_int(5), vec![second], &mut ids),
                &mut journal,
            )
            .unwrap();
        let two_clips = project.clone();
        assert_eq!(project.sequences[0].tracks[0].clips.len(), 2);

        assert!(history.undo(&mut project, &mut journal).unwrap());
        assert_eq!(project.sequences[0].tracks[0].clips.len(), 1);
        assert!(history.undo(&mut project, &mut journal).unwrap());
        assert_eq!(project, empty);
        assert!(!history.undo(&mut project, &mut journal).unwrap());

        assert!(history.redo(&mut project, &mut journal).unwrap());
        assert!(history.redo(&mut project, &mut journal).unwrap());
        assert_eq!(project, two_clips);
        assert!(!history.can_redo());

        // Replaying the journal from the empty project reproduces the final state
        // (this is what crash recovery and peers do).
        let mut replayed = empty.clone();
        for cmd in journal.replay().unwrap() {
            cmd.apply(&mut replayed).unwrap();
        }
        assert_eq!(replayed, two_clips);
    }

    #[test]
    fn a_new_command_clears_redo() {
        let (mut project, target, mut ids, clip) = setup();
        let mut journal = MemoryJournal::default();
        let mut history = History::default();
        history
            .execute(
                &mut project,
                Command::insert(target, Rational::ZERO, vec![clip.clone()], &mut ids),
                &mut journal,
            )
            .unwrap();
        history.undo(&mut project, &mut journal).unwrap();
        assert!(history.can_redo());
        let mut other = clip;
        other.id = ids.fresh();
        history
            .execute(
                &mut project,
                Command::overwrite(target, other, &mut ids),
                &mut journal,
            )
            .unwrap();
        assert!(!history.can_redo());
    }

    #[test]
    fn squash_folds_steps_into_one_undo() {
        let mut ids = IdGen::new(7);
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 64, 36);
        let track = Track::new(ids.fresh(), TrackKind::Video);
        let target = Target {
            sequence: seq.id,
            track: track.id,
        };
        seq.tracks.push(track);
        let mut project = Project::new(ids.fresh(), "p");
        project.sequences.push(seq);
        let before = project.clone();
        let mut journal = MemoryJournal::default();
        let mut h = History::default();
        let mark = h.len();
        for n in 0..3 {
            let clip = Clip::new(
                ids.fresh(),
                ClipSource::Media(ids.fresh()),
                Rational::from_int(n * 2),
                Rational::from_int(1),
                Rational::ZERO,
            );
            h.execute(
                &mut project,
                Command::overwrite(target, clip, &mut ids),
                &mut journal,
            )
            .unwrap();
        }
        assert_eq!(h.squash_since(mark), 3);
        assert_eq!(h.len(), 1);
        let after = project.clone();
        assert!(h.undo(&mut project, &mut journal).unwrap());
        assert_eq!(project, before);
        assert!(h.redo(&mut project, &mut journal).unwrap());
        assert_eq!(project, after);
        // Nothing or one step: left as is.
        assert_eq!(h.squash_since(h.len()), 0);
    }
}
