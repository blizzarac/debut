//! Reference backend: plain `f32` math on the CPU. Slow, obviously correct, and the
//! yardstick every GPU backend is compared against (PLT-04).

use crate::backend::{Backend, BlendMode, Rgba, Transform2D};
use crate::color::{ColorTransform, Grade};
use crate::lut::Lut3d;
use crate::nodes::{ChromaKey, Mask, PolyMask};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct CpuImage {
    pub w: u32,
    pub h: u32,
    pub px: Arc<Vec<Rgba>>,
}

impl CpuImage {
    fn at(&self, x: i64, y: i64) -> Rgba {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            [0.0; 4]
        } else {
            self.px[(y as u32 * self.w + x as u32) as usize]
        }
    }

    /// Bilinear sample at a continuous position in pixel space (pixel centres at +0.5).
    fn sample(&self, x: f32, y: f32) -> Rgba {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let p00 = self.at(x0, y0);
        let p10 = self.at(x0 + 1, y0);
        let p01 = self.at(x0, y0 + 1);
        let p11 = self.at(x0 + 1, y0 + 1);
        let mut out = [0.0; 4];
        for c in 0..4 {
            let top = p00[c] * (1.0 - tx) + p10[c] * tx;
            let bot = p01[c] * (1.0 - tx) + p11[c] * tx;
            out[c] = top * (1.0 - ty) + bot * ty;
        }
        out
    }
}

#[derive(Default)]
pub struct CpuBackend;

fn blend_px(b: Rgba, t: Rgba, mode: BlendMode, opacity: f32) -> Rgba {
    let t = [
        t[0] * opacity,
        t[1] * opacity,
        t[2] * opacity,
        t[3] * opacity,
    ];
    let mut out = [0.0; 4];
    match mode {
        // Premultiplied source-over.
        BlendMode::Normal => {
            for c in 0..4 {
                out[c] = t[c] + b[c] * (1.0 - t[3]);
            }
        }
        BlendMode::Add => {
            for c in 0..4 {
                out[c] = (t[c] + b[c]).min(1.0);
            }
        }
        BlendMode::Multiply => {
            for c in 0..3 {
                out[c] = t[c] * b[c] + t[c] * (1.0 - b[3]) + b[c] * (1.0 - t[3]);
            }
            out[3] = t[3] + b[3] * (1.0 - t[3]);
        }
        BlendMode::Screen => {
            for c in 0..3 {
                out[c] = t[c] + b[c] - t[c] * b[c];
            }
            out[3] = t[3] + b[3] * (1.0 - t[3]);
        }
    }
    out
}

impl Backend for CpuBackend {
    type Image = CpuImage;

    fn size(&self, img: &CpuImage) -> (u32, u32) {
        (img.w, img.h)
    }

    fn solid(&mut self, w: u32, h: u32, color: Rgba) -> CpuImage {
        CpuImage {
            w,
            h,
            px: Arc::new(vec![color; (w * h) as usize]),
        }
    }

    fn upload(&mut self, w: u32, h: u32, pixels: &[Rgba]) -> CpuImage {
        assert_eq!(pixels.len(), (w * h) as usize);
        CpuImage {
            w,
            h,
            px: Arc::new(pixels.to_vec()),
        }
    }

    fn transform(&mut self, src: &CpuImage, xf: &Transform2D, w: u32, h: u32) -> CpuImage {
        let mut px = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = xf.apply(x as f32 + 0.5, y as f32 + 0.5);
                px.push(src.sample(sx, sy));
            }
        }
        CpuImage {
            w,
            h,
            px: Arc::new(px),
        }
    }

    fn blend(
        &mut self,
        bottom: &CpuImage,
        top: &CpuImage,
        mode: BlendMode,
        opacity: f32,
    ) -> CpuImage {
        assert_eq!(
            (bottom.w, bottom.h),
            (top.w, top.h),
            "blend inputs must match"
        );
        let px = bottom
            .px
            .iter()
            .zip(top.px.iter())
            .map(|(b, t)| blend_px(*b, *t, mode, opacity))
            .collect();
        CpuImage {
            w: bottom.w,
            h: bottom.h,
            px: Arc::new(px),
        }
    }

    fn dissolve(&mut self, a: &CpuImage, b: &CpuImage, progress: f32) -> CpuImage {
        assert_eq!((a.w, a.h), (b.w, b.h), "dissolve inputs must match");
        let p = progress.clamp(0.0, 1.0);
        let px =
            a.px.iter()
                .zip(b.px.iter())
                .map(|(a, b)| {
                    [
                        a[0] * (1.0 - p) + b[0] * p,
                        a[1] * (1.0 - p) + b[1] * p,
                        a[2] * (1.0 - p) + b[2] * p,
                        a[3] * (1.0 - p) + b[3] * p,
                    ]
                })
                .collect();
        CpuImage {
            w: a.w,
            h: a.h,
            px: Arc::new(px),
        }
    }

    fn download(&mut self, img: &CpuImage) -> Vec<Rgba> {
        img.px.as_ref().clone()
    }

    fn color_transform(&mut self, src: &CpuImage, xf: &ColorTransform) -> CpuImage {
        map_rgb(src, |c| xf.apply_rgb(c))
    }

    fn lut3d(&mut self, src: &CpuImage, lut: &Lut3d) -> CpuImage {
        map_rgb(src, |c| lut.apply_rgb(c))
    }

    fn grade(&mut self, src: &CpuImage, grade: &Grade) -> CpuImage {
        map_rgb(src, |c| grade.apply_rgb(c))
    }

    fn mask(&mut self, src: &CpuImage, mask: &Mask) -> CpuImage {
        let w = src.w as usize;
        let px = src
            .px
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let m = mask.coverage((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                [p[0] * m, p[1] * m, p[2] * m, p[3] * m]
            })
            .collect();
        CpuImage {
            w: src.w,
            h: src.h,
            px: Arc::new(px),
        }
    }

    fn poly_mask(&mut self, src: &CpuImage, mask: &PolyMask) -> CpuImage {
        let w = src.w as usize;
        let px = src
            .px
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let m = mask.coverage((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                [p[0] * m, p[1] * m, p[2] * m, p[3] * m]
            })
            .collect();
        CpuImage {
            w: src.w,
            h: src.h,
            px: Arc::new(px),
        }
    }

    fn chroma_key(&mut self, src: &CpuImage, key: &ChromaKey) -> CpuImage {
        let px = src
            .px
            .iter()
            .map(|p| {
                let a = p[3];
                if a <= 0.0 {
                    return [0.0; 4];
                }
                let (c, k) = key.apply([p[0] / a, p[1] / a, p[2] / a]);
                let a = a * k;
                [c[0] * a, c[1] * a, c[2] * a, a]
            })
            .collect();
        CpuImage {
            w: src.w,
            h: src.h,
            px: Arc::new(px),
        }
    }
}

