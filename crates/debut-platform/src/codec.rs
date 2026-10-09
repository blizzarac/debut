//! Decode and encode (MED-01, MED-02, EXP-01).
//! Desktop: FFmpeg + VideoToolbox / NVDEC / QSV / AMF. Browser: WebCodecs, WASM fallback.

use debut_core::{Rational, Result};

/// A decoded video frame handed to the render graph. The payload is a GPU-resident
/// texture handle or a CPU buffer depending on the decode path.
pub struct VideoFrame {
    pub pts: Rational,
    pub width: u32,
    pub height: u32,
}

pub struct AudioBlock {
    pub pts: Rational,
    pub channels: u16,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

pub trait Decoder: Send {
    fn seek(&mut self, to: Rational) -> Result<()>;
    fn next_video(&mut self) -> Result<Option<VideoFrame>>;
    fn next_audio(&mut self) -> Result<Option<AudioBlock>>;
}

pub trait Encoder: Send {
    fn push_video(&mut self, frame: &VideoFrame) -> Result<()>;
    fn push_audio(&mut self, block: &AudioBlock) -> Result<()>;
    fn finish(self: Box<Self>) -> Result<()>;
}
