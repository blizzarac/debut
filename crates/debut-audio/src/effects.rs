//! Built-in track inserts (AUD-05): parametric EQ (biquads), compressor, limiter,
//! noise gate, de-esser and a small reverb. Each is a [`Processor`] over
//! interleaved stereo at the engine rate, allocation-free once built, with a
//! serializable [`AudioEffect`] description the project stores.

pub const CHANNELS: usize = 2;

pub use debut_project::audio_fx::{AudioEffect, EqBand, EqKind};

pub trait Processor: Send {
    /// Process interleaved stereo in place.
    fn process(&mut self, buf: &mut [f32]);
    fn reset(&mut self);
}

pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn lin_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-12).log10()
}

/// Build the processor for a description.
pub fn build(effect: &AudioEffect, sample_rate: u32) -> Box<dyn Processor> {
    let sr = sample_rate as f32;
    match effect {
        AudioEffect::Eq { bands } => Box::new(Eq::new(bands, sr)),
        AudioEffect::Compressor {
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            makeup_db,
        } => Box::new(Dynamics::compressor(
            sr,
            *threshold_db,
            *ratio,
            *attack_ms,
            *release_ms,
            *makeup_db,
        )),
        AudioEffect::Limiter {
            ceiling_db,
            release_ms,
        } => Box::new(Dynamics::limiter(sr, *ceiling_db, *release_ms)),
        AudioEffect::Gate {
            threshold_db,
            attack_ms,
            release_ms,
        } => Box::new(Gate::new(sr, *threshold_db, *attack_ms, *release_ms)),
        AudioEffect::DeEsser {
            frequency_hz,
            threshold_db,
            ratio,
        } => Box::new(DeEsser::new(sr, *frequency_hz, *threshold_db, *ratio)),
        AudioEffect::Reverb { room, damping, mix } => {
            Box::new(Reverb::new(sr, *room, *damping, *mix))
        }
        // Needs the plugin host: see `crate::plugin::build`.
        AudioEffect::Plugin { .. } => crate::plugin::build(effect, sample_rate, None),
    }
}

// ---- biquad ---------------------------------------------------------------------

/// Direct-form-I biquad (Audio EQ Cookbook), one state per channel.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: [f32; CHANNELS],
    x2: [f32; CHANNELS],
    y1: [f32; CHANNELS],
    y2: [f32; CHANNELS],
}

