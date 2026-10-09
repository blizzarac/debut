//! Color-space tags that travel with media and sequences. Transforms live in
//! `debut-render::color` under the managed pipeline (FX-08).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorSpace {
    /// The engine's working space: scene-linear, Rec.709 primaries, D65.
    Linear709,
    Srgb,
    /// Video Rec.709 displayed per BT.1886 (gamma 2.4).
    #[default]
    Rec709,
    Rec2020Pq,
    Rec2020Hlg,
    P3D65,
    /// ACEScg: scene-linear, AP1 primaries.
    AcesCg,
    /// Sony S-Log3 / S-Gamut3.Cine.
    SLog3SGamut3Cine,
    /// ARRI LogC3 (EI 800) / ARRI Wide Gamut 3.
    LogC3Awg3,
    /// Panasonic V-Log / V-Gamut.
    VLogVGamut,
    /// Any other space, named as OCIO knows it; resolved by an OCIO config.
    Named(String),
}
