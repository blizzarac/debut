//! Saved versions of a sequence (TL-14): a named copy taken at some point,
//! to compare against or go back to.

use crate::sequence::Sequence;
use debut_core::id::SnapshotId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub name: String,
    /// The sequence as it was; its id is the sequence the snapshot belongs to.
    pub sequence: Sequence,
}