/// Apply `f` to straight (un-premultiplied) RGB, keeping alpha.
fn map_rgb(src: &CpuImage, f: impl Fn([f32; 3]) -> [f32; 3]) -> CpuImage {
    let px = src
        .px
        .iter()
        .map(|p| {
            let a = p[3];
            if a <= 0.0 {
                return [0.0; 4];
            }
            let o = f([p[0] / a, p[1] / a, p[2] / a]);
            [o[0] * a, o[1] * a, o[2] * a, a]
        })
        .collect();
    CpuImage {
        w: src.w,
        h: src.h,
        px: Arc::new(px),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Rgba, b: Rgba) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn source_over_is_premultiplied() {
        let out = blend_px(
            [0.0, 0.0, 1.0, 1.0],
            [0.5, 0.0, 0.0, 0.5],
            BlendMode::Normal,
            1.0,
        );
        assert!(close(out, [0.5, 0.0, 0.5, 1.0]));
        let half = blend_px(
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            BlendMode::Normal,
            0.5,
        );
        assert!(close(half, [0.5, 0.0, 0.5, 1.0]));
    }

    #[test]
    fn identity_transform_is_lossless() {
        let mut be = CpuBackend;
        let px: Vec<Rgba> = (0..16).map(|i| [i as f32 / 16.0, 0.25, 0.5, 1.0]).collect();
        let img = be.upload(4, 4, &px);
        let out = be.transform(&img, &Transform2D::IDENTITY, 4, 4);
        assert!(be
            .download(&out)
            .iter()
            .zip(&px)
            .all(|(a, b)| close(*a, *b)));
    }

    #[test]
    fn translate_moves_pixels_and_fills_transparent() {
        let mut be = CpuBackend;
        let mut px = vec![[0.0; 4]; 16];
        px[0] = [1.0, 1.0, 1.0, 1.0]; // top-left pixel
        let img = be.upload(4, 4, &px);
        let xf = Transform2D::from_srt((4, 4), (4, 4), (1.0, 1.0), 0.0, (1.0, 2.0));
        let t = be.transform(&img, &xf, 4, 4);
        let out = be.download(&t);
        assert!(close(out[2 * 4 + 1], [1.0, 1.0, 1.0, 1.0]));
        assert!(close(out[0], [0.0; 4]));
    }

    #[test]
    fn scale_two_fills_canvas_from_quarter() {
        let mut be = CpuBackend;
        let img = be.solid(2, 2, [0.2, 0.4, 0.6, 1.0]);
        let xf = Transform2D::from_srt((2, 2), (4, 4), (2.0, 2.0), 0.0, (0.0, 0.0));
        let t = be.transform(&img, &xf, 4, 4);
        let out = be.download(&t);
        // Centre pixels are fully inside the source.
        assert!(close(out[4 + 1], [0.2, 0.4, 0.6, 1.0]));
        assert!(close(out[2 * 4 + 2], [0.2, 0.4, 0.6, 1.0]));
    }

    #[test]
    fn rotation_by_quarter_turn_moves_corner() {
        let mut be = CpuBackend;
        let mut px = vec![[0.0; 4]; 9];
        px[2] = [1.0; 4]; // top-right of a 3x3
        let img = be.upload(3, 3, &px);
        let xf = Transform2D::from_srt(
            (3, 3),
            (3, 3),
            (1.0, 1.0),
            std::f32::consts::FRAC_PI_2,
            (0.0, 0.0),
        );
        let t = be.transform(&img, &xf, 3, 3);
        let out = be.download(&t);
        // Rotating +90° (y down) sends top-right to bottom-right.
        assert!(close(out[2 * 3 + 2], [1.0; 4]), "{out:?}");
    }
}
