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
            ipc::save_project,
            ipc::open_project_file,
            ipc::file_status,
            ipc::execute,
            ipc::undo,
            ipc::redo,
            ipc::can_undo,
            ipc::import_media,
            ipc::ensure_sequence,
            ipc::sequence,
            ipc::sequences,
            ipc::open_sequence,
            ipc::add_clip,
            ipc::add_title,
            ipc::add_multicam,
            ipc::switch_angle,
            ipc::set_title,
            ipc::edit,
            ipc::clip_effects,
            ipc::add_effect,
            ipc::remove_effect,
            ipc::set_effect_options,
            ipc::set_param,
            ipc::set_track_mix,
            ipc::add_insert,
            ipc::remove_insert,
            ipc::export_presets,
            ipc::export_start,
            ipc::export_status,
            ipc::export_pause,
            ipc::export_resume,
            ipc::export_cancel,
            ipc::transport,
            ipc::tick,
            ipc::frame_pixels,
            ipc::scopes,
            ipc::set_preview_quality,
            ipc::markers,
            ipc::add_marker,
            ipc::update_marker,
            ipc::remove_marker,
            ipc::export_markers,
            ipc::captions,
            ipc::add_caption,
            ipc::update_caption,
            ipc::remove_caption,
            ipc::import_srt,
            ipc::export_srt,
        ])
        .run(tauri::generate_context!())
        .expect("error while running debut");
}
