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
pub use codec::{Decoder, Encoder};
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

/// Bundle of implementations the engine is constructed with.
pub trait Platform: Send + Sync + 'static {
    type Decoder: Decoder;
    type Encoder: Encoder;
    type FileStore: FileStore;
    type AudioOut: AudioOut;
    type Threads: Threads;
    type PluginHost: PluginHost;
    type Display: Display;

    fn capabilities(&self) -> Capabilities;
}
