//! Managed color (FX-08, FX-09): transfer functions, primaries and the primary
//! grade, as plain math shared by the CPU backend and mirrored in WGSL. The
//! working space is scene-linear Rec.709 / D65. Matrices are the published
//! RGB→XYZ matrices, with Bradford adaptation from the ACES D60 white to D65;
//! HDR scene scales treat 100 nits as 1.0.

// Standards constants are kept verbatim rather than truncated to f32 digits.
#![allow(clippy::excessive_precision)]

use debut_core::color::ColorSpace;
use debut_core::{Error, Result};
use serde::{Deserialize, Serialize};

pub type Mat3 = [[f32; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Bradford chromatic adaptation from the ACES D60 white to D65 (XYZ → XYZ).
pub const D60_TO_D65: Mat3 = [
    [0.987224, -0.00611327, 0.0159533],
    [-0.00759836, 1.00186, 0.00533002],
    [0.00307257, -0.00509595, 1.08168],
];

/// Encoding curves. Numeric values are what the shaders switch on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum Transfer {
    Linear = 0,
    Srgb = 1,
    /// BT.1886 display gamma 2.4.
    Bt1886 = 2,
    Pq = 3,
    Hlg = 4,
    SLog3 = 5,
    LogC3 = 6,
    VLog = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Primaries {
    Rec709,
    Rec2020,
    P3D65,
    Ap1,
    SGamut3Cine,
    Awg3,
    VGamut,
}

impl Primaries {
    /// RGB → XYZ.
    pub fn to_xyz(self) -> Mat3 {
        match self {
            Primaries::Rec709 => [
                [0.4124564, 0.3575761, 0.1804375],
                [0.2126729, 0.7151522, 0.0721750],
                [0.0193339, 0.1191920, 0.9503041],
            ],
            Primaries::Rec2020 => [
                [0.6369580, 0.1446169, 0.1688810],
                [0.2627002, 0.6779981, 0.0593017],
                [0.0, 0.0280727, 1.0609851],
            ],
            Primaries::P3D65 => [
                [0.4865709, 0.2656677, 0.1982173],
                [0.2289746, 0.6917385, 0.0792869],
                [0.0, 0.0451134, 1.0439444],
            ],
            // ACES primaries are defined for a D60 white; adapt to D65 (Bradford)
            // so neutral stays neutral in the Rec.709 working space.
            Primaries::Ap1 => mul(
                &D60_TO_D65,
                &[
                    [0.6624542, 0.1340042, 0.1561877],
                    [0.2722287, 0.6740818, 0.0536895],
                    [-0.0055746, 0.0040607, 1.0103391],
                ],
            ),
            Primaries::SGamut3Cine => [
                [0.5990839, 0.2489255, 0.1024464],
                [0.2150758, 0.8850685, -0.1001443],
                [-0.0320658, -0.0276583, 1.1487819],
            ],
            Primaries::Awg3 => [
                [0.638008, 0.214704, 0.097744],
                [0.291954, 0.823841, -0.115795],
                [0.002798, -0.067034, 1.153294],
            ],
            Primaries::VGamut => [
                [0.679644, 0.152211, 0.118600],
                [0.260686, 0.774894, -0.035580],
                [-0.009310, -0.004612, 1.102980],
            ],
        }
    }
}

/// Split a tagged space into its curve and primaries.
pub fn describe(cs: &ColorSpace) -> Result<(Transfer, Primaries)> {
    Ok(match cs {
        ColorSpace::Linear709 => (Transfer::Linear, Primaries::Rec709),
        ColorSpace::Srgb => (Transfer::Srgb, Primaries::Rec709),
        ColorSpace::Rec709 => (Transfer::Bt1886, Primaries::Rec709),
        ColorSpace::Rec2020Pq => (Transfer::Pq, Primaries::Rec2020),
        ColorSpace::Rec2020Hlg => (Transfer::Hlg, Primaries::Rec2020),
        ColorSpace::P3D65 => (Transfer::Srgb, Primaries::P3D65),
        ColorSpace::AcesCg => (Transfer::Linear, Primaries::Ap1),
        ColorSpace::SLog3SGamut3Cine => (Transfer::SLog3, Primaries::SGamut3Cine),
        ColorSpace::LogC3Awg3 => (Transfer::LogC3, Primaries::Awg3),
        ColorSpace::VLogVGamut => (Transfer::VLog, Primaries::VGamut),
        ColorSpace::Named(n) => {
            return Err(Error::Unsupported(format!(
                "color space {n:?} needs an OCIO config"
            )))
        }
    })
}

pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    m
}

pub fn invert(m: &Mat3) -> Mat3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let inv = 1.0 / det;
    [
        [
            (e * i - f * h) * inv,
            (c * h - b * i) * inv,
            (b * f - c * e) * inv,
        ],
        [
            (f * g - d * i) * inv,
            (a * i - c * g) * inv,
            (c * d - a * f) * inv,
        ],
        [
            (d * h - e * g) * inv,
            (b * g - a * h) * inv,
            (a * e - b * d) * inv,
        ],
    ]
}

