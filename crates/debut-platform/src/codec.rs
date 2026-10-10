//! Decode and encode (MED-01, MED-02, EXP-01).
//! Desktop: FFmpeg + VideoToolbox / NVDEC / QSV / AMF. Browser: WebCodecs, WASM fallback.

use debut_core::{FrameRate, Rational, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub duration: Rational,
    pub codec: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioInfo {
    pub channels: u16,
    pub sample_rate: u32,
    pub duration: Rational,
    pub codec: String,
}

/// A decoded video frame. Pixels are 8-bit RGBA in the source's display encoding
/// (Rec.709 / sRGB transfer); the render graph linearizes under its managed
/// pipeline (FX-08). GPU-resident decode paths will add a texture variant.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoFrame {
    pub pts: Rational,
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

/// Decoded audio, interleaved `f32` at the source sample rate.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBlock {
    pub pts: Rational,
    pub channels: u16,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

pub trait Decoder: Send {
    fn video_info(&self) -> Option<&VideoInfo>;
    fn audio_info(&self) -> Option<&AudioInfo>;
    /// Position so that the next frames come from at or before `to`.
    fn seek(&mut self, to: Rational) -> Result<()>;
    /// Next video frame in presentation order; `None` at end of stream.
    fn next_video(&mut self) -> Result<Option<VideoFrame>>;
    /// Next audio block in presentation order; `None` at end of stream.
    fn next_audio(&mut self) -> Result<Option<AudioBlock>>;
    /// Decode only these streams. A reader that pulls one kind should switch
    /// the other off, or the decoder buffers it while searching. Both are on
    /// by default.
    fn select(&mut self, _video: bool, _audio: bool) {}
    /// Container and stream tags worth keeping as media metadata (MED-04).
    fn tags(&self) -> SourceTags {
        SourceTags::default()
    }
    /// Where the video frames decoded so far came from (NFR-09).
    fn decode_path(&self) -> DecodePath {
        DecodePath::Software
    }
}

/// How a decoder is decoding its video.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DecodePath {
    /// On the CPU, as asked (or before any frame was decoded).
    #[default]
    Software,
    /// A device of this API was given to the decoder; no frame has shown
    /// yet whether its driver takes the stream.
    Requested(String),
    /// On a hardware decoder through this device API ("vaapi", "videotoolbox").
    Hardware(String),
    /// Hardware was asked for, but the device or its driver could not take
    /// this stream, so it decodes on the CPU.
    Fallback { wanted: String, reason: String },
}

/// Tags a camera or recorder leaves in the file: start timecode as written
/// ("HH:MM:SS:FF", ';' before the frames for drop-frame), reel / tape name and
/// camera model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceTags {
    pub timecode: Option<String>,
    pub reel: Option<String>,
    pub camera: Option<String>,
}

pub trait Encoder: Send {
    fn push_video(&mut self, frame: &VideoFrame) -> Result<()>;
    fn push_audio(&mut self, block: &AudioBlock) -> Result<()>;
    fn finish(self: Box<Self>) -> Result<()>;
    /// The codec implementation actually in use (e.g. `libx264`, `h264_nvenc`).
    fn encoder_name(&self) -> &str {
        ""
    }
    /// True when the requested encoder could not open and a fallback is used.
    fn used_fallback(&self) -> bool {
        false
    }
}

/// Encode settings for one output file (EXP-01): H.264 video plus optional AAC
/// audio. `encoder` names a preferred implementation; one that cannot open
/// falls back to software (NFR-09).
#[derive(Clone, Debug)]
pub struct EncodeSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    /// Constant-rate-factor-like quality (lower = better, 18–28 is typical).
    pub crf: u8,
    pub audio: Option<AudioEncodeSettings>,
    pub encoder: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AudioEncodeSettings {
    pub channels: u16,
    pub sample_rate: u32,
    pub bitrate: usize,
}

/// A hardware encoder this machine can actually open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HwEncoder {
    /// Implementation name, e.g. `hevc_videotoolbox`.
    pub name: String,
    /// Codec family: "h264" or "hevc".
    pub codec: String,
    /// The acceleration API behind it, e.g. "NVENC".
    pub api: String,
}
