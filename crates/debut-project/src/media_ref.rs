//! A reference to source media plus its metadata and proxy state (MED-04, MED-05, MED-06).

use debut_core::id::MediaId;
use debut_core::{color::ColorSpace, FrameRate, Timecode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaRef {
    pub id: MediaId,
    pub path: String,
    pub online: bool,
    pub metadata: MediaMetadata,
    pub proxies: Vec<Proxy>,
    /// Free-form keywords for search and smart bins (MED-08).
    #[serde(default)]
    pub keywords: Vec<String>,
    /// 0 = unrated, else 1..=5 stars (MED-08).
    #[serde(default)]
    pub rating: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaMetadata {
    pub frame_rate: Option<FrameRate>,
    pub start_timecode: Option<Timecode>,
    pub reel: Option<String>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub color_space: Option<ColorSpace>,
    pub audio_channels: u16,
    /// True for VFR sources; conform rules apply on the timeline (MED-03).
    pub variable_frame_rate: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proxy {
    /// 2, 4 or 8 for 1/2, 1/4, 1/8 resolution.
    pub divisor: u8,
    pub path: String,
}
