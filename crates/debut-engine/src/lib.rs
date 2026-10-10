//! The single engine entry point (PLT-01). The desktop (Tauri) and web (WASM)
//! shells talk only to [`api::Session`]; the TypeScript UI never reaches past it.

pub mod api; // the command surface exposed over Tauri IPC / wasm-bindgen
pub mod frames; // decoder -> render graph bridge
pub mod playback; // PB-01 .. PB-11 transport, A/V sync against the audio clock
pub mod player; // one sequence playing: transport + audio + frames
pub mod samples; // decoder -> audio engine bridge
pub mod session;
pub mod settings; // per-user settings: plugin approvals (NFR-13), telemetry consent (NFR-15)
pub mod telemetry; // NFR-15 opt-in, anonymous, local usage counts and crash reports
pub mod trust; // NFR-13 plugin binaries run only once approved, pinned by SHA-256 // open/save/autosave (MED-09), crash recovery (NFR-05), workspaces (NFR-18)

pub use api::Session;
pub use frames::FrameSource;
pub use playback::{Stats, Transport};
pub use player::Player;
pub use samples::SampleCache;
pub use session::{FileJournal, Opened, Workspace};
