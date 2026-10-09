//! The project model (MED-07, MED-09, MED-10, TL-07, TL-10, NFR-06, PLT-03).
//!
//! A project is pure data: bins, media references, sequences and markers. It is only
//! mutated through `debut-command`, so every change is undoable and syncable.
//! The on-disk schema is versioned; `schema::migrate` upgrades any older version.

pub mod audio_fx;
pub mod bin;
pub mod effect;
pub mod marker;
pub mod media_ref;
pub mod schema;
pub mod sequence;

pub use audio_fx::{AudioEffect, EqBand, EqKind};
pub use effect::{Effect, GradeFx, Param, TransformFx};
pub use marker::{marker_list, Marker};
pub use sequence::{
    Clip, ClipSource, Layer, Sequence, Track, TrackKind, TrackMix, Transition, TransitionKind,
};

use debut_core::{ProjectId, SequenceId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub schema_version: u32,
    pub name: String,
    pub bins: Vec<bin::Bin>,
    pub media: Vec<media_ref::MediaRef>,
    pub sequences: Vec<Sequence>,
}

impl Project {
    pub fn new(id: ProjectId, name: impl Into<String>) -> Self {
        Self {
            id,
            schema_version: schema::CURRENT_SCHEMA,
            name: name.into(),
            bins: Vec::new(),
            media: Vec::new(),
            sequences: Vec::new(),
        }
    }

    pub fn sequence(&self, id: SequenceId) -> Option<&Sequence> {
        self.sequences.iter().find(|s| s.id == id)
    }

    pub fn sequence_mut(&mut self, id: SequenceId) -> Option<&mut Sequence> {
        self.sequences.iter_mut().find(|s| s.id == id)
    }
}
