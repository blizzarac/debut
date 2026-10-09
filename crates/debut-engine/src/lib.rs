//! The single engine entry point (PLT-01). The desktop (Tauri) and web (WASM) shells
//! talk only to [`Engine`]; the TypeScript UI never reaches past it.

use debut_command::History;
use debut_platform::Platform;
use debut_project::Project;

pub mod api; // the stable surface exposed over Tauri IPC / wasm-bindgen
pub mod playback; // PB-01 .. PB-11 transport, A/V sync against the audio clock
pub mod session; // open/save/autosave (MED-09), crash recovery (NFR-05), workspaces (NFR-18)

pub struct Engine<P: Platform> {
    pub platform: P,
    pub project: Option<Project>,
    pub history: History,
}

impl<P: Platform> Engine<P> {
    pub fn new(platform: P) -> Self {
        Self {
            platform,
            project: None,
            history: History::default(),
        }
    }
}
