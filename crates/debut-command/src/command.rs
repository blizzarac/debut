//! Edit primitives and their inverses.
//!
//! Four track-level primitives cover the TL-03 core edits:
//! - [`Command::Replace`]: clear a range (trimming or splitting what straddles it)
//!   and place clips. Overwrite and lift are this.
//! - [`Command::Shift`]: move every clip at or after a point by a delta. Ripple.
//! - [`Command::Blade`] / [`Command::Join`]: cut a clip or re-merge a continuous pair.
//! - [`Command::Group`]: several primitives as one undo step.
//!
//! Insert and extract are groups built by the constructors at the bottom.

use debut_core::{ClipId, Error, IdGen, Rational, Result, SequenceId, TrackId};
use debut_project::{Clip, Project, Track};
use serde::{Deserialize, Serialize};

/// Which track a primitive operates on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub sequence: SequenceId,
    pub track: TrackId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Clear `[start, end)` plus the spans of `clips`, then place `clips`.
    /// `split_id` names the tail if a single clip has to be split around the range.
    Replace {
        target: Target,
        start: Rational,
        end: Rational,
        clips: Vec<Clip>,
        split_id: ClipId,
    },
    /// Move every clip whose `timeline_in >= from` by `by`. The caller guarantees no
    /// clip straddles `from` and that a negative shift creates no overlap.
    Shift {
        target: Target,
        from: Rational,
        by: Rational,
    },
    /// Split the clip strictly containing `at`; the tail gets `tail_id`. No-op if
    /// no clip contains `at`.
    Blade {
        target: Target,
        at: Rational,
        tail_id: ClipId,
    },
    /// Merge the two clips meeting at `at` if the second continues the first. No-op
    /// otherwise. The first clip's ID survives.
    Join { target: Target, at: Rational },
    /// Move a clip's head by `delta` keeping its tail and source in sync
    /// (positive shortens). The clip must stay non-empty and non-overlapping.
    TrimHead {
        target: Target,
        clip: ClipId,
        delta: Rational,
    },
    /// Move a clip's tail by `delta` (positive lengthens).
    TrimTail {
        target: Target,
        clip: ClipId,
        delta: Rational,
    },
    /// Change which part of the source plays without moving the clip (TL-04 slip).
    Slip {
        target: Target,
        clip: ClipId,
        delta: Rational,
    },
    /// Move a clip along the timeline by `delta`; nothing else moves.
    Move {
        target: Target,
        clip: ClipId,
        delta: Rational,
    },
    /// One undo step made of several commands, applied in order.
    Group(Vec<Command>),
    /// Inverse of a primitive that did nothing.
    Noop,
}

fn track_mut(project: &mut Project, t: Target) -> Result<&mut Track> {
    project
        .sequence_mut(t.sequence)
        .ok_or_else(|| Error::NotFound(format!("sequence {:?}", t.sequence)))?
        .track_mut(t.track)
        .ok_or_else(|| Error::NotFound(format!("track {:?}", t.track)))
}

fn track(project: &Project, t: Target) -> Result<&Track> {
    project
        .sequence(t.sequence)
        .ok_or_else(|| Error::NotFound(format!("sequence {:?}", t.sequence)))?
        .track(t.track)
        .ok_or_else(|| Error::NotFound(format!("track {:?}", t.track)))
}

/// `[start, end)` widened to cover every clip in `clips`.
fn span_with(start: Rational, end: Rational, clips: &[Clip]) -> (Rational, Rational) {
    clips.iter().fold((start, end), |(s, e), c| {
        (s.min(c.timeline_in), e.max(c.timeline_out()))
    })
}

/// Remove everything inside `[start, end)` from `track`, trimming clips that cross
/// an edge and splitting a clip that contains the whole range.
fn clear_range(track: &mut Track, start: Rational, end: Rational, split_id: ClipId) {
    if end <= start {
        return;
    }
    let mut kept = Vec::with_capacity(track.clips.len() + 1);
    for mut c in track.clips.drain(..) {
        let (cin, cout) = (c.timeline_in, c.timeline_out());
        if cout <= start || cin >= end {
            kept.push(c);
        } else if cin < start && cout > end {
            let mut tail = c.split_at(end, split_id);
            c.trim_tail_to(start);
            tail.trim_head_to(end);
            kept.push(c);
            kept.push(tail);
        } else if cin < start {
            c.trim_tail_to(start);
            kept.push(c);
        } else if cout > end {
            c.trim_head_to(end);
            kept.push(c);
        }
        // else: fully inside, dropped
    }
    track.clips = kept;
}

