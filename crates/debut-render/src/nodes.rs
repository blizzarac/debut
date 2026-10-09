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

/// Most outline vertices a polygon mask renders with (the GPU uniform holds
/// this many); curved outlines are flattened to fit.
pub const POLY_MAX_POINTS: usize = 64;

/// Most points a user may place on a polygon mask; curves between them are
/// flattened into the remaining vertex budget.
pub const POLY_MAX_EDIT_POINTS: usize = 32;

/// Flatten a closed outline whose points carry cubic Bézier handles (FX-04)
/// into at most [`POLY_MAX_POINTS`] vertices. `handles[i]` is `[in_x, in_y,
/// out_x, out_y]` relative to `points[i]`; a missing or zero pair makes that
/// side of the point a corner. Segments without handles stay straight, so a
/// plain polygon comes back unchanged.
pub fn flatten_outline(points: &[[f32; 2]], handles: &[[f32; 4]]) -> Vec<[f32; 2]> {
    let n = points.len().min(POLY_MAX_EDIT_POINTS);
    let h = |i: usize| handles.get(i).copied().unwrap_or([0.0; 4]);
    let curved = |i: usize| {
        let (out, inn) = (h(i), h((i + 1) % n));
        out[2] != 0.0 || out[3] != 0.0 || inn[0] != 0.0 || inn[1] != 0.0
    };
    let c = (0..n).filter(|&i| curved(i)).count();
    if c == 0 {
        return points[..n].to_vec();
    }
    // Every straight segment adds its start vertex; curves share the rest.
    let steps = ((POLY_MAX_POINTS - (n - c)) / c).clamp(1, 12);
    let mut out = Vec::with_capacity(POLY_MAX_POINTS);
    for i in 0..n {
        let p0 = points[i];
        out.push(p0);
        if !curved(i) {
            continue;
        }
        let p3 = points[(i + 1) % n];
        let p1 = [p0[0] + h(i)[2], p0[1] + h(i)[3]];
        let hn = h((i + 1) % n);
        let p2 = [p3[0] + hn[0], p3[1] + hn[1]];
        for k in 1..steps {
            let t = k as f32 / steps as f32;
            let u = 1.0 - t;
            let (a, b, c2, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            out.push([
                a * p0[0] + b * p1[0] + c2 * p2[0] + d * p3[0],
                a * p0[1] + b * p1[1] + c2 * p2[1] + d * p3[1],
            ]);
        }
    }
    out.truncate(POLY_MAX_POINTS);
    out
}

/// Handles that make a smooth closed curve through `points` (Catmull-Rom
/// tangents: each handle is a sixth of the chord between the neighbours).
pub fn smooth_handles(points: &[[f32; 2]]) -> Vec<[f32; 4]> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[(i + n - 1) % n], points[(i + 1) % n]);
            let (dx, dy) = ((b[0] - a[0]) / 6.0, (b[1] - a[1]) / 6.0);
            [-dx, -dy, dx, dy]
        })
        .collect()
}

/// A closed polygon mask in output pixel space with a feathered edge (FX-04).
/// Coverage multiplies the layer's alpha like [`Mask`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PolyMask {
    /// At least three vertices, in order; the last joins back to the first.
    pub points: Vec<[f32; 2]>,
    pub feather: f32,
    pub invert: bool,
}

impl PolyMask {
    /// Signed distance to the outline: negative inside (even-odd rule).
    pub fn signed_distance(&self, x: f32, y: f32) -> f32 {
        let n = self.points.len();
        if n < 3 {
            return f32::MAX;
        }
        let mut inside = false;
        let mut best = f32::MAX;
        for i in 0..n {
            let a = self.points[i];
            let b = self.points[(i + 1) % n];
            // Even-odd crossing test.
            if (a[1] > y) != (b[1] > y) {
                let t = (y - a[1]) / (b[1] - a[1]);
                if x < a[0] + t * (b[0] - a[0]) {
                    inside = !inside;
                }
            }
            // Distance to the segment.
            let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
            let len2 = (ex * ex + ey * ey).max(1e-12);
            let t = (((x - a[0]) * ex + (y - a[1]) * ey) / len2).clamp(0.0, 1.0);
            let (px, py) = (a[0] + t * ex - x, a[1] + t * ey - y);
            best = best.min((px * px + py * py).sqrt());
        }
        if inside {
            -best
        } else {
            best
        }
    }

    pub fn coverage(&self, x: f32, y: f32) -> f32 {
        let d = self.signed_distance(x, y);
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
    fn outlines_flatten_curves_and_keep_corners() {
        let square = [[-12.0, -12.0], [12.0, -12.0], [12.0, 12.0], [-12.0, 12.0]];
        assert_eq!(
            flatten_outline(&square, &[]),
            square.to_vec(),
            "no handles: unchanged"
        );
        let h = smooth_handles(&square);
        assert_eq!(h[0], [-4.0, 4.0, 4.0, -4.0]);
        let round = flatten_outline(&square, &h);
        assert!(round.len() <= POLY_MAX_POINTS && round.len() > 16);
        // Passes through the points and bulges to -15 at the top midpoint.
        assert!(square.iter().all(|p| round.contains(p)));
        let top = round.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert!((top + 15.0).abs() < 0.1, "{top}");
        // One curved side among corners: the rest stays straight.
        let mut one = vec![[0.0; 4]; 4];
        one[0] = [0.0, 0.0, 4.0, -4.0];
        let shape = flatten_outline(&square, &one);
        assert!(shape.len() > 4 && shape.len() <= POLY_MAX_POINTS);
        assert_eq!(&shape[shape.len() - 3..], &square[1..]);
    }

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
    fn polygon_mask_uses_even_odd_inside_and_edge_distance() {
        // A square 10..30 with a square hole 15..25 (second loop reversed).
        let m = PolyMask {
            points: vec![
                [10.0, 10.0],
                [30.0, 10.0],
                [30.0, 30.0],
                [10.0, 30.0],
                [10.0, 10.0],
                [15.0, 15.0],
                [15.0, 25.0],
                [25.0, 25.0],
                [25.0, 15.0],
                [15.0, 15.0],
            ],
            feather: 0.0,
            invert: false,
        };
        assert_eq!(m.coverage(12.0, 20.0), 1.0, "in the ring");
        assert_eq!(m.coverage(20.0, 20.0), 0.0, "in the hole");
        assert_eq!(m.coverage(5.0, 20.0), 0.0, "outside");
        assert!((m.signed_distance(12.0, 20.0) + 2.0).abs() < 1e-5);
        assert!((m.signed_distance(5.0, 20.0) - 5.0).abs() < 1e-5);
        let tri = PolyMask {
            points: vec![[0.0, 0.0], [40.0, 0.0], [0.0, 40.0]],
            feather: 4.0,
            invert: true,
        };
        assert_eq!(tri.coverage(5.0, 5.0), 0.0, "inverted: inside is cut");
        assert!(
            (tri.coverage(20.0, 20.0) - 0.5).abs() < 1e-6,
            "on the hypotenuse"
        );
        assert_eq!(
            PolyMask {
                points: vec![[0.0, 0.0]],
                feather: 0.0,
                invert: false
            }
            .coverage(0.0, 0.0),
            0.0
        );
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
