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
pub mod color;
pub mod compose;
pub mod cpu;
pub mod gpu;
pub mod graph;
pub mod lut;

pub mod nodes; // FX-04 masks, FX-05 chroma key
pub mod ofx; // FX-15 OpenFX bridge over PluginHost (pending)
pub mod scopes; // PB-08 waveform, vectorscope, histogram
pub mod tracking; // FX-06 point tracker

pub use backend::{encode_rgba8, Backend, BlendMode, FrameProvider, Rgba, Transform2D};
pub use cache::RenderCache;
pub use cpu::CpuBackend;
pub use gpu::GpuBackend;

/// Whichever backend the machine has: the GPU one when an adapter exists, else
/// the CPU reference. Lets callers that only need pixels out stay generic-free.
pub enum AnyBackend {
    Cpu(CpuBackend),
    Gpu(Box<GpuBackend>),
}

impl AnyBackend {
    pub fn detect() -> Self {
        match GpuBackend::new() {
            Some(g) => AnyBackend::Gpu(Box::new(g)),
            None => AnyBackend::Cpu(CpuBackend),
        }
    }

    pub fn is_gpu(&self) -> bool {
        matches!(self, AnyBackend::Gpu(_))
    }

    /// Render `graph` and read it back as straight, `transfer`-encoded RGBA8:
    /// the output transform runs on the GPU when there is one.
    pub fn render_rgba8(
        &mut self,
        graph: &Graph,
        frames: &mut dyn FrameProvider,
        transfer: Transfer,
    ) -> debut_core::Result<(u32, u32, Vec<u8>)> {
        match self {
            AnyBackend::Cpu(b) => {
                let img = graph.render(b, frames)?;
                let (w, h) = b.size(&img);
                Ok((w, h, b.download_rgba8(&img, transfer)))
            }
            AnyBackend::Gpu(b) => {
                let img = graph.render(b.as_mut(), frames)?;
                let (w, h) = b.size(&img);
                Ok((w, h, b.download_rgba8(&img, transfer)))
            }
        }
    }

    /// Render `graph` and read the pixels back as linear premultiplied RGBA.
    pub fn render_pixels(
        &mut self,
        graph: &Graph,
        frames: &mut dyn FrameProvider,
    ) -> debut_core::Result<(u32, u32, Vec<Rgba>)> {
        match self {
            AnyBackend::Cpu(b) => {
                let img = graph.render(b, frames)?;
                let (w, h) = b.size(&img);
                Ok((w, h, b.download(&img)))
            }
            AnyBackend::Gpu(b) => {
                let img = graph.render(b.as_mut(), frames)?;
                let (w, h) = b.size(&img);
                Ok((w, h, b.download(&img)))
            }
        }
    }
}
pub use color::{ColorTransform, Grade, Transfer};
pub use compose::{compose, compose_at};
pub use debut_core::keyframe::{self, Curve, Interp, Keyframe};
pub use graph::{Graph, Image8, ImageRef, LutRef, Node, NodeId};
pub use lut::Lut3d;
pub use nodes::{
    flatten_outline, smooth_handles, ChromaKey, Mask, MaskShape, PolyMask, POLY_MAX_EDIT_POINTS,
    POLY_MAX_POINTS,
};
pub use scopes::Scopes;
pub use tracking::Tracker;
