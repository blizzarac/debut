//! Audio insert descriptions stored per track (AUD-05). `debut-audio` builds the
//! processors from these.

use debut_core::TrackId;
use serde::{Deserialize, Serialize};

/// Auto-ducking (AUD-08): lower this track by `amount_db` while the `key`
/// track (typically dialogue) is above `threshold_db` after its fader. The
/// gain falls over `attack_ms`, holds through short pauses and recovers over
/// `release_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Duck {
    pub key: TrackId,
    pub amount_db: f32,
    pub threshold_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

impl Duck {
    /// Music under dialogue: -12 dB, keyed above -40 dBFS, 80 ms down, 500 ms up.
    pub fn under(key: TrackId) -> Self {
        Self {
            key,
            amount_db: -12.0,
            threshold_db: -40.0,
            attack_ms: 80.0,
            release_ms: 500.0,
        }
    }
}

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
    /// A CLAP audio effect, run by the plugin host (AUD-09).
    Plugin {
        path: String,
        index: u32,
        id: String,
        name: String,
        params: Vec<AudioPluginParam>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioPluginParam {
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub value: f64,
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
