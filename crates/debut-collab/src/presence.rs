//! Presence (COL-02): where each collaborator is, so others can see them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    pub client: u64,
    pub name: String,
    pub role: crate::Role,
    /// Sequence being viewed and the playhead in seconds.
    pub sequence: Option<String>,
    pub playhead: f64,
    /// Selected clip, if any.
    pub clip: Option<String>,
}
