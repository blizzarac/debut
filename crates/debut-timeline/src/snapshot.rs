//! Compare a sequence with a snapshot of it (TL-14). Clips are matched by id
//! across all tracks; each one that differs is reported with what changed.
//! Blading keeps the head's id and gives the tail a new one, so a blade shows
//! as a trimmed clip plus an added one.

use debut_core::{ClipId, Rational, TrackId};
use debut_project::{Clip, Sequence};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKind {
    Added,
    Removed,
    /// Same material, other place or track.
    Moved,
    /// Different in or out point.
    Trimmed,
    /// Speed or ramp changed.
    Retimed,
    /// What it plays, its effects or its transition changed.
    Changed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub clip: ClipId,
    /// The track in the newer sequence (the older one for removed clips).
    pub track: TrackId,
    /// Where the clip starts (in the newer sequence unless removed).
    pub at: Rational,
    pub kinds: Vec<ChangeKind>,
}

/// Differences between two versions of a sequence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diff {
    pub clips: Vec<Change>,
    pub markers_added: usize,
    pub markers_removed: usize,
    pub captions_added: usize,
    pub captions_removed: usize,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
            && self.markers_added
                + self.markers_removed
                + self.captions_added
                + self.captions_removed
                == 0
    }
}

fn index(seq: &Sequence) -> HashMap<ClipId, (TrackId, &Clip)> {
    seq.tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(move |c| (c.id, (t.id, c))))
        .collect()
}

/// What changed going from `old` to `new`, ordered by time.
pub fn diff(old: &Sequence, new: &Sequence) -> Diff {
    let (a, b) = (index(old), index(new));
    let mut clips = Vec::new();
    for (id, (track, c)) in &b {
        let Some((old_track, o)) = a.get(id) else {
            clips.push(Change {
                clip: *id,
                track: *track,
                at: c.timeline_in,
                kinds: vec![ChangeKind::Added],
            });
            continue;
        };
        let mut kinds = Vec::new();
        let trimmed = c.duration != o.duration || c.source_in != o.source_in;
        if c.speed != o.speed || c.ramp != o.ramp {
            kinds.push(ChangeKind::Retimed);
        } else if trimmed {
            kinds.push(ChangeKind::Trimmed);
        }
        // A head trim moves the start too; only call it a move when the
        // material did not change, or when the track did.
        if track != old_track || (c.timeline_in != o.timeline_in && !trimmed) {
            kinds.push(ChangeKind::Moved);
        }
        if c.source != o.source || c.effects != o.effects || c.transition_in != o.transition_in {
            kinds.push(ChangeKind::Changed);
        }
        if !kinds.is_empty() {
            kinds.sort();
            clips.push(Change {
                clip: *id,
                track: *track,
                at: c.timeline_in,
                kinds,
            });
        }
    }
    for (id, (track, o)) in &a {
        if !b.contains_key(id) {
            clips.push(Change {
                clip: *id,
                track: *track,
                at: o.timeline_in,
                kinds: vec![ChangeKind::Removed],
            });
        }
    }
    clips.sort_by(|x, y| x.at.cmp(&y.at).then(x.kinds.cmp(&y.kinds)));
    let count = |from: &[debut_core::id::MarkerId], to: &[debut_core::id::MarkerId]| {
        from.iter().filter(|m| !to.contains(m)).count()
    };
    let (om, nm): (Vec<_>, Vec<_>) = (
        old.markers.iter().map(|m| m.id).collect(),
        new.markers.iter().map(|m| m.id).collect(),
    );
    let (oc, nc): (Vec<_>, Vec<_>) = (
        old.captions.iter().map(|c| c.id).collect(),
        new.captions.iter().map(|c| c.id).collect(),
    );
    Diff {
        clips,
        markers_added: count(&nm, &om),
        markers_removed: count(&om, &nm),
        captions_added: nc.iter().filter(|c| !oc.contains(c)).count(),
        captions_removed: oc.iter().filter(|c| !nc.contains(c)).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, IdGen, MediaId};
    use debut_project::{ClipSource, Marker, Track, TrackKind};

    #[test]
    fn reports_added_removed_moved_trimmed_retimed() {
        let mut ids = IdGen::new(4);
        let sec = Rational::from_int;
        let m: MediaId = ids.fresh();
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 16, 9);
        let mut v = Track::new(ids.fresh(), TrackKind::Video);
        let v2 = Track::new(ids.fresh(), TrackKind::Video);
        for i in 0..4 {
            v.clips.push(Clip::new(
                ids.fresh(),
                ClipSource::Media(m),
                sec(i * 10),
                sec(10),
                sec(0),
            ));
        }
        let ids_of: Vec<ClipId> = v.clips.iter().map(|c| c.id).collect();
        seq.tracks = vec![v, v2];
        let old = seq.clone();
        assert!(diff(&old, &seq).is_empty());

        let mut new = old.clone();
        let t = &mut new.tracks[0].clips;
        t[0].duration = sec(5); // trimmed
        t[1].speed = sec(2); // retimed
        let moved = t.remove(2);
        new.tracks[1].clips.push(moved); // other track
        new.tracks[0].clips.remove(2); // the fourth clip is gone
        let added = Clip::new(ids.fresh(), ClipSource::Media(m), sec(50), sec(2), sec(0));
        let added_id = added.id;
        new.tracks[0].clips.push(added);
        new.markers.push(Marker::new(ids.fresh(), sec(1), "new"));

        let d = diff(&old, &new);
        let kinds: Vec<(ClipId, Vec<ChangeKind>)> =
            d.clips.iter().map(|c| (c.clip, c.kinds.clone())).collect();
        assert_eq!(
            kinds,
            vec![
                (ids_of[0], vec![ChangeKind::Trimmed]),
                (ids_of[1], vec![ChangeKind::Retimed]),
                (ids_of[2], vec![ChangeKind::Moved]),
                (ids_of[3], vec![ChangeKind::Removed]),
                (added_id, vec![ChangeKind::Added]),
            ]
        );
        assert_eq!((d.markers_added, d.markers_removed), (1, 0));
        // Reversed, adds and removes swap.
        let back = diff(&new, &old);
        assert!(back
            .clips
            .iter()
            .any(|c| c.clip == added_id && c.kinds == vec![ChangeKind::Removed]));
        assert_eq!(back.markers_removed, 1);
    }
}
