//! Snapping and gap removal (TL-06). The engine lists where an edit may snap
//! to; the UI picks the nearest one while dragging. `close_gaps` removes the
//! empty space between clips on a track as one undo step.

use debut_command::{Command, Target};
use debut_core::{ClipId, Error, Rational, Result};
use debut_project::{Project, Sequence};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind {
    SequenceStart,
    Playhead,
    ClipEdge,
    Marker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapTarget {
    pub t: Rational,
    pub kind: SnapKind,
}

/// Every place an edit may snap to: the sequence start, the playhead, the
/// edges of every clip on every track except `exclude` (the clips being
/// dragged), and timeline and clip markers. Sorted by time; a time that is
/// several kinds keeps the first kind in `SnapKind` order.
pub fn snap_targets(seq: &Sequence, playhead: Rational, exclude: &[ClipId]) -> Vec<SnapTarget> {
    let mut out = vec![
        SnapTarget {
            t: Rational::ZERO,
            kind: SnapKind::SequenceStart,
        },
        SnapTarget {
            t: playhead,
            kind: SnapKind::Playhead,
        },
    ];
    for track in &seq.tracks {
        for c in track.clips.iter().filter(|c| !exclude.contains(&c.id)) {
            for t in [c.timeline_in, c.timeline_out()] {
                out.push(SnapTarget {
                    t,
                    kind: SnapKind::ClipEdge,
                });
            }
            out.extend(c.markers.iter().map(|m| SnapTarget {
                t: c.timeline_in + m.at,
                kind: SnapKind::Marker,
            }));
        }
    }
    out.extend(seq.markers.iter().map(|m| SnapTarget {
        t: m.at,
        kind: SnapKind::Marker,
    }));
    out.sort_by_key(|s| (s.t, s.kind));
    out.dedup_by_key(|s| s.t);
    out
}

/// Close every gap between clips on a track, rippling later clips left.
/// Leading space before the first clip is kept. `Noop` when there are none.
pub fn close_gaps(project: &Project, target: Target) -> Result<Command> {
    let track = project
        .sequence(target.sequence)
        .and_then(|s| s.track(target.track))
        .ok_or_else(|| Error::NotFound(format!("track {:?}", target.track)))?;
    // Shifts run right to left so each one only moves clips after its gap.
    let shifts: Vec<Command> = track
        .clips
        .windows(2)
        .filter(|w| w[1].timeline_in > w[0].timeline_out())
        .map(|w| Command::Shift {
            target,
            from: w[1].timeline_in,
            by: w[0].timeline_out() - w[1].timeline_in,
        })
        .rev()
        .collect();
    Ok(if shifts.is_empty() {
        Command::Noop
    } else {
        Command::Group(shifts)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, IdGen};
    use debut_project::{Clip, ClipSource, Marker, Track, TrackKind};

    fn secs(n: i64) -> Rational {
        Rational::from_int(n)
    }

    fn project() -> (Project, Target, Vec<ClipId>) {
        let mut ids = IdGen::new(9);
        let mut p = Project::new(ids.fresh(), "p");
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 16, 9);
        let mut v = Track::new(ids.fresh(), TrackKind::Video);
        let media = ids.fresh();
        let mut clip_ids = Vec::new();
        // Clips at 1..3, 5..6, 9..10: gaps of 2 s and 3 s, 1 s of lead-in.
        for (at, len) in [(1, 2), (5, 1), (9, 1)] {
            let c = Clip::new(
                ids.fresh(),
                ClipSource::Media(media),
                secs(at),
                secs(len),
                Rational::ZERO,
            );
            clip_ids.push(c.id);
            v.clips.push(c);
        }
        v.clips[1]
            .markers
            .push(Marker::new(ids.fresh(), Rational::new(1, 2), "beat"));
        seq.markers.push(Marker::new(ids.fresh(), secs(7), "cue"));
        let target = Target {
            sequence: seq.id,
            track: v.id,
        };
        seq.tracks.push(v);
        p.sequences.push(seq);
        (p, target, clip_ids)
    }

    #[test]
    fn targets_cover_edges_markers_playhead_and_skip_the_dragged_clip() {
        let (p, _, clips) = project();
        let seq = &p.sequences[0];
        let ts = |exclude: &[ClipId]| -> Vec<Rational> {
            snap_targets(seq, Rational::new(5, 2), exclude)
                .iter()
                .map(|s| s.t)
                .collect()
        };
        assert_eq!(
            ts(&[]),
            vec![
                secs(0),
                secs(1),
                Rational::new(5, 2),
                secs(3),
                secs(5),
                Rational::new(11, 2),
                secs(6),
                secs(7),
                secs(9),
                secs(10)
            ]
        );
        // Dragging the middle clip: its own edges and markers are not targets.
        assert!(!ts(&[clips[1]]).contains(&secs(5)));
        assert!(!ts(&[clips[1]]).contains(&Rational::new(11, 2)));
        // Where the playhead sits on a clip edge, the target is reported as the playhead.
        let on_edge = snap_targets(seq, secs(3), &[]);
        assert_eq!(
            on_edge.iter().find(|s| s.t == secs(3)).unwrap().kind,
            SnapKind::Playhead
        );
    }

    #[test]
    fn close_gaps_ripples_clips_together_and_undoes() {
        let (mut p, target, _) = project();
        let cmd = close_gaps(&p, target).unwrap();
        let inverse = cmd.invert(&p).unwrap();
        cmd.apply(&mut p).unwrap();
        let spans: Vec<(Rational, Rational)> = p.sequences[0].tracks[0]
            .clips
            .iter()
            .map(|c| (c.timeline_in, c.timeline_out()))
            .collect();
        assert_eq!(
            spans,
            vec![(secs(1), secs(3)), (secs(3), secs(4)), (secs(4), secs(5))]
        );
        assert_eq!(
            close_gaps(&p, target).unwrap(),
            Command::Noop,
            "nothing left to close"
        );
        inverse.apply(&mut p).unwrap();
        assert_eq!(p.sequences[0].tracks[0].clips[2].timeline_in, secs(9));
    }
}
