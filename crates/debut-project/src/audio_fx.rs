//! Audio insert descriptions stored per track (AUD-05). `debut-audio` builds the
//! processors from these.

use serde::{Deserialize, Serialize};

/// What the project stores per track; the renderer builds processors from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AudioEffect {
    Eq {
        bands: Vec<EqBand>,
    },
    Compressor {
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
    },
    Limiter {
        ceiling_db: f32,
        release_ms: f32,
    },
    Gate {
        threshold_db: f32,
        attack_ms: f32,
        release_ms: f32,
    },
    /// Compresses only a sibilance band (default 5–9 kHz).
    DeEsser {
        frequency_hz: f32,
        threshold_db: f32,
        ratio: f32,
    },
    Reverb {
        room: f32,
        damping: f32,
        mix: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EqBand {
    pub kind: EqKind,
    pub frequency_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EqKind {
    LowShelf,
    Peak,
    HighShelf,
    HighPass,
    LowPass,
}
