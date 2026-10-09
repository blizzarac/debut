//! Captions / subtitles on a sequence (GFX-05, GFX-06): timed text that is
//! burned into the picture by the renderer and round-trips through SRT.

use crate::title::{TextAlign, TitleStyle};
use debut_core::id::CaptionId;
use debut_core::Rational;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub id: CaptionId,
    /// Sequence time, inclusive start and exclusive end.
    pub start: Rational,
    pub end: Rational,
    pub text: String,
}

impl Caption {
    pub fn new(id: CaptionId, start: Rational, end: Rational, text: impl Into<String>) -> Self {
        Self {
            id,
            start,
            end,
            text: text.into(),
        }
    }

    pub fn contains(&self, t: Rational) -> bool {
        self.start <= t && t < self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionPosition {
    Bottom,
    Top,
}

/// How a sequence burns its captions in (GFX-06). Sizes are for a 1080-line
/// frame and scale with the sequence height.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionSettings {
    /// Render into the picture (viewer and export). Off = sidecar only.
    pub burn_in: bool,
    pub position: CaptionPosition,
    pub font: String,
    pub size_px: f32,
    pub color: [u8; 4],
    pub background: [u8; 4],
}

impl Default for CaptionSettings {
    fn default() -> Self {
        Self {
            burn_in: true,
            position: CaptionPosition::Bottom,
            font: "DejaVu Sans".into(),
            size_px: 48.0,
            color: [255, 255, 255, 255],
            background: [0, 0, 0, 150],
        }
    }
}

impl CaptionSettings {
    /// The title style for a frame `height` lines tall.
    pub fn style_for_height(&self, height: u32) -> TitleStyle {
        let k = height as f32 / 1080.0;
        TitleStyle {
            font: self.font.clone(),
            size_px: (self.size_px * k).max(8.0),
            color: self.color,
            align: TextAlign::Center,
            background: self.background,
            padding_px: (12.0 * k).max(2.0),
            ..TitleStyle::default()
        }
    }
}

impl TitleStyle {
    /// The default burn-in look for captions on a 1080-line frame: white 48 px
    /// on a translucent box.
    pub fn captions() -> Self {
        Self::captions_for_height(1080)
    }

    /// The default look scaled to a frame `height` lines tall, so captions take
    /// the same share of the picture at any sequence size.
    pub fn captions_for_height(height: u32) -> Self {
        CaptionSettings::default().style_for_height(height)
    }
}
