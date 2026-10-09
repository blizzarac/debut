//! Time is exact. A [`Rational`] of seconds represents both frame-aligned video time
//! (TL-01) and sample-aligned audio time (AUD-01) without drift (MED-03).

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

fn gcd(mut a: i64, mut b: i64) -> i64 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    if a == 0 {
        1
    } else {
        a
    }
}

/// Exact rational number `num / den`, always stored normalized with `den > 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const ZERO: Rational = Rational { num: 0, den: 1 };
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    /// Build and normalize. Panics on `den == 0`.
    pub fn new(num: i64, den: i64) -> Self {
        assert!(den != 0, "denominator must be non-zero");
        let g = gcd(num, den);
        let sign = if den < 0 { -1 } else { 1 };
        Self {
            num: sign * num / g,
            den: sign * den / g,
        }
    }

    pub fn from_int(n: i64) -> Self {
        Self { num: n, den: 1 }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    pub fn is_zero(self) -> bool {
        self.num == 0
    }

    pub fn is_negative(self) -> bool {
        self.num < 0
    }

    pub fn min(self, other: Self) -> Self {
        if self <= other {
            self
        } else {
            other
        }
    }

    pub fn max(self, other: Self) -> Self {
        if self >= other {
            self
        } else {
            other
        }
    }

    /// Largest integer <= self.
    pub fn floor(self) -> i64 {
        self.num.div_euclid(self.den)
    }

    /// Smallest integer >= self.
    pub fn ceil(self) -> i64 {
        -((-self.num).div_euclid(self.den))
    }

    /// Nearest integer, half rounds up.
    pub fn round(self) -> i64 {
        (self + Rational::new(1, 2)).floor()
    }

    /// Checked arithmetic over i128 to avoid overflow for large denominators.
    fn combine(self, other: Self, f: impl Fn(i128, i128) -> i128) -> Self {
        let a = self.num as i128 * other.den as i128;
        let b = other.num as i128 * self.den as i128;
        let den = self.den as i128 * other.den as i128;
        let num = f(a, b);
        Self::from_i128(num, den)
    }

    fn from_i128(num: i128, den: i128) -> Self {
        let g = {
            let (mut a, mut b) = (num.abs(), den.abs());
            while b != 0 {
                let t = a % b;
                a = b;
                b = t;
            }
            if a == 0 {
                1
            } else {
                a
            }
        };
        let sign = if den < 0 { -1 } else { 1 };
        let num = sign * num / g;
        let den = sign * den / g;
        Self {
            num: i64::try_from(num).expect("rational numerator overflow"),
            den: i64::try_from(den).expect("rational denominator overflow"),
        }
    }
}

impl Add for Rational {
    type Output = Rational;
    fn add(self, rhs: Self) -> Self {
        self.combine(rhs, |a, b| a + b)
    }
}

impl Sub for Rational {
    type Output = Rational;
    fn sub(self, rhs: Self) -> Self {
        self.combine(rhs, |a, b| a - b)
    }
}

impl Mul for Rational {
    type Output = Rational;
    fn mul(self, rhs: Self) -> Self {
        Self::from_i128(
            self.num as i128 * rhs.num as i128,
            self.den as i128 * rhs.den as i128,
        )
    }
}

impl Neg for Rational {
    type Output = Rational;
    fn neg(self) -> Self {
        Self {
            num: -self.num,
            den: self.den,
        }
    }
}

impl AddAssign for Rational {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign for Rational {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        let a = self.num as i128 * other.den as i128;
        let b = other.num as i128 * self.den as i128;
        a.cmp(&b)
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// Timeline or clip frame rate in frames per second, e.g. 24000/1001 for 23.976.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate(pub Rational);

impl FrameRate {
    pub const FPS_24: FrameRate = FrameRate(Rational { num: 24, den: 1 });
    pub const FPS_23_976: FrameRate = FrameRate(Rational {
        num: 24000,
        den: 1001,
    });
    pub const FPS_25: FrameRate = FrameRate(Rational { num: 25, den: 1 });
    pub const FPS_29_97: FrameRate = FrameRate(Rational {
        num: 30000,
        den: 1001,
    });
    pub const FPS_30: FrameRate = FrameRate(Rational { num: 30, den: 1 });
    pub const FPS_50: FrameRate = FrameRate(Rational { num: 50, den: 1 });
    pub const FPS_59_94: FrameRate = FrameRate(Rational {
        num: 60000,
        den: 1001,
    });
    pub const FPS_60: FrameRate = FrameRate(Rational { num: 60, den: 1 });

