//! Time is exact. A [`Rational`] of seconds represents both frame-aligned video time
//! (TL-01) and sample-aligned audio time (AUD-01) without drift (MED-03).

use serde::{Deserialize, Serialize};

/// Exact rational number `num / den`, `den > 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const ZERO: Rational = Rational { num: 0, den: 1 };

    pub fn new(num: i64, den: i64) -> Self {
        assert!(den > 0, "denominator must be positive");
        Self { num, den }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

/// Timeline or clip frame rate, e.g. 24000/1001 for 23.976.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate(pub Rational);

impl FrameRate {
    pub const FPS_24: FrameRate = FrameRate(Rational { num: 24, den: 1 });
    pub const FPS_23_976: FrameRate = FrameRate(Rational { num: 24000, den: 1001 });
    pub const FPS_25: FrameRate = FrameRate(Rational { num: 25, den: 1 });
    pub const FPS_29_97: FrameRate = FrameRate(Rational { num: 30000, den: 1001 });
    pub const FPS_30: FrameRate = FrameRate(Rational { num: 30, den: 1 });
    pub const FPS_60: FrameRate = FrameRate(Rational { num: 60, den: 1 });
}

/// SMPTE timecode (MED-04). Drop-frame is a display concern only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timecode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u16,
    pub drop_frame: bool,
}
