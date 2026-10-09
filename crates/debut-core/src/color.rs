//! Color-space tags that travel with media and sequences. Transforms live in
//! `debut-render` under the OCIO-managed pipeline (FX-08).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorSpace {
    Srgb,
    Rec709,
    Rec2020Pq,
    Rec2020Hlg,
    P3D65,
    AcesCg,
    /// Camera log, identified by its OCIO colorspace name.
    Log(String),
}
