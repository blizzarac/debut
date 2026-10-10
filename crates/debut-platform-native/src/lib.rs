//! Desktop implementations of the `debut-platform` traits (NFR-08, NFR-09).

pub mod ai; // GFX-04 local transcription via whisper.cpp
pub mod audio_out; // cpal: CoreAudio / WASAPI / ALSA real-time callback
pub mod codec; // FFmpeg; NVENC / VideoToolbox / Quick Sync / AMF encoders
pub mod display; // native window, SDI/HDMI output (PB-09) — pending
pub mod file_store; // native FS
pub mod fonts; // system font discovery
pub mod hwdecode; // NFR-09 hardware video decode (VAAPI, NVDEC, Vulkan, VideoToolbox, D3D11VA)
pub mod net; // TCP line connections (collaboration)
pub mod plugin_host; // OpenFX and CLAP, out of process (NFR-07)
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
    /// The plugin-host helper, when one ships next to the app (FX-15).
    plugins: Option<Arc<plugin_host::NativePluginHost>>,
    /// Decode video on a hardware device (NFR-09).
    hw_decode: std::sync::atomic::AtomicBool,
    /// Local speech-to-text (GFX-04), from the environment or the settings.
    transcriber: Mutex<Option<Arc<ai::WhisperCli>>>,
    /// Per-user settings folder (plugin approvals, telemetry consent).
    settings_dir: Option<String>,
}

/// `DEBUT_SETTINGS_DIR`, else the OS's per-user config folder + `debut`.
fn default_settings_dir() -> Option<String> {
    use std::path::PathBuf;
    if let Some(d) = std::env::var_os("DEBUT_SETTINGS_DIR") {
        return Some(PathBuf::from(d).to_string_lossy().into_owned());
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".config")))
    }?;
    Some(base.join("debut").to_string_lossy().into_owned())
}

impl NativePlatform {
    /// Paths are absolute (the store is rooted at `/`).
    pub fn new() -> Self {
        Self {
            store: Arc::new(file_store::NativeFileStore::new("/")),
            origin: Instant::now(),
            fonts: Mutex::new(HashMap::new()),
            plugins: plugin_host::NativePluginHost::from_environment().map(Arc::new),
            hw_decode: std::sync::atomic::AtomicBool::new(false),
            transcriber: Mutex::new(ai::WhisperCli::from_environment().map(Arc::new)),
            settings_dir: default_settings_dir(),
        }
    }

    /// Keep per-user settings in `dir` (tests use a scratch folder).
    pub fn with_settings_dir(mut self, dir: impl Into<String>) -> Self {
        self.settings_dir = Some(dir.into());
        self
    }

    /// With a given plugin host (tests point it at their own helper and plugins).
    pub fn with_plugins(host: plugin_host::NativePluginHost) -> Self {
        Self {
            plugins: Some(Arc::new(host)),
            ..Self::new()
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
            hardware_decode: hwdecode::available(),
            hardware_encode: false,
            camera_raw: false,
            native_plugins: self.plugins.is_some(),
            reference_monitor: false,
            max_resolution: (8192, 8192),
        }
    }

    fn file_store(&self) -> Arc<dyn FileStore> {
        self.store.clone()
    }

    fn settings_dir(&self) -> Option<String> {
        self.settings_dir.clone()
    }

    fn open_decoder(&self, path: &str) -> debut_core::Result<Box<dyn Decoder>> {
        let hw = self.hw_decode.load(std::sync::atomic::Ordering::Relaxed);
        Ok(Box::new(codec::FfmpegDecoder::open_with(path, hw)?))
    }

    fn create_encoder(
        &self,
        path: &str,
        settings: EncodeSettings,
    ) -> debut_core::Result<Box<dyn Encoder>> {
        Ok(Box::new(codec::FfmpegEncoder::create(path, settings)?))
    }

    fn stream_copy_info(&self, path: &str) -> debut_core::Result<debut_platform::StreamCopyInfo> {
        codec::stream_copy_info(path)
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

    fn transcriber(&self) -> Option<Arc<dyn debut_platform::Transcriber>> {
        let t = self.transcriber.lock().unwrap_or_else(|e| e.into_inner());
        t.clone().map(|t| t as Arc<dyn debut_platform::Transcriber>)
    }

    fn configure_transcriber(&self, engine: &str, model: &str) -> debut_core::Result<()> {
        let t = ai::WhisperCli::new(engine, model)?;
        *self.transcriber.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(t));
        Ok(())
    }

    fn set_hardware_decode(&self, on: bool) {
        self.hw_decode
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    fn hardware_decode(&self) -> bool {
        self.hw_decode.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn plugins(&self) -> Option<Arc<dyn debut_platform::PluginHost>> {
        self.plugins
            .clone()
            .map(|p| p as Arc<dyn debut_platform::PluginHost>)
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
