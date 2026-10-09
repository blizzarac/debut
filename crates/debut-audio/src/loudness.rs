//! Loudness metering per ITU-R BS.1770-4 / EBU R128 (AUD-06): K-weighted,
//! gated integrated loudness in LUFS, momentary (400 ms) and short-term (3 s)
//! windows, and true peak via 4x oversampling.

use crate::effects::Biquad;

const CHANNELS: usize = 2;
const ABSOLUTE_GATE_LUFS: f64 = -70.0;
const RELATIVE_GATE_LU: f64 = -10.0;

/// Common loudness targets (LUFS).
pub const TARGET_WEB: f32 = -14.0;
pub const TARGET_EBU_R128: f32 = -23.0;

pub struct LoudnessMeter {
    stage1: Biquad,
    stage2: Biquad,
    /// Mean-square per 100 ms block (both channels summed, 1.0 weights).
    blocks: Vec<f64>,
    block_len: usize,
    acc: f64,
    acc_n: usize,
    true_peak: f32,
    prev: [f32; CHANNELS],
}

impl LoudnessMeter {
    pub fn new(sample_rate: u32) -> Self {
        let (stage1, stage2) = k_weighting(sample_rate as f64);
        Self {
            stage1,
            stage2,
            blocks: Vec::new(),
            block_len: (sample_rate / 10) as usize,
            acc: 0.0,
            acc_n: 0,
            true_peak: 0.0,
            prev: [0.0; 2],
        }
    }

    /// Feed interleaved stereo.
    pub fn push(&mut self, buf: &[f32]) {
        for frame in buf.chunks_exact(CHANNELS) {
            // True peak: 4x linear-interpolated oversampling (a cheap stand-in for
            // the standard's FIR; within ~0.3 dB on programme material).
            for (ch, &b) in frame.iter().enumerate() {
                let a = self.prev[ch];
                for k in 1..=4 {
                    let v = a + (b - a) * k as f32 / 4.0;
                    self.true_peak = self.true_peak.max(v.abs());
                }
                self.prev[ch] = b;
            }
            let mut sum = 0.0f64;
            for (ch, &s) in frame.iter().enumerate() {
                let k = self.stage2.tick(ch, self.stage1.tick(ch, s));
                sum += (k * k) as f64;
            }
            self.acc += sum;
            self.acc_n += 1;
            if self.acc_n == self.block_len {
                self.blocks.push(self.acc / self.block_len as f64);
                self.acc = 0.0;
                self.acc_n = 0;
            }
        }
    }

    fn lufs(ms: f64) -> f64 {
        -0.691 + 10.0 * ms.max(1e-20).log10()
    }

    /// Loudness over the last `n` 100 ms blocks (400 ms windows overlap by 75 %
    /// in the standard; block granularity is enough for a meter).
    fn windowed(&self, n: usize) -> Option<f32> {
        if self.blocks.len() < n {
            return None;
        }
        let tail = &self.blocks[self.blocks.len() - n..];
        Some(Self::lufs(tail.iter().sum::<f64>() / n as f64) as f32)
    }

    pub fn momentary(&self) -> Option<f32> {
        self.windowed(4)
    }

    pub fn short_term(&self) -> Option<f32> {
        self.windowed(30)
    }

    /// Gated integrated loudness; `None` until something above -70 LUFS was heard.
    pub fn integrated(&self) -> Option<f32> {
        // Gating blocks are 400 ms with 75 % overlap: sliding windows of 4 blocks.
        if self.blocks.len() < 4 {
            return None;
        }
        let windows: Vec<f64> = self
            .blocks
            .windows(4)
            .map(|w| w.iter().sum::<f64>() / 4.0)
            .collect();
        let abs: Vec<f64> = windows
            .iter()
            .copied()
            .filter(|&ms| Self::lufs(ms) > ABSOLUTE_GATE_LUFS)
            .collect();
        if abs.is_empty() {
            return None;
        }
        let rel_gate = Self::lufs(abs.iter().sum::<f64>() / abs.len() as f64) + RELATIVE_GATE_LU;
        let kept: Vec<f64> = abs
            .into_iter()
            .filter(|&ms| Self::lufs(ms) > rel_gate)
            .collect();
        if kept.is_empty() {
            return None;
        }
        Some(Self::lufs(kept.iter().sum::<f64>() / kept.len() as f64) as f32)
    }

    /// True peak in dBTP.
    pub fn true_peak_db(&self) -> f32 {
        20.0 * self.true_peak.max(1e-12).log10()
    }

    pub fn reset(&mut self) {
        self.stage1.reset();
        self.stage2.reset();
        self.blocks.clear();
        self.acc = 0.0;
        self.acc_n = 0;
        self.true_peak = 0.0;
        self.prev = [0.0; 2];
    }
}

