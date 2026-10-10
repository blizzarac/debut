//! Shape clips (GFX-03): vector rectangles, ellipses, polygons, stars, arrows
//! and lines with a fill and an outline, rendered by `debut-graphics`.

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ShapeKind {
    /// `corner_px` rounds the corners (clamped to half the short side).
    Rectangle {
        corner_px: f32,
    },
    Ellipse,
    /// A regular polygon with `sides` corners, point up.
    Polygon {
        sides: u32,
    },
    /// A star with `points` tips; `inner` is the inner radius as a fraction
    /// of the outer.
    Star {
        points: u32,
        inner: f32,
    },
    /// A right-pointing arrow; `head` is the head's length as a fraction of
    /// the width, `shaft` the shaft's thickness as a fraction of the height.
    Arrow {
        head: f32,
        shaft: f32,
    },
    /// A straight line across the box, left to right; drawn with the stroke.
    Line,
}

/// How a shape is filled. Colours are straight sRGB RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Fill {
    None,
    Solid {
        color: [u8; 4],
    },
    /// Linear gradient across the shape's box at `angle_deg` (0 = left to
    /// right, 90 = top to bottom).
    Linear {
        from: [u8; 4],
        to: [u8; 4],
        angle_deg: f32,
    },
}

/// A shape authored in sequence pixels; position, scale, rotation and opacity
/// come from the clip's Transform effect like any other layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    #[serde(flatten)]
    pub kind: ShapeKind,
    pub width: f32,
    pub height: f32,
    pub fill: Fill,
    /// Outline width, centred on the edge; 0 = none.
    pub stroke_px: f32,
    pub stroke_color: [u8; 4],
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            kind: ShapeKind::Rectangle { corner_px: 0.0 },
            width: 400.0,
            height: 225.0,
            fill: Fill::Solid {
                color: [255, 255, 255, 255],
            },
            stroke_px: 0.0,
            stroke_color: [0, 0, 0, 255],
        }
    }
}

impl Shape {
    /// Content hash: the render cache key.
    pub fn hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        "shape".hash(&mut h);
        serde_json::to_string(self).unwrap_or_default().hash(&mut h);
        h.finish()
    }

    /// A short label for timelines and interchange formats.
    pub fn label(&self) -> &'static str {
        match self.kind {
            ShapeKind::Rectangle { .. } => "Rectangle",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Polygon { .. } => "Polygon",
            ShapeKind::Star { .. } => "Star",
            ShapeKind::Arrow { .. } => "Arrow",
            ShapeKind::Line => "Line",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_a_kind_tag() {
        let s = Shape {
            kind: ShapeKind::Star {
                points: 5,
                inner: 0.5,
            },
            ..Shape::default()
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["kind"], "star");
        assert_eq!(json["points"], 5);
        assert_eq!(json["fill"]["kind"], "solid");
        let back: Shape = serde_json::from_value(json).unwrap();
        assert_eq!(back, s);
        assert_ne!(s.hash(), Shape::default().hash());
    }
}