pub fn apply(m: &Mat3, c: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * c[0] + m[0][1] * c[1] + m[0][2] * c[2],
        m[1][0] * c[0] + m[1][1] * c[1] + m[1][2] * c[2],
        m[2][0] * c[0] + m[2][1] * c[1] + m[2][2] * c[2],
    ]
}

/// RGB in `from` primaries → RGB in `to` primaries.
pub fn primaries_matrix(from: Primaries, to: Primaries) -> Mat3 {
    if from == to {
        return IDENTITY;
    }
    mul(&invert(&to.to_xyz()), &from.to_xyz())
}

// ---- transfer functions (per channel, scene-linear <-> encoded) ------------------

const PQ_M1: f32 = 0.159_301_76;
const PQ_M2: f32 = 78.84375;
const PQ_C1: f32 = 0.8359375;
const PQ_C2: f32 = 18.8515625;
const PQ_C3: f32 = 18.6875;
/// Scene-linear 1.0 = 100 nits; PQ's 1.0 = 10 000 nits.
const PQ_SCALE: f32 = 100.0;
const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 0.284_668_92;
const HLG_C: f32 = 0.559_910_73;
/// HLG nominal peak (1 000 nits) relative to 100-nit reference white.
const HLG_SCALE: f32 = 10.0;

