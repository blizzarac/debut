//! Markers on clips and timelines (MED-07, TL-10, COL-04).

use debut_core::id::MarkerId;
use debut_core::Rational;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Marker {
    pub id: MarkerId,
    pub at: Rational,
    pub duration: Rational,
    pub color: [u8; 3],
    pub note: String,
    /// Set when the marker was imported from a review comment (COL-04).
    pub resolved: Option<bool>,
}
