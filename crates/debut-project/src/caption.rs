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

impl TitleStyle {
    /// The burn-in look for captions on a 1080-line frame: white 48 px on a
    /// translucent box.
    pub fn captions() -> Self {
        Self::captions_for_height(1080)
    }

    /// The same look scaled to a frame `height` lines tall, so captions take
    /// the same share of the picture at any sequence size.
    pub fn captions_for_height(height: u32) -> Self {
        let k = height as f32 / 1080.0;
        Self {
            size_px: (48.0 * k).max(8.0),
            align: TextAlign::Center,
            background: [0, 0, 0, 150],
            padding_px: (12.0 * k).max(2.0),
            ..Self::default()
        }
    }
}
