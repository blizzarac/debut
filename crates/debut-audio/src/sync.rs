//! Audio-based alignment of two recordings of the same event (MED-11 multicam
//! sync): coarse envelopes, normalised cross-correlation over a lag window,
//! then the lag with the highest match. Robust to level differences and to
//! one recording being a different mix of the same sound.

/// Envelope sample rate used for the correlation: cheap, and plenty for
/// sync accuracy within a video frame.
const ENV_RATE: u32 = 400;

/// RMS envelope of a mono signal at `ENV_RATE`, mean-removed so silence does
/// not correlate with silence.
fn envelope(mono: &[f32], sample_rate: u32) -> Vec<f32> {
    let block = (sample_rate / ENV_RATE).max(1) as usize;
    let mut env: Vec<f32> = mono
        .chunks(block)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
        .collect();
    // Log-ish compression evens out loud transients vs. quiet tails.
    for v in &mut env {
        *v = (1.0 + *v * 100.0).ln();
    }
    let mean = env.iter().sum::<f32>() / env.len().max(1) as f32;
    for v in &mut env {
        *v -= mean;
    }
    env
}

/// Result of an alignment attempt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alignment {
    /// How many samples (at the input rate) `b` lags `a`: positive means the
    /// event happens later in `b`, so `b` must start `offset` samples in to
    /// line up.
    pub offset_samples: i64,
    /// Normalised correlation at the best lag, 0..1; below about 0.3 the
    /// recordings probably do not share audio.
    pub confidence: f32,
}

/// Find how far `b` lags `a` (both mono at `sample_rate`), searching lags up
/// to `max_offset_s` either way. `None` when either input is too short.
pub fn align(a: &[f32], b: &[f32], sample_rate: u32, max_offset_s: f32) -> Option<Alignment> {
    let ea = envelope(a, sample_rate);
    let eb = envelope(b, sample_rate);
    let max_lag = (max_offset_s * ENV_RATE as f32).round() as i64;
    let min_len = ea.len().min(eb.len()) as i64;
    if min_len < 2 * ENV_RATE as i64 / 4 || max_lag <= 0 {
        return None;
    }
    let max_lag = max_lag.min(min_len - 1);
    let norm = |e: &[f32]| e.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    let (na, nb) = (norm(&ea), norm(&eb));
    let mut best = (0i64, f32::MIN);
    for lag in -max_lag..=max_lag {
        // Overlap of ea[i] with eb[i + lag].
        let (start, end) = (
            lag.max(0) - lag,
            (ea.len() as i64).min(eb.len() as i64 - lag),
        );
        if end - start < min_len / 2 {
            continue;
        }
        let mut dot = 0.0f32;
        for i in start..end {
            dot += ea[i as usize] * eb[(i + lag) as usize];
        }
        let score = dot / (na * nb);
        if score > best.1 {
            best = (lag, score);
        }
    }
    Some(Alignment {
        offset_samples: best.0 * sample_rate as i64 / ENV_RATE as i64,
        confidence: best.1.clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A noise burst train: distinctive enough to align on.
    fn signal(len: usize, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..len)
            .map(|i| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (x >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
                // Bursts every 0.7 s with varying lengths.
                let t = i as f32 / 48_000.0;
                let phase = t % 0.7;
                let on = phase < 0.05 + 0.1 * ((t / 0.7).floor() % 3.0);
                if on {
                    noise
                } else {
                    noise * 0.02
                }
            })
            .collect()
    }

    #[test]
    fn finds_the_delay_between_two_takes_of_the_same_sound() {
        let sr = 48_000;
        let a = signal(sr as usize * 6, 7);
        // b is a quieter copy that starts 0.9 s later (silence padded in front).
        let delay = (0.9 * sr as f32) as usize;
        let mut b = vec![0.0f32; delay];
        b.extend(a.iter().map(|s| s * 0.3));
        let al = align(&a, &b, sr, 2.0).unwrap();
        let err = (al.offset_samples - delay as i64).abs();
        assert!(
            err <= (sr / ENV_RATE) as i64 * 2,
            "offset off by {err} samples"
        );
        assert!(al.confidence > 0.8, "{}", al.confidence);
        // And the other way round.
        let back = align(&b, &a, sr, 2.0).unwrap();
        assert!((back.offset_samples + delay as i64).abs() <= (sr / ENV_RATE) as i64 * 2);
        // An unrelated recording (noise gated with a different, irregular
        // rhythm) scores clearly lower.
        let mut x = 99u32;
        let c: Vec<f32> = (0..sr as usize * 6)
            .map(|i| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (x >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
                let t = i as f32 / sr as f32;
                let on = (t * 1.37).sin() > 0.6 || (t * 0.41 + 1.0).sin() > 0.9;
                if on {
                    noise
                } else {
                    noise * 0.02
                }
            })
            .collect();
        let bad = align(&a, &c, sr, 2.0).unwrap();
        assert!(
            bad.confidence < 0.5 && bad.confidence < al.confidence * 0.6,
            "{} vs {}",
            bad.confidence,
            al.confidence
        );
        assert!(align(&a[..100], &b, sr, 2.0).is_none());
    }
}
