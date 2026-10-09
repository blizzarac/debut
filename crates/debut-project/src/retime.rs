//! Speed ramps (TL-09): a clip's speed as a piecewise-linear curve over
//! clip-local time, and the source offset it accumulates.
//!
//! Constant speed (including 0 = freeze and negative = reverse) stays exact in
//! `Clip::speed`. A ramp replaces it when present; the offset is the integral of
//! the curve, computed in `f64` and rounded to a microsecond, which is far below
//! a frame or sample and keeps the rationals small.

use debut_core::Rational;
use serde::{Deserialize, Serialize};

/// One point of a speed ramp: at clip-local time `at`, the clip plays at `speed`
/// times normal. Speed is held before the first and after the last key.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeedKey {
    pub at: Rational,
    pub speed: f64,
}

/// Microsecond grid for ramped source times.
const GRID: i64 = 1_000_000;

pub fn to_rational(x: f64) -> Rational {
    Rational::new((x * GRID as f64).round() as i64, GRID)
}

/// Speed of the ramp at clip-local time `x` (seconds). `keys` is sorted and
/// non-empty.
pub fn speed_at(keys: &[SpeedKey], x: f64) -> f64 {
    let first = keys[0];
    if x <= first.at.as_f64() {
        return first.speed;
    }
    for w in keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (xa, xb) = (a.at.as_f64(), b.at.as_f64());
        if x <= xb {
            let f = if xb > xa { (x - xa) / (xb - xa) } else { 1.0 };
            return a.speed + (b.speed - a.speed) * f;
        }
    }
    keys[keys.len() - 1].speed
}

/// Source seconds played from clip-local 0 to `x`: the integral of the ramp.
/// Negative `x` (a head extended past the first key) uses the held first speed.
pub fn offset(keys: &[SpeedKey], x: f64) -> f64 {
    let first = keys[0];
    let x0 = first.at.as_f64();
    if x <= x0 {
        return first.speed * x;
    }
    let mut acc = first.speed * x0;
    for w in keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (xa, xb) = (a.at.as_f64(), b.at.as_f64());
        let end = x.min(xb);
        if end > xa {
            let (sa, se) = (a.speed, speed_at(keys, end));
            acc += (sa + se) * 0.5 * (end - xa);
        }
        if x <= xb {
            return acc;
        }
    }
    let last = keys[keys.len() - 1];
    acc + last.speed * (x - last.at.as_f64())
}

/// Re-base `keys` so clip-local `cut` becomes 0, keeping the curve's shape: a
/// key is inserted at the cut and earlier keys are dropped. A negative `cut`
/// (head extended) shifts every key later.
pub fn rebase(keys: &mut Vec<SpeedKey>, cut: Rational) {
    if keys.is_empty() || cut.is_zero() {
        return;
    }
    let at_cut = speed_at(keys, cut.as_f64());
    keys.retain(|k| k.at > cut);
    for k in keys.iter_mut() {
        k.at -= cut;
    }
    // A head extended before 0 keeps all keys; the held first speed then
    // starts at the new 0.
    keys.insert(
        0,
        SpeedKey {
            at: Rational::ZERO,
            speed: at_cut,
        },
    );
}

/// Sort by time and keep the last key at each time.
pub fn normalize(keys: &mut Vec<SpeedKey>) {
    keys.sort_by_key(|k| k.at);
    let mut out: Vec<SpeedKey> = Vec::with_capacity(keys.len());
    for k in keys.drain(..) {
        match out.last_mut() {
            Some(l) if l.at == k.at => *l = k,
            _ => out.push(k),
        }
    }
    *keys = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(at: i64, speed: f64) -> SpeedKey {
        SpeedKey {
            at: Rational::from_int(at),
            speed,
        }
    }

    #[test]
    fn offset_integrates_the_curve() {
        // 1x for 1 s, ramp to 3x over 2 s, then hold.
        let keys = [k(1, 1.0), k(3, 3.0)];
        assert_eq!(speed_at(&keys, 0.5), 1.0);
        assert_eq!(speed_at(&keys, 2.0), 2.0);
        assert_eq!(speed_at(&keys, 9.0), 3.0);
        assert!((offset(&keys, 1.0) - 1.0).abs() < 1e-12);
        // Trapezoid 1..2 over 1 s = 1.5.
        assert!((offset(&keys, 2.0) - 2.5).abs() < 1e-12);
        // Full ramp area 4, then 3 s/s.
        assert!((offset(&keys, 3.0) - 5.0).abs() < 1e-12);
        assert!((offset(&keys, 4.0) - 8.0).abs() < 1e-12);
        assert!((offset(&keys, -1.0) + 1.0).abs() < 1e-12);
    }

    #[test]
    fn rebase_keeps_the_shape_after_the_cut() {
        let keys = vec![k(1, 1.0), k(3, 3.0)];
        let mut cut = keys.clone();
        rebase(&mut cut, Rational::from_int(2));
        assert_eq!(cut, vec![k(0, 2.0), k(1, 3.0)]);
        for x in [0.0, 0.5, 1.0, 2.5] {
            let whole = offset(&keys, 2.0 + x) - offset(&keys, 2.0);
            assert!((offset(&cut, x) - whole).abs() < 1e-12, "{x}");
        }
        let mut ext = keys.clone();
        rebase(&mut ext, Rational::from_int(-1));
        assert_eq!(ext, vec![k(0, 1.0), k(2, 1.0), k(4, 3.0)]);
    }
}
