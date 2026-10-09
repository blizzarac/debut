//! Trim tools (TL-04), each one undo step. `delta` is in timeline seconds and
//! can be negative. Callers clamp to source bounds; these functions only keep the
//! track consistent (no overlaps, no empty clips) and fail otherwise on apply.

use debut_command::{Command, Target};
use debut_core::{ClipId, Error, Rational, Result};
use debut_project::{Project, Track};

fn track(project: &Project, t: Target) -> Result<&Track> {
    project
        .sequence(t.sequence)
        .and_then(|s| s.track(t.track))
        .ok_or_else(|| Error::NotFound(format!("track {:?}", t.track)))
}

/// Ripple-trim the head: positive `delta` removes material from the front and
/// pulls everything after the clip left to close the gap.
pub fn ripple_head(
    project: &Project,
    target: Target,
    clip: ClipId,
    delta: Rational,
) -> Result<Command> {
    let tr = track(project, target)?;
    let c = tr
        .clip(clip)
        .ok_or_else(|| Error::NotFound(format!("clip {clip:?}")))?;
    let out = c.timeline_out();
    let trim = Command::TrimHead {
        target,
        clip,
        delta,
    };
    // Keep the clip where it was: move its start back to the original in-point.
    let keep = Command::Move {
        target,
        clip,
        delta: -delta,
    };
    let shift = Command::Shift {
        target,
        from: out,
        by: -delta,
    };
    Ok(Command::Group(if delta > Rational::ZERO {
        vec![trim, keep, shift]
    } else {
        vec![shift, keep, trim]
    }))
}

/// Ripple-trim the tail: positive `delta` lengthens the clip and pushes everything
/// after it right.
pub fn ripple_tail(
    project: &Project,
    target: Target,
    clip: ClipId,
    delta: Rational,
) -> Result<Command> {
    let tr = track(project, target)?;
    let c = tr
        .clip(clip)
        .ok_or_else(|| Error::NotFound(format!("clip {clip:?}")))?;
    let out = c.timeline_out();
    let trim = Command::TrimTail {
        target,
        clip,
        delta,
    };
    let shift = Command::Shift {
        target,
        from: out,
        by: delta,
    };
    Ok(Command::Group(if delta > Rational::ZERO {
        vec![shift, trim]
    } else {
        vec![trim, shift]
    }))
}

/// Roll the cut between `first` and the clip that starts where it ends: positive
/// `delta` moves the cut later. Total duration is unchanged.
pub fn roll(project: &Project, target: Target, first: ClipId, delta: Rational) -> Result<Command> {
    let tr = track(project, target)?;
    let i = tr
        .clip_index(first)
        .ok_or_else(|| Error::NotFound(format!("clip {first:?}")))?;
    let a = &tr.clips[i];
    let b = tr
        .clips
        .get(i + 1)
        .filter(|b| b.timeline_in == a.timeline_out())
        .ok_or_else(|| Error::InvalidArgument("no adjacent clip to roll against".into()))?;
    let grow_a = Command::TrimTail {
        target,
        clip: a.id,
        delta,
    };
    let trim_b = Command::TrimHead {
        target,
        clip: b.id,
        delta,
    };
    Ok(Command::Group(if delta > Rational::ZERO {
        vec![trim_b, grow_a]
    } else {
        vec![grow_a, trim_b]
    }))
}

/// Slip: play a different part of the source in the same timeline slot.
pub fn slip(target: Target, clip: ClipId, delta: Rational) -> Command {
    Command::Slip {
        target,
        clip,
        delta,
    }
}