impl Command {
    pub fn apply(&self, project: &mut Project) -> Result<()> {
        match self {
            Command::Replace {
                target,
                start,
                end,
                clips,
                split_id,
            } => {
                let tr = track_mut(project, *target)?;
                let (s, e) = span_with(*start, *end, clips);
                let mut next = tr.clone();
                clear_range(&mut next, s, e, *split_id);
                next.clips.extend(clips.iter().cloned());
                next.sort();
                if !next.is_consistent() || next.clips.iter().any(|c| c.timeline_in.is_negative()) {
                    return Err(Error::InvalidArgument(
                        "replace produced an invalid layout".into(),
                    ));
                }
                tr.clips = next.clips;
                Ok(())
            }
            Command::Shift { target, from, by } => {
                let tr = track_mut(project, *target)?;
                if tr
                    .clips
                    .iter()
                    .any(|c| c.timeline_in < *from && c.timeline_out() > *from)
                {
                    return Err(Error::InvalidArgument(
                        "shift point lies inside a clip".into(),
                    ));
                }
                let mut next = tr.clone();
                for c in next.clips.iter_mut().filter(|c| c.timeline_in >= *from) {
                    c.timeline_in += *by;
                }
                if !next.is_consistent() || next.clips.iter().any(|c| c.timeline_in.is_negative()) {
                    return Err(Error::InvalidArgument(
                        "shift produced an invalid layout".into(),
                    ));
                }
                tr.clips = next.clips;
                Ok(())
            }
            Command::Blade {
                target,
                at,
                tail_id,
            } => {
                let tr = track_mut(project, *target)?;
                if let Some(i) = tr
                    .clips
                    .iter()
                    .position(|c| c.timeline_in < *at && *at < c.timeline_out())
                {
                    let tail = tr.clips[i].split_at(*at, *tail_id);
                    tr.clips.insert(i + 1, tail);
                }
                Ok(())
            }
            Command::Join { target, at } => {
                let tr = track_mut(project, *target)?;
                if let Some(i) = joinable_at(tr, *at) {
                    let tail = tr.clips.remove(i + 1);
                    tr.clips[i].duration += tail.duration;
                }
                Ok(())
            }
            Command::TrimHead {
                target,
                clip,
                delta,
            } => edit_clip(project, *target, *clip, |c| {
                let t = c.timeline_in + *delta;
                if t >= c.timeline_out() {
                    return Err(Error::InvalidArgument("trim would empty the clip".into()));
                }
                c.source_in = c.source_at(t);
                c.duration = c.timeline_out() - t;
                c.timeline_in = t;
                Ok(())
            }),
            Command::TrimTail {
                target,
                clip,
                delta,
            } => edit_clip(project, *target, *clip, |c| {
                if c.duration + *delta <= Rational::ZERO {
                    return Err(Error::InvalidArgument("trim would empty the clip".into()));
                }
                c.duration += *delta;
                Ok(())
            }),
            Command::Slip {
                target,
                clip,
                delta,
            } => edit_clip(project, *target, *clip, |c| {
                c.source_in += *delta;
                Ok(())
            }),
            Command::Move {
                target,
                clip,
                delta,
            } => edit_clip(project, *target, *clip, |c| {
                c.timeline_in += *delta;
                Ok(())
            }),
            Command::Group(cmds) => {
                for c in cmds {
                    c.apply(project)?;
                }
                Ok(())
            }
            Command::Noop => Ok(()),
        }
    }

