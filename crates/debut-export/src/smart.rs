//! Smart render (EXP-05): find the stretches of a sequence where the picture
//! is one source frame for frame, untouched, so the export can copy its
//! compressed packets instead of decoding and re-encoding them.
//!
//! A stretch qualifies when exactly one clip is visible across all video
//! tracks, it is plain media at normal speed with no effects (or a Transform
//! left at its defaults), no transition or burned-in caption touches it, and
//! the source has the sequence's size and frame rate at a constant rate.
//! Audio is mixed and encoded as usual either way.

use debut_core::{FrameRate, MediaId, Rational};
use debut_project::{ClipSource, Effect, Sequence, TrackKind, TransformFx};

/// What the planner needs to know about a media file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceFacts {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub variable_frame_rate: bool,
}

/// One stretch of the export range.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// Timeline range, start inclusive, end exclusive.
    pub start: Rational,
    pub end: Rational,
    /// Set when the stretch can be copied: the media and the source time
    /// its first frame comes from.
    pub copy: Option<Copy>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Copy {
    pub media: MediaId,
    pub source_in: Rational,
    /// The media's file, filled in by whoever runs the plan.
    pub path: String,
}

impl Span {
    /// Source range of a copy span.
    pub fn source_range(&self) -> Option<(Rational, Rational)> {
        self.copy
            .as_ref()
            .map(|c| (c.source_in, c.source_in + (self.end - self.start)))
    }
}

/// Fraction of `spans` (by duration) that can be copied.
pub fn copied_fraction(spans: &[Span]) -> f64 {
    let total: Rational = spans
        .iter()
        .map(|s| s.end - s.start)
        .fold(Rational::ZERO, |a, b| a + b);
    if total <= Rational::ZERO {
        return 0.0;
    }
    let copied = spans
        .iter()
        .filter(|s| s.copy.is_some())
        .map(|s| s.end - s.start)
        .fold(Rational::ZERO, |a, b| a + b);
    copied.as_f64() / total.as_f64()
}

fn untouched(effects: &[Effect]) -> bool {
    effects
        .iter()
        .all(|e| matches!(e, Effect::Transform(t) if *t == TransformFx::default()))
}

/// Split `range` of `seq` into copy and render spans. Neighbouring copy spans
/// that continue the same source are merged, as are neighbouring render spans.
pub fn plan(
    seq: &Sequence,
    range: (Rational, Rational),
    facts: impl Fn(MediaId) -> Option<SourceFacts>,
) -> Vec<Span> {
    let (from, to) = range;
    if to <= from {
        return Vec::new();
    }
    let video: Vec<_> = seq
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .collect();
    // Every point where what is on screen can change.
    let mut cuts = vec![from, to];
    let mut blocked: Vec<(Rational, Rational)> = Vec::new();
    for track in &video {
        for (i, c) in track.clips.iter().enumerate() {
            cuts.push(c.timeline_in);
            cuts.push(c.timeline_out());
            if let Some(tr) = c.transition_in {
                let h = tr.half();
                blocked.push((c.timeline_in - h, c.timeline_in + h));
                cuts.push(c.timeline_in - h);
                cuts.push(c.timeline_in + h);
                // The outgoing clip plays on into its handle.
                if i > 0 {
                    cuts.push(track.clips[i - 1].timeline_out() + h);
                }
            }
        }
    }
    if seq.caption_settings.burn_in {
        for cap in &seq.captions {
            blocked.push((cap.start, cap.end));
            cuts.push(cap.start);
            cuts.push(cap.end);
        }
    }
    cuts.retain(|t| *t >= from && *t <= to);
    cuts.sort();
    cuts.dedup();

    let mut spans: Vec<Span> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let copy = copy_source(seq, &video, &blocked, a, b, &facts);
        match (spans.last_mut(), copy) {
            (Some(last), None) if last.copy.is_none() => last.end = b,
            (Some(last), Some(c))
                if last.copy.as_ref().is_some_and(|l| {
                    l.media == c.media && l.source_in + (last.end - last.start) == c.source_in
                }) =>
            {
                last.end = b
            }
            (_, copy) => spans.push(Span {
                start: a,
                end: b,
                copy,
            }),
        }
    }
    spans
}

