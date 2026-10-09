//! Transport (PB-01 .. PB-11). Video is presented against the audio clock
//! (`debut_audio::clock`): on every UI tick the transport asks which frame the
//! clock says is current and presents that one; frames the renderer couldn't
//! finish in time are counted as dropped (PB-07). Audio is never dropped.

use debut_audio::clock::Clock;
use debut_core::{FrameRate, Rational};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub presented: u64,
    pub dropped: u64,
}

/// JKL shuttle rates (PB-04).
const SHUTTLE: [i64; 4] = [1, 2, 4, 8];

pub struct Transport {
    clock: Arc<Clock>,
    frame_rate: FrameRate,
    last_frame: Option<i64>,
    stats: Stats,
    loop_range: Option<(Rational, Rational)>,
    end: Rational,
}

impl Transport {
    pub fn new(clock: Arc<Clock>, frame_rate: FrameRate, end: Rational) -> Self {
        Self {
            clock,
            frame_rate,
            last_frame: None,
            stats: Stats::default(),
            loop_range: None,
            end,
        }
    }

    pub fn clock(&self) -> &Arc<Clock> {
        &self.clock
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn position(&self) -> Rational {
        self.clock.position()
    }

    pub fn is_playing(&self) -> bool {
        self.clock.is_playing()
    }

    pub fn frame_rate(&self) -> FrameRate {
        self.frame_rate
    }

    /// Sequence length; playback stops here (or wraps inside a loop range).
    pub fn set_end(&mut self, end: Rational) {
        self.end = end;
    }

    pub fn set_loop(&mut self, range: Option<(Rational, Rational)>) {
        self.loop_range = range;
    }

    // ---- controls -----------------------------------------------------------

    pub fn play(&mut self) {
        self.clock.set_rate(Rational::ONE);
        self.clock.play();
    }

    pub fn pause(&mut self) {
        self.clock.pause();
        self.snap_to_frame();
    }

    pub fn toggle(&mut self) {
        if self.is_playing() {
            self.pause()
        } else {
            self.play()
        }
    }

    pub fn seek(&mut self, t: Rational) {
        let t = t.max(Rational::ZERO).min(self.end);
        self.clock.seek(self.frame_rate.snap(t));
        self.last_frame = None;
    }

    /// Step by `n` frames (negative = back) from the current frame; pauses.
    pub fn step(&mut self, n: i64) {
        self.clock.pause();
        let frame = self.current_frame() + n;
        self.seek(self.frame_rate.frame_to_time(frame.max(0)));
    }

    /// `L`: forward, faster on repeat. `J`: the same backwards. `K`: pause.
    pub fn shuttle(&mut self, forward: bool) {
        let cur = self.clock.rate();
        let dir = if forward { 1 } else { -1 };
        let same_dir = (cur.num > 0) == forward && self.is_playing();
        let next = if same_dir {
            let mag = cur.num.abs() / cur.den.max(1);
            SHUTTLE
                .iter()
                .copied()
                .find(|&s| s > mag)
                .unwrap_or(SHUTTLE[SHUTTLE.len() - 1])
        } else {
            1
        };
        self.clock.set_rate(Rational::from_int(dir * next));
        self.clock.play();
    }

    // ---- per-tick -------------------------------------------------------------

    pub fn current_frame(&self) -> i64 {
        self.frame_rate.time_to_frame(self.clock.position())
    }

    /// Called once per display refresh. Returns the frame to present now, or
    /// `None` if the one on screen is still current. Handles end-of-sequence and
    /// looping; counts skipped frames as dropped while playing forward.
    pub fn tick(&mut self) -> Option<i64> {
        if self.is_playing() {
            let pos = self.clock.position();
            if let Some((start, end)) = self.loop_range {
                if pos >= end && self.clock.rate().num > 0 {
                    self.clock.seek(start);
                    self.last_frame = None;
                } else if pos < start && self.clock.rate().num < 0 {
                    self.clock.seek(end - self.frame_rate.frame_duration());
                    self.last_frame = None;
                }
            } else if pos >= self.end || (pos <= Rational::ZERO && self.clock.rate().num < 0) {
                self.clock.pause();
                self.seek(pos);
            }
        }
        let frame = self.current_frame();
        if self.last_frame == Some(frame) {
            return None;
        }
        if let Some(last) = self.last_frame {
            if self.is_playing() && self.clock.rate() == Rational::ONE && frame > last + 1 {
                self.stats.dropped += (frame - last - 1) as u64;
            }
        }
        self.stats.presented += 1;
        self.last_frame = Some(frame);
        Some(frame)
    }

    fn snap_to_frame(&mut self) {
        let t = self.frame_rate.snap(self.clock.position());
        self.clock.seek(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Transport {
        let clock = Clock::new(48_000);
        Transport::new(clock, FrameRate::FPS_25, Rational::from_int(10))
    }

    /// 48 000 / 25 = 1 920 samples per frame.
    const SPF: u64 = 1920;

    #[test]
    fn presents_each_frame_once_and_counts_drops() {
        let mut t = setup();
        t.play();
        assert_eq!(t.tick(), Some(0));
        assert_eq!(t.tick(), None);
        t.clock().advance(SPF);
        assert_eq!(t.tick(), Some(1));
        // The renderer stalled for three frames' worth of audio.
        t.clock().advance(SPF * 3);
        assert_eq!(t.tick(), Some(4));
        assert_eq!(
            t.stats(),
            Stats {
                presented: 3,
                dropped: 2
            }
        );
    }

    #[test]
    fn step_pauses_and_moves_by_whole_frames() {
        let mut t = setup();
        t.play();
        t.clock().advance(SPF * 2 + 100);
        t.step(1);
        assert!(!t.is_playing());
        assert_eq!(t.position(), FrameRate::FPS_25.frame_to_time(3));
        t.step(-5);
        assert_eq!(t.position(), Rational::ZERO);
    }

    #[test]
    fn shuttle_ramps_and_reverses() {
        let mut t = setup();
        t.seek(Rational::from_int(5));
        t.shuttle(true);
        assert_eq!(t.clock().rate(), Rational::from_int(1));
        t.shuttle(true);
        t.shuttle(true);
        assert_eq!(t.clock().rate(), Rational::from_int(4));
        t.shuttle(false);
        assert_eq!(t.clock().rate(), Rational::from_int(-1));
        t.clock().advance(SPF * 25);
        assert_eq!(t.position(), Rational::from_int(4));
    }

    #[test]
    fn stops_at_the_end_and_loops_in_a_range() {
        let mut t = setup();
        t.seek(Rational::from_int(9));
        t.play();
        t.clock().advance(SPF * 50); // 2 s past the end
        t.tick();
        assert!(!t.is_playing());
        assert_eq!(t.position(), Rational::from_int(10));

        t.set_loop(Some((Rational::from_int(2), Rational::from_int(4))));
        t.seek(Rational::from_int(3));
        t.play();
        t.clock().advance(SPF * 30); // 1.2 s -> wraps to 2
        assert_eq!(t.tick(), Some(50));
        assert!(t.is_playing());
    }
}
