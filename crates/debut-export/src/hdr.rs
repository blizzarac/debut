//! HDR10 / HLG output (EXP-06): the render's linear Rec.709 pixels become
//! Rec.2020, PQ- or HLG-encoded 16-bit RGBA for a 10-bit encoder, and the
//! encoder writes the matching colour tags and static metadata.
//!
//! Levels follow the render's own convention (`debut_render::color`):
//! scene-linear 1.0 is 100 nits, so an HDR source decoded through the colour
//! pipeline comes back out at the light level it went in with.

use debut_platform::codec::HdrTransfer;
use debut_render::color::{apply, encode, primaries_matrix, Primaries};
use debut_render::{Rgba, Transfer};

fn curve(t: HdrTransfer) -> Transfer {
    match t {
        HdrTransfer::Pq => Transfer::Pq,
        HdrTransfer::Hlg => Transfer::Hlg,
    }
}

/// Premultiplied linear Rec.709 → straight, full-range 16-bit RGBA in
/// Rec.2020 primaries with the HDR curve applied.
pub fn encode_rgba16(px: &[Rgba], transfer: HdrTransfer) -> Vec<u16> {
    let m = primaries_matrix(Primaries::Rec709, Primaries::Rec2020);
    let t = curve(transfer);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
    let mut out = Vec::with_capacity(px.len() * 4);
    for p in px {
        let a = p[3];
        let un = |c: f32| if a > 0.0 { c / a } else { 0.0 };
        let rgb = apply(&m, [un(p[0]), un(p[1]), un(p[2])]);
        for c in rgb {
            out.push(q(encode(t, c.max(0.0))));
        }
        out.push(q(a));
    }
    out
}

/// Content light levels of a programme (CTA-861.3), measured over the frames
/// as they are exported: the brightest pixel and the brightest frame average,
/// in nits (the max of R, G and B per pixel, as the spec defines it).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightLevels {
    pub max_cll: f32,
    pub max_fall: f32,
}

impl LightLevels {
    pub fn add_frame(&mut self, px: &[Rgba]) {
        if px.is_empty() {
            return;
        }
        let m = primaries_matrix(Primaries::Rec709, Primaries::Rec2020);
        let mut sum = 0.0f64;
        for p in px {
            let a = p[3];
            let un = |c: f32| if a > 0.0 { c / a } else { 0.0 };
            let [r, g, b] = apply(&m, [un(p[0]), un(p[1]), un(p[2])]);
            // PQ tops out at 10 000 nits.
            let nits = (r.max(g).max(b).max(0.0) * 100.0).min(10_000.0);
            self.max_cll = self.max_cll.max(nits);
            sum += nits as f64;
        }
        self.max_fall = self.max_fall.max((sum / px.len() as f64) as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode16(v: u16, t: HdrTransfer) -> f32 {
        debut_render::color::decode(curve(t), v as f32 / 65535.0)
    }

    #[test]
    fn pq_puts_reference_white_at_100_nits() {
        let out = encode_rgba16(&[[1.0, 1.0, 1.0, 1.0]], HdrTransfer::Pq);
        // 100 nits on the PQ curve is code 0.508 (520 of 1023 in 10-bit).
        let v = out[0] as f32 / 65535.0;
        assert!((v - 0.508).abs() < 0.002, "{v}");
        assert_eq!(out[3], 65535);
        assert!((decode16(out[1], HdrTransfer::Pq) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn highlights_above_white_survive_in_pq() {
        // 10x reference white = 1 000 nits: code 0.752 (769 of 1023).
        let out = encode_rgba16(&[[10.0, 10.0, 10.0, 1.0]], HdrTransfer::Pq);
        let v = out[0] as f32 / 65535.0;
        assert!((v - 0.752).abs() < 0.002, "{v}");
    }

    #[test]
    fn hlg_round_trips_and_clips_at_its_peak() {
        let out = encode_rgba16(&[[1.0, 1.0, 1.0, 1.0]], HdrTransfer::Hlg);
        assert!((decode16(out[0], HdrTransfer::Hlg) - 1.0).abs() < 2e-3);
        let peak = encode_rgba16(&[[50.0, 50.0, 50.0, 1.0]], HdrTransfer::Hlg);
        assert_eq!(peak[0], 65535);
    }

    #[test]
    fn rec709_red_moves_inside_rec2020() {
        let out = encode_rgba16(&[[1.0, 0.0, 0.0, 1.0]], HdrTransfer::Pq);
        // Pure 709 red is a mix in 2020: red dominates, green and blue are
        // small but not zero.
        let r = decode16(out[0], HdrTransfer::Pq);
        let g = decode16(out[1], HdrTransfer::Pq);
        let b = decode16(out[2], HdrTransfer::Pq);
        assert!((r - 0.627).abs() < 0.01, "{r}");
        assert!((g - 0.069).abs() < 0.01, "{g}");
        assert!((b - 0.016).abs() < 0.01, "{b}");
    }

    #[test]
    fn premultiplied_input_is_unpremultiplied() {
        let a = encode_rgba16(&[[0.5, 0.5, 0.5, 0.5]], HdrTransfer::Pq);
        let b = encode_rgba16(&[[1.0, 1.0, 1.0, 1.0]], HdrTransfer::Pq);
        assert_eq!(a[0], b[0]);
        assert_eq!(a[3], 32768);
    }

    #[test]
    fn measures_light_levels() {
        let mut l = LightLevels::default();
        l.add_frame(&[[1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]]);
        l.add_frame(&[[4.0, 4.0, 4.0, 1.0], [4.0, 4.0, 4.0, 1.0]]);
        assert!((l.max_cll - 400.0).abs() < 1.0);
        assert!((l.max_fall - 400.0).abs() < 1.0);
    }
}
