//! Speed changes (TL-09), each one undo step: constant speed that keeps the
//! clip's material (the clip gets longer or shorter), reverse, freeze frame and
//! ramps. A clip that grows either ripples the rest of its track later or, with
//! ripple off, stops at the next clip.

use debut_command::{Command, Target};
use debut_core::{ClipId, Error, Rational, Result};
use debut_project::retime::{normalize, SpeedKey};
use debut_project::{Clip, Project, Sequence, Track};

/// A reversed clip starts this far before its source out-point, so the first
/// frame shown is the last frame of the material rather than the one after.
pub const REVERSE_LEAD: Rational = Rational { num: 1, den: 1000 };

/// Fastest speed accepted either way.
pub const MAX_SPEED: i64 = 100;

fn find(project: &Project, target: Target, clip: ClipId) -> Result<(&Sequence, &Track, &Clip)> {
    let seq = project
        .sequence(target.sequence)
        .ok_or_else(|| Error::NotFound(format!("sequence {:?}", target.sequence)))?;
    let tr = seq
        .track(target.track)
        .ok_or_else(|| Error::NotFound(format!("track {:?}", target.track)))?;
    let c = tr
        .clip(clip)
        .ok_or_else(|| Error::NotFound(format!("clip {clip:?}")))?;
    Ok((seq, tr, c))
}

/// The source span the clip shows, low to high, undoing a reverse lead.
fn material(c: &Clip) -> (Rational, Rational) {
    let (a, b) = (c.source_in, c.source_out());
    if c.ramp.is_empty() && c.speed.is_zero() {
        // A freeze frame covers no material: unfreezing plays on from the
        // held frame for the clip's length.
        (a, a + c.duration)
    } else if c.ramp.is_empty() && c.speed.is_negative() {
        (b + REVERSE_LEAD, a + REVERSE_LEAD)
    } else {
        (a.min(b), a.max(b))
    }
}

/// Play the clip at constant `speed` (negative reverses, 0 freezes the frame
/// under the clip's start). Non-zero speeds keep the clip's material, so its
/// duration becomes material / |speed| rounded to whole frames; a ramp is
/// replaced. With `ripple`, later clips on the track move by the change;
/// without, a longer clip stops at the next clip.
pub fn set_speed(
    project: &Project,
    target: Target,
    clip: ClipId,
    speed: Rational,
    ripple: bool,
) -> Result<Command> {
    if speed > Rational::from_int(MAX_SPEED) || speed < Rational::from_int(-MAX_SPEED) {
        return Err(Error::InvalidArgument(format!(
            "speed must be within ±{MAX_SPEED}x"
        )));
    }
    let (seq, tr, c) = find(project, target, clip)?;
    let (lo, hi) = material(c);
    let (source_in, mut duration) = if speed.is_zero() {
        (c.source_in, c.duration)
    } else {
        let mag = if speed.is_negative() { -speed } else { speed };
        let len = (hi - lo) * Rational::new(mag.den, mag.num);
        // Nearest whole frame, at least one.
        let fr = seq.frame_rate;
        let len = fr
            .frame_to_time((len * fr.0).round())
            .max(fr.frame_duration());
        let start = if speed.is_negative() {
            hi - REVERSE_LEAD
        } else {
            lo
        };
        (start, len)
    };
    let old_out = c.timeline_out();
    let next_in = tr
        .clips
        .iter()
        .filter(|o| o.id != clip && o.timeline_in >= old_out)
        .map(|o| o.timeline_in)
        .min();
    if !ripple {
        if let Some(n) = next_in {
            duration = duration.min(n - c.timeline_in);
        }
    }
    let timing = Command::SetTiming {
        target,
        clip,
        speed,
        ramp: Vec::new(),
        source_in,
        duration,
    };
    let delta = duration - c.duration;
    Ok(if !ripple || delta.is_zero() || next_in.is_none() {
        timing
    } else {
        let shift = Command::Shift {
            target,
            from: old_out,
            by: delta,
        };
        // Make room before growing; shrink before pulling the rest in.
        if delta > Rational::ZERO {
            Command::Group(vec![shift, timing])
        } else {
            Command::Group(vec![timing, shift])
        }
    })
}

