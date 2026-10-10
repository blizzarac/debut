//! The narrow platform layer (PLT-02). Every OS or browser call the engine makes goes
//! through one of these six traits. Implementations:
//! `debut-platform-native` (desktop) and `debut-platform-web` (WASM).
//!
//! No feature logic lives here or in the implementations (PLT-01). Where a target
//! can't provide a capability, it reports so via [`Capabilities`] and the UI shows
//! the difference instead of failing at runtime (PLT-05).

pub mod ai;
pub mod audio_out;
pub mod codec;
pub mod display;
pub mod file_store;
pub mod net;
pub mod plugin_host;
pub mod threads;

pub use ai::{Transcriber, TranscriptSegment};
pub use audio_out::AudioOut;
pub use codec::{
    AudioBlock, AudioEncodeSettings, AudioInfo, DecodePath, Decoder, EncodeSettings, Encoder,
    HdrSettings, HdrTransfer, HwEncoder, SourceTags, StreamCopyInfo, VideoFrame, VideoInfo,
};
pub use display::Display;
pub use file_store::FileStore;
pub use net::Connection;
pub use plugin_host::{PluginHost, PluginInfo, PluginKind, PluginRef};
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

    /// A folder for per-user settings (plugin approvals, telemetry
    /// consent), through `file_store`. `None` keeps them in memory only.
    fn settings_dir(&self) -> Option<String> {
        None
    }

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

    /// What a file's video stream allows for packet copying (EXP-05).
    fn stream_copy_info(&self, path: &str) -> debut_core::Result<StreamCopyInfo> {
        Err(debut_core::Error::Unsupported(format!(
            "packet copying is not available here ({path})"
        )))
    }

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

    /// Decode video on the GPU where a device takes the stream (NFR-09);
    /// applies to decoders opened afterwards. Off by default.
    fn set_hardware_decode(&self, _on: bool) {}

    fn hardware_decode(&self) -> bool {
        false
    }

    /// Monotonic time since an arbitrary origin, for autosave intervals and
    /// frame timing. (The browser has no `std::time::Instant`.)
    fn now(&self) -> std::time::Duration;

    /// Run a long job (an export) on its own background thread: an OS thread
    /// natively, a Web Worker in the browser.
    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> debut_core::Result<()>;

    /// Bytes of a TrueType/OpenType font for `family` (or a font file path), or
    /// of a fallback face; `None` when the platform has no usable font (GFX-01).
    fn font(&self, family: &str) -> Option<std::sync::Arc<[u8]>>;

    /// The local speech-to-text backend (GFX-04), when one is set up.
    fn transcriber(&self) -> Option<std::sync::Arc<dyn Transcriber>> {
        None
    }

    /// Point transcription at a local engine and model (desktop: the
    /// whisper.cpp command-line tool and a ggml model file).
    fn configure_transcriber(&self, engine: &str, model: &str) -> debut_core::Result<()> {
        let _ = (engine, model);
        Err(debut_core::Error::Unsupported(
            "local transcription is not available here".into(),
        ))
    }

    /// The out-of-process plugin host (FX-15, AUD-09), where the target has one.
    fn plugins(&self) -> Option<std::sync::Arc<dyn PluginHost>> {
        None
    }

    /// Open a line connection to `addr` ("host:port"), for collaboration.
    fn connect(&self, addr: &str) -> debut_core::Result<Box<dyn Connection>> {
        Err(debut_core::Error::Unsupported(format!(
            "network connections are not available here ({addr})"
        )))
    }
}
