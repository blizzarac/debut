//! Desktop implementations of the `debut-platform` traits (NFR-08, NFR-09).

pub mod audio_out; // cpal: CoreAudio / WASAPI / ALSA real-time callback
pub mod codec; // FFmpeg; VideoToolbox / NVDEC / QSV / AMF
pub mod display; // native window, SDI/HDMI output (PB-09) — pending
pub mod file_store; // native FS
pub mod plugin_host; // OpenFX, VST3, AU, out-of-process (NFR-07) — pending
pub mod threads; // native pool

use debut_platform::{Capabilities, Platform};

pub struct NativePlatform;

impl Platform for NativePlatform {
    type Decoder = codec::FfmpegDecoder;
    type Encoder = codec::UnimplementedEncoder;
    type FileStore = file_store::NativeFileStore;
    type AudioOut = audio_out::CpalAudioOut;
    type Threads = threads::NativeThreads;
    type PluginHost = plugin_host::NativePluginHost;
    type Display = display::NativeDisplay;

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

    fn open_decoder(&self, path: &str) -> debut_core::Result<Self::Decoder> {
        codec::FfmpegDecoder::open(path)
    }
}
