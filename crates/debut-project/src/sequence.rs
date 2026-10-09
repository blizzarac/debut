//! Sequences, tracks and clip instances (TL-01, TL-02, TL-07, TL-08).

use debut_core::id::{ClipId, MediaId, SequenceId, TrackId};
use debut_core::{FrameRate, Rational};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sequence {
    pub id: SequenceId,
    pub name: String,
    pub frame_rate: FrameRate,
    pub width: u32,
    pub height: u32,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub clips: Vec<Clip>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
    /// Applies effects to all tracks below (FX-07).
    Adjustment,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub source: ClipSource,
    pub timeline_in: Rational,
    pub duration: Rational,
    pub source_in: Rational,
    /// 1.0 = normal speed; ramps live in the effect stack (TL-09).
    pub speed: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClipSource {
    Media(MediaId),
    /// Nested sequence / compound clip (TL-07).
    Sequence(SequenceId),
    /// Multicam clip; `active` is the currently switched angle (MED-11, TL-08).
    Multicam { angles: Vec<MediaId>, active: usize },
}
