//! The node graph for one output frame, and its pull evaluator.

use crate::backend::{Backend, BlendMode, FrameProvider, Rgba, Transform2D};
use crate::color::{ColorTransform, Grade};
use crate::lut::Lut3d;
use debut_core::{Error, MediaId, Rational, Result};
use serde::{Serialize, Serializer};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct NodeId(pub u32);

/// A LUT shared between graphs; serializes (and so hashes) as its content hash.
#[derive(Clone, Debug)]
pub struct LutRef(pub Arc<Lut3d>);

impl PartialEq for LutRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.hash == other.0.hash
    }
}

impl Serialize for LutRef {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_u64(self.0.hash)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Node {
    Solid {
        w: u32,
        h: u32,
        color: Rgba,
    },
    /// A decoded frame of `media` at `source_time`, provided by the [`FrameProvider`].
    Source {
        media: MediaId,
        source_time: Rational,
    },
    /// Resample `input` onto a `w x h` canvas through `xf` (FX-02).
    Transform {
        input: NodeId,
        xf: Transform2D,
        w: u32,
        h: u32,
    },
    /// `top` over `bottom` (FX-02 blend modes and opacity).
    Blend {
        bottom: NodeId,
        top: NodeId,
        mode: BlendMode,
        opacity: f32,
    },
    /// Cross-dissolve transition (FX-03).
    Dissolve {
        a: NodeId,
        b: NodeId,
        progress: f32,
    },
    /// Color-space conversion (FX-08): input transforms into the working space,
    /// output transforms for display.
    ColorTransform {
        input: NodeId,
        xf: ColorTransform,
    },
    /// 3D LUT (FX-11).
    Lut3d {
        input: NodeId,
        lut: LutRef,
    },
    /// Primary correction (FX-09).
    Grade {
        input: NodeId,
        grade: Grade,
    },
}

impl Node {
    fn inputs(&self) -> Vec<NodeId> {
        match self {
            Node::Solid { .. } | Node::Source { .. } => vec![],
            Node::Transform { input, .. }
            | Node::ColorTransform { input, .. }
            | Node::Lut3d { input, .. }
            | Node::Grade { input, .. } => vec![*input],
            Node::Blend { bottom, top, .. } => vec![*bottom, *top],
            Node::Dissolve { a, b, .. } => vec![*a, *b],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Graph {
    nodes: Vec<Node>,
    output: Option<NodeId>,
}

impl Graph {
    pub fn add(&mut self, node: Node) -> NodeId {
        for input in node.inputs() {
            assert!(
                (input.0 as usize) < self.nodes.len(),
                "node input must already exist"
            );
        }
        self.nodes.push(node);
        NodeId(self.nodes.len() as u32 - 1)
    }

    pub fn set_output(&mut self, id: NodeId) {
        self.output = Some(id);
    }

    pub fn output(&self) -> Option<NodeId> {
        self.output
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Stable hash of the whole graph: the render-cache key (PB-06). Two graphs that
    /// would produce the same pixels from the same sources hash the same.
    pub fn content_hash(&self) -> u64 {
        let json = serde_json::to_string(self).expect("graph is serializable");
        let mut h = std::collections::hash_map::DefaultHasher::new();
        json.hash(&mut h);
        h.finish()
    }

    /// Pull-evaluate the output node. Inputs are added before the nodes that use
    /// them, so evaluating in index order up to the output is a topological pass;
    /// nodes the output doesn't reach are skipped.
    pub fn render<B: Backend>(
        &self,
        backend: &mut B,
        frames: &mut dyn FrameProvider,
    ) -> Result<B::Image> {
        let out = self
            .output
            .ok_or_else(|| Error::InvalidArgument("graph has no output node".into()))?;
        let mut needed = vec![false; self.nodes.len()];
        let mut stack = vec![out];
        while let Some(id) = stack.pop() {
            if !needed[id.0 as usize] {
                needed[id.0 as usize] = true;
                stack.extend(self.nodes[id.0 as usize].inputs());
            }
        }
        let mut images: Vec<Option<B::Image>> = vec![None; self.nodes.len()];
        for (i, node) in self.nodes.iter().enumerate() {
            if !needed[i] {
                continue;
            }
            let get = |images: &Vec<Option<B::Image>>, id: NodeId| {
                images[id.0 as usize].clone().expect("input evaluated")
            };
            let img = match node {
                Node::Solid { w, h, color } => backend.solid(*w, *h, *color),
                Node::Source { media, source_time } => {
                    let (w, h, px) = frames.frame(*media, *source_time)?;
                    backend.upload_rgba8(w, h, &px)
                }
                Node::Transform { input, xf, w, h } => {
                    backend.transform(&get(&images, *input), xf, *w, *h)
                }
                Node::Blend {
                    bottom,
                    top,
                    mode,
                    opacity,
                } => backend.blend(&get(&images, *bottom), &get(&images, *top), *mode, *opacity),
                Node::Dissolve { a, b, progress } => {
                    backend.dissolve(&get(&images, *a), &get(&images, *b), *progress)
                }
                Node::ColorTransform { input, xf } => {
                    backend.color_transform(&get(&images, *input), xf)
                }
                Node::Lut3d { input, lut } => backend.lut3d(&get(&images, *input), &lut.0),
                Node::Grade { input, grade } => backend.grade(&get(&images, *input), grade),
            };
            images[i] = Some(img);
        }
        Ok(images[out.0 as usize].take().expect("output evaluated"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CpuBackend;
    use debut_core::IdGen;

    /// Every media is a solid frame whose colour encodes the request.
    struct SolidSource {
        w: u32,
        h: u32,
    }

    impl FrameProvider for SolidSource {
        fn frame(&mut self, _media: MediaId, t: Rational) -> Result<(u32, u32, Vec<u8>)> {
            // Red = t, encoded as 8-bit; the test reads it back through the graph.
            let v = (t.as_f64() * 255.0).round() as u8;
            Ok((
                self.w,
                self.h,
                (0..self.w * self.h).flat_map(|_| [v, 0, 0, 255]).collect(),
            ))
        }
    }

    #[test]
    fn two_layer_composite_renders_top_over_bottom() {
        let mut g = Graph::default();
        let bg = g.add(Node::Solid {
            w: 2,
            h: 2,
            color: [0.0, 0.0, 1.0, 1.0],
        });
        let src = g.add(Node::Source {
            media: IdGen::new(1).fresh(),
            source_time: Rational::new(1, 2),
        });
        let fit = g.add(Node::Transform {
            input: src,
            xf: Transform2D::IDENTITY,
            w: 2,
            h: 2,
        });
        let out = g.add(Node::Blend {
            bottom: bg,
            top: fit,
            mode: BlendMode::Normal,
            opacity: 0.5,
        });
        g.set_output(out);

        let mut be = CpuBackend;
        let img = g.render(&mut be, &mut SolidSource { w: 2, h: 2 }).unwrap();
        let px = be.download(&img);
        assert_eq!(px.len(), 4);
        let p = px[0];
        assert!(
            (p[0] - 0.25).abs() < 2e-3 && (p[2] - 0.5).abs() < 1e-6 && (p[3] - 1.0).abs() < 1e-6,
            "{p:?}"
        );
    }

    #[test]
    fn unreachable_nodes_are_not_evaluated() {
        struct Panics;
        impl FrameProvider for Panics {
            fn frame(&mut self, _: MediaId, _: Rational) -> Result<(u32, u32, Vec<u8>)> {
                panic!("should not be pulled")
            }
        }
        let mut g = Graph::default();
        let _orphan = g.add(Node::Source {
            media: IdGen::new(1).fresh(),
            source_time: Rational::ZERO,
        });
        let bg = g.add(Node::Solid {
            w: 1,
            h: 1,
            color: [1.0; 4],
        });
        g.set_output(bg);
        g.render(&mut CpuBackend, &mut Panics).unwrap();
    }

    #[test]
    fn content_hash_tracks_parameters() {
        let mut a = Graph::default();
        let s = a.add(Node::Solid {
            w: 1,
            h: 1,
            color: [1.0; 4],
        });
        a.set_output(s);
        let mut b = a.clone();
        assert_eq!(a.content_hash(), b.content_hash());
        b.nodes[0] = Node::Solid {
            w: 1,
            h: 1,
            color: [0.5; 4],
        };
        assert_ne!(a.content_hash(), b.content_hash());
    }
}
