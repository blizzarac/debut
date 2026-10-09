//! Real-time audio output through cpal (CoreAudio, WASAPI, ALSA). The device
//! callback calls the engine's [`AudioCallback`] and nothing else: no allocation,
//! no locks on this thread.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use debut_core::{Error, Result};
use debut_platform::audio_out::{AudioCallback, AudioOut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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
