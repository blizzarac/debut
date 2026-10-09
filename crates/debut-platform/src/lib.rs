//! The narrow platform layer (PLT-02). Every OS or browser call the engine makes goes
//! through one of these six traits. Implementations:
//! `debut-platform-native` (desktop) and `debut-platform-web` (WASM).
//!
//! No feature logic lives here or in the implementations (PLT-01). Where a target
//! can't provide a capability, it reports so via [`Capabilities`] and the UI shows
//! the difference instead of failing at runtime (PLT-05).

pub mod audio_out;
pub mod codec;
pub mod display;
pub mod file_store;
pub mod plugin_host;
pub mod threads;

pub use audio_out::AudioOut;
pub use codec::{
    AudioBlock, AudioEncodeSettings, AudioInfo, Decoder, EncodeSettings, Encoder, HwEncoder,
    VideoFrame, VideoInfo,
};
pub use display::Display;
pub use file_store::FileStore;
pub use plugin_host::PluginHost;
pub use threads::Threads;

/// What this target can do. Drives feature flags in the UI (PLT-05).
#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub hardware_decode: bool,
    pub hardware_encode: bool,
    pub camera_raw: bool,
    pub native_plugins: bool,
    pub reference_monitor: bool,
    pub max_resolution: (u32, u32),
}

/// Bundle of implementations the engine is constructed with. Object-safe, so
/// the engine holds an `Arc<dyn Platform>` and shells pick the implementation.
pub trait Platform: Send + Sync + 'static {
    fn capabilities(&self) -> Capabilities;

    /// Where project files, sidecars and exports are read and written.
    fn file_store(&self) -> std::sync::Arc<dyn FileStore>;

    /// Open a media file for decoding.
    fn open_decoder(&self, path: &str) -> debut_core::Result<Box<dyn Decoder>>;

    /// Create an output file to encode into.
    fn create_encoder(
        &self,
        path: &str,
        settings: EncodeSettings,
    ) -> debut_core::Result<Box<dyn Encoder>>;

    /// The playback device; a silent stand-in when none is available, so the
    /// transport still runs (the audio clock is the master clock).
    fn open_audio_out(&self) -> Box<dyn AudioOut>;

    /// Hardware encoders that really open on this machine (NFR-09).
    fn hardware_encoders(&self) -> Vec<HwEncoder> {
        Vec::new()
    }

    /// Hardware decoders the codec layer carries.
    fn hardware_decoders(&self) -> Vec<String> {
        Vec::new()
    }
}