/// The copy source for `[a, b)`, if that stretch qualifies.
fn copy_source(
    seq: &Sequence,
    video: &[&debut_project::Track],
    blocked: &[(Rational, Rational)],
    a: Rational,
    b: Rational,
    facts: &impl Fn(MediaId) -> Option<SourceFacts>,
) -> Option<Copy> {
    if blocked.iter().any(|&(s, e)| s < b && a < e) {
        return None;
    }
    let mut visible = video
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.timeline_in < b && a < c.timeline_out());
    let clip = visible.next()?;
    if visible.next().is_some() {
        return None;
    }
    // Inside the cuts the clip covers the whole stretch.
    if clip.timeline_in > a || clip.timeline_out() < b {
        return None;
    }
    let ClipSource::Media(media) = clip.source else {
        return None;
    };
    if clip.is_retimed() || !untouched(&clip.effects) {
        return None;
    }
    let f = facts(media)?;
    if (f.width, f.height) != (seq.width, seq.height)
        || f.frame_rate != seq.frame_rate
        || f.variable_frame_rate
    {
        return None;
    }
    let source_in = clip.source_at(a);
    // Frame-aligned in the source, or copied frames would be off by a part.
    if seq.frame_rate.snap(source_in) != source_in {
        return None;
    }
    Some(Copy {
        media,
        source_in,
        path: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::IdGen;
    use debut_project::{Clip, GradeFx, Track, Transition, TransitionKind};

    fn s(n: i64) -> Rational {
        Rational::from_int(n)
    }

    struct Fx {
        seq: Sequence,
        a: MediaId,
        b: MediaId,
        ids: IdGen,
    }

    fn fixture() -> Fx {
        let mut ids = IdGen::new(1);
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 64, 36);
        let (a, b) = (ids.fresh(), ids.fresh());
        let mut v = Track::new(ids.fresh(), TrackKind::Video);
        v.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(a),
            s(0),
            s(4),
            s(1),
        ));
        v.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(b),
            s(4),
            s(4),
            s(0),
        ));
        seq.tracks.push(v);
        Fx { seq, a, b, ids }
    }

    fn facts(m: MediaId) -> Option<SourceFacts> {
        let _ = m;
        Some(SourceFacts {
            width: 64,
            height: 36,
            frame_rate: FrameRate::FPS_25,
            variable_frame_rate: false,
        })
    }

    #[test]
    fn plain_cuts_copy_every_clip() {
        let f = fixture();
        let spans = plan(&f.seq, (s(0), s(8)), facts);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].source_range(), Some((s(1), s(5))));
        assert_eq!(spans[0].copy.as_ref().unwrap().media, f.a);
        assert_eq!(spans[1].source_range(), Some((s(0), s(4))));
        assert_eq!(spans[1].copy.as_ref().unwrap().media, f.b);
        assert_eq!(copied_fraction(&spans), 1.0);
        // A sub-range starts mid-clip.
        let spans = plan(&f.seq, (s(2), s(6)), facts);
        assert_eq!(spans[0].source_range(), Some((s(3), s(5))));
        assert_eq!(spans[1].source_range(), Some((s(0), s(2))));
    }

    #[test]
    fn effects_overlays_and_gaps_render() {
        let mut f = fixture();
        f.seq.tracks[0].clips[1]
            .effects
            .push(Effect::Grade(GradeFx::default()));
        // A title over seconds 1..2 of the first clip.
        let mut v2 = Track::new(f.ids.fresh(), TrackKind::Video);
        v2.clips.push(Clip::new(
            f.ids.fresh(),
            ClipSource::Media(f.b),
            s(1),
            s(1),
            s(0),
        ));
        f.seq.tracks.push(v2);
        let spans = plan(&f.seq, (s(0), s(10)), facts);
        let kinds: Vec<_> = spans
            .iter()
            .map(|s| (s.start, s.end, s.copy.is_some()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (s(0), s(1), true),
                (s(1), s(2), false),
                (s(2), s(4), true),
                // Graded clip, then the gap after the last clip.
                (s(4), s(10), false),
            ]
        );
        // The copy after the overlay picks up the source where it left off.
        assert_eq!(spans[2].source_range(), Some((s(3), s(5))));
        assert!((copied_fraction(&spans) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn a_default_transform_still_copies_but_a_moved_one_does_not() {
        let mut f = fixture();
        f.seq.tracks[0].clips[0]
            .effects
            .push(Effect::Transform(TransformFx::default()));
        assert!(plan(&f.seq, (s(0), s(4)), facts)[0].copy.is_some());
        let moved = TransformFx {
            scale: debut_core::keyframe::Curve::constant(1.1),
            ..Default::default()
        };
        f.seq.tracks[0].clips[0].effects = vec![Effect::Transform(moved)];
        assert!(plan(&f.seq, (s(0), s(4)), facts)[0].copy.is_none());
    }

    #[test]
    fn transitions_speed_and_mismatched_sources_render() {
        let mut f = fixture();
        f.seq.tracks[0].clips[1].transition_in = Some(Transition {
            kind: TransitionKind::Dissolve,
            duration: s(2),
        });
        let spans = plan(&f.seq, (s(0), s(8)), facts);
        let kinds: Vec<_> = spans
            .iter()
            .map(|s| (s.start, s.end, s.copy.is_some()))
            .collect();
        assert_eq!(
            kinds,
            vec![(s(0), s(3), true), (s(3), s(5), false), (s(5), s(8), true)]
        );

        let mut f = fixture();
        f.seq.tracks[0].clips[0].speed = Rational::new(1, 2);
        assert!(plan(&f.seq, (s(0), s(4)), facts)[0].copy.is_none());

        let f = fixture();
        let other = |_| {
            Some(SourceFacts {
                width: 1920,
                height: 1080,
                frame_rate: FrameRate::FPS_25,
                variable_frame_rate: false,
            })
        };
        assert!(plan(&f.seq, (s(0), s(4)), other)[0].copy.is_none());
        let vfr = |m| {
            facts(m).map(|f| SourceFacts {
                variable_frame_rate: true,
                ..f
            })
        };
        assert!(plan(&f.seq, (s(0), s(4)), vfr)[0].copy.is_none());
    }

    #[test]
    fn burned_in_captions_render_their_span() {
        let mut f = fixture();
        f.seq.captions.push(debut_project::Caption {
            id: f.ids.fresh(),
            start: s(1),
            end: s(2),
            text: "hi".into(),
        });
        let copied = |seq: &Sequence| {
            plan(seq, (s(0), s(4)), facts)
                .iter()
                .filter(|s| s.copy.is_some())
                .count()
        };
        f.seq.caption_settings.burn_in = false;
        assert_eq!(copied(&f.seq), 1);
        f.seq.caption_settings.burn_in = true;
        assert_eq!(copied(&f.seq), 2, "split around the caption");
    }
}
