//! Desktop implementations of the `debut-platform` traits (NFR-08, NFR-09).

pub mod audio_out; // cpal: CoreAudio / WASAPI / ALSA real-time callback
pub mod codec; // FFmpeg; NVENC / VideoToolbox / Quick Sync / AMF encoders
pub mod display; // native window, SDI/HDMI output (PB-09) — pending
pub mod file_store; // native FS
pub mod fonts; // system font discovery
pub mod net; // TCP line connections (collaboration)
pub mod plugin_host; // OpenFX, VST3, AU, out-of-process (NFR-07) — pending
pub mod threads; // native pool

use debut_platform::{
    AudioOut, Capabilities, Decoder, EncodeSettings, Encoder, FileStore, HwEncoder, Platform,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The desktop platform: FFmpeg codecs, cpal audio, the native file system,
/// OS threads and system fonts.
pub struct NativePlatform {
    store: Arc<file_store::NativeFileStore>,
    origin: Instant,
    /// Font bytes by requested family (`None`: nothing found).
    fonts: Mutex<HashMap<String, Option<Arc<[u8]>>>>,
}

impl NativePlatform {
    /// Paths are absolute (the store is rooted at `/`).
    pub fn new() -> Self {
        Self {
            store: Arc::new(file_store::NativeFileStore::new("/")),
            origin: Instant::now(),
            fonts: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for NativePlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for NativePlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            hardware_decode: false,
            hardware_encode: false,
            camera_raw: false,
            native_plugins: false,
            reference_monitor: false,
            max_resolution: (8192, 8192),
        }
    }

    fn file_store(&self) -> Arc<dyn FileStore> {
        self.store.clone()
    }

    fn open_decoder(&self, path: &str) -> debut_core::Result<Box<dyn Decoder>> {
        Ok(Box::new(codec::FfmpegDecoder::open(path)?))
    }

    fn create_encoder(
        &self,
        path: &str,
        settings: EncodeSettings,
    ) -> debut_core::Result<Box<dyn Encoder>> {
        Ok(Box::new(codec::FfmpegEncoder::create(path, settings)?))
    }

    fn open_audio_out(&self) -> Box<dyn AudioOut> {
        match audio_out::CpalAudioOut::default_device() {
            Ok(dev) => Box::new(dev),
            Err(_) => Box::new(audio_out::SilentAudioOut::new()),
        }
    }

    fn hardware_encoders(&self) -> Vec<HwEncoder> {
        codec::hardware_encoders()
    }

    fn hardware_decoders(&self) -> Vec<String> {
        codec::hardware_decoders()
    }

    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> debut_core::Result<()> {
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(job)
            .map(|_| ())
            .map_err(|e| debut_core::Error::Other(format!("spawn {name}: {e}")))
    }

    fn connect(&self, addr: &str) -> debut_core::Result<Box<dyn debut_platform::Connection>> {
        Ok(Box::new(net::TcpConnection::connect(addr)?))
    }

    fn font(&self, family: &str) -> Option<Arc<[u8]>> {
        let mut cache = self.fonts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = cache.get(family) {
            return hit.clone();
        }
        let bytes = fonts::find_font(family)
            .and_then(|p| std::fs::read(p).ok())
            .map(Arc::from);
        cache.insert(family.to_string(), bytes.clone());
        bytes
    }
}
