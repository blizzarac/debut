//! Desktop shell (Tauri). Exposes the engine to `apps/ui` over IPC. The project
//! commands mirror `debut-web`, so the UI code is identical on both targets
//! (PLT-01); the media and playback commands are desktop-only until the web
//! platform layer lands, and the UI treats their absence as a capability flag
//! (PLT-05).

mod ipc;

pub use debut_engine::Session;

use debut_platform_native::NativePlatform;
use std::sync::{Arc, Mutex};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let platform: Arc<NativePlatform> = Arc::new(NativePlatform::new());
    // Crash reports (NFR-15): recorded only if the user opted in.
    let hook_platform = Arc::clone(&platform);
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        debut_engine::telemetry::record_crash(hook_platform.as_ref(), &message, &location);
        default_hook(info);
    }));
    tauri::Builder::default()
        .manage(Mutex::new(Session::new(platform)))
        .invoke_handler(tauri::generate_handler![
            ipc::version,
            ipc::about,
            ipc::telemetry_status,
            ipc::set_telemetry,
            ipc::set_telemetry_endpoint,
            ipc::send_telemetry,
            ipc::new_project,
            ipc::open_project,
            ipc::project_json,
            ipc::save_project,
            ipc::open_project_file,
            ipc::import_fcpxml,
            ipc::collab_host,
            ipc::collab_join,
            ipc::collab_leave,
            ipc::collab_poll,
            ipc::collab_lock,
            ipc::file_status,
            ipc::execute,
            ipc::undo,
            ipc::redo,
            ipc::can_undo,
            ipc::import_media,
            ipc::media_list,
            ipc::waveform,
            ipc::snap_points,
            ipc::linked_selection,
            ipc::targeting,
            ipc::set_source_patch,
            ipc::set_track_targeted,
            ipc::insert_media,
            ipc::blade_targeted,
            ipc::shortcuts,
            ipc::resolve_keymap,
            ipc::snapshots,
            ipc::take_snapshot,
            ipc::restore_snapshot,
            ipc::remove_snapshot,
            ipc::compare_snapshot,
            ipc::set_linked_selection,
            ipc::relink_media,
            ipc::relink_folder,
            ipc::ingest_folder,
            ipc::create_proxies,
            ipc::proxy_status,
            ipc::use_proxies,
            ipc::set_use_proxies,
            ipc::bins,
            ipc::add_bin,
            ipc::add_smart_bin,
            ipc::set_media_tags,
            ipc::rename_bin,
            ipc::remove_bin,
            ipc::assign_media,
            ipc::ensure_sequence,
            ipc::sequence,
            ipc::sequences,
            ipc::open_sequence,
            ipc::add_clip,
            ipc::add_title,
            ipc::title_templates,
            ipc::save_title_template,
            ipc::remove_title_template,
            ipc::add_multicam,
            ipc::switch_angle,
            ipc::set_title,
            ipc::add_shape,
            ipc::run_script,
            ipc::script_api,
            ipc::set_shape,
            ipc::edit,
            ipc::clip_effects,
            ipc::add_effect,
            ipc::remove_effect,
            ipc::set_effect_options,
            ipc::track_mask,
            ipc::track_planar,
            ipc::clear_planar,
            ipc::set_param,
            ipc::set_track_mix,
            ipc::add_insert,
            ipc::remove_insert,
            ipc::scan_plugins,
            ipc::set_hardware_decode,
            ipc::ai_status,
            ipc::set_ai_enabled,
            ipc::configure_transcriber,
            ipc::transcribe_clip,
            ipc::search_captions,
            ipc::monitors,
            ipc::open_program_window,
            ipc::close_program_window,
            ipc::program_fullscreen,
            ipc::program_window,
            ipc::hardware_decode_status,
            ipc::plugins,
            ipc::add_plugin_effect,
            ipc::set_plugin_param,
            ipc::add_plugin_insert,
            ipc::set_insert_param,
            ipc::plugin_error,
            ipc::approve_plugin,
            ipc::revoke_plugin,
            ipc::set_track_duck,
            ipc::export_presets,
            ipc::export_interchange,
            ipc::codec_capabilities,
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
            ipc::caption_settings,
            ipc::set_caption_settings,
            ipc::export_srt,
        ])
        .run(tauri::generate_context!())
        .expect("error while running debut");
}
