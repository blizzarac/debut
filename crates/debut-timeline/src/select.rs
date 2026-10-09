//! Linked selection (TL-05). Clips on other tracks that play the same source
//! over the same span as a clip (the picture and sound of one insert) are its
//! partners; with linked selection on, edits apply to them too so they stay
//! in sync. The link is derived, not stored: blades and undo keep partners
//! matched, and editing one side alone simply unlinks it.

use debut_core::{ClipId, Rational, TrackId};
use debut_project::{Clip, Sequence};

fn same_material(a: &Clip, b: &Clip) -> bool {
    a.source == b.source
        && a.timeline_in == b.timeline_in
        && a.duration == b.duration
        && a.source_in == b.source_in
        && a.speed == b.speed
        && a.ramp == b.ramp
}

/// The partners of `clip` on `track`: matching clips on every other track.
pub fn linked(seq: &Sequence, track: TrackId, clip: ClipId) -> Vec<(TrackId, ClipId)> {
    let Some(c) = seq.track(track).and_then(|t| t.clip(clip)) else {
        return Vec::new();
    };
    seq.tracks
        .iter()
        .filter(|t| t.id != track)
        .flat_map(|t| {
            t.clips
                .iter()
                .filter(|o| same_material(c, o))
                .map(move |o| (t.id, o.id))
        })
        .collect()
}

/// The clip on `track` covering `t` (start inclusive, end exclusive).
pub fn clip_at(seq: &Sequence, track: TrackId, t: Rational) -> Option<ClipId> {
    seq.track(track)?
        .clips
        .iter()
        .find(|c| c.timeline_in <= t && t < c.timeline_out())
        .map(|c| c.id)
}

/// The clip on `track` spanning exactly `[start, end)`.
pub fn clip_spanning(
    seq: &Sequence,
    track: TrackId,
    start: Rational,
    end: Rational,
) -> Option<ClipId> {
    seq.track(track)?
        .clips
        .iter()
        .find(|c| c.timeline_in == start && c.timeline_out() == end)
        .map(|c| c.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, IdGen, MediaId};
    use debut_project::{ClipSource, Track, TrackKind};

    #[test]
    fn partners_share_source_and_span() {
        let mut ids = IdGen::new(3);
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 16, 9);
        let m: MediaId = ids.fresh();
        let sec = Rational::from_int;
        let clip = |ids: &mut IdGen, at: i64, src: i64| {
            Clip::new(ids.fresh(), ClipSource::Media(m), sec(at), sec(2), sec(src))
        };
        let mut v = Track::new(ids.fresh(), TrackKind::Video);
        let mut a = Track::new(ids.fresh(), TrackKind::Audio);
        let mut a2 = Track::new(ids.fresh(), TrackKind::Audio);
        v.clips.push(clip(&mut ids, 0, 0));
        a.clips.push(clip(&mut ids, 0, 0));
        // Same media and place, other source offset: not linked.
        a2.clips.push(clip(&mut ids, 0, 5));
        let (vc, ac) = (v.clips[0].id, a.clips[0].id);
        let (vt, at) = (v.id, a.id);
        seq.tracks = vec![v, a, a2];
        assert_eq!(linked(&seq, vt, vc), vec![(at, ac)]);
        assert_eq!(linked(&seq, at, ac), vec![(vt, vc)]);
        assert_eq!(clip_at(&seq, vt, sec(1)), Some(vc));
        assert_eq!(clip_at(&seq, vt, sec(2)), None);
        assert_eq!(clip_spanning(&seq, at, sec(0), sec(2)), Some(ac));
        // A different speed breaks the link.
        seq.tracks[1].clips[0].speed = sec(2);
        assert!(linked(&seq, vt, vc).is_empty());
    }
}
