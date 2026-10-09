//! Desktop shell (Tauri). Exposes the engine to `apps/ui` over IPC. The project
//! commands mirror `debut-web`, so the UI code is identical on both targets
//! (PLT-01); the media and playback commands are desktop-only until the web
//! platform layer lands, and the UI treats their absence as a capability flag
//! (PLT-05).

mod ipc;

pub use ipc::Session;

use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(Session::new()))
        .invoke_handler(tauri::generate_handler![
            ipc::version,
            ipc::new_project,
            ipc::open_project,
            ipc::project_json,
            ipc::execute,
            ipc::undo,
            ipc::redo,
            ipc::can_undo,
            ipc::import_media,
            ipc::ensure_sequence,
            ipc::sequence,
            ipc::add_clip,
            ipc::edit,
            ipc::clip_effects,
            ipc::add_effect,
            ipc::remove_effect,
            ipc::set_param,
            ipc::transport,
            ipc::tick,
            ipc::frame_pixels,
        ])
        .run(tauri::generate_context!())
        .expect("error while running debut");
}
