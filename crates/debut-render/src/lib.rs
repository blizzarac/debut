//! GPU render graph (FX-01 .. FX-15, PB-01, PB-03, PB-06, PB-08, PLT-04).
//!
//! Pull-based: the viewer or exporter requests a frame at time `t`; nodes evaluate
//! lazily upstream. All math is 16-bit float linear light under OCIO (FX-08).
//! Shaders are WGSL in `/shaders`, shared with the browser build; preview and export
//! run the same graph so output matches pixel for pixel.

pub mod graph;      // node graph, pull evaluation
pub mod nodes;      // transform, blend, transition, key, mask, LUT, color wheels
pub mod keyframe;   // FX-01 curves and interpolation
pub mod color;      // FX-08 .. FX-12 OCIO pipeline, LUT I/O
pub mod tracking;   // FX-04, FX-06 point/planar tracking, stabilization
pub mod cache;      // PB-06 content-hash keyed render cache
pub mod scopes;     // PB-08 waveform, vectorscope, parade, histogram, false color
pub mod ofx;        // FX-15 OpenFX bridge over PluginHost