/// Encoded → scene-linear.
pub fn decode(t: Transfer, v: f32) -> f32 {
    match t {
        Transfer::Linear => v,
        Transfer::Srgb => {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        Transfer::Bt1886 => v.max(0.0).powf(2.4),
        Transfer::Pq => {
            let p = v.max(0.0).powf(1.0 / PQ_M2);
            let num = (p - PQ_C1).max(0.0);
            let den = PQ_C2 - PQ_C3 * p;
            (num / den).powf(1.0 / PQ_M1) * PQ_SCALE
        }
        Transfer::Hlg => {
            let e = if v <= 0.5 {
                v * v / 3.0
            } else {
                (((v - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
            };
            e * HLG_SCALE
        }
        Transfer::SLog3 => {
            if v >= 171.210_3 / 1023.0 {
                10f32.powf((v * 1023.0 - 420.0) / 261.5) * (0.18 + 0.01) - 0.01
            } else {
                (v * 1023.0 - 95.0) * 0.011_25 / (171.210_3 - 95.0)
            }
        }
        Transfer::LogC3 => {
            let (cut, a, b, c, d, e, f) = (
                0.010591, 5.555556, 0.052272, 0.247190, 0.385537, 5.367655, 0.092809,
            );
            if v > e * cut + f {
                (10f32.powf((v - d) / c) - b) / a
            } else {
                (v - f) / e
            }
        }
        Transfer::VLog => {
            let (b, c, d) = (0.00873, 0.241514, 0.598206);
            if v >= 0.181 {
                10f32.powf((v - d) / c) - b
            } else {
                (v - 0.125) / 5.6
            }
        }
    }
}

/// Scene-linear → encoded.
pub fn encode(t: Transfer, l: f32) -> f32 {
    match t {
        Transfer::Linear => l,
        Transfer::Srgb => {
            if l <= 0.003_130_8 {
                l * 12.92
            } else {
                1.055 * l.max(0.0).powf(1.0 / 2.4) - 0.055
            }
        }
        Transfer::Bt1886 => l.max(0.0).powf(1.0 / 2.4),
        Transfer::Pq => {
            let y = (l / PQ_SCALE).max(0.0).powf(PQ_M1);
            ((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y)).powf(PQ_M2)
        }
        Transfer::Hlg => {
            let e = (l / HLG_SCALE).max(0.0);
            if e <= 1.0 / 12.0 {
                (3.0 * e).sqrt()
            } else {
                HLG_A * (12.0 * e - HLG_B).ln() + HLG_C
            }
        }
        Transfer::SLog3 => {
            if l >= 0.011_25 {
                (420.0 + ((l + 0.01) / (0.18 + 0.01)).log10() * 261.5) / 1023.0
            } else {
                (l * (171.210_3 - 95.0) / 0.011_25 + 95.0) / 1023.0
            }
        }
        Transfer::LogC3 => {
            let (cut, a, b, c, d, e, f) = (
                0.010591, 5.555556, 0.052272, 0.247190, 0.385537, 5.367655, 0.092809,
            );
            if l > cut {
                c * (a * l + b).log10() + d
            } else {
                e * l + f
            }
        }
        Transfer::VLog => {
            let (b, c, d) = (0.00873, 0.241514, 0.598206);
            if l >= 0.01 {
                c * (l + b).log10() + d
            } else {
                5.6 * l + 0.125
            }
        }
    }
}

/// One color-space conversion as the backends execute it: decode, matrix, encode.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorTransform {
    pub decode: Transfer,
    pub matrix: Mat3,
    pub encode: Transfer,
}

impl ColorTransform {
    pub fn between(from: &ColorSpace, to: &ColorSpace) -> Result<Self> {
        let (dt, dp) = describe(from)?;
        let (et, ep) = describe(to)?;
        Ok(Self {
            decode: dt,
            matrix: primaries_matrix(dp, ep),
            encode: et,
        })
    }

    pub fn is_identity(&self) -> bool {
        self.decode == self.encode && self.matrix == IDENTITY
    }

    pub fn apply_rgb(&self, c: [f32; 3]) -> [f32; 3] {
        let lin = [
            decode(self.decode, c[0]),
            decode(self.decode, c[1]),
            decode(self.decode, c[2]),
        ];
        let m = apply(&self.matrix, lin);
        [
            encode(self.encode, m[0]),
            encode(self.encode, m[1]),
            encode(self.encode, m[2]),
        ]
    }
}

/// Primary correction (FX-09), applied to scene-linear RGB in this order:
/// exposure → white balance → lift/gamma/gain → contrast (pivot 0.18) → saturation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grade {
    /// Stops.
    pub exposure: f32,
    /// -1 (cool) .. +1 (warm).
    pub temperature: f32,
    /// -1 (magenta) .. +1 (green).
    pub tint: f32,
    pub lift: [f32; 3],
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    pub contrast: f32,
    pub saturation: f32,
}

impl Default for Grade {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            temperature: 0.0,
            tint: 0.0,
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
            contrast: 1.0,
            saturation: 1.0,
        }
    }
}

pub const LUMA_709: [f32; 3] = [0.2126, 0.7152, 0.0722];

impl Grade {
    pub fn is_identity(&self) -> bool {
        *self == Grade::default()
    }

