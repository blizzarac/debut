//! Shared primitives used by every engine crate.
//!
//! Nothing here touches the OS, the GPU or a codec. It compiles identically
//! for native and `wasm32` targets (PLT-01).

pub mod error;
pub mod id;
pub mod time;
pub mod color;

pub use error::{Error, Result};
pub use id::{ClipId, ProjectId, SequenceId, TrackId};
pub use time::{FrameRate, Rational, Timecode};
