//! Render graph (FX-01 .. FX-15, PB-01, PB-03, PB-06, PB-08, PLT-04).
//!
//! Pull-based: the viewer or exporter asks for a frame at time `t`; `compose` turns
//! the sequence into a [`Graph`] for that instant and [`Graph::render`] evaluates it
//! lazily, memoizing each node. All math is linear-light `f32` RGBA, premultiplied.
//!
//! Nodes don't touch pixels themselves: they call a [`Backend`]. The CPU backend is
//! the reference implementation and runs in tests; the wgpu backend executes the
//! same ops with WGSL shaders from `/shaders`. Preview and export share this graph,
//! so output matches the viewer pixel for pixel.

pub mod backend;
pub mod cache;
pub mod compose;
pub mod cpu;
pub mod gpu;
pub mod graph;
pub mod keyframe;

pub mod color; // FX-08 .. FX-12 OCIO pipeline, LUT I/O (pending)
pub mod nodes; // further node kinds: masks, keys, LUTs, color wheels (pending)
pub mod ofx; // FX-15 OpenFX bridge over PluginHost (pending)
pub mod scopes; // PB-08 (pending)
pub mod tracking; // FX-04, FX-06 (pending)

pub use backend::{Backend, BlendMode, FrameProvider, Rgba, Transform2D};
pub use cache::RenderCache;
pub use cpu::CpuBackend;
pub use gpu::GpuBackend;
pub use graph::{Graph, Node, NodeId};
pub use keyframe::{Curve, Interp, Keyframe};
