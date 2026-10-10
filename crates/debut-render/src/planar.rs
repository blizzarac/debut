//! Planar tracking (FX-06): follow a flat surface (a screen, a sign, a wall)
//! through perspective changes. Several textured points inside the region are
//! tracked with the point tracker; each frame a homography is fitted from
//! their reference positions to where they are now, points that disagree with
//! it are dropped, and it is fitted again on the rest. The homography maps
//! the reference frame's plane to the current frame, so anything drawn on
//! the plane (a mask outline) follows the surface.

use crate::tracking::Tracker;

/// A 3x3 homography, row-major, normalised so the last element is 1.
pub type Homography = [f64; 9];

pub const IDENTITY: Homography = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// Map `p` through `h`.
pub fn apply(h: &Homography, p: [f64; 2]) -> [f64; 2] {
    let w = h[6] * p[0] + h[7] * p[1] + h[8];
    let w = if w.abs() < 1e-12 { 1e-12 } else { w };
    [
        (h[0] * p[0] + h[1] * p[1] + h[2]) / w,
        (h[3] * p[0] + h[4] * p[1] + h[5]) / w,
    ]
}

fn mul(a: &Homography, b: &Homography) -> Homography {
    let mut out = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    out
}

fn normalise(h: Homography) -> Option<Homography> {
    (h[8].abs() > 1e-12 && h.iter().all(|v| v.is_finite())).then(|| h.map(|v| v / h[8]))
}

/// Inverse of `h`, if it has one.
pub fn invert(h: &Homography) -> Option<Homography> {
    let [a, b, c, d, e, f, g, i, k] = *h;
    let det = a * (e * k - f * i) - b * (d * k - f * g) + c * (d * i - e * g);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = [
        (e * k - f * i) / det,
        (c * i - b * k) / det,
        (b * f - c * e) / det,
        (f * g - d * k) / det,
        (a * k - c * g) / det,
        (c * d - a * f) / det,
        (d * i - e * g) / det,
        (b * g - a * i) / det,
        (a * e - b * d) / det,
    ];
    normalise(inv)
}

/// Element-wise blend, for interpolating between keyed homographies a frame
/// apart.
pub fn lerp(a: &Homography, b: &Homography, t: f64) -> Homography {
    let mut out = [0.0; 9];
    for i in 0..9 {
        out[i] = a[i] + (b[i] - a[i]) * t;
    }
    out
}

/// Similarity transform moving the points' centroid to 0 and their mean
/// distance to √2 (Hartley normalisation), so the fit is well conditioned.
fn conditioner(pts: &[[f64; 2]]) -> Homography {
    let n = pts.len() as f64;
    let (cx, cy) = (
        pts.iter().map(|p| p[0]).sum::<f64>() / n,
        pts.iter().map(|p| p[1]).sum::<f64>() / n,
    );
    let mean = pts
        .iter()
        .map(|p| ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt())
        .sum::<f64>()
        / n;
    let s = if mean > 1e-12 {
        2f64.sqrt() / mean
    } else {
        1.0
    };
    [s, 0.0, -s * cx, 0.0, s, -s * cy, 0.0, 0.0, 1.0]
}

/// Solve the 8x8 system `m x = v` by Gaussian elimination with pivoting.
fn solve8(mut m: [[f64; 8]; 8], mut v: [f64; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let pivot = (col..8).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[pivot][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, pivot);
        v.swap(col, pivot);
        for row in col + 1..8 {
            let f = m[row][col] / m[col][col];
            let pivot_row = m[col];
            for (k, cell) in m[row].iter_mut().enumerate().skip(col) {
                *cell -= f * pivot_row[k];
            }
            v[row] -= f * v[col];
        }
    }
    let mut x = [0.0; 8];
    for row in (0..8).rev() {
        let s: f64 = (row + 1..8).map(|k| m[row][k] * x[k]).sum();
        x[row] = (v[row] - s) / m[row][row];
    }
    Some(x)
}

/// Least-squares homography taking each `src[i]` to `dst[i]` (at least four
/// pairs, not all on a line).
pub fn fit(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Option<Homography> {
    if src.len() < 4 || src.len() != dst.len() {
        return None;
    }
    let (ts, td) = (conditioner(src), conditioner(dst));
    let mut ata = [[0.0; 8]; 8];
    let mut atb = [0.0; 8];
    for (s, d) in src.iter().zip(dst) {
        let [x, y] = apply(&ts, *s);
        let [u, v] = apply(&td, *d);
        let rows = [
            ([x, y, 1.0, 0.0, 0.0, 0.0, -x * u, -y * u], u),
            ([0.0, 0.0, 0.0, x, y, 1.0, -x * v, -y * v], v),
        ];
        for (r, b) in rows {
            for i in 0..8 {
                atb[i] += r[i] * b;
                for j in 0..8 {
                    ata[i][j] += r[i] * r[j];
                }
            }
        }
    }
    let h = solve8(ata, atb)?;
    let hn = [h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], 1.0];
    normalise(mul(&invert(&td)?, &mul(&hn, &ts)))
}