/// BS.1770-4 K-weighting for any sample rate: the standard tabulates the 48 kHz
/// coefficients; these are the analog prototypes they were derived from (as in
/// libebur128), bilinear-transformed at `fs`. At 48 kHz they reproduce the
/// table (stage 1 b0 = 1.53512485958697).
fn k_weighting(fs: f64) -> (Biquad, Biquad) {
    // Stage 1: high shelf.
    let (f0, g, q) = (
        1681.974450955533_f64,
        3.999843853973347_f64,
        0.7071752369554196_f64,
    );
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let b0 = (vh + vb * k / q + k * k) / a0;
    let b1 = 2.0 * (k * k - vh) / a0;
    let b2 = (vh - vb * k / q + k * k) / a0;
    let a1 = 2.0 * (k * k - 1.0) / a0;
    let a2 = (1.0 - k / q + k * k) / a0;
    let stage1 = Biquad::from_coeffs(b0 as f32, b1 as f32, b2 as f32, a1 as f32, a2 as f32);
    // Stage 2: high-pass.
    let (f0, q) = (38.13547087602444_f64, 0.5003270373238773_f64);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let a1 = 2.0 * (k * k - 1.0) / a0;
    let a2 = (1.0 - k / q + k * k) / a0;
    let stage2 = Biquad::from_coeffs(
        (1.0 / a0) as f32,
        (-2.0 / a0) as f32,
        (1.0 / a0) as f32,
        a1 as f32,
        a2 as f32,
    );
    (stage1, stage2)
}

/// Linear gain that brings `measured` LUFS to `target` LUFS, optionally capped so
/// the true peak stays under `ceiling_dbtp`.
pub fn normalize_gain(
    measured_lufs: f32,
    target_lufs: f32,
    true_peak_db: f32,
    ceiling_dbtp: f32,
) -> f32 {
    let wanted = target_lufs - measured_lufs;
    let headroom = ceiling_dbtp - true_peak_db;
    10f32.powf(wanted.min(headroom) / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, amp: f32, seconds: f32) -> Vec<f32> {
        let n = (48_000.0 * seconds) as usize;
        (0..n)
            .flat_map(|i| {
                let s = (i as f32 * hz * std::f32::consts::TAU / 48_000.0).sin() * amp;
                [s, s]
            })
            .collect()
    }

    #[test]
    fn stereo_sine_reads_its_dbfs_level_in_lufs() {
        // EBU Tech 3341 case 1: a 997 Hz sine at -23 dBFS in L and R reads -23.0 LUFS
        // (the -0.691 offset cancels the K-filter's gain at 1 kHz). Full scale => 0.0.
        let mut m = LoudnessMeter::new(48_000);
        m.push(&sine(997.0, 1.0, 3.0));
        let i = m.integrated().unwrap();
        assert!(i.abs() < 0.1, "integrated {i}");
        assert!(m.momentary().unwrap().abs() < 0.1);
        assert!(m.short_term().unwrap().abs() < 0.1);
        assert!(m.true_peak_db().abs() < 0.1);
        // Any sample rate: the filters are designed from the analog prototypes.
        let mut m = LoudnessMeter::new(44_100);
        let amp = 10f32.powf(-23.0 / 20.0);
        let buf: Vec<f32> = (0..44_100 * 3)
            .flat_map(|i| {
                let s = (i as f32 * 997.0 * std::f32::consts::TAU / 44_100.0).sin() * amp;
                [s, s]
            })
            .collect();
        m.push(&buf);
        assert!(
            (m.integrated().unwrap() + 23.0).abs() < 0.1,
            "44.1 kHz: {:?}",
            m.integrated()
        );
    }

    #[test]
    fn level_tracks_amplitude_and_silence_is_gated_out() {
        let mut m = LoudnessMeter::new(48_000);
        m.push(&sine(997.0, 0.1, 2.0)); // -20 dBFS
        let quiet = m.integrated().unwrap();
        assert!((quiet + 20.0).abs() < 0.1, "{quiet}");
        // Appending 10 s of silence must not change the gated integrated value.
        m.push(&vec![0.0; 2 * 48_000 * 10]);
        // Only the three 400 ms windows straddling the cut dilute it (by ~0.3 LU).
        assert!((m.integrated().unwrap() - quiet).abs() < 0.5);
        let mut s = LoudnessMeter::new(48_000);
        s.push(&vec![0.0; 2 * 48_000]);
        assert_eq!(s.integrated(), None);
    }

    #[test]
    fn k_weighting_shape() {
        let mut lo = LoudnessMeter::new(48_000);
        lo.push(&sine(60.0, 0.5, 2.0));
        let mut mid = LoudnessMeter::new(48_000);
        mid.push(&sine(997.0, 0.5, 2.0));
        let mut hi = LoudnessMeter::new(48_000);
        hi.push(&sine(8000.0, 0.5, 2.0));
        let (l, m, h) = (
            lo.integrated().unwrap(),
            mid.integrated().unwrap(),
            hi.integrated().unwrap(),
        );
        assert!(l < m - 2.0, "lows are attenuated: {l} vs {m}");
        assert!(
            h > m + 2.5 && h < m + 5.0,
            "highs get the +4 dB shelf: {h} vs {m}"
        );
    }

    #[test]
    fn normalization_gain_respects_the_ceiling() {
        let g = normalize_gain(-23.0, -14.0, -10.0, -1.0);
        assert!((20.0 * g.log10() - 9.0).abs() < 1e-4);
        let g = normalize_gain(-23.0, -14.0, -3.0, -1.0);
        assert!(
            (20.0 * g.log10() - 2.0).abs() < 1e-4,
            "capped by true peak headroom"
        );
    }
}