/// Give the clip a speed ramp (empty clears it back to the constant speed).
/// The clip keeps its place and length; the ramp decides how much material
/// plays. Keys are in clip-local seconds and must lie within ±`MAX_SPEED`.
pub fn set_ramp(
    project: &Project,
    target: Target,
    clip: ClipId,
    mut keys: Vec<SpeedKey>,
) -> Result<Command> {
    let (_, _, c) = find(project, target, clip)?;
    if keys
        .iter()
        .any(|k| !k.speed.is_finite() || k.speed.abs() > MAX_SPEED as f64)
    {
        return Err(Error::InvalidArgument(format!(
            "ramp speeds must be within ±{MAX_SPEED}x"
        )));
    }
    normalize(&mut keys);
    Ok(Command::SetTiming {
        target,
        clip,
        speed: c.speed,
        ramp: keys,
        source_in: c.source_in,
        duration: c.duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, IdGen};
    use debut_project::{ClipSource, TrackKind};

    fn sec(n: i64) -> Rational {
        Rational::from_int(n)
    }

    /// A[0,10) B[10,20), sources at 100 and 200.
    fn fixture() -> (Project, Target, [ClipId; 2]) {
        let mut ids = IdGen::new(91);
        let mut project = Project::new(ids.fresh(), "speed");
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 16, 9);
        let mut track = Track::new(ids.fresh(), TrackKind::Video);
        let media = ids.fresh();
        let mut clip_ids = [ClipId(0); 2];
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

    fn clips(p: &Project, t: Target) -> Vec<(Rational, Rational, Rational)> {
        p.sequence(t.sequence)
            .unwrap()
            .track(t.track)
            .unwrap()
            .clips
            .iter()
            .map(|c| (c.timeline_in, c.duration, c.source_in))
            .collect()
    }

    fn run(p: &mut Project, make: impl FnOnce(&Project) -> Result<Command>) -> Command {
        let cmd = make(p).unwrap();
        let inv = cmd.invert(p).unwrap();
        cmd.apply(p).unwrap();
        inv
    }

    #[test]
    fn half_speed_ripples_and_undoes() {
        let (mut p, t, [a, _]) = fixture();
        let before = clips(&p, t);
        let inv = run(&mut p, |p| set_speed(p, t, a, Rational::new(1, 2), true));
        assert_eq!(
            clips(&p, t),
            vec![(sec(0), sec(20), sec(100)), (sec(20), sec(10), sec(200))]
        );
        let c = &p.sequences[0].tracks[0].clips[0];
        assert_eq!(
            c.source_at(sec(20)),
            sec(110),
            "same material, twice as long"
        );
        inv.apply(&mut p).unwrap();
        assert_eq!(clips(&p, t), before);
    }

    #[test]
    fn without_ripple_a_longer_clip_stops_at_the_next() {
        let (mut p, t, [a, b]) = fixture();
        run(&mut p, |p| set_speed(p, t, a, Rational::new(1, 2), false));
        assert_eq!(clips(&p, t)[0].1, sec(10));
        // Faster: shorter, and with ripple the next clip follows.
        run(&mut p, |p| set_speed(p, t, b, sec(2), true));
        assert_eq!(clips(&p, t)[1], (sec(10), sec(5), sec(200)));
    }

    #[test]
    fn reverse_and_freeze() {
        let (mut p, t, [a, _]) = fixture();
        run(&mut p, |p| set_speed(p, t, a, sec(-1), true));
        let c = p.sequences[0].tracks[0].clips[0].clone();
        assert_eq!(c.source_at(sec(0)), sec(110) - REVERSE_LEAD);
        assert_eq!(c.source_out(), sec(100) - REVERSE_LEAD);
        // Back to forward: the original material, no drift from the lead.
        run(&mut p, |p| set_speed(p, t, a, sec(1), true));
        assert_eq!(clips(&p, t)[0], (sec(0), sec(10), sec(100)));
        run(&mut p, |p| set_speed(p, t, a, Rational::ZERO, true));
        let c = &p.sequences[0].tracks[0].clips[0];
        assert_eq!((c.source_at(sec(7)), c.duration), (sec(100), sec(10)));
        run(&mut p, |p| set_speed(p, t, a, sec(1), true));
        assert_eq!(clips(&p, t)[0], (sec(0), sec(10), sec(100)), "unfreeze");
        assert!(set_speed(&p, t, a, sec(500), true).is_err());
    }

    #[test]
    fn ramp_keeps_the_clip_and_changes_the_material() {
        let (mut p, t, [a, _]) = fixture();
        let keys = vec![
            SpeedKey {
                at: sec(5),
                speed: 3.0,
            },
            SpeedKey {
                at: sec(0),
                speed: 1.0,
            },
        ];
        let inv = run(&mut p, |p| set_ramp(p, t, a, keys));
        let c = &p.sequences[0].tracks[0].clips[0];
        assert_eq!(c.ramp[0].at, sec(0), "sorted");
        // 0..5 s: trapezoid (1+3)/2*5 = 10, then 3x for 5 s = 15.
        assert_eq!(c.source_out(), sec(125));
        assert_eq!(c.duration, sec(10));
        inv.apply(&mut p).unwrap();
        assert!(p.sequences[0].tracks[0].clips[0].ramp.is_empty());
        assert!(set_ramp(
            &p,
            t,
            a,
            vec![SpeedKey {
                at: sec(0),
                speed: f64::NAN
            }]
        )
        .is_err());
    }
}
