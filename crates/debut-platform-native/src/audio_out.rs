//! Real-time audio output through cpal (CoreAudio, WASAPI, ALSA). The device
//! callback calls the engine's [`AudioCallback`] and nothing else: no allocation,
//! no locks on this thread. [`SilentAudioOut`] drives the same callback from a
//! timer when there is no device (headless, CI, render nodes), so the clock still
//! runs.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use debut_core::{Error, Result};
use debut_platform::audio_out::{AudioCallback, AudioOut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct CpalAudioOut {
    stream: Option<cpal::Stream>,
    position: Arc<AtomicU64>,
    sample_rate: u32,
    channels: u16,
}

impl CpalAudioOut {
    /// Bind to the default output device. Fails if the machine has none.
    pub fn default_device() -> Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| Error::Unsupported("no audio output device".into()))?;
        let config = device
            .default_output_config()
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self {
            stream: None,
            position: Arc::new(AtomicU64::new(0)),
            sample_rate: config.sample_rate().0,
            channels: config.channels(),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

impl AudioOut for CpalAudioOut {
    fn start(&mut self, mut callback: Box<dyn AudioCallback>) -> Result<()> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| Error::Unsupported("no audio output device".into()))?;
        let config = cpal::StreamConfig {
            channels: self.channels,
            sample_rate: cpal::SampleRate(self.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };
        let position = Arc::clone(&self.position);
        let (channels, rate) = (self.channels, self.sample_rate);
        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    callback.fill(data, channels, rate);
                    position.fetch_add((data.len() / channels as usize) as u64, Ordering::AcqRel);
                },
                |e| eprintln!("audio stream error: {e}"),
                None,
            )
            .map_err(|e| Error::Other(e.to_string()))?;
        stream.play().map_err(|e| Error::Other(e.to_string()))?;
        self.stream = Some(stream);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.stream = None;
        Ok(())
    }

    fn position_samples(&self) -> u64 {
        self.position.load(Ordering::Acquire)
    }
}

/// A timer-driven stand-in for a sound card: calls the callback for 10 ms of
/// stereo 48 kHz every 10 ms of wall time and discards the samples.
pub struct SilentAudioOut {
    running: Arc<AtomicBool>,
    position: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SilentAudioOut {
    pub const SAMPLE_RATE: u32 = 48_000;
    pub const CHANNELS: u16 = 2;
    const BLOCK: usize = 480;

    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            position: Arc::new(AtomicU64::new(0)),
            thread: None,
        }
    }
}

impl Default for SilentAudioOut {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioOut for SilentAudioOut {
    fn start(&mut self, mut callback: Box<dyn AudioCallback>) -> Result<()> {
        self.stop()?;
        self.running.store(true, Ordering::Release);
        let running = Arc::clone(&self.running);
        let position = Arc::clone(&self.position);
        let handle = std::thread::Builder::new()
            .name("debut-silent-audio".into())
            .spawn(move || {
                let mut buf = vec![0.0f32; Self::BLOCK * Self::CHANNELS as usize];
                let period = Duration::from_micros(
                    Self::BLOCK as u64 * 1_000_000 / Self::SAMPLE_RATE as u64,
                );
                let mut next = Instant::now();
                while running.load(Ordering::Acquire) {
                    callback.fill(&mut buf, Self::CHANNELS, Self::SAMPLE_RATE);
                    position.fetch_add(Self::BLOCK as u64, Ordering::AcqRel);
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        next = now;
                    }
                }
            })
            .map_err(|e| Error::Other(e.to_string()))?;
        self.thread = Some(handle);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        Ok(())
    }

    fn position_samples(&self) -> u64 {
        self.position.load(Ordering::Acquire)
    }
}

impl Drop for SilentAudioOut {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_output_runs_the_callback_in_real_time() {
        struct Count(Arc<AtomicU64>);
        impl AudioCallback for Count {
            fn fill(&mut self, buffer: &mut [f32], channels: u16, _: u32) {
                self.0
                    .fetch_add((buffer.len() / channels as usize) as u64, Ordering::Relaxed);
            }
        }
        let frames = Arc::new(AtomicU64::new(0));
        let mut out = SilentAudioOut::new();
        out.start(Box::new(Count(Arc::clone(&frames)))).unwrap();
        std::thread::sleep(Duration::from_millis(120));
        out.stop().unwrap();
        let n = frames.load(Ordering::Relaxed);
        assert!(
            (3_000..=10_000).contains(&n),
            "{n} frames in ~120 ms (expected ~5 760)"
        );
        assert_eq!(out.position_samples(), n);
    }
}
