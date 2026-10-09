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
}
