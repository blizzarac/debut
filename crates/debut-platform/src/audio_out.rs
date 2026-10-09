//! Real-time audio output: the master clock (PB-02, AUD).
//! Desktop: CoreAudio / WASAPI on a dedicated thread. Browser: AudioWorklet.

use debut_core::Result;

/// Called from the real-time thread. Must not allocate, lock or block.
pub trait AudioCallback: Send + 'static {
    fn fill(&mut self, buffer: &mut [f32], channels: u16, sample_rate: u32);
}

pub trait AudioOut: Send {
    fn start(&mut self, callback: Box<dyn AudioCallback>) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    /// Samples played so far; the timeline clock derives from this.
    fn position_samples(&self) -> u64;
}
