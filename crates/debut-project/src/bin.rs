//! Bins, smart bins, keywords, ratings and favorites (MED-07, MED-08).

use debut_core::id::{BinId, MediaId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bin {
    pub id: BinId,
    pub name: String,
    pub kind: BinKind,
    pub items: Vec<MediaId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BinKind {
    Manual,
    /// Rule-based membership, re-evaluated on metadata change.
    Smart {
        rules: Vec<SmartRule>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartRule {
    pub field: String,
    pub op: String,
    pub value: String,
}
