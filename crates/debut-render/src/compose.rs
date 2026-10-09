//! Turn a sequence at time `t` into a [`Graph`]: the bridge from the timeline to the
//! renderer. Tracks composite bottom to top over black; each clip is fitted to the
//! canvas. Effect stacks, transitions and adjustment layers hook in here.

use crate::backend::{BlendMode, Transform2D};
use crate::color::{ColorTransform, Grade};
use crate::graph::{Graph, LutRef, Node, NodeId};
use crate::lut::Lut3d;
use debut_core::color::ColorSpace;
use debut_core::Rational;
use debut_project::{ClipSource, Effect, Param, Sequence, TrackKind};
use std::sync::Arc;

/// Source frame dimensions are needed to fit a clip onto the canvas; the caller
/// resolves them from media metadata (or the proxy actually being decoded).
pub trait SourceInfo {
    fn dimensions(&self, media: debut_core::MediaId) -> (u32, u32);
    /// The space the decoded pixels are encoded in; Rec.709 video when untagged.
    fn color_space(&self, _media: debut_core::MediaId) -> ColorSpace {
        ColorSpace::Rec709
    }
    /// Resolve a LUT referenced from a clip's effect stack by content hash.
    fn lut(&self, _hash: u64) -> Option<Arc<Lut3d>> {
        None
    }
}

/// The working space every layer is converted into before compositing.
pub const WORKING: ColorSpace = ColorSpace::Linear709;

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
        let mut src = g.add(Node::Source {
            media,
            source_time: clip.source_at(t),
        });
        if let Ok(xf) = ColorTransform::between(&info.color_space(media), &WORKING) {
            if !xf.is_identity() {
                src = g.add(Node::ColorTransform { input: src, xf });
            }
        }
        let (sw, sh) = info.dimensions(media);
        let fit = (w as f32 / sw as f32).min(h as f32 / sh as f32);
        let local = t - clip.timeline_in;
        // Transform effect (first one wins); defaults fit the frame to the canvas.
        let (mut scale, mut rotation, mut dx, mut dy, mut opacity) =
            (1.0f32, 0.0f32, 0.0f32, 0.0f32, 1.0f32);
        if let Some(Effect::Transform(_)) = clip.effect("transform") {
            let i = clip.effect_index("transform").unwrap();
            let v = |p: Param| clip.param_at(i, p, t).unwrap_or(0.0) as f32;
            scale = v(Param::Scale);
            rotation = v(Param::Rotation).to_radians();
            dx = v(Param::X);
            dy = v(Param::Y);
            opacity = v(Param::Opacity).clamp(0.0, 1.0);
        }
        let xf = Transform2D::from_srt(
            (sw, sh),
            (w, h),
            (fit * scale, fit * scale),
            rotation,
            (dx, dy),
        );
        let mut node = g.add(Node::Transform {
            input: src,
            xf,
            w,
            h,
        });
        for effect in &clip.effects {
            match effect {
                Effect::Grade(_) => {
                    let i = clip
                        .effects
                        .iter()
                        .position(|e| std::ptr::eq(e, effect))
                        .unwrap();
                    let v = |p: Param| effect.value(p, local).unwrap_or(0.0) as f32;
                    let _ = i;
                    let grade = Grade {
                        exposure: v(Param::Exposure),
                        contrast: v(Param::Contrast),
                        saturation: v(Param::Saturation),
                        temperature: v(Param::Temperature),
                        tint: v(Param::Tint),
                        ..Grade::default()
                    };
                    if !grade.is_identity() {
                        node = g.add(Node::Grade { input: node, grade });
                    }
                }
                Effect::Lut { hash, .. } => {
                    if let Some(lut) = info.lut(*hash) {
                        node = g.add(Node::Lut3d {
                            input: node,
                            lut: LutRef(lut),
                        });
                    }
                }
                Effect::Transform(_) => {}
            }
        }
        acc = g.add(Node::Blend {
            bottom: acc,
            top: node,
            mode: BlendMode::Normal,
            opacity,
        });
    }
    g.set_output(acc);
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{Curve, FrameRate, IdGen, MediaId};
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
        assert_eq!(
            g.len(),
            1 + 4,
            "solid + source, input transform, fit, blend"
        );

        let g = compose(&seq, Rational::from_int(6), &Fixed(1280, 720));
        assert_eq!(g.len(), 1 + 8, "solid + two layers");
        // The top layer's source time follows the clip's source_in.
        let times: Vec<Rational> = (0..g.len() as u32)
            .filter_map(|i| match g.node(NodeId(i)) {
                Node::Source { source_time, .. } => Some(*source_time),
                _ => None,
            })
            .collect();
        assert_eq!(times, vec![Rational::from_int(6), Rational::from_int(41)]);
    }

    #[test]
    fn effects_drive_transform_grade_and_opacity() {
        use debut_core::Interp;
        use debut_project::{GradeFx, TransformFx};
        let mut ids = IdGen::new(4);
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 100, 100);
        let mut v1 = Track::new(ids.fresh(), TrackKind::Video);
        let m: MediaId = ids.fresh();
        let mut clip = Clip::new(
            ids.fresh(),
            ClipSource::Media(m),
            Rational::from_int(10),
            Rational::from_int(10),
            Rational::ZERO,
        );
        let mut tf = TransformFx::default();
        tf.opacity.set(Rational::ZERO, 0.0, Interp::Linear);
        tf.opacity.set(Rational::from_int(10), 1.0, Interp::Linear);
        tf.scale = Curve::constant(0.5);
        clip.effects.push(Effect::Transform(tf));
        let gr = GradeFx {
            exposure: Curve::constant(1.0),
            ..Default::default()
        };
        clip.effects.push(Effect::Grade(gr));
        v1.clips.push(clip);
        seq.tracks.push(v1);

        // Halfway through the clip: opacity 0.5 (clip-local keys), grade present.
        let g = compose(&seq, Rational::from_int(15), &Fixed(50, 50));
        let nodes: Vec<&Node> = (0..g.len() as u32).map(|i| g.node(NodeId(i))).collect();
        let blend = nodes.iter().find_map(|n| match n {
            Node::Blend { opacity, .. } => Some(*opacity),
            _ => None,
        });
        assert_eq!(blend, Some(0.5));
        assert!(nodes
            .iter()
            .any(|n| matches!(n, Node::Grade { grade, .. } if grade.exposure == 1.0)));
        let xf = nodes.iter().find_map(|n| match n {
            Node::Transform { xf, .. } => Some(*xf),
            _ => None,
        });
        // fit = 2 (50 -> 100), user scale 0.5 => net 1: the inverse map has unit scale.
        assert!((xf.unwrap().m[0][0] - 1.0).abs() < 1e-6);
    }
}
