//! Track mixer strips and insert effects (AUD-02, AUD-05).

use super::*;

pub(crate) fn insert_name(e: &AudioEffect) -> String {
    match e {
        AudioEffect::Eq { bands } if bands.iter().all(|b| matches!(b.kind, EqKind::HighPass)) => {
            "Low cut".into()
        }
        AudioEffect::Eq { .. } => "EQ".into(),
        AudioEffect::Compressor { .. } => "Compressor".into(),
        AudioEffect::Limiter { .. } => "Limiter".into(),
        AudioEffect::Gate { .. } => "Gate".into(),
        AudioEffect::DeEsser { .. } => "De-esser".into(),
        AudioEffect::Reverb { .. } => "Reverb".into(),
    }
}

pub(crate) fn insert_preset(kind: &str) -> Option<AudioEffect> {
    Some(match kind {
        "eq_lowcut" => AudioEffect::Eq {
            bands: vec![EqBand {
                kind: EqKind::HighPass,
                frequency_hz: 80.0,
                gain_db: 0.0,
                q: 0.707,
            }],
        },
        "eq_presence" => AudioEffect::Eq {
            bands: vec![EqBand {
                kind: EqKind::Peak,
                frequency_hz: 3000.0,
                gain_db: 3.0,
                q: 1.0,
            }],
        },
        "compressor" => AudioEffect::Compressor {
            threshold_db: -18.0,
            ratio: 3.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup_db: 3.0,
        },
        "limiter" => AudioEffect::Limiter {
            ceiling_db: -1.0,
            release_ms: 50.0,
        },
        "gate" => AudioEffect::Gate {
            threshold_db: -45.0,
            attack_ms: 1.0,
            release_ms: 50.0,
        },
        "de_esser" => AudioEffect::DeEsser {
            frequency_hz: 6000.0,
            threshold_db: -24.0,
            ratio: 6.0,
        },
        "reverb" => AudioEffect::Reverb {
            room: 0.6,
            damping: 0.4,
            mix: 0.25,
        },
        _ => return None,
    })
}

impl Session {
    pub(crate) fn track_target(&self, track: &str) -> Result<Target, String> {
        let seq = self.first_sequence()?;
        let track_id = TrackId(parse_id(track)?);
        seq.track(track_id).ok_or("track not found")?;
        Ok(Target {
            sequence: seq.id,
            track: track_id,
        })
    }

    pub fn set_track_mix(&mut self, track: &str, mix: TrackMix) -> Result<(), String> {
        let target = self.track_target(track)?;
        self.exec(Command::SetTrackMix { target, mix })
    }

    pub(crate) fn track_inserts(&self, track: &str) -> Result<(Target, Vec<AudioEffect>), String> {
        let target = self.track_target(track)?;
        let effects = self
            .first_sequence()?
            .track(target.track)
            .unwrap()
            .audio_effects
            .clone();
        Ok((target, effects))
    }

    pub fn add_insert(&mut self, track: &str, kind: &str) -> Result<(), String> {
        let (target, mut effects) = self.track_inserts(track)?;
        effects.push(insert_preset(kind).ok_or_else(|| format!("unknown insert {kind}"))?);
        self.exec(Command::SetTrackAudio { target, effects })
    }

    pub fn remove_insert(&mut self, track: &str, index: usize) -> Result<(), String> {
        let (target, mut effects) = self.track_inserts(track)?;
        if index >= effects.len() {
            return Err(format!("no insert {index}"));
        }
        effects.remove(index);
        self.exec(Command::SetTrackAudio { target, effects })
    }
}