/// Slide: move the clip by `delta`; the previous clip's tail and the next clip's
/// head absorb the change so the sequence length is unchanged.
pub fn slide(project: &Project, target: Target, clip: ClipId, delta: Rational) -> Result<Command> {
    let tr = track(project, target)?;
    let i = tr
        .clip_index(clip)
        .ok_or_else(|| Error::NotFound(format!("clip {clip:?}")))?;
    let c = &tr.clips[i];
    let prev = i
        .checked_sub(1)
        .map(|p| &tr.clips[p])
        .filter(|p| p.timeline_out() == c.timeline_in);
    let next = tr
        .clips
        .get(i + 1)
        .filter(|n| n.timeline_in == c.timeline_out());
    let mv = Command::Move {
        target,
        clip,
        delta,
    };
    let mut cmds = Vec::with_capacity(3);
    if delta > Rational::ZERO {
        if let Some(n) = next {
            cmds.push(Command::TrimHead {
                target,
                clip: n.id,
                delta,
            });
        }
        cmds.push(mv);
        if let Some(p) = prev {
            cmds.push(Command::TrimTail {
                target,
                clip: p.id,
                delta,
            });
        }
    } else {
        if let Some(p) = prev {
            cmds.push(Command::TrimTail {
                target,
                clip: p.id,
                delta,
            });
        }
        cmds.push(mv);
        if let Some(n) = next {
            cmds.push(Command::TrimHead {
                target,
                clip: n.id,
                delta,
            });
        }
    }
    Ok(Command::Group(cmds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, IdGen};
    use debut_project::{Clip, ClipSource, Sequence, TrackKind};

    fn sec(n: i64) -> Rational {
        Rational::from_int(n)
    }

    /// A[0,10) B[10,20) C[20,30), sources at 100, 200, 300.
    fn fixture() -> (Project, Target, [ClipId; 3]) {
        let mut ids = IdGen::new(77);
        let mut project = Project::new(ids.fresh(), "trim");
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 16, 9);
        let mut track = Track::new(ids.fresh(), TrackKind::Video);
        let media = ids.fresh();
        let mut clip_ids = [ClipId(0); 3];
        for (i, id) in clip_ids.iter_mut().enumerate() {
            *id = ids.fresh();
            track.clips.push(Clip::new(
                *id,
                ClipSource::Media(media),
                sec(10 * i as i64),
                sec(10),
                sec(100 * (i as i64 + 1)),
            ));
        }
        let target = Target {
            sequence: seq.id,
            track: track.id,
        };
        seq.tracks.push(track);
        project.sequences.push(seq);
        (project, target, clip_ids)
    }

    fn layout(p: &Project, t: Target) -> Vec<(i64, i64, i64)> {
        track(p, t)
            .unwrap()
            .clips
            .iter()
            .map(|c| (c.timeline_in.num, c.timeline_out().num, c.source_in.num))
            .collect()
    }

    fn check(p: &mut Project, t: Target, cmd: Command, after: &[(i64, i64, i64)]) {
        let before = p.clone();
        let inv = cmd.invert(p).unwrap();
        cmd.apply(p).unwrap();
        assert_eq!(layout(p, t), after);
        inv.apply(p).unwrap();
        assert_eq!(*p, before, "undo restores");
    }

    #[test]
    fn ripple_head_shortens_and_closes_the_gap() {
        let (mut p, t, [_, b, _]) = fixture();
        let cmd = ripple_head(&p, t, b, sec(3)).unwrap();
        check(
            &mut p,
            t,
            cmd,
            &[(0, 10, 100), (10, 17, 203), (17, 27, 300)],
        );
        let cmd = ripple_head(&p, t, b, sec(-2)).unwrap();
        check(
            &mut p,
            t,
            cmd,
            &[(0, 10, 100), (10, 22, 198), (22, 32, 300)],
        );
    }

    #[test]
    fn ripple_tail_moves_everything_after() {
        let (mut p, t, [a, _, _]) = fixture();
        let cmd = ripple_tail(&p, t, a, sec(5)).unwrap();
        check(
            &mut p,
            t,
            cmd,
            &[(0, 15, 100), (15, 25, 200), (25, 35, 300)],
        );
        let cmd = ripple_tail(&p, t, a, sec(-4)).unwrap();
        check(&mut p, t, cmd, &[(0, 6, 100), (6, 16, 200), (16, 26, 300)]);
    }

    #[test]
    fn roll_moves_the_cut_only() {
        let (mut p, t, [a, _, _]) = fixture();
        let cmd = roll(&p, t, a, sec(4)).unwrap();
        check(
            &mut p,
            t,
            cmd,
            &[(0, 14, 100), (14, 20, 204), (20, 30, 300)],
        );
        let cmd = roll(&p, t, a, sec(-4)).unwrap();
        check(&mut p, t, cmd, &[(0, 6, 100), (6, 20, 196), (20, 30, 300)]);
    }

    #[test]
    fn slip_changes_source_only() {
        let (mut p, t, [_, b, _]) = fixture();
        check(
            &mut p,
            t,
            slip(t, b, sec(7)),
            &[(0, 10, 100), (10, 20, 207), (20, 30, 300)],
        );
    }

    #[test]
    fn slide_keeps_neighbours_touching_and_length_fixed() {
        let (mut p, t, [_, b, _]) = fixture();
        let cmd = slide(&p, t, b, sec(3)).unwrap();
        check(
            &mut p,
            t,
            cmd,
            &[(0, 13, 100), (13, 23, 200), (23, 30, 303)],
        );
        let cmd = slide(&p, t, b, sec(-3)).unwrap();
        check(&mut p, t, cmd, &[(0, 7, 100), (7, 17, 200), (17, 30, 297)]);
    }

    #[test]
    fn trims_that_would_empty_or_overlap_are_rejected() {
        let (mut p, t, [a, b, _]) = fixture();
        assert!(Command::TrimTail {
            target: t,
            clip: a,
            delta: sec(-10)
        }
        .apply(&mut p)
        .is_err());
        assert!(Command::TrimTail {
            target: t,
            clip: a,
            delta: sec(1)
        }
        .apply(&mut p)
        .is_err());
        assert!(Command::Move {
            target: t,
            clip: b,
            delta: sec(1)
        }
        .apply(&mut p)
        .is_err());
        assert!(roll(&p, t, b, sec(11)).unwrap().apply(&mut p).is_err());
    }
}
