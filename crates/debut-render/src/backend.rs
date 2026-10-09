//! The op set every backend implements. Kept deliberately small: nodes are built
//! from these, so a new backend (or a conformance test between two) is bounded.

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
    /// Frame of `media` at source time `t`, as linear premultiplied RGBA.
    fn frame(&mut self, media: MediaId, t: Rational) -> Result<(u32, u32, Vec<Rgba>)>;
}

pub trait Backend {
    type Image: Clone;

    fn size(&self, img: &Self::Image) -> (u32, u32);
    fn solid(&mut self, w: u32, h: u32, color: Rgba) -> Self::Image;
    fn upload(&mut self, w: u32, h: u32, pixels: &[Rgba]) -> Self::Image;
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
    fn download(&mut self, img: &Self::Image) -> Vec<Rgba>;
}