    pub fn new(num: i64, den: i64) -> Self {
        let r = Rational::new(num, den);
        assert!(r.num > 0, "frame rate must be positive");
        FrameRate(r)
    }

    /// Duration of one frame in seconds.
    pub fn frame_duration(self) -> Rational {
        Rational::new(self.0.den, self.0.num)
    }

    /// Time at the start of frame `n`.
    pub fn frame_to_time(self, n: i64) -> Rational {
        Rational::from_int(n) * self.frame_duration()
    }

    /// Frame containing time `t` (floor).
    pub fn time_to_frame(self, t: Rational) -> i64 {
        (t * self.0).floor()
    }

    /// Snap `t` down to the nearest frame boundary.
    pub fn snap(self, t: Rational) -> Rational {
        self.frame_to_time(self.time_to_frame(t))
    }

    /// Nominal integer rate used for timecode counting (30 for 29.97, 24 for 23.976).
    pub fn nominal(self) -> i64 {
        self.0.ceil()
    }

    /// True for the NTSC family (x/1001) where drop-frame timecode applies.
    pub fn is_fractional(self) -> bool {
        self.0.den != 1
    }
}

/// SMPTE timecode (MED-04). Drop-frame changes only the label, never the frame count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timecode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u16,
    pub drop_frame: bool,
}

impl Timecode {
    /// Label for absolute frame `n` at `rate`. Drop-frame applies only to 29.97/59.94.
    pub fn from_frames(n: i64, rate: FrameRate, drop_frame: bool) -> Self {
        let nominal = rate.nominal();
        let drop = drop_frame && rate.is_fractional() && nominal % 30 == 0;
        let n = n.max(0);
        let n = if drop {
            // Drop 2 frames (x2 for 59.94) every minute except every tenth minute.
            let drop_per_min = 2 * (nominal / 30);
            let frames_per_10min = nominal * 600 - 9 * drop_per_min;
            let frames_per_min = nominal * 60 - drop_per_min;
            let d = n / frames_per_10min;
            let m = n % frames_per_10min;
            let extra = if m > drop_per_min {
                drop_per_min * 9 * d + drop_per_min * ((m - drop_per_min) / frames_per_min)
            } else {
                drop_per_min * 9 * d
            };
            n + extra
        } else {
            n
        };
        let frames = (n % nominal) as u16;
        let total_secs = n / nominal;
        Self {
            hours: ((total_secs / 3600) % 24) as u8,
            minutes: ((total_secs / 60) % 60) as u8,
            seconds: (total_secs % 60) as u8,
            frames,
            drop_frame: drop,
        }
    }

    /// Parse "HH:MM:SS:FF"; ';' or '.' before the frames marks drop-frame.
    /// `None` for anything else or out-of-range fields.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let drop_frame = text.contains(';') || text.matches('.').count() == 1;
        let parts: Vec<&str> = text.split([':', ';', '.']).collect();
        let [h, m, s, f] = parts.as_slice() else {
            return None;
        };
        let (hours, minutes, seconds, frames) = (
            h.parse::<u8>().ok()?,
            m.parse::<u8>().ok()?,
            s.parse::<u8>().ok()?,
            f.parse::<u16>().ok()?,
        );
        (hours < 24 && minutes < 60 && seconds < 60).then_some(Self {
            hours,
            minutes,
            seconds,
            frames,
            drop_frame,
        })
    }

    /// Absolute frame number for this label at `rate`.
    pub fn to_frames(self, rate: FrameRate) -> i64 {
        let nominal = rate.nominal();
        let h = self.hours as i64;
        let m = self.minutes as i64;
        let s = self.seconds as i64;
        let f = self.frames as i64;
        let total_minutes = h * 60 + m;
        let mut n = ((total_minutes * 60) + s) * nominal + f;
        if self.drop_frame && rate.is_fractional() && nominal % 30 == 0 {
            let drop_per_min = 2 * (nominal / 30);
            n -= drop_per_min * (total_minutes - total_minutes / 10);
        }
        n
    }
}

