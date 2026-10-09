//! Auto-ducking (AUD-08): a sidechain gain that lowers one track while a key
//! track is active. The key is followed with a fast peak envelope; once it
//! crosses the threshold the gain glides down to the duck amount over the
//! attack time, holds through pauses between words, and glides back over the
//! release time. Gain moves in dB so the fades sound even.

use debut_project::Duck;

/// Envelope follower release: short, the hold bridges pauses.
const DETECT_RELEASE_MS: f32 = 30.0;
/// Attack and release are fade lengths: the gain is within 1% of its target
/// after this many one-pole time constants (ln 100).
const FADE_TAUS: f32 = 4.6;
/// How long the key may stay quiet before the gain starts to recover.
const HOLD_MS: f32 = 250.0;

fn coeff(ms: f32, sample_rate: u32) -> f32 {
    let samples = (ms.max(0.1) * 0.001 * sample_rate as f32).max(1.0);
    1.0 - (-1.0 / samples).exp()
}

pub struct Ducker {
    settings: Duck,
    env: f32,
    gain_db: f32,
    held: u32,
    hold: u32,
    threshold: f32,
    detect_release: f32,
    attack: f32,
    release: f32,
}

impl Ducker {
    pub fn new(settings: Duck, sample_rate: u32) -> Self {
        Self {
            settings,
            env: 0.0,
            gain_db: 0.0,
            held: 0,
            hold: (HOLD_MS * 0.001 * sample_rate as f32) as u32,
            threshold: 10f32.powf(settings.threshold_db / 20.0),
            detect_release: coeff(DETECT_RELEASE_MS, sample_rate),
            attack: coeff(settings.attack_ms / FADE_TAUS, sample_rate),
            release: coeff(settings.release_ms / FADE_TAUS, sample_rate),
        }
    }

    pub fn settings(&self) -> &Duck {
        &self.settings
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
        self.gain_db = 0.0;
        self.held = 0;
    }

    /// Current gain change in dB (0 = untouched, negative while ducking).
    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    /// Duck the interleaved `buf` (`channels` wide) by the level of `key`
    /// (same layout and length). A missing key counts as silence.
    pub fn process(&mut self, key: Option<&[f32]>, buf: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        for (i, frame) in buf.chunks_exact_mut(channels).enumerate() {
            let peak = key
                .and_then(|k| k.get(i * channels..(i + 1) * channels))
                .map(|f| f.iter().fold(0.0f32, |m, s| m.max(s.abs())))
                .unwrap_or(0.0);
            // Instant attack on the detector, short release.
            self.env = if peak > self.env {
                peak
            } else {
                self.env + (peak - self.env) * self.detect_release
            };
            if self.env > self.threshold {
                self.held = self.hold;
            } else if self.held > 0 {
                self.held -= 1;
            }
            let (target, k) = if self.held > 0 {
                (self.settings.amount_db.min(0.0), self.attack)
            } else {
                (0.0, self.release)
            };
            self.gain_db += (target - self.gain_db) * k;
            if self.gain_db != 0.0 {
                let g = 10f32.powf(self.gain_db / 20.0);
                frame.iter_mut().for_each(|s| *s *= g);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::TrackId;

    const SR: u32 = 48_000;

    fn level_db(buf: &[f32]) -> f32 {
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        20.0 * peak.log10()
    }

    #[test]
    fn ducks_under_the_key_holds_through_pauses_and_recovers() {
        let mut d = Ducker::new(Duck::under(TrackId(1)), SR);
        let ms = |n: usize| n * SR as usize / 1000 * 2;
        let music = |n: usize| vec![0.5f32; n];
        let voice = |n: usize| {
            (0..n)
                .map(|i| if i % 2 == 0 { 0.3 } else { -0.3 })
                .collect::<Vec<f32>>()
        };
        // No key: untouched.
        let mut m = music(ms(100));
        d.process(None, &mut m, 2);
        assert_eq!(m[m.len() - 1], 0.5);
        // Voice for 1 s: fully ducked by the end (80 ms attack).
        let mut m = music(ms(1000));
        d.process(Some(&voice(ms(1000))), &mut m, 2);
        assert!((level_db(&m[m.len() - 2..]) - (-6.02 - 12.0)).abs() < 0.2);
        // A 150 ms pause stays ducked (hold).
        let mut m = music(ms(150));
        d.process(Some(&vec![0.0; ms(150)]), &mut m, 2);
        assert!(d.gain_db() < -11.5, "{}", d.gain_db());
        // Hold (250 ms) plus the 500 ms release: back to unity.
        let mut m = music(ms(800));
        d.process(Some(&vec![0.0; ms(800)]), &mut m, 2);
        assert!(d.gain_db() > -0.15, "{}", d.gain_db());
        // Below the threshold never ducks.
        let mut d = Ducker::new(Duck::under(TrackId(1)), SR);
        let mut m = music(ms(500));
        d.process(Some(&vec![0.001; ms(500)]), &mut m, 2);
        assert_eq!(d.gain_db(), 0.0);
    }
}