    /// The command that undoes `self`, computed against `project` *before* `self`
    /// is applied.
    pub fn invert(&self, project: &Project) -> Result<Command> {
        match self {
            Command::Replace {
                target,
                start,
                end,
                clips,
                split_id,
            } => {
                let tr = track(project, *target)?;
                let (s, e) = span_with(*start, *end, clips);
                let originals: Vec<Clip> = tr
                    .clips
                    .iter()
                    .filter(|c| c.timeline_in < e && c.timeline_out() > s)
                    .cloned()
                    .collect();
                Ok(Command::Replace {
                    target: *target,
                    start: s,
                    end: e,
                    clips: originals,
                    split_id: *split_id,
                })
            }
            Command::Shift { target, from, by } => Ok(Command::Shift {
                target: *target,
                from: *from + *by,
                by: -*by,
            }),
            Command::Blade { target, at, .. } => {
                let tr = track(project, *target)?;
                let acts = tr
                    .clips
                    .iter()
                    .any(|c| c.timeline_in < *at && *at < c.timeline_out());
                Ok(if acts {
                    Command::Join {
                        target: *target,
                        at: *at,
                    }
                } else {
                    Command::Noop
                })
            }
            Command::Join { target, at } => {
                let tr = track(project, *target)?;
                Ok(match joinable_at(tr, *at) {
                    Some(i) => Command::Blade {
                        target: *target,
                        at: *at,
                        tail_id: tr.clips[i + 1].id,
                    },
                    None => Command::Noop,
                })
            }
            Command::TrimHead {
                target,
                clip,
                delta,
            } => Ok(Command::TrimHead {
                target: *target,
                clip: *clip,
                delta: -*delta,
            }),
            Command::TrimTail {
                target,
                clip,
                delta,
            } => Ok(Command::TrimTail {
                target: *target,
                clip: *clip,
                delta: -*delta,
            }),
            Command::Slip {
                target,
                clip,
                delta,
            } => Ok(Command::Slip {
                target: *target,
                clip: *clip,
                delta: -*delta,
            }),
            Command::Move {
                target,
                clip,
                delta,
            } => Ok(Command::Move {
                target: *target,
                clip: *clip,
                delta: -*delta,
            }),
            Command::Group(cmds) => {
                let mut scratch = project.clone();
                let mut inverses = Vec::with_capacity(cmds.len());
                for c in cmds {
                    inverses.push(c.invert(&scratch)?);
                    c.apply(&mut scratch)?;
                }
                inverses.reverse();
                Ok(Command::Group(inverses))
            }
            Command::Noop => Ok(Command::Noop),
        }
    }

    // ---- TL-03 constructors -------------------------------------------------

    /// Overwrite: place `clip`, replacing whatever it covers. Nothing moves.
    pub fn overwrite(target: Target, clip: Clip, ids: &mut IdGen) -> Command {
        Command::Replace {
            target,
            start: clip.timeline_in,
            end: clip.timeline_out(),
            clips: vec![clip],
            split_id: ids.fresh(),
        }
    }

    /// Lift: clear `[start, end)`, leaving a gap.
    pub fn lift(target: Target, start: Rational, end: Rational, ids: &mut IdGen) -> Command {
        Command::Replace {
            target,
            start,
            end,
            clips: Vec::new(),
            split_id: ids.fresh(),
        }
    }

    /// Insert: open a gap at `at` long enough for `clips` (which are placed starting
    /// at `at`, in order) and ripple everything after it to the right.
    pub fn insert(target: Target, at: Rational, mut clips: Vec<Clip>, ids: &mut IdGen) -> Command {
        let mut cursor = at;
        for c in &mut clips {
            c.timeline_in = cursor;
            cursor += c.duration;
        }
        let len = cursor - at;
        Command::Group(vec![
            Command::Blade {
                target,
                at,
                tail_id: ids.fresh(),
            },
            Command::Shift {
                target,
                from: at,
                by: len,
            },
            Command::Replace {
                target,
                start: at,
                end: cursor,
                clips,
                split_id: ids.fresh(),
            },
        ])
    }

    /// Extract: remove `[start, end)` and ripple everything after it to the left.
    pub fn extract(target: Target, start: Rational, end: Rational, ids: &mut IdGen) -> Command {
        Command::Group(vec![
            Command::Blade {
                target,
                at: start,
                tail_id: ids.fresh(),
            },
            Command::Blade {
                target,
                at: end,
                tail_id: ids.fresh(),
            },
            Command::Replace {
                target,
                start,
                end,
                clips: Vec::new(),
                split_id: ids.fresh(),
            },
            Command::Shift {
                target,
                from: end,
                by: start - end,
            },
        ])
    }
}

