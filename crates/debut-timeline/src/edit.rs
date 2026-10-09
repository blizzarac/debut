//! Three- and four-point editing (TL-03). Given a source range and a sequence
//! range with any three of the four points set, derive the clip; with all four,
//! fit the source to the gap by changing speed.

use debut_command::{Command, Target};
use debut_core::{ClipId, Error, IdGen, MediaId, Rational, Result};
use debut_project::{Clip, ClipSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    Insert,
    Overwrite,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditPoints {
    pub source_in: Option<Rational>,
    pub source_out: Option<Rational>,
    pub sequence_in: Option<Rational>,
    pub sequence_out: Option<Rational>,
}

/// Build the edit. `source_len` bounds a missing source out-point.
pub fn three_point(
    target: Target,
    media: MediaId,
    source_len: Rational,
    points: EditPoints,
    mode: EditMode,
    ids: &mut IdGen,
) -> Result<Command> {
    let EditPoints {
        source_in,
        source_out,
        sequence_in,
        sequence_out,
    } = points;
    let set = [source_in, source_out, sequence_in, sequence_out]
        .iter()
        .filter(|p| p.is_some())
        .count();
    if set < 3 && !(set == 2 && sequence_in.is_some() && source_in.is_some()) {
        return Err(Error::InvalidArgument(
            "three-point edit needs three points (or source in + sequence in)".into(),
        ));
    }
    let src_len = match (source_in, source_out) {
        (Some(i), Some(o)) if o > i => Some(o - i),
        (Some(_), Some(_)) => {
            return Err(Error::InvalidArgument("source out must follow in".into()))
        }
        _ => None,
    };
    let seq_len = match (sequence_in, sequence_out) {
        (Some(i), Some(o)) if o > i => Some(o - i),
        (Some(_), Some(_)) => {
            return Err(Error::InvalidArgument("sequence out must follow in".into()))
        }
        _ => None,
    };

    let (clip_src_in, duration, speed) = match (src_len, seq_len) {
        // Four points: fit by speed.
        (Some(s), Some(q)) => (
            source_in.unwrap(),
            q,
            Rational::new(s.num * q.den, s.den * q.num),
        ),
        // Source range, one sequence point.
        (Some(s), None) => (source_in.unwrap(), s, Rational::ONE),
        // Sequence range, one source point.
        (None, Some(q)) => {
            let src_in = match (source_in, source_out) {
                (Some(i), _) => i,
                (None, Some(o)) => o - q,
                (None, None) => Rational::ZERO,
            };
            (src_in, q, Rational::ONE)
        }
        // Source in + sequence in: use the rest of the source.
        (None, None) => {
            let src_in = source_in.unwrap_or(Rational::ZERO);
            (src_in, source_len - src_in, Rational::ONE)
        }
    };
    if duration <= Rational::ZERO || clip_src_in.is_negative() {
        return Err(Error::InvalidArgument("edit has no duration".into()));
    }

    let timeline_in = match (sequence_in, sequence_out) {
        (Some(i), _) => i,
        (None, Some(o)) => o - duration,
        (None, None) => Rational::ZERO,
    };
    if timeline_in.is_negative() {
        return Err(Error::InvalidArgument(
            "edit would start before zero".into(),
        ));
    }

    let id: ClipId = ids.fresh();
    let mut clip = Clip::new(
        id,
        ClipSource::Media(media),
        timeline_in,
        duration,
        clip_src_in,
    );
    clip.speed = speed;
    Ok(match mode {
        EditMode::Insert => Command::insert(target, timeline_in, vec![clip], ids),
        EditMode::Overwrite => Command::overwrite(target, clip, ids),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, Rational};
    use debut_project::{Project, Sequence, Track, TrackKind};

    fn sec(n: i64) -> Rational {
        Rational::from_int(n)
    }

    fn setup() -> (Project, Target, MediaId, IdGen) {
        let mut ids = IdGen::new(3);
        let mut project = Project::new(ids.fresh(), "edit");
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 16, 9);
        let track = Track::new(ids.fresh(), TrackKind::Video);
        let target = Target {
            sequence: seq.id,
            track: track.id,
        };
        seq.tracks.push(track);
        project.sequences.push(seq);
        let media = ids.fresh();
        (project, target, media, ids)
    }

    fn only_clip(p: &Project, t: Target) -> Clip {
        let tr = p.sequence(t.sequence).unwrap().track(t.track).unwrap();
        assert_eq!(tr.clips.len(), 1);
        tr.clips[0].clone()
    }

    #[test]
    fn source_range_to_sequence_in() {
        let (mut p, t, m, mut ids) = setup();
        let pts = EditPoints {
            source_in: Some(sec(5)),
            source_out: Some(sec(9)),
            sequence_in: Some(sec(2)),
            ..Default::default()
        };
        three_point(t, m, sec(60), pts, EditMode::Overwrite, &mut ids)
            .unwrap()
            .apply(&mut p)
            .unwrap();
        let c = only_clip(&p, t);
        assert_eq!(
            (c.timeline_in, c.duration, c.source_in, c.speed),
            (sec(2), sec(4), sec(5), Rational::ONE)
        );
    }

    #[test]
    fn sequence_range_backtimed_from_source_out() {
        let (mut p, t, m, mut ids) = setup();
        let pts = EditPoints {
            source_out: Some(sec(20)),
            sequence_in: Some(sec(10)),
            sequence_out: Some(sec(13)),
            ..Default::default()
        };
        three_point(t, m, sec(60), pts, EditMode::Insert, &mut ids)
            .unwrap()
            .apply(&mut p)
            .unwrap();
        let c = only_clip(&p, t);
        assert_eq!(
            (c.timeline_in, c.duration, c.source_in),
            (sec(10), sec(3), sec(17))
        );
    }

    #[test]
    fn sequence_out_only_backtimes_the_clip() {
        let (mut p, t, m, mut ids) = setup();
        let pts = EditPoints {
            source_in: Some(sec(0)),
            source_out: Some(sec(4)),
            sequence_out: Some(sec(10)),
            ..Default::default()
        };
        three_point(t, m, sec(60), pts, EditMode::Overwrite, &mut ids)
            .unwrap()
            .apply(&mut p)
            .unwrap();
        assert_eq!(only_clip(&p, t).timeline_in, sec(6));
    }

    #[test]
    fn four_points_fit_to_fill_by_changing_speed() {
        let (mut p, t, m, mut ids) = setup();
        let pts = EditPoints {
            source_in: Some(sec(0)),
            source_out: Some(sec(10)),
            sequence_in: Some(sec(0)),
            sequence_out: Some(sec(4)),
        };
        three_point(t, m, sec(60), pts, EditMode::Overwrite, &mut ids)
            .unwrap()
            .apply(&mut p)
            .unwrap();
        let c = only_clip(&p, t);
        assert_eq!((c.duration, c.speed), (sec(4), Rational::new(5, 2)));
        assert_eq!(c.source_out(), sec(10));
    }

    #[test]
    fn two_ins_use_the_rest_of_the_source_and_bad_input_is_rejected() {
        let (mut p, t, m, mut ids) = setup();
        let pts = EditPoints {
            source_in: Some(sec(50)),
            sequence_in: Some(sec(0)),
            ..Default::default()
        };
        three_point(t, m, sec(60), pts, EditMode::Overwrite, &mut ids)
            .unwrap()
            .apply(&mut p)
            .unwrap();
        assert_eq!(only_clip(&p, t).duration, sec(10));
        let bad = EditPoints {
            source_in: Some(sec(5)),
            source_out: Some(sec(1)),
            sequence_in: Some(sec(0)),
            ..Default::default()
        };
        assert!(three_point(t, m, sec(60), bad, EditMode::Overwrite, &mut ids).is_err());
        let few = EditPoints {
            sequence_in: Some(sec(0)),
            ..Default::default()
        };
        assert!(three_point(t, m, sec(60), few, EditMode::Overwrite, &mut ids).is_err());
    }
}
