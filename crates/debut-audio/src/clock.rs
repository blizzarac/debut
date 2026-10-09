//! Audio is the master clock (PB-02). The real-time output callback counts the
//! samples it has played; the timeline position is derived from that count, never
//! from a wall clock, so video presentation can't drift from audio.
//!
//! The playhead lives in two atomics so the real-time thread and the UI thread
//! share it without locks.

use debut_core::Rational;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

pub const SAMPLE_RATE: u32 = 48_000;

/// Shared, lock-free transport state.
#[derive(Debug)]
pub struct Clock {
    sample_rate: u32,
    /// Timeline position (in samples) at which playback was last started.
    anchor_samples: AtomicI64,
    /// Samples the output has rendered since `anchor_samples` was set.
    played: AtomicU64,
    playing: AtomicBool,
    /// Signed playback rate in 1/256 units: 256 = 1x forward, -512 = 2x reverse.
    rate_q8: AtomicI64,
}

impl Clock {
    pub fn new(sample_rate: u32) -> Arc<Self> {
        Arc::new(Self {
            sample_rate,
            anchor_samples: AtomicI64::new(0),
            played: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            rate_q8: AtomicI64::new(256),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    /// Playback rate as a rational (1 = normal, -1 = reverse, 2 = double).
    pub fn rate(&self) -> Rational {
        Rational::new(self.rate_q8.load(Ordering::Acquire), 256)
    }

    /// Current timeline position in samples.
    pub fn position_samples(&self) -> i64 {
        let anchor = self.anchor_samples.load(Ordering::Acquire);
        if !self.is_playing() {
            return anchor;
        }
        let played = self.played.load(Ordering::Acquire) as i64;
        let rate = self.rate_q8.load(Ordering::Acquire);
        anchor + (played * rate) / 256
    }

    /// Current timeline position in seconds.
    pub fn position(&self) -> Rational {
        Rational::new(self.position_samples(), self.sample_rate as i64)
    }

    /// Jump to `t`. Allowed while playing: the anchor moves and the played count
    /// restarts, so audio and video re-sync from the new point.
    pub fn seek(&self, t: Rational) {
        let samples = (t * Rational::from_int(self.sample_rate as i64)).round();
        self.played.store(0, Ordering::Release);
        self.anchor_samples.store(samples, Ordering::Release);
    }

    pub fn play(&self) {
        if !self.is_playing() {
            self.played.store(0, Ordering::Release);
            self.playing.store(true, Ordering::Release);
        }
    }

    /// Stop and freeze the position where the audio actually got to.
    pub fn pause(&self) {
        let pos = self.position_samples();
        self.playing.store(false, Ordering::Release);
        self.anchor_samples.store(pos, Ordering::Release);
        self.played.store(0, Ordering::Release);
    }

    /// Change speed without losing position (JKL shuttle, PB-04).
    pub fn set_rate(&self, rate: Rational) {
        let pos = self.position_samples();
        let q8 = (rate * Rational::from_int(256)).round();
        self.anchor_samples.store(pos, Ordering::Release);
        self.played.store(0, Ordering::Release);
        self.rate_q8.store(q8, Ordering::Release);
    }

    /// Called by the real-time callback after rendering `n` output frames.
    /// Wait-free; never called from the UI thread.
    pub fn advance(&self, n: u64) {
        if self.is_playing() {
            self.played.fetch_add(n, Ordering::AcqRel);
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, clippy::disallowed_types)] // tests may use the OS directly
mod tests {
    use super::*;

    #[test]
    fn position_follows_played_samples_only_while_playing() {
        let c = Clock::new(48_000);
        c.seek(Rational::from_int(2));
        c.advance(48_000); // ignored: not playing
        assert_eq!(c.position(), Rational::from_int(2));
        c.play();
        c.advance(24_000);
        assert_eq!(c.position(), Rational::new(5, 2));
        c.pause();
        c.advance(48_000);
        assert_eq!(c.position(), Rational::new(5, 2));
    }

    #[test]
    fn seek_while_playing_resyncs() {
        let c = Clock::new(48_000);
        c.play();
        c.advance(10_000);
        c.seek(Rational::from_int(10));
        c.advance(48_000);
        assert_eq!(c.position(), Rational::from_int(11));
    }

    #[test]
    fn rate_changes_keep_position_and_scale_advance() {
        let c = Clock::new(48_000);
        c.seek(Rational::from_int(5));
        c.play();
        c.advance(48_000);
        c.set_rate(Rational::from_int(-2));
        assert_eq!(c.position(), Rational::from_int(6));
        c.advance(24_000);
        assert_eq!(c.position(), Rational::from_int(5));
        c.set_rate(Rational::new(1, 2));
        c.advance(48_000);
        assert_eq!(c.position(), Rational::new(11, 2));
    }

    #[test]
    fn shared_between_threads() {
        let c = Clock::new(48_000);
        c.play();
        let rt = {
            let c = Arc::clone(&c);
            std::thread::spawn(move || {
                for _ in 0..1000 {
                    c.advance(48);
                }
            })
        };
        rt.join().unwrap();
        assert_eq!(c.position(), Rational::from_int(1));
    }
}