impl Biquad {
    pub fn new(band: EqBand, sr: f32) -> Self {
        let w0 = std::f32::consts::TAU * band.frequency_hz.clamp(10.0, sr * 0.49) / sr;
        let (sin, cos) = w0.sin_cos();
        let q = band.q.max(0.05);
        let alpha = sin / (2.0 * q);
        let a = 10f32.powf(band.gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match band.kind {
            EqKind::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            EqKind::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - s),
                    (a + 1.0) + (a - 1.0) * cos + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - s,
                )
            }
            EqKind::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - s),
                    (a + 1.0) - (a - 1.0) * cos + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - s,
                )
            }
            EqKind::HighPass => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            EqKind::LowPass => (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: [0.0; 2],
            x2: [0.0; 2],
            y1: [0.0; 2],
            y2: [0.0; 2],
        }
    }

    /// Normalized coefficients (a0 = 1).
    pub fn from_coeffs(b0: f32, b1: f32, b2: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            x1: [0.0; 2],
            x2: [0.0; 2],
            y1: [0.0; 2],
            y2: [0.0; 2],
        }
    }

    #[inline]
    pub fn tick(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1[ch] + self.b2 * self.x2[ch]
            - self.a1 * self.y1[ch]
            - self.a2 * self.y2[ch];
        self.x2[ch] = self.x1[ch];
        self.x1[ch] = x;
        self.y2[ch] = self.y1[ch];
        self.y1[ch] = y;
        y
    }

    pub fn reset(&mut self) {
        self.x1 = [0.0; 2];
        self.x2 = [0.0; 2];
        self.y1 = [0.0; 2];
        self.y2 = [0.0; 2];
    }

    /// Magnitude response at `hz`, for tests and the EQ display.
    pub fn magnitude_db(&self, hz: f32, sr: f32) -> f32 {
        let w = std::f32::consts::TAU * hz / sr;
        let (c1, s1) = (w.cos(), w.sin());
        let (c2, s2) = ((2.0 * w).cos(), (2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        lin_to_db(((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt())
    }
}

pub struct Eq {
    stages: Vec<Biquad>,
}

impl Eq {
    pub fn new(bands: &[EqBand], sr: f32) -> Self {
        Self {
            stages: bands.iter().map(|b| Biquad::new(*b, sr)).collect(),
        }
    }
}

impl Processor for Eq {
    fn process(&mut self, buf: &mut [f32]) {
        for frame in buf.chunks_exact_mut(CHANNELS) {
            for (ch, s) in frame.iter_mut().enumerate() {
                for st in &mut self.stages {
                    *s = st.tick(ch, *s);
                }
            }
        }
    }
    fn reset(&mut self) {
        self.stages.iter_mut().for_each(Biquad::reset);
    }
}

// ---- dynamics -----------------------------------------------------------------

fn coef(ms: f32, sr: f32) -> f32 {
    (-1.0 / (ms.max(0.01) * 0.001 * sr)).exp()
}

/// Feed-forward peak compressor / limiter, stereo-linked, log-domain gain
/// computer with separate attack and release smoothing.
pub struct Dynamics {
    threshold_db: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup: f32,
    env_db: f32,
}

impl Dynamics {
    pub fn compressor(
        sr: f32,
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
    ) -> Self {
        Self {
            threshold_db,
            ratio: ratio.max(1.0),
            attack: coef(attack_ms, sr),
            release: coef(release_ms, sr),
            makeup: db_to_lin(makeup_db),
            env_db: 0.0,
        }
    }

    pub fn limiter(sr: f32, ceiling_db: f32, release_ms: f32) -> Self {
        Self::compressor(sr, ceiling_db, 1000.0, 0.05, release_ms, 0.0)
    }

    /// Gain reduction in dB for a given input level in dB.
    fn reduction_db(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold_db;
        if over <= 0.0 {
            0.0
        } else {
            over - over / self.ratio
        }
    }

    /// Current smoothed gain reduction (dB, >= 0); for metering.
    pub fn gain_reduction_db(&self) -> f32 {
        self.env_db
    }
}

impl Processor for Dynamics {
    fn process(&mut self, buf: &mut [f32]) {
        for frame in buf.chunks_exact_mut(CHANNELS) {
            let peak = frame.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            let target = self.reduction_db(lin_to_db(peak));
            let c = if target > self.env_db {
                self.attack
            } else {
                self.release
            };
            self.env_db = target + c * (self.env_db - target);
            let g = db_to_lin(-self.env_db) * self.makeup;
            for s in frame.iter_mut() {
                *s *= g;
            }
        }
    }
    fn reset(&mut self) {
        self.env_db = 0.0;
    }
}

/// Downward expander with a hard floor: below threshold the gain falls to zero.
pub struct Gate {
    threshold: f32,
    attack: f32,
    release: f32,
    gain: f32,
}

impl Gate {
    pub fn new(sr: f32, threshold_db: f32, attack_ms: f32, release_ms: f32) -> Self {
        Self {
            threshold: db_to_lin(threshold_db),
            attack: coef(attack_ms, sr),
            release: coef(release_ms, sr),
            gain: 0.0,
        }
    }
}

impl Processor for Gate {
    fn process(&mut self, buf: &mut [f32]) {
        for frame in buf.chunks_exact_mut(CHANNELS) {
            let peak = frame.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            let target = if peak >= self.threshold { 1.0 } else { 0.0 };
            let c = if target > self.gain {
                self.attack
            } else {
                self.release
            };
            self.gain = target + c * (self.gain - target);
            for s in frame.iter_mut() {
                *s *= self.gain;
            }
        }
    }
    fn reset(&mut self) {
        self.gain = 0.0;
    }
}

/// Split-band de-esser. The band split is allpass-complementary: `low` is a
/// 2nd-order Butterworth low-pass and `high = allpass(x) - low` with the same
/// poles, so `low + high` has flat magnitude and only the high band is compressed.
pub struct DeEsser {
    lp: Biquad,
    ap: Biquad,
    comp: Dynamics,
    scratch: Vec<f32>,
}

impl DeEsser {
    pub fn new(sr: f32, frequency_hz: f32, threshold_db: f32, ratio: f32) -> Self {
        let lp = Biquad::new(
            EqBand {
                kind: EqKind::LowPass,
                frequency_hz,
                gain_db: 0.0,
                q: 0.707,
            },
            sr,
        );
        // Allpass sharing the low-pass poles: b = (a2, a1, 1).
        let ap = Biquad::from_coeffs(lp.a2, lp.a1, 1.0, lp.a1, lp.a2);
        Self {
            lp,
            ap,
            comp: Dynamics::compressor(sr, threshold_db, ratio, 0.5, 40.0, 0.0),
            scratch: Vec::new(),
        }
    }
}

impl Processor for DeEsser {
    fn process(&mut self, buf: &mut [f32]) {
        self.scratch.clear();
        self.scratch.resize(buf.len(), 0.0);
        // buf = low band, scratch = high band.
        for (frame, hi) in buf
            .chunks_exact_mut(CHANNELS)
            .zip(self.scratch.chunks_exact_mut(CHANNELS))
        {
            for ch in 0..CHANNELS {
                let x = frame[ch];
                let low = self.lp.tick(ch, x);
                hi[ch] = self.ap.tick(ch, x) - low;
                frame[ch] = low;
            }
        }
        self.comp.process(&mut self.scratch);
        for (s, hi) in buf.iter_mut().zip(&self.scratch) {
            *s += hi;
        }
    }
    fn reset(&mut self) {
        self.lp.reset();
        self.ap.reset();
        self.comp.reset();
    }
}

// ---- reverb ---------------------------------------------------------------------

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    feedback: f32,
    damp: f32,
    store: f32,
}

impl Comb {
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.buf[self.pos];
        self.store = y * (1.0 - self.damp) + self.store * self.damp;
        self.buf[self.pos] = x + self.store * self.feedback;
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.pos];
        let y = b - x;
        self.buf[self.pos] = x + b * 0.5;
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

/// Freeverb-style: four combs and two allpasses per channel.
pub struct Reverb {
    combs: [Vec<Comb>; CHANNELS],
    allpasses: [Vec<Allpass>; CHANNELS],
    mix: f32,
}

impl Reverb {
    pub fn new(sr: f32, room: f32, damping: f32, mix: f32) -> Self {
        let scale = sr / 44_100.0;
        let feedback = 0.7 + 0.28 * room.clamp(0.0, 1.0);
        let damp = damping.clamp(0.0, 1.0) * 0.4;
        let mk = |offset: usize| {
            [1116, 1188, 1277, 1356]
                .iter()
                .map(|&n| Comb {
                    buf: vec![0.0; ((n + offset) as f32 * scale) as usize],
                    pos: 0,
                    feedback,
                    damp,
                    store: 0.0,
                })
                .collect::<Vec<_>>()
        };
        let ap = |offset: usize| {
            [556, 441]
                .iter()
                .map(|&n| Allpass {
                    buf: vec![0.0; ((n + offset) as f32 * scale) as usize],
                    pos: 0,
                })
                .collect::<Vec<_>>()
        };
        Self {
            combs: [mk(0), mk(23)],
            allpasses: [ap(0), ap(23)],
            mix: mix.clamp(0.0, 1.0),
        }
    }
}

impl Processor for Reverb {
    fn process(&mut self, buf: &mut [f32]) {
        for frame in buf.chunks_exact_mut(CHANNELS) {
            let input = (frame[0] + frame[1]) * 0.5 * 0.015;
            for (ch, out) in frame.iter_mut().enumerate() {
                let mut wet: f32 = self.combs[ch].iter_mut().map(|c| c.tick(input)).sum();
                for a in &mut self.allpasses[ch] {
                    wet = a.tick(wet);
                }
                *out = *out * (1.0 - self.mix) + wet * self.mix;
            }
        }
    }
    fn reset(&mut self) {
        for ch in 0..CHANNELS {
            for c in &mut self.combs[ch] {
                c.buf.fill(0.0);
                c.store = 0.0;
            }
            for a in &mut self.allpasses[ch] {
                a.buf.fill(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let s = (i as f32 * hz * std::f32::consts::TAU / SR).sin() * amp;
                [s, s]
            })
            .collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |p, s| p.max(s.abs()))
    }

    #[test]
    fn biquad_responses_match_their_design() {
        let peak = Biquad::new(
            EqBand {
                kind: EqKind::Peak,
                frequency_hz: 1000.0,
                gain_db: 6.0,
                q: 1.0,
            },
            SR,
        );
        assert!((peak.magnitude_db(1000.0, SR) - 6.0).abs() < 0.05);
        assert!(peak.magnitude_db(100.0, SR).abs() < 0.2);
        let hp = Biquad::new(
            EqBand {
                kind: EqKind::HighPass,
                frequency_hz: 1000.0,
                gain_db: 0.0,
                q: 0.707,
            },
            SR,
        );
        assert!((hp.magnitude_db(1000.0, SR) + 3.0).abs() < 0.2);
        assert!(hp.magnitude_db(100.0, SR) < -35.0);
        let ls = Biquad::new(
            EqBand {
                kind: EqKind::LowShelf,
                frequency_hz: 200.0,
                gain_db: -12.0,
                q: 0.707,
            },
            SR,
        );
        assert!((ls.magnitude_db(20.0, SR) + 12.0).abs() < 0.3);
        assert!(ls.magnitude_db(5000.0, SR).abs() < 0.2);
        let hs = Biquad::new(
            EqBand {
                kind: EqKind::HighShelf,
                frequency_hz: 4000.0,
                gain_db: 3.0,
                q: 0.707,
            },
            SR,
        );
        assert!((hs.magnitude_db(18000.0, SR) - 3.0).abs() < 0.3);
    }

    #[test]
    fn eq_processes_a_tone_by_its_response() {
        let mut eq = Eq::new(
            &[EqBand {
                kind: EqKind::LowPass,
                frequency_hz: 500.0,
                gain_db: 0.0,
                q: 0.707,
            }],
            SR,
        );
        let mut hi = sine(8000.0, 0.5, 4800);
        eq.process(&mut hi);
        assert!(
            peak(&hi[4000..]) < 0.01,
            "8 kHz through a 500 Hz low-pass: {}",
            peak(&hi[4000..])
        );
        eq.reset();
        let mut lo = sine(50.0, 0.5, 9600);
        eq.process(&mut lo);
        assert!((peak(&lo[4800..]) - 0.5).abs() < 0.02);
    }

    #[test]
    fn compressor_reduces_by_the_ratio_and_limiter_holds_the_ceiling() {
        let mut c = Dynamics::compressor(SR, -20.0, 4.0, 1.0, 50.0, 0.0);
        let mut buf = sine(1000.0, 1.0, 48_000); // 0 dBFS: 20 dB over, ratio 4 -> -20 + 5 = -15 dBFS
        c.process(&mut buf);
        let out_db = lin_to_db(peak(&buf[24_000..]));
        assert!(
            (out_db + 15.0).abs() < 0.5,
            "expected ≈ -15 dBFS, got {out_db}"
        );
        let mut quiet = sine(1000.0, db_to_lin(-30.0), 4800);
        c.reset();
        c.process(&mut quiet);
        assert!(
            (lin_to_db(peak(&quiet)) + 30.0).abs() < 0.1,
            "below threshold is untouched"
        );

        let mut l = Dynamics::limiter(SR, -1.0, 20.0);
        let mut loud = sine(1000.0, 2.0, 48_000);
        l.process(&mut loud);
        assert!(
            lin_to_db(peak(&loud[9600..])) < -0.8,
            "{}",
            lin_to_db(peak(&loud[9600..]))
        );
    }

    #[test]
    fn gate_closes_on_silence_and_opens_on_signal() {
        let mut g = Gate::new(SR, -40.0, 1.0, 20.0);
        let mut noise = sine(1000.0, db_to_lin(-60.0), 9600);
        g.process(&mut noise);
        assert!(peak(&noise[4800..]) < 1e-4);
        let mut voice = sine(200.0, 0.3, 9600);
        g.process(&mut voice);
        assert!((peak(&voice[4800..]) - 0.3).abs() < 0.01);
    }

    #[test]
    fn de_esser_tames_sibilance_but_leaves_lows() {
        let mut d = DeEsser::new(SR, 5000.0, -30.0, 8.0);
        let mut ess = sine(9000.0, 0.5, 48_000);
        d.process(&mut ess);
        assert!(peak(&ess[24_000..]) < 0.25, "{}", peak(&ess[24_000..]));
        d.reset();
        let mut low = sine(150.0, 0.5, 48_000);
        d.process(&mut low);
        assert!((peak(&low[24_000..]) - 0.5).abs() < 0.03);
    }

    #[test]
    fn reverb_adds_a_tail_and_dry_mix_is_identity() {
        let mut r = Reverb::new(SR, 0.8, 0.3, 0.5);
        let mut buf = vec![0.0f32; 2 * 48_000];
        buf[0] = 1.0;
        buf[1] = 1.0;
        r.process(&mut buf);
        assert!(
            peak(&buf[2 * 20_000..2 * 30_000]) > 1e-4,
            "tail present after 0.4 s"
        );
        let mut dry = Reverb::new(SR, 0.8, 0.3, 0.0);
        let mut sig = sine(440.0, 0.3, 4800);
        let orig = sig.clone();
        dry.process(&mut sig);
        assert_eq!(sig, orig);
    }
}
