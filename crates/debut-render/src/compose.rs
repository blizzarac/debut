//! Turn a sequence at time `t` into a [`Graph`]: the bridge from the timeline to the
//! renderer. Tracks composite bottom to top over black; each clip is fitted to the
//! canvas. Effect stacks, transitions and adjustment layers hook in here.

use crate::backend::{BlendMode, Transform2D};
use crate::color::{ColorTransform, Grade};
use crate::graph::{Graph, Image8, ImageRef, LutRef, Node, NodeId};
use crate::lut::Lut3d;
use debut_core::color::ColorSpace;
use debut_core::Rational;
use debut_project::{Clip, ClipSource, Effect, Layer, Param, Sequence, Title, TrackKind};
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
    /// Rasterize (or fetch from a cache) a title clip's text (GFX-01) as straight
    /// sRGB RGBA8. `None` leaves the title out of the picture.
    fn title(&self, _title: &Title) -> Option<Arc<Image8>> {
        None
    }
}

/// The working space every layer is converted into before compositing.
pub const WORKING: ColorSpace = ColorSpace::Linear709;

pub fn compose(seq: &Sequence, t: Rational, info: &dyn SourceInfo) -> Graph {
    compose_at(seq, t, info, (seq.width, seq.height))
}

/// Compose onto a canvas of another size: the preview at 1/2 or 1/4 resolution
/// (PB-03) renders the same picture with positions scaled, so the viewer and the
/// full-size export agree.
pub fn compose_at(seq: &Sequence, t: Rational, info: &dyn SourceInfo, canvas: (u32, u32)) -> Graph {
    let (w, h) = (canvas.0.max(1), canvas.1.max(1));
    let px_scale = w as f32 / seq.width.max(1) as f32;
    let mut g = Graph::default();
    let mut acc: NodeId = g.add(Node::Solid {
        w,
        h,
        color: [0.0, 0.0, 0.0, 1.0],
    });
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video) {
        let Some(layer) = track.layer_at(t) else {
            continue;
        };
        let (node, opacity) = match layer {
            Layer::Single(clip) => match clip_layer(&mut g, clip, t, (w, h), px_scale, info) {
                Some(l) => l,
                None => continue,
            },
            Layer::Transition { from, to, progress } => {
                let a = clip_layer(&mut g, from, t, (w, h), px_scale, info);
                let b = clip_layer(&mut g, to, t, (w, h), px_scale, info);
                match (a, b) {
                    (Some((na, oa)), Some((nb, ob))) => {
                        let node = g.add(Node::Dissolve {
                            a: na,
                            b: nb,
                            progress,
                        });
                        (node, oa + (ob - oa) * progress)
                    }
                    (Some(l), None) | (None, Some(l)) => l,
                    (None, None) => continue,
                }
            }
        };
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

/// One clip's node chain at sequence time `t` (which may lie in its transition
/// handle, outside `[timeline_in, timeline_out)`): source, input transform, fit +
/// user transform, then its effect stack. Returns the node and the blend opacity.
fn clip_layer(
    g: &mut Graph,
    clip: &Clip,
    t: Rational,
    canvas: (u32, u32),
    px_scale: f32,
    info: &dyn SourceInfo,
) -> Option<(NodeId, f32)> {
    let (w, h) = canvas;
    // Source node, its colour space, its size and how it fits the canvas: media
    // is scaled to fill the frame, a title is authored in sequence pixels.
    let (mut src, space, (sw, sh), fit) = match &clip.source {
        ClipSource::Media(_) | ClipSource::Multicam { .. } => {
            let media = match &clip.source {
                ClipSource::Media(m) => *m,
                ClipSource::Multicam { angles, active } => *angles.get(*active)?,
                _ => unreachable!(),
            };
            let src = g.add(Node::Source {
                media,
                source_time: clip.source_at(t),
            });
            let (sw, sh) = info.dimensions(media);
            let fit = (w as f32 / sw as f32).min(h as f32 / sh as f32);
            (src, info.color_space(media), (sw, sh), fit)
        }
        ClipSource::Title(title) => {
            let image = info.title(title)?;
            let size = (image.width, image.height);
            let src = g.add(Node::Image {
                image: ImageRef(image),
            });
            (src, ColorSpace::Srgb, size, px_scale)
        }
        // Nested sequences render recursively once Source can hold a sub-graph.
        ClipSource::Sequence(_) => return None,
    };
    if let Ok(xf) = ColorTransform::between(&space, &WORKING) {
        if !xf.is_identity() {
            src = g.add(Node::ColorTransform { input: src, xf });
        }
    }
    let local = t - clip.timeline_in;
    let (mut scale, mut rotation, mut dx, mut dy, mut opacity) =
        (1.0f32, 0.0f32, 0.0f32, 0.0f32, 1.0f32);
    if let Some(i) = clip.effect_index("transform") {
        let v = |p: Param| clip.param_at(i, p, t).unwrap_or(0.0) as f32;
        scale = v(Param::Scale);
        rotation = v(Param::Rotation).to_radians();
        dx = v(Param::X) * px_scale;
        dy = v(Param::Y) * px_scale;
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
                let v = |p: Param| effect.value(p, local).unwrap_or(0.0) as f32;
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
    Some((node, opacity))
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

    #[test]
    fn dissolve_renders_both_clips_through_a_dissolve_node() {
        use debut_project::{Transition, TransitionKind};
        let mut ids = IdGen::new(9);
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 100, 100);
        let mut v1 = Track::new(ids.fresh(), TrackKind::Video);
        let m: MediaId = ids.fresh();
        v1.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(m),
            Rational::ZERO,
            Rational::from_int(10),
            Rational::ZERO,
        ));
        let mut b = Clip::new(
            ids.fresh(),
            ClipSource::Media(m),
            Rational::from_int(10),
            Rational::from_int(10),
            Rational::from_int(5),
        );
        b.transition_in = Some(Transition {
            kind: TransitionKind::Dissolve,
            duration: Rational::from_int(2),
        });
        v1.clips.push(b);
        seq.tracks.push(v1);
        // 9.5 s: a quarter into the dissolve. A plays source 9.5, B plays its handle at 4.5.
        let g = compose(&seq, Rational::new(19, 2), &Fixed(100, 100));
        let nodes: Vec<&Node> = (0..g.len() as u32).map(|i| g.node(NodeId(i))).collect();
        let times: Vec<Rational> = nodes
            .iter()
            .filter_map(|n| match n {
                Node::Source { source_time, .. } => Some(*source_time),
                _ => None,
            })
            .collect();
        assert_eq!(times, vec![Rational::new(19, 2), Rational::new(9, 2)]);
        let progress = nodes.iter().find_map(|n| match n {
            Node::Dissolve { progress, .. } => Some(*progress),
            _ => None,
        });
        assert_eq!(progress, Some(0.25));
        // Outside the window there is no dissolve.
        let g = compose(&seq, Rational::from_int(5), &Fixed(100, 100));
        assert!((0..g.len() as u32).all(|i| !matches!(g.node(NodeId(i)), Node::Dissolve { .. })));
    }

    #[test]
    fn compose_at_scales_positions_with_the_canvas() {
        use debut_project::TransformFx;
        let mut ids = IdGen::new(12);
        let mut seq = Sequence::new(ids.fresh(), "s", FrameRate::FPS_25, 1920, 1080);
        let mut v1 = Track::new(ids.fresh(), TrackKind::Video);
        let m: MediaId = ids.fresh();
        let mut clip = Clip::new(
            ids.fresh(),
            ClipSource::Media(m),
            Rational::ZERO,
            Rational::from_int(10),
            Rational::ZERO,
        );
        let tf = TransformFx {
            x: Curve::constant(400.0),
            ..Default::default()
        };
        clip.effects.push(Effect::Transform(tf));
        v1.clips.push(clip);
        seq.tracks.push(v1);
        let full = compose(&seq, Rational::ONE, &Fixed(1920, 1080));
        let quarter = compose_at(&seq, Rational::ONE, &Fixed(1920, 1080), (480, 270));
        let xf = |g: &Graph| {
            (0..g.len() as u32)
                .find_map(|i| match g.node(NodeId(i)) {
                    Node::Transform { xf, w, h, .. } => Some((*xf, *w, *h)),
                    _ => None,
                })
                .unwrap()
        };
        let (f, fw, fh) = xf(&full);
        let (q, qw, qh) = xf(&quarter);
        assert_eq!((fw, fh, qw, qh), (1920, 1080, 480, 270));
        // The same source point lands at the same relative canvas position.
        let (sx_f, sy_f) = f.apply(960.0 + 400.0, 540.0);
        let (sx_q, sy_q) = q.apply(240.0 + 100.0, 135.0);
        assert!(
            (sx_f - sx_q).abs() < 1e-3 && (sy_f - sy_q).abs() < 1e-3,
            "{sx_f},{sy_f} vs {sx_q},{sy_q}"
        );
    }
}
