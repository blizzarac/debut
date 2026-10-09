//! Desktop implementations of the `debut-platform` traits (NFR-08, NFR-09).
//! Planned deps: ffmpeg-next, cpal (or direct CoreAudio/WASAPI), memmap2, rayon.

pub mod codec;       // FFmpeg; VideoToolbox / NVDEC / QSV / AMF
pub mod file_store;  // native FS, memory-mapped reads
pub mod audio_out;   // CoreAudio / WASAPI real-time thread
pub mod threads;     // native pool
pub mod plugin_host; // OpenFX, VST3, AU, out-of-process (NFR-07)
pub mod display;     // native window, SDI/HDMI output (PB-09)