    pub fn apply_rgb(&self, c: [f32; 3]) -> [f32; 3] {
        let ex = 2f32.powf(self.exposure);
        let wb = [
            1.0 + 0.3 * self.temperature,
            1.0 + 0.3 * self.tint,
            1.0 - 0.3 * self.temperature,
        ];
        let mut out = [0.0; 3];
        for i in 0..3 {
            let mut v = c[i] * ex * wb[i];
            v = v * self.gain[i] + self.lift[i];
            v = v.max(0.0).powf(1.0 / self.gamma[i].max(1e-4));
            v = (v - 0.18) * self.contrast + 0.18;
            out[i] = v;
        }
        let luma = out[0] * LUMA_709[0] + out[1] * LUMA_709[1] + out[2] * LUMA_709[2];
        for v in &mut out {
            *v = luma + (*v - luma) * self.saturation;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transfer_round_trips() {
        for t in [
            Transfer::Linear,
            Transfer::Srgb,
            Transfer::Bt1886,
            Transfer::Pq,
            Transfer::Hlg,
            Transfer::SLog3,
            Transfer::LogC3,
            Transfer::VLog,
        ] {
            for l in [0.0, 0.001, 0.005, 0.02, 0.18, 0.5, 1.0, 4.0, 9.0] {
                let v = encode(t, l);
                let back = decode(t, v);
                assert!(
                    (back - l).abs() < 1e-3 * l.max(0.05),
                    "{t:?} at {l}: {v} -> {back}"
                );
            }
        }
    }

    #[test]
    fn known_anchor_values() {
        assert!((encode(Transfer::Srgb, 0.2158605) - 0.5019608).abs() < 1e-4);
        assert!(
            (decode(Transfer::Pq, 0.5080) - 1.0).abs() < 0.02,
            "PQ 0.508 ≈ 100 nits"
        );
        assert!((decode(Transfer::Hlg, 0.5) - 10.0 / 12.0).abs() < 1e-4);
        assert!(
            (encode(Transfer::SLog3, 0.18) - 0.41055).abs() < 1e-3,
            "S-Log3 mid grey"
        );
        assert!(
            (encode(Transfer::LogC3, 0.18) - 0.391).abs() < 2e-3,
            "LogC3 mid grey"
        );
        assert!(
            (encode(Transfer::VLog, 0.18) - 0.423).abs() < 2e-3,
            "V-Log mid grey"
        );
    }

    #[test]
    fn primaries_matrices_invert_and_keep_white() {
        for p in [
            Primaries::Rec709,
            Primaries::Rec2020,
            Primaries::P3D65,
            Primaries::Ap1,
            Primaries::SGamut3Cine,
            Primaries::Awg3,
            Primaries::VGamut,
        ] {
            let there = primaries_matrix(p, Primaries::Rec709);
            let back = primaries_matrix(Primaries::Rec709, p);
            let id = mul(&there, &back);
            for i in 0..3 {
                for j in 0..3 {
                    assert!((id[i][j] - IDENTITY[i][j]).abs() < 1e-4, "{p:?}");
                }
            }
            // Equal-energy white maps to white in every space (ACES via Bradford).
            let w = apply(&there, [1.0, 1.0, 1.0]);
            assert!(
                w.iter().all(|c| (c - 1.0).abs() < 0.05),
                "{p:?} white -> {w:?}"
            );
        }
    }

    #[test]
    fn transform_between_spaces() {
        let t = ColorTransform::between(&ColorSpace::Srgb, &ColorSpace::Linear709).unwrap();
        assert_eq!(t.apply_rgb([1.0, 1.0, 1.0]), [1.0, 1.0, 1.0]);
        assert!((t.apply_rgb([0.5, 0.5, 0.5])[0] - 0.2140411).abs() < 1e-5);
        let rec2020 =
            ColorTransform::between(&ColorSpace::Rec2020Hlg, &ColorSpace::Linear709).unwrap();
        // Pure Rec.2020 green is outside Rec.709: negative red/blue.
        let g = rec2020.apply_rgb([0.0, 0.75, 0.0]);
        assert!(g[0] < 0.0 && g[1] > 0.0 && g[2] < 0.0, "{g:?}");
        assert!(
            ColorTransform::between(&ColorSpace::Named("x".into()), &ColorSpace::Srgb).is_err()
        );
        assert!(
            ColorTransform::between(&ColorSpace::Rec709, &ColorSpace::Rec709)
                .unwrap()
                .is_identity()
        );
    }

    #[test]
    fn grade_math() {
        let g = Grade::default();
        assert_eq!(g.apply_rgb([0.2, 0.4, 0.6]), [0.2, 0.4, 0.6]);
        let g = Grade {
            exposure: 1.0,
            ..Default::default()
        };
        assert!((g.apply_rgb([0.18, 0.18, 0.18])[0] - 0.36).abs() < 1e-6);
        let g = Grade {
            saturation: 0.0,
            ..Default::default()
        };
        let out = g.apply_rgb([1.0, 0.0, 0.0]);
        assert!((out[0] - out[1]).abs() < 1e-6 && (out[0] - LUMA_709[0]).abs() < 1e-6);
        let g = Grade {
            contrast: 2.0,
            ..Default::default()
        };
        assert!(
            (g.apply_rgb([0.18; 3])[0] - 0.18).abs() < 1e-6,
            "pivot is fixed"
        );
        assert!((g.apply_rgb([0.28; 3])[0] - 0.38).abs() < 1e-6);
    }
}
