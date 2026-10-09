//! Point tracking (FX-06): follow a small luma patch from frame to frame by
//! normalised cross-correlation over a search window, with sub-pixel
//! refinement. Feeds keyframes for masks and transforms.

/// A tracker holding the reference patch and the last known position.
#[derive(Clone, Debug)]
pub struct Tracker {
    patch: Vec<f32>,
    size: usize,
    /// Centre of the patch in frame pixels.
    pub pos: [f32; 2],
    search: i32,
}

fn luma(rgba8: &[u8], w: usize, h: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(w * h);
    for p in rgba8.chunks_exact(4).take(w * h) {
        out.push(0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32);
    }
    out
}

/// Sample `img` at integer `(x, y)` clamped to the edges.
fn at(img: &[f32], w: usize, h: usize, x: i64, y: i64) -> f32 {
    let x = x.clamp(0, w as i64 - 1) as usize;
    let y = y.clamp(0, h as i64 - 1) as usize;
    img[y * w + x]
}

fn extract(img: &[f32], w: usize, h: usize, cx: f32, cy: f32, size: usize) -> Vec<f32> {
    let half = size as i64 / 2;
    let (x0, y0) = (cx.round() as i64 - half, cy.round() as i64 - half);
    let mut out = Vec::with_capacity(size * size);
    for y in 0..size as i64 {
        for x in 0..size as i64 {
            out.push(at(img, w, h, x0 + x, y0 + y));
        }
    }
    out
}

/// Normalised cross-correlation of two equally sized patches, -1..1.
fn ncc(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        let (p, q) = (x - ma, y - mb);
        dot += p * q;
        na += p * p;
        nb += q * q;
    }
    dot / (na.sqrt() * nb.sqrt()).max(1e-3)
}

impl Tracker {
    /// Start tracking the `size` x `size` patch centred on `center` in the
    /// first frame (`rgba8`, `w` x `h`), looking up to `search` pixels per step.
    pub fn new(rgba8: &[u8], w: u32, h: u32, center: [f32; 2], size: u32, search: u32) -> Self {
        let (w, h) = (w as usize, h as usize);
        let img = luma(rgba8, w, h);
        let size = (size.max(4) as usize) | 1;
        Self {
            patch: extract(&img, w, h, center[0], center[1], size),
            size,
            pos: center,
            search: search.max(1) as i32,
        }
    }

    /// Find the patch in the next frame. Returns the new centre and the match
    /// quality (normalised correlation, 1 = identical); the position is kept
    /// even for weak matches so the caller can decide to stop.
    pub fn step(&mut self, rgba8: &[u8], w: u32, h: u32) -> ([f32; 2], f32) {
        let (w, h) = (w as usize, h as usize);
        let img = luma(rgba8, w, h);
        let (cx, cy) = (self.pos[0].round() as i64, self.pos[1].round() as i64);
        let mut best = (0i64, 0i64, f32::MIN);
        let mut scores = std::collections::HashMap::new();
        for dy in -self.search as i64..=self.search as i64 {
            for dx in -self.search as i64..=self.search as i64 {
                let cand = extract(&img, w, h, (cx + dx) as f32, (cy + dy) as f32, self.size);
                let s = ncc(&self.patch, &cand);
                scores.insert((dx, dy), s);
                if s > best.2 {
                    best = (dx, dy, s);
                }
            }
        }
        let (bx, by, score) = best;
        // Sub-pixel refinement: fit a parabola through the neighbours.
        let refine = |m: f32, l: Option<&f32>, r: Option<&f32>| -> f32 {
            match (l, r) {
                (Some(&l), Some(&r)) => {
                    let denom = l - 2.0 * m + r;
                    if denom.abs() < 1e-6 {
                        0.0
                    } else {
                        (0.5 * (l - r) / denom).clamp(-0.5, 0.5)
                    }
                }
                _ => 0.0,
            }
        };
        let fx = refine(score, scores.get(&(bx - 1, by)), scores.get(&(bx + 1, by)));
        let fy = refine(score, scores.get(&(bx, by - 1)), scores.get(&(bx, by + 1)));
        self.pos = [(cx + bx) as f32 + fx, (cy + by) as f32 + fy];
        (self.pos, score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bright blob with a gradient on a textured background, at `(x, y)`.
    fn frame(w: u32, h: u32, x: f32, y: f32) -> Vec<u8> {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for j in 0..h {
            for i in 0..w {
                let bg = ((i * 7 + j * 13) % 23) as f32 * 2.0;
                let d = ((i as f32 - x).powi(2) + (j as f32 - y).powi(2)).sqrt();
                let blob = if d < 6.0 { 200.0 - d * 20.0 } else { 0.0 };
                let v = (bg + blob).min(255.0) as u8;
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        px
    }

    #[test]
    fn follows_a_moving_blob_within_a_pixel() {
        let (w, h) = (120, 80);
        let mut t = Tracker::new(&frame(w, h, 30.0, 40.0), w, h, [30.0, 40.0], 15, 6);
        let mut path = Vec::new();
        for k in 1..=10 {
            let (x, y) = (30.0 + 3.0 * k as f32, 40.0 - 2.0 * k as f32);
            let (pos, score) = t.step(&frame(w, h, x, y), w, h);
            assert!(score > 0.9, "step {k}: score {score}");
            assert!(
                (pos[0] - x).abs() < 1.0 && (pos[1] - y).abs() < 1.0,
                "step {k}: {pos:?} vs ({x}, {y})"
            );
            path.push(pos);
        }
        // Losing the target shows as a weak match.
        let (_, score) = t.step(&frame(w, h, 100.0, 10.0), w, h);
        assert!(score < 0.6, "lost: {score}");
    }
}
