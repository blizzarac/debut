//! The op set every backend implements. Kept deliberately small: nodes are built
//! from these, so a new backend (or a conformance test between two) is bounded.

use crate::color::{encode, ColorTransform, Grade, Transfer};
use crate::lut::Lut3d;
use crate::nodes::{ChromaKey, Mask, PolyMask};
use debut_core::{MediaId, Rational, Result};
use serde::{Deserialize, Serialize};

/// Linear-light, premultiplied RGBA.
pub type Rgba = [f32; 4];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BlendMode {
    Normal,
    Add,
    Multiply,
    Screen,
}

/// Maps output pixel centres back into the source: `src = M * out + t`. Built by
/// [`Transform2D::from_srt`] from scale, rotation and translation about the centre.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    pub m: [[f32; 2]; 2],
    pub t: [f32; 2],
}

impl Transform2D {
    pub const IDENTITY: Transform2D = Transform2D {
        m: [[1.0, 0.0], [0.0, 1.0]],
        t: [0.0, 0.0],
    };

    /// Forward scale/rotate/translate of a `src_w x src_h` image centred on a
    /// `dst_w x dst_h` canvas, inverted so backends sample destination -> source.
    pub fn from_srt(
        src: (u32, u32),
        dst: (u32, u32),
        scale: (f32, f32),
        rotation_rad: f32,
        translate: (f32, f32),
    ) -> Self {
        let (sc, ss) = (rotation_rad.cos(), rotation_rad.sin());
        // Forward: src_centred -> scale -> rotate -> translate -> dst_centred.
        // Inverse: dst_centred - translate -> rotate(-r) -> scale(1/s) -> src_centred.
        let inv_s = (1.0 / scale.0, 1.0 / scale.1);
        let m = [[sc * inv_s.0, ss * inv_s.0], [-ss * inv_s.1, sc * inv_s.1]];
        let dc = (dst.0 as f32 * 0.5, dst.1 as f32 * 0.5);
        let sc_ = (src.0 as f32 * 0.5, src.1 as f32 * 0.5);
        let ox = dc.0 + translate.0;
        let oy = dc.1 + translate.1;
        let t = [
            sc_.0 - (m[0][0] * ox + m[0][1] * oy),
            sc_.1 - (m[1][0] * ox + m[1][1] * oy),
        ];
        Transform2D { m, t }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.m[0][0] * x + self.m[0][1] * y + self.t[0],
            self.m[1][0] * x + self.m[1][1] * y + self.t[1],
        )
    }
}

/// Where decoded frames come from. The playback engine implements this over the
/// platform decoder and its frame cache; tests use synthetic sources.
pub trait FrameProvider {
    /// Frame of `media` at source time `t` as display-encoded, straight-alpha
    /// RGBA8 (what decoders produce); the graph's input transform linearizes it.
    fn frame(&mut self, media: MediaId, t: Rational) -> Result<(u32, u32, Vec<u8>)>;

    /// Run a third-party filter over `px` (`w x h` premultiplied RGBA, top
    /// row first) in place; `Ok(false)` when there is nothing to run it with.
    fn plugin(
        &mut self,
        _op: &crate::graph::PluginOp,
        _w: u32,
        _h: u32,
        _px: &mut [Rgba],
    ) -> Result<bool> {
        Ok(false)
    }
}

/// Straight RGBA8 -> premultiplied f32 (same encoding), for CPU paths.
pub fn rgba8_to_f32(rgba8: &[u8]) -> Vec<Rgba> {
    rgba8
        .chunks_exact(4)
        .map(|p| {
            let a = p[3] as f32 / 255.0;
            [
                p[0] as f32 / 255.0 * a,
                p[1] as f32 / 255.0 * a,
                p[2] as f32 / 255.0 * a,
                a,
            ]
        })
        .collect()
}

pub trait Backend {
    type Image: Clone;

    fn size(&self, img: &Self::Image) -> (u32, u32);
    fn solid(&mut self, w: u32, h: u32, color: Rgba) -> Self::Image;
    fn upload(&mut self, w: u32, h: u32, pixels: &[Rgba]) -> Self::Image;
    /// Upload straight-alpha RGBA8 (decoded video); backends keep it 8-bit until
    /// a pass reads it, so a 4K frame costs 32 MB, not 128.
    fn upload_rgba8(&mut self, w: u32, h: u32, pixels: &[u8]) -> Self::Image {
        self.upload(w, h, &rgba8_to_f32(pixels))
    }
    /// Resample `src` into a `w x h` image through `xf` (bilinear, transparent outside).
    fn transform(&mut self, src: &Self::Image, xf: &Transform2D, w: u32, h: u32) -> Self::Image;
    /// Composite `top` over `bottom`; both the same size.
    fn blend(
        &mut self,
        bottom: &Self::Image,
        top: &Self::Image,
        mode: BlendMode,
        opacity: f32,
    ) -> Self::Image;
    /// Cross-dissolve: `a * (1 - p) + b * p`.
    fn dissolve(&mut self, a: &Self::Image, b: &Self::Image, progress: f32) -> Self::Image;
    /// Color-space conversion on un-premultiplied RGB (FX-08).
    fn color_transform(&mut self, src: &Self::Image, xf: &ColorTransform) -> Self::Image;
    /// 3D LUT on un-premultiplied RGB (FX-11).
    fn lut3d(&mut self, src: &Self::Image, lut: &Lut3d) -> Self::Image;
    /// Primary grade on un-premultiplied scene-linear RGB (FX-09).
    fn grade(&mut self, src: &Self::Image, grade: &Grade) -> Self::Image;
    /// Multiply by a shape's coverage (FX-04).
    fn mask(&mut self, src: &Self::Image, mask: &Mask) -> Self::Image;
    /// Multiply by a polygon's coverage (FX-04).
    fn poly_mask(&mut self, src: &Self::Image, mask: &PolyMask) -> Self::Image;
    /// Chroma key with despill (FX-05).
    fn chroma_key(&mut self, src: &Self::Image, key: &ChromaKey) -> Self::Image;
    fn download(&mut self, img: &Self::Image) -> Vec<Rgba>;
    /// Read back as straight-alpha 8-bit, encoded with `transfer` (the viewer's
    /// and the encoder's input). Backends override this to do it on the GPU.
    fn download_rgba8(&mut self, img: &Self::Image, transfer: Transfer) -> Vec<u8> {
        encode_rgba8(&self.download(img), transfer)
    }
}

/// Premultiplied linear-ish f32 -> straight, `transfer`-encoded RGBA8.
pub fn encode_rgba8(px: &[Rgba], transfer: Transfer) -> Vec<u8> {
    let mut out = Vec::with_capacity(px.len() * 4);
    for p in px {
        let a = p[3];
        let un = |c: f32| if a > 0.0 { c / a } else { 0.0 };
        let q = |c: f32| (encode(transfer, un(c)).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        out.push(q(p[0]));
        out.push(q(p[1]));
        out.push(q(p[2]));
        out.push((a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    }
    out
}