/// Apply `f` to one clip, then re-check the track's layout invariants.
fn edit_clip(
    project: &mut Project,
    target: Target,
    clip: ClipId,
    f: impl FnOnce(&mut Clip) -> Result<()>,
) -> Result<()> {
    let tr = track_mut(project, target)?;
    let i = tr
        .clip_index(clip)
        .ok_or_else(|| Error::NotFound(format!("clip {clip:?}")))?;
    let mut next = tr.clone();
    f(&mut next.clips[i])?;
    if next.clips[i].timeline_in.is_negative() {
        return Err(Error::InvalidArgument(
            "clip would start before zero".into(),
        ));
    }
    next.sort();
    if !next.is_consistent() {
        return Err(Error::InvalidArgument(
            "edit produced overlapping clips".into(),
        ));
    }
    tr.clips = next.clips;
    Ok(())
}

/// Index of the first clip of a joinable pair meeting exactly at `at`.
fn joinable_at(track: &Track, at: Rational) -> Option<usize> {
    track
        .clips
        .windows(2)
        .position(|w| w[0].timeline_out() == at && w[0].is_continuous_with(&w[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, MediaId};
    use debut_project::{ClipSource, Sequence, TrackKind};

    struct Fx {
        project: Project,
        target: Target,
        ids: IdGen,
        media: MediaId,
    }

    fn sec(n: i64) -> Rational {
        Rational::from_int(n)
    }

    /// One video track with clips A[0,10) B[10,20) C[25,30), each starting at source 0.
    fn fixture() -> Fx {
        let mut ids = IdGen::new(42);
        let mut project = Project::new(ids.fresh(), "fx");
        let mut seq = Sequence::new(ids.fresh(), "seq", FrameRate::FPS_25, 1920, 1080);
        let mut track = Track::new(ids.fresh(), TrackKind::Video);
        let media = ids.fresh();
        for (i, d) in [(0, 10), (10, 10), (25, 5)] {
            track.clips.push(Clip::new(
                ids.fresh(),
                ClipSource::Media(media),
                sec(i),
                sec(d),
                Rational::ZERO,
            ));
        }
        let target = Target {
            sequence: seq.id,
            track: track.id,
        };
        seq.tracks.push(track);
        project.sequences.push(seq);
        Fx {
            project,
            target,
            ids,
            media,
        }
    }

    impl Fx {
        fn clip(&mut self, at: i64, dur: i64, src: i64) -> Clip {
            Clip::new(
                self.ids.fresh(),
                ClipSource::Media(self.media),
                sec(at),
                sec(dur),
                sec(src),
            )
        }
        fn layout(&self) -> Vec<(i64, i64, i64)> {
            track(&self.project, self.target)
                .unwrap()
                .clips
                .iter()
                .map(|c| (c.timeline_in.num, c.timeline_out().num, c.source_in.num))
                .collect()
        }
        /// Apply, then undo via the computed inverse; assert the project is restored.
        fn apply_and_undo(&mut self, cmd: &Command, expected_after: &[(i64, i64, i64)]) {
            let before = self.project.clone();
            let inv = cmd.invert(&self.project).unwrap();
            cmd.apply(&mut self.project).unwrap();
            assert_eq!(self.layout(), expected_after, "layout after apply");
            assert!(track(&self.project, self.target).unwrap().is_consistent());
            inv.apply(&mut self.project).unwrap();
            assert_eq!(self.project, before, "project after undo");
        }
    }

    #[test]
    fn overwrite_in_the_middle_splits_the_clip() {
        let mut fx = fixture();
        let new = fx.clip(3, 4, 100);
        let cmd = Command::overwrite(fx.target, new, &mut fx.ids);
        fx.apply_and_undo(
            &cmd,
            &[(0, 3, 0), (3, 7, 100), (7, 10, 7), (10, 20, 0), (25, 30, 0)],
        );
    }

    #[test]
    fn overwrite_across_a_cut_trims_both_sides() {
        let mut fx = fixture();
        let new = fx.clip(8, 4, 0);
        let cmd = Command::overwrite(fx.target, new, &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 8, 0), (8, 12, 0), (12, 20, 2), (25, 30, 0)]);
    }

    #[test]
    fn overwrite_can_swallow_whole_clips_and_gaps() {
        let mut fx = fixture();
        let new = fx.clip(5, 25, 0);
        let cmd = Command::overwrite(fx.target, new, &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 5, 0), (5, 30, 0)]);
    }

    #[test]
    fn lift_leaves_a_gap() {
        let mut fx = fixture();
        let cmd = Command::lift(fx.target, sec(5), sec(15), &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 5, 0), (15, 20, 5), (25, 30, 0)]);
    }

    #[test]
    fn insert_at_a_cut_ripples_later_clips() {
        let mut fx = fixture();
        let new = fx.clip(0, 3, 50);
        let cmd = Command::insert(fx.target, sec(10), vec![new], &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 10, 0), (10, 13, 50), (13, 23, 0), (28, 33, 0)]);
    }

    #[test]
    fn insert_inside_a_clip_splits_and_ripples() {
        let mut fx = fixture();
        let new = fx.clip(0, 2, 50);
        let cmd = Command::insert(fx.target, sec(4), vec![new], &mut fx.ids);
        fx.apply_and_undo(
            &cmd,
            &[(0, 4, 0), (4, 6, 50), (6, 12, 4), (12, 22, 0), (27, 32, 0)],
        );
    }

    #[test]
    fn extract_closes_the_gap() {
        let mut fx = fixture();
        let cmd = Command::extract(fx.target, sec(5), sec(15), &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 5, 0), (5, 10, 5), (15, 20, 0)]);
    }

    #[test]
    fn extract_across_a_gap_pulls_in_the_tail() {
        let mut fx = fixture();
        let cmd = Command::extract(fx.target, sec(18), sec(27), &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 10, 0), (10, 18, 0), (18, 21, 2)]);
    }

    #[test]
    fn blade_then_join_restores_the_clip_id() {
        let mut fx = fixture();
        let original = track(&fx.project, fx.target).unwrap().clips[0].clone();
        let tail_id: ClipId = fx.ids.fresh();
        let blade = Command::Blade {
            target: fx.target,
            at: sec(4),
            tail_id,
        };
        fx.apply_and_undo(&blade, &[(0, 4, 0), (4, 10, 4), (10, 20, 0), (25, 30, 0)]);
        assert_eq!(track(&fx.project, fx.target).unwrap().clips[0], original);
    }

    #[test]
    fn join_does_not_merge_a_pre_existing_discontinuity() {
        // A[0,10) and B[10,20) both start at source 0, so B does not continue A.
        let mut fx = fixture();
        let join = Command::Join {
            target: fx.target,
            at: sec(10),
        };
        assert_eq!(join.invert(&fx.project).unwrap(), Command::Noop);
        fx.apply_and_undo(&join, &[(0, 10, 0), (10, 20, 0), (25, 30, 0)]);
    }

    #[test]
    fn blade_at_an_existing_cut_is_a_noop_whose_inverse_does_not_join() {
        let mut fx = fixture();
        // Make B a true continuation of A, then cut at 10 (already a cut).
        track_mut(&mut fx.project, fx.target).unwrap().clips[1].source_in = sec(10);
        let blade = Command::Blade {
            target: fx.target,
            at: sec(10),
            tail_id: fx.ids.fresh(),
        };
        assert_eq!(blade.invert(&fx.project).unwrap(), Command::Noop);
    }

    #[test]
    fn a_rejected_apply_leaves_the_project_untouched() {
        let mut fx = fixture();
        let before = fx.project.clone();
        let b = track(&fx.project, fx.target).unwrap().clips[1].id;
        assert!(Command::Move {
            target: fx.target,
            clip: b,
            delta: sec(6)
        }
        .apply(&mut fx.project)
        .is_err());
        assert!(Command::Shift {
            target: fx.target,
            from: sec(25),
            by: sec(-20)
        }
        .apply(&mut fx.project)
        .is_err());
        let big = fx.clip(-5, 3, 0);
        assert!(Command::overwrite(fx.target, big, &mut fx.ids)
            .apply(&mut fx.project)
            .is_err());
        assert_eq!(fx.project, before);
    }

    #[test]
    fn commands_round_trip_through_json() {
        let mut fx = fixture();
        let new = fx.clip(0, 2, 50);
        let cmd = Command::insert(fx.target, sec(4), vec![new], &mut fx.ids);
        let json = serde_json::to_string(&cmd).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, back);
    }
}
