//! Title clips (GFX-01, GFX-02): text plus a style, rendered by `debut-graphics`.

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TitleStyle {
    /// Font family or file path; falls back to a system sans.
    pub font: String,
    pub size_px: f32,
    /// Straight sRGB RGBA.
    pub color: [u8; 4],
    pub align: TextAlign,
    pub line_height: f32,
    pub letter_spacing: f32,
    pub stroke_px: f32,
    pub stroke_color: [u8; 4],
    /// Offset of the drop shadow (0 = none); also its blur radius.
    pub shadow_px: f32,
    pub shadow_color: [u8; 4],
    /// Box behind the text; alpha 0 = none.
    pub background: [u8; 4],
    pub padding_px: f32,
    /// Widen the raster (and its box) to at least this many pixels; text aligns
    /// inside per `align`. 0 = fit the text.
    #[serde(default)]
    pub min_width_px: f32,
}

impl Default for TitleStyle {
    fn default() -> Self {
        Self {
            font: "DejaVu Sans".into(),
            size_px: 72.0,
            color: [255, 255, 255, 255],
            align: TextAlign::Center,
            line_height: 1.2,
            letter_spacing: 0.0,
            stroke_px: 0.0,
            stroke_color: [0, 0, 0, 255],
            shadow_px: 0.0,
            shadow_color: [0, 0, 0, 160],
            background: [0, 0, 0, 0],
            padding_px: 0.0,
            min_width_px: 0.0,
        }
    }
}

/// A title: text and style; position/scale/opacity come from the clip's
/// Transform effect like any other layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Title {
    pub text: String,
    pub style: TitleStyle,
}

impl Title {
    /// Content hash: the render cache key.
    pub fn hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut h);
        serde_json::to_string(&self.style)
            .unwrap_or_default()
            .hash(&mut h);
        h.finish()
    }
}
