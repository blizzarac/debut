//! Keyframed parameters (FX-01): hold, linear, eased and cubic-bezier segments.

use crate::Rational;
use serde::{Deserialize, Serialize};

/// Interpolation of the segment that *leaves* a keyframe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Interp {
    Hold,
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// CSS-style cubic bezier timing: control points `(x1, y1)`, `(x2, y2)` with
    /// `x` in `[0, 1]`; the graph editor exposes these as handles.
    Bezier {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub t: Rational,
    pub value: f64,
    pub interp: Interp,
}

/// Keyframes sorted by time. Outside the first/last key the curve holds that value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    keys: Vec<Keyframe>,
}

impl Curve {
    pub fn constant(value: f64) -> Self {
        Self {
            keys: vec![Keyframe {
                t: Rational::ZERO,
                value,
                interp: Interp::Hold,
            }],
        }
    }

    pub fn keys(&self) -> &[Keyframe] {
        &self.keys
    }

    /// Insert or replace the key at `t`.
    pub fn set(&mut self, t: Rational, value: f64, interp: Interp) {
        match self.keys.binary_search_by(|k| k.t.cmp(&t)) {
            Ok(i) => self.keys[i] = Keyframe { t, value, interp },
            Err(i) => self.keys.insert(i, Keyframe { t, value, interp }),
        }
    }

    pub fn remove(&mut self, t: Rational) -> bool {
        match self.keys.binary_search_by(|k| k.t.cmp(&t)) {
            Ok(i) => {
                self.keys.remove(i);
                true
            }
            Err(_) => false,
        }
    }

    pub fn eval(&self, t: Rational) -> f64 {
        let Some(first) = self.keys.first() else {
            return 0.0;
        };
        if t <= first.t {
            return first.value;
        }
        let last = self.keys.last().unwrap();
        if t >= last.t {
            return last.value;
        }
        let i = match self.keys.binary_search_by(|k| k.t.cmp(&t)) {
            Ok(i) => return self.keys[i].value,
            Err(i) => i - 1,
        };
        let a = self.keys[i];
        let b = self.keys[i + 1];
        let u = ((t - a.t).as_f64()) / ((b.t - a.t).as_f64());
        let s = match a.interp {
            Interp::Hold => 0.0,
            Interp::Linear => u,
            Interp::EaseIn => bezier_timing(0.42, 0.0, 1.0, 1.0, u),
            Interp::EaseOut => bezier_timing(0.0, 0.0, 0.58, 1.0, u),
            Interp::EaseInOut => bezier_timing(0.42, 0.0, 0.58, 1.0, u),
            Interp::Bezier { x1, y1, x2, y2 } => bezier_timing(x1, y1, x2, y2, u),
        };
        a.value + (b.value - a.value) * s
    }
}

fn cubic(p1: f64, p2: f64, s: f64) -> f64 {
    // Bernstein form with p0 = 0, p3 = 1.
    let inv = 1.0 - s;
    3.0 * inv * inv * s * p1 + 3.0 * inv * s * s * p2 + s * s * s
}

/// Solve the bezier's x(s) = u for s by Newton with a bisection fallback, then
/// return y(s).
fn bezier_timing(x1: f64, y1: f64, x2: f64, y2: f64, u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    let mut s = u;
    for _ in 0..8 {
        let x = cubic(x1, x2, s) - u;
        if x.abs() < 1e-7 {
            return cubic(y1, y2, s);
        }
        let inv = 1.0 - s;
        let dx = 3.0 * inv * inv * x1 + 6.0 * inv * s * (x2 - x1) + 3.0 * s * s * (1.0 - x2);
        if dx.abs() < 1e-6 {
            break;
        }
        s -= x / dx;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        s = 0.5 * (lo + hi);
        if cubic(x1, x2, s) < u {
            lo = s;
        } else {
            hi = s;
        }
    }
    cubic(y1, y2, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i64) -> Rational {
        Rational::from_int(n)
    }

    #[test]
    fn holds_outside_and_on_keys() {
        let mut c = Curve::default();
        c.set(r(2), 10.0, Interp::Linear);
        c.set(r(4), 20.0, Interp::Linear);
        assert_eq!(c.eval(r(0)), 10.0);
        assert_eq!(c.eval(r(2)), 10.0);
        assert_eq!(c.eval(r(4)), 20.0);
        assert_eq!(c.eval(r(9)), 20.0);
    }

    #[test]
    fn linear_and_hold_segments() {
        let mut c = Curve::default();
        c.set(r(0), 0.0, Interp::Linear);
        c.set(r(4), 8.0, Interp::Hold);
        c.set(r(8), 100.0, Interp::Linear);
        assert_eq!(c.eval(r(1)), 2.0);
        assert_eq!(c.eval(r(3)), 6.0);
        assert_eq!(c.eval(r(6)), 8.0); // held until the next key
        assert_eq!(c.eval(r(8)), 100.0);
    }

    #[test]
    fn eases_are_monotone_and_hit_endpoints() {
        for interp in [
            Interp::EaseIn,
            Interp::EaseOut,
            Interp::EaseInOut,
            Interp::Bezier {
                x1: 0.2,
                y1: 0.8,
                x2: 0.8,
                y2: 0.2,
            },
        ] {
            let mut c = Curve::default();
            c.set(r(0), 0.0, interp);
            c.set(r(100), 1.0, Interp::Linear);
            let mut prev = -1.0;
            for i in 0..=100 {
                let v = c.eval(r(i));
                assert!(
                    v >= prev - 1e-9,
                    "{interp:?} not monotone at {i}: {v} < {prev}"
                );
                prev = v;
            }
            assert!((c.eval(r(0))).abs() < 1e-9);
            assert!((c.eval(r(100)) - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn ease_in_starts_slow_and_ease_out_starts_fast() {
        let mut ci = Curve::default();
        ci.set(r(0), 0.0, Interp::EaseIn);
        ci.set(r(10), 1.0, Interp::Linear);
        let mut co = Curve::default();
        co.set(r(0), 0.0, Interp::EaseOut);
        co.set(r(10), 1.0, Interp::Linear);
        assert!(ci.eval(r(2)) < 0.2);
        assert!(co.eval(r(2)) > 0.2);
    }

    #[test]
    fn set_replaces_and_remove_works() {
        let mut c = Curve::constant(5.0);
        c.set(r(0), 7.0, Interp::Hold);
        assert_eq!(c.keys().len(), 1);
        assert_eq!(c.eval(r(3)), 7.0);
        assert!(c.remove(r(0)));
        assert!(!c.remove(r(0)));
    }
}
