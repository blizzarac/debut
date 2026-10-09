//! Browser (wasm32) implementations of the `debut-platform` traits (PLT-06 .. PLT-09).
//! Planned deps: web-sys, js-sys, wasm-bindgen-futures.
//! Limits the engine must respect here: no ProRes/BRAW/R3D in WebCodecs (proxies on
//! upload, PLT-07), capped WASM heap (stream frames, cache on GPU), COOP/COEP for
//! SharedArrayBuffer.

pub mod codec;       // WebCodecs; WASM software fallback
pub mod file_store;  // OPFS + File System Access API; offline cache (PLT-09)
pub mod audio_out;   // AudioWorklet
pub mod threads;     // Web Workers + SharedArrayBuffer
pub mod plugin_host; // sandboxed WASM plugins only
pub mod display;     // canvas with WebGPU context