fn inside(poly: &[[f32; 2]], x: f32, y: f32) -> bool {
    let mut c = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a[1] > y) != (b[1] > y) && x < a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            c = !c;
        }
    }
    c
}

/// Tracks a polygon-shaped surface region.
pub struct PlanarTracker {
    /// Each tracked point with its position in the reference frame.
    points: Vec<(Tracker, [f64; 2])>,
}

impl PlanarTracker {
    /// Most points tracked inside the region.
    pub const MAX_POINTS: usize = 16;
    /// Patch size and search radius of each point, in frame pixels.
    const PATCH: u32 = 15;
    const SEARCH: u32 = 8;

    /// Pick textured points inside `region` (frame pixels) of the reference
    /// frame. `None` when fewer than four usable points are found.
    pub fn new(rgba8: &[u8], w: u32, h: u32, region: &[[f32; 2]]) -> Option<Self> {
        if region.len() < 3 {
            return None;
        }
        let luma = |x: i64, y: i64| -> f32 {
            let x = x.clamp(0, w as i64 - 1) as usize;
            let y = y.clamp(0, h as i64 - 1) as usize;
            let p = &rgba8[(y * w as usize + x) * 4..];
            0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
        };
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in region {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        let half = Self::PATCH as i64 / 2;
        let margin = half as f32 + Self::SEARCH as f32;
        let (x0, y0) = (x0.max(margin), y0.max(margin));
        let (x1, y1) = (x1.min(w as f32 - margin), y1.min(h as f32 - margin));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        // Candidates on a grid, scored by patch contrast.
        const GRID: usize = 7;
        let mut cands = Vec::new();
        for j in 0..GRID {
            for i in 0..GRID {
                let x = x0 + (x1 - x0) * (i as f32 + 0.5) / GRID as f32;
                let y = y0 + (y1 - y0) * (j as f32 + 0.5) / GRID as f32;
                if !inside(region, x, y) {
                    continue;
                }
                let (cx, cy) = (x.round() as i64, y.round() as i64);
                let vals: Vec<f32> = (-half..=half)
                    .flat_map(|dy| (-half..=half).map(move |dx| (dx, dy)))
                    .map(|(dx, dy)| luma(cx + dx, cy + dy))
                    .collect();
                let mean = vals.iter().sum::<f32>() / vals.len() as f32;
                let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / vals.len() as f32;
                if var > 40.0 {
                    cands.push((var, [x, y]));
                }
            }
        }
        cands.sort_by(|a, b| b.0.total_cmp(&a.0));
        cands.truncate(Self::MAX_POINTS);
        if cands.len() < 4 {
            return None;
        }
        let points = cands
            .into_iter()
            .map(|(_, p)| {
                (
                    Tracker::new(rgba8, w, h, p, Self::PATCH, Self::SEARCH),
                    [p[0] as f64, p[1] as f64],
                )
            })
            .collect();
        Some(Self { points })
    }

    pub fn points(&self) -> usize {
        self.points.len()
    }

    /// Track into the next frame. Returns the homography from the reference
    /// frame to this one and the share of points that agree with it; `None`
    /// once fewer than four points can be trusted.
    pub fn step(&mut self, rgba8: &[u8], w: u32, h: u32) -> Option<(Homography, f32)> {
        const MIN_SCORE: f32 = 0.6;
        const MAX_ERROR: f64 = 2.0;
        let mut pairs = Vec::new();
        for (i, (t, r)) in self.points.iter_mut().enumerate() {
            let (p, score) = t.step(rgba8, w, h);
            if score >= MIN_SCORE {
                pairs.push((i, *r, [p[0] as f64, p[1] as f64]));
            }
        }
        if pairs.len() < 4 {
            return None;
        }
        let total = pairs.len();
        let src: Vec<[f64; 2]> = pairs.iter().map(|p| p.1).collect();
        let dst: Vec<[f64; 2]> = pairs.iter().map(|p| p.2).collect();
        let h0 = fit(&src, &dst)?;
        let err = |h: &Homography, p: &(usize, [f64; 2], [f64; 2])| {
            let q = apply(h, p.1);
            ((q[0] - p.2[0]).powi(2) + (q[1] - p.2[1]).powi(2)).sqrt()
        };
        let inliers: Vec<_> = pairs
            .iter()
            .filter(|p| err(&h0, p) <= MAX_ERROR)
            .cloned()
            .collect();
        if inliers.len() < 4 {
            return None;
        }
        let src: Vec<[f64; 2]> = inliers.iter().map(|p| p.1).collect();
        let dst: Vec<[f64; 2]> = inliers.iter().map(|p| p.2).collect();
        let h1 = fit(&src, &dst)?;
        // Points that drifted off the plane are put back where the plane says.
        for (i, (t, r)) in self.points.iter_mut().enumerate() {
            if !inliers.iter().any(|p| p.0 == i) {
                let q = apply(&h1, *r);
                t.pos = [q[0] as f32, q[1] as f32];
            }
        }
        Some((h1, inliers.len() as f32 / total as f32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
        (a[0] - b[0]).abs() < tol && (a[1] - b[1]).abs() < tol
    }

    #[test]
    fn fits_exact_homographies_and_inverts() {
        let h: Homography = [1.1, 0.05, 12.0, -0.03, 0.95, -7.0, 0.0004, -0.0002, 1.0];
        let src = [
            [10.0, 10.0],
            [200.0, 15.0],
            [190.0, 140.0],
            [5.0, 150.0],
            [100.0, 80.0],
        ];
        let dst: Vec<[f64; 2]> = src.iter().map(|p| apply(&h, *p)).collect();
        let got = fit(&src, &dst).unwrap();
        for p in src {
            assert!(close(apply(&got, p), apply(&h, p), 1e-6));
        }
        let back = invert(&got).unwrap();
        assert!(close(
            apply(&back, apply(&got, [50.0, 60.0])),
            [50.0, 60.0],
            1e-6
        ));
        assert!(fit(&src[..3], &dst[..3]).is_none(), "needs four points");
        let line = [[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0]];
        assert!(fit(&line, &line).is_none(), "collinear");
    }

    /// A noisy texture seen through `h` (frame pixel -> texture pixel by the
    /// inverse), 200x150.
    fn view(h: &Homography) -> Vec<u8> {
        let (w, hh) = (200u32, 150u32);
        let inv = invert(h).unwrap();
        let tex = |x: f64, y: f64| -> f64 {
            // Blocky noise with a little smooth variation: trackable texture.
            let (bx, by) = ((x / 5.0).floor() as i64, (y / 5.0).floor() as i64);
            let n = ((bx * 73_856_093) ^ (by * 19_349_663)).rem_euclid(1000) as f64 / 1000.0;
            40.0 + 160.0 * n + 20.0 * ((x * 0.07).sin() + (y * 0.05).cos())
        };
        let mut px = Vec::with_capacity((w * hh * 4) as usize);
        for y in 0..hh {
            for x in 0..w {
                let [u, v] = apply(&inv, [x as f64 + 0.5, y as f64 + 0.5]);
                let l = tex(u, v).clamp(0.0, 255.0) as u8;
                px.extend_from_slice(&[l, l, l, 255]);
            }
        }
        px
    }

    #[test]
    fn follows_a_surface_through_perspective_changes() {
        let region = [
            [50.0f32, 35.0],
            [150.0, 35.0],
            [150.0, 115.0],
            [50.0, 115.0],
        ];
        let mut tracker = PlanarTracker::new(&view(&IDENTITY), 200, 150, &region).unwrap();
        assert!(tracker.points() >= 8);
        for k in 1..=12 {
            let t = k as f64;
            // Drift right and up, turn a little, and tilt in depth.
            let truth: Homography = [
                1.0 + 0.004 * t,
                0.01 * t,
                1.5 * t,
                -0.008 * t,
                1.0 - 0.003 * t,
                -0.8 * t,
                0.00008 * t,
                0.00004 * t,
                1.0,
            ];
            let (h, agree) = tracker.step(&view(&truth), 200, 150).unwrap();
            assert!(agree > 0.6, "frame {k}: {agree}");
            for c in region {
                let c = [c[0] as f64, c[1] as f64];
                assert!(
                    close(apply(&h, c), apply(&truth, c), 1.0),
                    "frame {k}: corner {c:?} at {:?}, truth {:?}",
                    apply(&h, c),
                    apply(&truth, c)
                );
            }
        }
    }
}
