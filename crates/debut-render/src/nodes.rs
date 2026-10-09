//! Per-pixel node parameters shared by the CPU reference and the GPU shaders:
//! shape masks (FX-04) and the chroma keyer (FX-05). The maths lives here once;
//! `shaders/mask.wgsl` and `shaders/key.wgsl` mirror it line for line.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskShape {
    Rectangle,
    Ellipse,
}

impl MaskShape {
    pub fn index(self) -> u32 {
        match self {
            MaskShape::Rectangle => 0,
            MaskShape::Ellipse => 1,
        }
    }
}

/// A soft-edged shape in output pixel space; coverage multiplies the layer's
/// alpha (premultiplied, so all four channels).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Mask {
    pub shape: MaskShape,
    /// Centre in pixels.
    pub center: [f32; 2],
    /// Half extents in pixels.
    pub half: [f32; 2],
    /// Width of the soft edge in pixels, centred on the shape boundary.
    pub feather: f32,
    pub invert: bool,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Mask {
    /// Coverage in `0..=1` at pixel centre `(x, y)`.
    pub fn coverage(&self, x: f32, y: f32) -> f32 {
        let dx = (x - self.center[0]).abs();
        let dy = (y - self.center[1]).abs();
        let hw = self.half[0].max(1e-3);
        let hh = self.half[1].max(1e-3);
        // Signed distance to the boundary in pixels: negative inside.
        let d = match self.shape {
            MaskShape::Rectangle => (dx - hw).max(dy - hh),
            MaskShape::Ellipse => {
                let r = ((dx / hw) * (dx / hw) + (dy / hh) * (dy / hh)).sqrt();
                (r - 1.0) * hw.min(hh)
            }
        };
        let m = if self.feather <= 0.0 {
            if d <= 0.0 {
                1.0
            } else {
                0.0
            }
        } else {
            1.0 - smoothstep(-0.5 * self.feather, 0.5 * self.feather, d)
        };
        if self.invert {
            1.0 - m
        } else {
            m
        }
    }
}

/// Chroma key on straight linear RGB: pixels near `key` in chroma become
/// transparent, with a soft band, and the key colour's spill is pulled out of
/// what remains.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ChromaKey {
    /// Linear RGB of the colour to remove.
    pub key: [f32; 3],
    /// Chroma distance that is fully keyed out.
    pub tolerance: f32,
    /// Width of the partial band past `tolerance`.
    pub softness: f32,
    /// 0 = leave spill, 1 = remove the key hue completely from kept pixels.
    pub spill: f32,
}

/// Rec.709 luma plus (Cb, Cr) chroma, on linear light; cheap and good enough
/// for keying since both backends agree.
fn ycbcr(c: [f32; 3]) -> (f32, [f32; 2]) {
    let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    (y, [(c[2] - y) / 1.8556, (c[0] - y) / 1.5748])
}

fn rgb(y: f32, cbcr: [f32; 2]) -> [f32; 3] {
    let b = y + cbcr[0] * 1.8556;
    let r = y + cbcr[1] * 1.5748;
    let g = (y - 0.2126 * r - 0.0722 * b) / 0.7152;
    [r, g, b]
}

impl ChromaKey {
    /// Returns the despilled straight RGB and the key alpha (0 = removed).
    pub fn apply(&self, c: [f32; 3]) -> ([f32; 3], f32) {
        let (_, kc) = ycbcr(self.key);
        let (y, cc) = ycbcr(c);
        let d = ((cc[0] - kc[0]).powi(2) + (cc[1] - kc[1]).powi(2)).sqrt();
        let alpha = smoothstep(self.tolerance, self.tolerance + self.softness.max(1e-4), d);
        let klen = (kc[0] * kc[0] + kc[1] * kc[1]).sqrt();
        if klen < 1e-4 || self.spill <= 0.0 {
            return (c, alpha);
        }
        let u = [kc[0] / klen, kc[1] / klen];
        let proj = (cc[0] * u[0] + cc[1] * u[1]).max(0.0) * self.spill;
        let out = rgb(y, [cc[0] - u[0] * proj, cc[1] - u[1] * proj]);
        (out.map(|v| v.max(0.0)), alpha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_mask_is_hard_without_feather_and_soft_with() {
        let m = Mask {
            shape: MaskShape::Rectangle,
            center: [50.0, 50.0],
            half: [10.0, 5.0],
            feather: 0.0,
            invert: false,
        };
        assert_eq!(m.coverage(50.5, 50.5), 1.0);
        assert_eq!(m.coverage(61.0, 50.5), 0.0);
        assert_eq!(m.coverage(50.5, 56.0), 0.0);
        let soft = Mask { feather: 4.0, ..m };
        assert!(
            (soft.coverage(60.0, 50.0) - 0.5).abs() < 1e-6,
            "edge is half"
        );
        assert_eq!(soft.coverage(57.0, 50.0), 1.0);
        assert_eq!(soft.coverage(63.0, 50.0), 0.0);
        let inv = Mask { invert: true, ..m };
        assert_eq!(inv.coverage(50.5, 50.5), 0.0);
    }

    #[test]
    fn ellipse_mask_follows_the_radius() {
        let m = Mask {
            shape: MaskShape::Ellipse,
            center: [0.0, 0.0],
            half: [20.0, 10.0],
            feather: 0.0,
            invert: false,
        };
        assert_eq!(m.coverage(19.0, 0.0), 1.0);
        assert_eq!(m.coverage(0.0, 9.0), 1.0);
        assert_eq!(
            m.coverage(15.0, 8.0),
            0.0,
            "outside the ellipse, inside its box"
        );
    }

    #[test]
    fn green_screen_keys_out_green_and_keeps_skin() {
        let k = ChromaKey {
            key: [0.0, 1.0, 0.0],
            tolerance: 0.2,
            softness: 0.1,
            spill: 1.0,
        };
        assert_eq!(k.apply([0.0, 1.0, 0.0]).1, 0.0);
        assert_eq!(k.apply([0.1, 0.9, 0.05]).1, 0.0, "near-green is keyed");
        let (skin, a) = k.apply([0.8, 0.5, 0.4]);
        assert_eq!(a, 1.0);
        // No green chroma in skin: the despill leaves it (nearly) alone.
        assert!((skin[0] - 0.8).abs() < 1e-3 && (skin[2] - 0.4).abs() < 1e-3);
        // A greenish grey edge keeps its luma but loses the green cast.
        let (edge, _) = k.apply([0.4, 0.6, 0.4]);
        assert!(edge[1] < 0.6 && edge[1] > 0.4, "{edge:?}");
        assert!(
            (edge[1] - edge[0]).abs() < 0.05,
            "despilled to neutral: {edge:?}"
        );
    }
}
