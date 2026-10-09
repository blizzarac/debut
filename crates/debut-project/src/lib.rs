//! The project model (MED-07, MED-09, MED-10, TL-07, TL-10, NFR-06, PLT-03).
//!
//! A project is pure data: bins, media references, sequences and markers. It is only
//! mutated through `debut-command`, so every change is undoable and syncable.
//! The on-disk schema is versioned; `migrate` upgrades any older version in place.

pub mod bin;
pub mod media_ref;
pub mod sequence;
pub mod marker;
pub mod schema;

use debut_core::ProjectId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub schema_version: u32,
    pub name: String,
    pub bins: Vec<bin::Bin>,
    pub media: Vec<media_ref::MediaRef>,
    pub sequences: Vec<sequence::Sequence>,
}