impl fmt::Display for Timecode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sep = if self.drop_frame { ';' } else { ':' };
        write!(
            f,
            "{:02}:{:02}:{:02}{}{:02}",
            self.hours, self.minutes, self.seconds, sep, self.frames
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_parses_and_round_trips() {
        let tc = Timecode::parse("10:00:00:12").unwrap();
        assert_eq!(tc.to_frames(FrameRate::FPS_25), 10 * 3600 * 25 + 12);
        assert_eq!(tc.to_string(), "10:00:00:12");
        let df = Timecode::parse("01:00:00;02").unwrap();
        assert!(df.drop_frame);
        assert_eq!(
            Timecode::from_frames(
                df.to_frames(FrameRate::FPS_29_97),
                FrameRate::FPS_29_97,
                true
            ),
            df
        );
        assert!(Timecode::parse("25:00:00:00").is_none());
        assert!(Timecode::parse("10:00:00").is_none());
        assert!(Timecode::parse("aa:00:00:00").is_none());
    }

    #[test]
    fn rational_normalizes_sign_and_gcd() {
        assert_eq!(Rational::new(6, -4), Rational { num: -3, den: 2 });
        assert_eq!(Rational::new(0, 7), Rational::ZERO);
    }

    #[test]
    fn rational_arithmetic_is_exact() {
        let a = Rational::new(1, 3);
        let b = Rational::new(1, 6);
        assert_eq!(a + b, Rational::new(1, 2));
        assert_eq!(a - b, Rational::new(1, 6));
        assert_eq!(a * b, Rational::new(1, 18));
        assert!(a > b);
        assert_eq!(Rational::new(7, 2).floor(), 3);
        assert_eq!(Rational::new(-7, 2).floor(), -4);
        assert_eq!(Rational::new(7, 2).ceil(), 4);
        assert_eq!(Rational::new(7, 2).round(), 4);
    }

    #[test]
    fn large_denominators_do_not_overflow() {
        let t = FrameRate::FPS_23_976.frame_to_time(1_000_000_000);
        let u = FrameRate::FPS_59_94.frame_to_time(1_000_000_000);
        let _ = t + u;
    }

    #[test]
    fn frames_round_trip_through_time() {
        for rate in [
            FrameRate::FPS_23_976,
            FrameRate::FPS_29_97,
            FrameRate::FPS_25,
            FrameRate::FPS_60,
        ] {
            for n in [0, 1, 999, 86_399, 1_000_000] {
                let t = rate.frame_to_time(n);
                assert_eq!(rate.time_to_frame(t), n, "{rate:?} frame {n}");
                assert_eq!(rate.snap(t), t);
            }
        }
    }

    #[test]
    fn time_to_frame_floors_mid_frame() {
        let rate = FrameRate::FPS_24;
        let mid = rate.frame_to_time(10) + Rational::new(1, 100);
        assert_eq!(rate.time_to_frame(mid), 10);
    }

    #[test]
    fn non_drop_timecode_round_trips() {
        let rate = FrameRate::FPS_25;
        let tc = Timecode::from_frames(25 * 3661 + 7, rate, false);
        assert_eq!(tc.to_string(), "01:01:01:07");
        assert_eq!(tc.to_frames(rate), 25 * 3661 + 7);
    }

    #[test]
    fn drop_frame_timecode_skips_frames_at_minute_boundaries() {
        let rate = FrameRate::FPS_29_97;
        // Frame 1800 is one nominal minute; drop-frame labels it 00:01:00;02.
        assert_eq!(
            Timecode::from_frames(1800, rate, true).to_string(),
            "00:01:00;02"
        );
        assert_eq!(
            Timecode::from_frames(1799, rate, true).to_string(),
            "00:00:59;29"
        );
        // Tenth minute does not drop.
        assert_eq!(
            Timecode::from_frames(17982, rate, true).to_string(),
            "00:10:00;00"
        );
        // One hour of drop-frame = 107892 frames.
        assert_eq!(
            Timecode::from_frames(107_892, rate, true).to_string(),
            "01:00:00;00"
        );
    }

    #[test]
    fn drop_frame_timecode_round_trips() {
        let rate = FrameRate::FPS_29_97;
        for n in [0, 1, 1799, 1800, 17981, 17982, 107_891, 107_892, 500_000] {
            let tc = Timecode::from_frames(n, rate, true);
            assert_eq!(tc.to_frames(rate), n, "frame {n} -> {tc}");
        }
        let rate = FrameRate::FPS_59_94;
        for n in [0, 3599, 3600, 35964, 215_784] {
            let tc = Timecode::from_frames(n, rate, true);
            assert_eq!(tc.to_frames(rate), n, "frame {n} -> {tc}");
        }
    }

    #[test]
    fn drop_frame_ignored_for_integer_rates() {
        let tc = Timecode::from_frames(1800, FrameRate::FPS_30, true);
        assert!(!tc.drop_frame);
        assert_eq!(tc.to_string(), "00:01:00:00");
    }
}
