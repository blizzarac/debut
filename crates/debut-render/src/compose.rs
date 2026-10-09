//! Turn a sequence at time `t` into a [`Graph`]: the bridge from the timeline to the
//! renderer. Tracks composite bottom to top over black; each clip is fitted to the
//! canvas. Effect stacks, transitions and adjustment layers hook in here.

use crate::backend::{BlendMode, Transform2D};
use crate::graph::{Graph, Node, NodeId};
use debut_core::Rational;
use debut_project::{ClipSource, Sequence, TrackKind};

/// Source frame dimensions are needed to fit a clip onto the canvas; the caller
/// resolves them from media metadata (or the proxy actually being decoded).
pub trait SourceInfo {
    fn dimensions(&self, media: debut_core::MediaId) -> (u32, u32);
}

pub fn compose(seq: &Sequence, t: Rational, info: &dyn SourceInfo) -> Graph {
    let (w, h) = (seq.width, seq.height);
    let mut g = Graph::default();
    let mut acc: NodeId = g.add(Node::Solid {
        w,
        h,
        color: [0.0, 0.0, 0.0, 1.0],
    });
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video) {
        let Some(clip) = track.clip_at(t) else {
            continue;
        };
        let media = match &clip.source {
            ClipSource::Media(m) => *m,
            ClipSource::Multicam { angles, active } => match angles.get(*active) {
                Some(m) => *m,
                None => continue,
            },
            // Nested sequences render recursively once Source can hold a sub-graph.
            ClipSource::Sequence(_) => continue,
        };
        let src = g.add(Node::Source {
            media,
            source_time: clip.source_at(t),
        });
        let (sw, sh) = info.dimensions(media);
        let scale = (w as f32 / sw as f32).min(h as f32 / sh as f32);
        let xf = Transform2D::from_srt((sw, sh), (w, h), (scale, scale), 0.0, (0.0, 0.0));
        let fit = g.add(Node::Transform {
            input: src,
            xf,
            w,
            h,
        });
        acc = g.add(Node::Blend {
            bottom: acc,
            top: fit,
            mode: BlendMode::Normal,
            opacity: 1.0,
        });
    }
    g.set_output(acc);
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, IdGen, MediaId};
    use debut_project::{Clip, Track};

    struct Fixed(u32, u32);
    impl SourceInfo for Fixed {
        fn dimensions(&self, _: MediaId) -> (u32, u32) {
            (self.0, self.1)
        }
    }

    #[test]
    fn composes_only_the_clips_under_the_playhead() {
        let mut ids = IdGen::new(3);
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 1920, 1080);
        let mut v1 = Track::new(ids.fresh(), TrackKind::Video);
        let mut v2 = Track::new(ids.fresh(), TrackKind::Video);
        let m: MediaId = ids.fresh();
        v1.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(m),
            Rational::ZERO,
            Rational::from_int(10),
            Rational::ZERO,
        ));
        v2.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(m),
            Rational::from_int(5),
            Rational::from_int(2),
            Rational::from_int(40),
        ));
        seq.tracks.push(v1);
        seq.tracks.push(v2);

        let g = compose(&seq, Rational::from_int(2), &Fixed(1280, 720));
        assert_eq!(g.len(), 1 + 3, "solid + one layer");

        let g = compose(&seq, Rational::from_int(6), &Fixed(1280, 720));
        assert_eq!(g.len(), 1 + 6, "solid + two layers");
        // The top layer's source time follows the clip's source_in.
        let times: Vec<Rational> = (0..g.len() as u32)
            .filter_map(|i| match g.node(NodeId(i)) {
                Node::Source { source_time, .. } => Some(*source_time),
                _ => None,
            })
            .collect();
        assert_eq!(times, vec![Rational::from_int(6), Rational::from_int(41)]);
    }
}
