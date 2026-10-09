//! Multi-resolution peak cache for waveform display (AUD-04). Level 0 holds the
//! min/max of every `BASE_BUCKET` mono samples; each further level halves the
//! resolution by merging neighbour pairs, so any zoom reads a level whose
//! buckets are no larger than one output pixel and aggregates only a few
//! entries per pixel.

/// Samples per bucket at the finest level.
pub const BASE_BUCKET: usize = 256;

/// The peaks of one recording, mixed to mono.
#[derive(Clone, Debug, PartialEq)]
pub struct Peaks {
    sample_rate: u32,
    samples: u64,
    /// `levels[k][i]` is the [min, max] of samples `[i, i + 1) * (BASE_BUCKET << k)`.
    levels: Vec<Vec<[f32; 2]>>,
}

/// Feeds samples in blocks as they are decoded; `finish` builds the levels.
#[derive(Clone, Debug)]
pub struct PeaksBuilder {
    sample_rate: u32,
    samples: u64,
    base: Vec<[f32; 2]>,
    cur: [f32; 2],
    in_cur: usize,
}

fn merge(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0].min(b[0]), a[1].max(b[1])]
}

const EMPTY: [f32; 2] = [f32::INFINITY, f32::NEG_INFINITY];

impl PeaksBuilder {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            samples: 0,
            base: Vec::new(),
            cur: EMPTY,
            in_cur: 0,
        }
    }

    /// Append mono samples.
    pub fn push(&mut self, mono: &[f32]) {
        for &s in mono {
            self.cur = merge(self.cur, [s, s]);
            self.in_cur += 1;
            if self.in_cur == BASE_BUCKET {
                self.base.push(self.cur);
                self.cur = EMPTY;
                self.in_cur = 0;
            }
        }
        self.samples += mono.len() as u64;
    }

    pub fn finish(mut self) -> Peaks {
        if self.in_cur > 0 {
            self.base.push(self.cur);
        }
        let mut levels = vec![self.base];
        while levels.last().is_some_and(|l| l.len() > 1) {
            let next = levels
                .last()
                .unwrap()
                .chunks(2)
                .map(|p| p.iter().copied().fold(EMPTY, merge))
                .collect();
            levels.push(next);
        }
        Peaks {
            sample_rate: self.sample_rate,
            samples: self.samples,
            levels,
        }
    }
}

impl Peaks {
    /// Peaks of a whole mono signal at once.
    pub fn from_mono(mono: &[f32], sample_rate: u32) -> Self {
        let mut b = PeaksBuilder::new(sample_rate);
        b.push(mono);
        b.finish()
    }

    pub fn duration_s(&self) -> f64 {
        self.samples as f64 / self.sample_rate.max(1) as f64
    }

    /// [min, max] over base buckets `[lo, hi)`: the bottom-up segment-tree
    /// query, touching at most two entries per level.
    fn query(&self, mut lo: usize, mut hi: usize) -> [f32; 2] {
        let mut acc = EMPTY;
        let mut k = 0;
        while lo < hi && k < self.levels.len() {
            let level = &self.levels[k];
            if lo & 1 == 1 {
                acc = merge(acc, level[lo]);
                lo += 1;
            }
            if hi & 1 == 1 {
                hi -= 1;
                acc = merge(acc, level[hi]);
            }
            lo >>= 1;
            hi >>= 1;
            k += 1;
        }
        acc
    }

    /// `buckets` [min, max] pairs covering `[start_s, end_s)`, exact to one
    /// base bucket at any zoom. Time outside the recording reads as silence.
    pub fn range(&self, start_s: f64, end_s: f64, buckets: usize) -> Vec<[f32; 2]> {
        if buckets == 0 || end_s <= start_s {
            return Vec::new();
        }
        let rate = self.sample_rate as f64;
        let per_out = (end_s - start_s) * rate / buckets as f64;
        let n = self.levels[0].len();
        (0..buckets)
            .map(|i| {
                let a = start_s * rate + i as f64 * per_out;
                let b = a + per_out;
                if b <= 0.0 || a >= self.samples as f64 {
                    return [0.0, 0.0];
                }
                let lo = (a.max(0.0) / BASE_BUCKET as f64).floor() as usize;
                let hi = ((b / BASE_BUCKET as f64).ceil() as usize).clamp(lo + 1, n);
                let v = self.query(lo.min(n), hi);
                if v[0] > v[1] {
                    [0.0, 0.0]
                } else {
                    v
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amp: f32, seconds: f32, rate: u32) -> Vec<f32> {
        (0..(seconds * rate as f32) as usize)
            .map(|i| amp * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin())
            .collect()
    }

    #[test]
    fn peaks_follow_the_envelope_at_every_zoom() {
        // A rate that is a whole number of base buckets per second, so the
        // one-second sections line up with bucket edges and the checks are exact.
        let rate = 192 * BASE_BUCKET as u32;
        // One second of silence, then one second at 0.5, then one at 0.9.
        let mut signal = vec![0.0f32; rate as usize];
        signal.extend(sine(0.5, 1.0, rate));
        signal.extend(sine(0.9, 1.0, rate));
        let p = Peaks::from_mono(&signal, rate);
        assert!((p.duration_s() - 3.0).abs() < 1e-9);
        for buckets in [3, 30, 300, 3000, 30_000] {
            let r = p.range(0.0, 3.0, buckets);
            assert_eq!(r.len(), buckets);
            let third = buckets / 3;
            let max_in = |s: &[[f32; 2]]| s.iter().map(|b| b[1]).fold(0.0f32, f32::max);
            assert_eq!(max_in(&r[..third]), 0.0, "silent first second at {buckets}");
            assert!(
                (max_in(&r[third..2 * third]) - 0.5).abs() < 0.01,
                "{buckets}"
            );
            assert!((max_in(&r[2 * third..]) - 0.9).abs() < 0.01, "{buckets}");
            assert!(
                r[2 * third..].iter().all(|b| b[0] < -0.8),
                "minima follow too"
            );
        }
        // Past the end: silence; reversed or empty requests: nothing.
        assert!(p.range(3.0, 4.0, 10).iter().all(|b| *b == [0.0, 0.0]));
        assert!(p.range(2.0, 1.0, 10).is_empty());
        // Coarse and fine levels agree on the same span.
        let coarse = p.range(1.0, 3.0, 2);
        let fine = p.range(1.0, 3.0, 2000);
        let fold = |r: &[[f32; 2]]| r.iter().copied().fold(EMPTY, merge);
        assert_eq!(fold(&coarse), fold(&fine));
    }

    #[test]
    fn builder_matches_one_shot_whatever_the_block_size() {
        let s = sine(0.7, 0.37, 48_000);
        let mut b = PeaksBuilder::new(48_000);
        for chunk in s.chunks(1000) {
            b.push(chunk);
        }
        assert_eq!(b.finish(), Peaks::from_mono(&s, 48_000));
    }
}
