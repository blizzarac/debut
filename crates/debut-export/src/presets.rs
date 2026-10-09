//! Delivery presets (EXP-02). Platform-neutral descriptions; the platform encoder
//! maps them onto the codecs it has.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
    ProRes422,
    DnxHr,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub codec: VideoCodec,
    /// Target quality (CRF-like, lower is better) for rate-factor codecs.
    pub quality: u8,
    pub audio_bitrate: usize,
    /// Scale the sequence to this size; `None` keeps sequence size.
    pub size: Option<(u32, u32)>,
    /// Loudness target the audio is normalized to (AUD-06), in LUFS.
    pub loudness_lufs: f32,
}

impl Preset {
    pub fn youtube_4k() -> Self {
        Self {
            name: "YouTube 4K".into(),
            codec: VideoCodec::H264,
            quality: 18,
            audio_bitrate: 384_000,
            size: Some((3840, 2160)),
            loudness_lufs: -14.0,
        }
    }

    pub fn youtube_1080p() -> Self {
        Self {
            name: "YouTube 1080p".into(),
            codec: VideoCodec::H264,
            quality: 20,
            audio_bitrate: 256_000,
            size: Some((1920, 1080)),
            loudness_lufs: -14.0,
        }
    }

    pub fn instagram_reel() -> Self {
        Self {
            name: "Instagram Reel".into(),
            codec: VideoCodec::H264,
            quality: 21,
            audio_bitrate: 192_000,
            size: Some((1080, 1920)),
            loudness_lufs: -14.0,
        }
    }

    pub fn broadcast_ebu() -> Self {
        Self {
            name: "Broadcast (EBU R128)".into(),
            codec: VideoCodec::DnxHr,
            quality: 0,
            audio_bitrate: 0,
            size: None,
            loudness_lufs: -23.0,
        }
    }

    pub fn master() -> Self {
        Self {
            name: "Master".into(),
            codec: VideoCodec::ProRes422,
            quality: 0,
            audio_bitrate: 0,
            size: None,
            loudness_lufs: -23.0,
        }
    }

    pub fn all() -> Vec<Preset> {
        vec![
            Self::youtube_4k(),
            Self::youtube_1080p(),
            Self::instagram_reel(),
            Self::broadcast_ebu(),
            Self::master(),
        ]
    }
}
