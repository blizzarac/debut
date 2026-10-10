//! Third-party plugins, run out of process so a crash never takes the app down
//! (FX-15, AUD-09, NFR-07, NFR-12). Desktop: OpenFX image filters and CLAP
//! audio effects. VST3 and AU are not hosted (VST3's C++ ABI and AU's macOS
//! frameworks need hosts of their own); the browser has none.
//!
//! The types here are also the wire format between the app and the
//! `debut-plugin-host` helper: one JSON line per request or reply, followed by
//! the raw sample bytes it announces.

use debut_core::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    /// An OpenFX image effect (video).
    OpenFx,
    /// A CLAP audio effect.
    Clap,
}

/// One plugin inside a binary: the binary's path and its index there.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginRef {
    pub kind: PluginKind,
    pub path: String,
    pub index: u32,
}

/// A parameter the plugin reports; multi-component parameters (colours,
/// points) appear once per component as `name[i]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginParamInfo {
    pub name: String,
    pub label: String,
    /// "double", "int", "bool" or "choice".
    pub kind: String,
    pub default: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub plugin: PluginRef,
    /// The plugin's own identifier ("com.vendor.Blur").
    pub id: String,
    pub name: String,
    pub params: Vec<PluginParamInfo>,
}

/// A binary that could not be used, and why (it failed to load, isn't a
/// filter, or crashed the helper while being scanned).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginProblem {
    pub path: String,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanResult {
    pub plugins: Vec<PluginInfo>,
    pub problems: Vec<PluginProblem>,
}

/// Run an OpenFX filter over one frame: premultiplied float RGBA, top-down.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoJob {
    pub plugin: PluginRef,
    pub width: u32,
    pub height: u32,
    /// Clip-local time in frames, and the frame rate.
    pub frame: f64,
    pub fps: f64,
    pub params: Vec<(String, f64)>,
}

/// Run a CLAP effect over interleaved float audio. `instance` names the
/// running instance (one per track insert), which keeps its state (a reverb's
/// tail) between blocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioJob {
    pub plugin: PluginRef,
    pub instance: u64,
    pub sample_rate: u32,
    pub channels: u32,
    pub params: Vec<(String, f64)>,
}

/// A request to the helper; `Video` and `Audio` are followed by `bytes` of
/// native-endian f32 samples.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Plugin binaries under these directories (no plugin is loaded).
    List {
        dirs: Vec<String>,
    },
    /// Load one binary and describe every plugin in it.
    Describe {
        kind: PluginKind,
        path: String,
    },
    Video {
        job: VideoJob,
        bytes: usize,
    },
    Audio {
        job: AudioJob,
        bytes: usize,
    },
    /// Stop an audio instance.
    Release {
        instance: u64,
    },
}

/// The helper's answer; a successful `Video` / `Audio` reply is followed by
/// the processed samples, `bytes` long.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Listed { files: Vec<(PluginKind, String)> },
    Described { plugins: Vec<PluginInfo> },
    Processed { bytes: usize },
    Done,
    Failed { error: String },
}

pub trait PluginHost: Send + Sync {
    /// Every usable plugin on the search path, and the binaries that weren't.
    fn scan(&self) -> Result<ScanResult>;
    /// Run an OpenFX filter over `rgba` (width x height x 4) in place.
    fn process_video(&self, job: &VideoJob, rgba: &mut [f32]) -> Result<()>;
    /// Run a CLAP effect over interleaved `samples` in place.
    fn process_audio(&self, job: &AudioJob, samples: &mut [f32]) -> Result<()>;
    /// Drop an audio instance (its insert was removed).
    fn release(&self, _instance: u64) {}
}
