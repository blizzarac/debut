//! Tauri commands: thin wrappers that lock the shared engine `Session` and
//! forward. Everything else lives in `debut_engine::api`, so the web shell can
//! offer the same surface.

use debut_engine::api::*;
use debut_project::{CaptionSettings, Param, Title, TrackMix};
use std::sync::Mutex;
use tauri::State;

pub type Shared = Mutex<Session>;

fn lock<'a>(state: &'a State<'a, Shared>) -> std::sync::MutexGuard<'a, Session> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

#[tauri::command]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
pub fn new_project(state: State<'_, Shared>, name: String) -> Result<(), String> {
    lock(&state).new_project(name)
}

#[tauri::command]
pub fn open_project(state: State<'_, Shared>, json: String) -> Result<(), String> {
    lock(&state).open_project_json(&json)
}

#[tauri::command]
pub fn project_json(state: State<'_, Shared>) -> Result<String, String> {
    lock(&state).project_json()
}

#[tauri::command]
pub fn save_project(state: State<'_, Shared>, path: Option<String>) -> Result<FileStatus, String> {
    lock(&state).save_project(path)
}

#[tauri::command]
pub fn open_project_file(state: State<'_, Shared>, path: String) -> Result<FileStatus, String> {
    lock(&state).open_project_file(path)
}

#[tauri::command]
pub fn file_status(state: State<'_, Shared>) -> FileStatus {
    lock(&state).file_status()
}

#[tauri::command]
pub fn execute(state: State<'_, Shared>, command_json: String) -> Result<(), String> {
    lock(&state).execute_json(&command_json)
}

#[tauri::command]
pub fn undo(state: State<'_, Shared>) -> Result<bool, String> {
    lock(&state).undo()
}

#[tauri::command]
pub fn redo(state: State<'_, Shared>) -> Result<bool, String> {
    lock(&state).redo()
}

#[tauri::command]
pub fn can_undo(state: State<'_, Shared>) -> bool {
    lock(&state).can_undo()
}

#[tauri::command]
pub fn clip_effects(
    state: State<'_, Shared>,
    track: String,
    clip: String,
) -> Result<Vec<EffectDto>, String> {
    lock(&state).clip_effects(&track, &clip)
}

#[tauri::command]
pub fn add_effect(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    kind: String,
) -> Result<(), String> {
    lock(&state).add_effect(&track, &clip, &kind)
}

#[tauri::command]
pub fn track_mask(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    effect: usize,
    seconds: f64,
) -> Result<TrackResultDto, String> {
    lock(&state).track_mask(&track, &clip, effect, seconds)
}

#[tauri::command]
pub fn set_effect_options(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    index: usize,
    options: EffectOptions,
) -> Result<(), String> {
    lock(&state).set_effect_options(&track, &clip, index, options)
}

#[tauri::command]
pub fn remove_effect(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    index: usize,
) -> Result<(), String> {
    lock(&state).remove_effect(&track, &clip, index)
}

#[tauri::command]
pub fn set_param(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    effect: usize,
    param: Param,
    value: f64,
    keyframe: bool,
) -> Result<(), String> {
    lock(&state).set_param(&track, &clip, effect, param, value, keyframe)
}

#[tauri::command]
pub fn set_track_mix(state: State<'_, Shared>, track: String, mix: TrackMix) -> Result<(), String> {
    lock(&state).set_track_mix(&track, mix)
}

#[tauri::command]
pub fn add_insert(state: State<'_, Shared>, track: String, kind: String) -> Result<(), String> {
    lock(&state).add_insert(&track, &kind)
}

#[tauri::command]
pub fn remove_insert(state: State<'_, Shared>, track: String, index: usize) -> Result<(), String> {
    lock(&state).remove_insert(&track, index)
}

#[tauri::command]
pub fn set_track_duck(
    state: State<'_, Shared>,
    track: String,
    duck: Option<DuckDto>,
) -> Result<(), String> {
    lock(&state).set_track_duck(&track, duck)
}

#[tauri::command]
pub fn export_presets(state: State<'_, Shared>) -> Vec<PresetDto> {
    lock(&state).export_presets()
}

#[tauri::command]
pub fn export_start(
    state: State<'_, Shared>,
    output: String,
    preset: String,
    normalize: Option<f32>,
    caption_sidecar: Option<bool>,
    hardware: Option<bool>,
) -> Result<u64, String> {
    lock(&state).export_start(
        output,
        &preset,
        normalize,
        caption_sidecar.unwrap_or(false),
        hardware.unwrap_or(false),
    )
}

#[tauri::command]
pub fn export_interchange(
    state: State<'_, Shared>,
    path: String,
    format: String,
) -> Result<(), String> {
    lock(&state).export_interchange(&path, &format)
}

#[tauri::command]
pub fn codec_capabilities(state: State<'_, Shared>) -> CodecCapabilitiesDto {
    lock(&state).codec_capabilities()
}

#[tauri::command]
pub fn export_status(state: State<'_, Shared>) -> Vec<ExportStatusDto> {
    lock(&state).export_status()
}

#[tauri::command]
pub fn export_pause(state: State<'_, Shared>, id: u64) {
    lock(&state).export_control(id, "pause")
}

#[tauri::command]
pub fn export_resume(state: State<'_, Shared>, id: u64) {
    lock(&state).export_control(id, "resume")
}

#[tauri::command]
pub fn export_cancel(state: State<'_, Shared>, id: u64) {
    lock(&state).export_control(id, "cancel")
}

#[tauri::command]
pub fn captions(state: State<'_, Shared>) -> Result<Vec<CaptionDto>, String> {
    lock(&state).captions()
}

#[tauri::command]
pub fn add_caption(
    state: State<'_, Shared>,
    start: f64,
    end: f64,
    text: String,
) -> Result<String, String> {
    lock(&state).add_caption(start, end, text)
}

#[tauri::command]
pub fn update_caption(
    state: State<'_, Shared>,
    id: String,
    edit: CaptionEdit,
) -> Result<(), String> {
    lock(&state).update_caption(&id, edit)
}

#[tauri::command]
pub fn remove_caption(state: State<'_, Shared>, id: String) -> Result<(), String> {
    lock(&state).remove_caption(&id)
}

#[tauri::command]
pub fn caption_settings(state: State<'_, Shared>) -> Result<CaptionSettings, String> {
    lock(&state).caption_settings()
}

#[tauri::command]
pub fn set_caption_settings(
    state: State<'_, Shared>,
    settings: CaptionSettings,
) -> Result<(), String> {
    lock(&state).set_caption_settings(settings)
}

#[tauri::command]
pub fn import_srt(state: State<'_, Shared>, path: String) -> Result<usize, String> {
    lock(&state).import_srt(&path)
}

#[tauri::command]
pub fn export_srt(state: State<'_, Shared>, path: String) -> Result<usize, String> {
    lock(&state).export_srt(&path)
}

#[tauri::command]
pub fn markers(state: State<'_, Shared>) -> Result<Vec<MarkerDto>, String> {
    lock(&state).markers()
}

#[tauri::command]
pub fn add_marker(
    state: State<'_, Shared>,
    at: f64,
    note: String,
    clip: Option<String>,
) -> Result<String, String> {
    lock(&state).add_marker(at, note, clip)
}

#[tauri::command]
pub fn update_marker(state: State<'_, Shared>, id: String, edit: MarkerEdit) -> Result<(), String> {
    lock(&state).update_marker(&id, edit)
}

#[tauri::command]
pub fn remove_marker(state: State<'_, Shared>, id: String) -> Result<(), String> {
    lock(&state).remove_marker(&id)
}

#[tauri::command]
pub fn export_markers(state: State<'_, Shared>, path: String) -> Result<usize, String> {
    lock(&state).export_markers(&path)
}

#[tauri::command]
pub fn import_media(state: State<'_, Shared>, path: String) -> Result<MediaDto, String> {
    lock(&state).import_media(path)
}

#[tauri::command]
pub fn relink_media(state: State<'_, Shared>, media: String, path: String) -> Result<(), String> {
    lock(&state).relink_media(&media, path)
}

#[tauri::command]
pub fn create_proxies(
    state: State<'_, Shared>,
    media: Vec<String>,
    divisor: u8,
) -> Result<(), String> {
    lock(&state).create_proxies(media, divisor)
}

#[tauri::command]
pub fn proxy_status(state: State<'_, Shared>) -> Result<Vec<ProxyStatusDto>, String> {
    lock(&state).proxy_status()
}

#[tauri::command]
pub fn use_proxies(state: State<'_, Shared>) -> bool {
    lock(&state).use_proxies()
}

#[tauri::command]
pub fn set_use_proxies(state: State<'_, Shared>, on: bool) -> Result<(), String> {
    lock(&state).set_use_proxies(on)
}

#[tauri::command]
pub fn media_list(state: State<'_, Shared>) -> Result<Vec<MediaDto>, String> {
    lock(&state).media_list()
}

#[tauri::command]
pub fn bins(state: State<'_, Shared>) -> Result<Vec<BinDto>, String> {
    lock(&state).bins()
}

#[tauri::command]
pub fn add_bin(
    state: State<'_, Shared>,
    name: String,
    filter: Option<String>,
) -> Result<String, String> {
    lock(&state).add_bin(name, filter)
}

#[tauri::command]
pub fn add_smart_bin(
    state: State<'_, Shared>,
    name: String,
    rule: RuleDto,
) -> Result<String, String> {
    lock(&state).add_smart_bin(name, rule)
}

#[tauri::command]
pub fn set_media_tags(
    state: State<'_, Shared>,
    media: String,
    keywords: Vec<String>,
    rating: u8,
) -> Result<(), String> {
    lock(&state).set_media_tags(&media, keywords, rating)
}

#[tauri::command]
pub fn rename_bin(state: State<'_, Shared>, id: String, name: String) -> Result<(), String> {
    lock(&state).rename_bin(&id, name)
}

#[tauri::command]
pub fn remove_bin(state: State<'_, Shared>, id: String) -> Result<(), String> {
    lock(&state).remove_bin(&id)
}

#[tauri::command]
pub fn assign_media(
    state: State<'_, Shared>,
    media: String,
    bin: Option<String>,
) -> Result<(), String> {
    lock(&state).assign_media(&media, bin)
}

#[tauri::command]
pub fn ensure_sequence(state: State<'_, Shared>) -> Result<SequenceDto, String> {
    lock(&state).ensure_sequence()
}

#[tauri::command]
pub fn sequences(state: State<'_, Shared>) -> Result<Vec<SequenceListDto>, String> {
    lock(&state).sequences()
}

#[tauri::command]
pub fn open_sequence(state: State<'_, Shared>, id: String) -> Result<SequenceDto, String> {
    lock(&state).open_sequence(&id)
}

#[tauri::command]
pub fn sequence(state: State<'_, Shared>) -> Result<SequenceDto, String> {
    lock(&state).sequence()
}

#[tauri::command]
pub fn add_clip(
    state: State<'_, Shared>,
    track: String,
    media: String,
    at: f64,
) -> Result<(), String> {
    lock(&state).add_clip(&track, &media, at)
}

#[tauri::command]
pub fn add_multicam(
    state: State<'_, Shared>,
    at: f64,
    media: Vec<String>,
    sync: Option<bool>,
    by: Option<SyncBy>,
) -> Result<MulticamSyncDto, String> {
    let by = by.unwrap_or(if sync.unwrap_or(false) {
        SyncBy::Audio
    } else {
        SyncBy::Start
    });
    lock(&state).add_multicam_by(at, media, by)
}

#[tauri::command]
pub fn switch_angle(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    angle: usize,
    cut: bool,
) -> Result<String, String> {
    lock(&state).switch_angle(&track, &clip, angle, cut)
}

#[tauri::command]
pub fn add_title(
    state: State<'_, Shared>,
    at: f64,
    text: String,
    template: Option<String>,
) -> Result<String, String> {
    lock(&state).add_title_from(at, text, template)
}

#[tauri::command]
pub fn title_templates(state: State<'_, Shared>) -> Vec<TemplateDto> {
    lock(&state).title_templates()
}

#[tauri::command]
pub fn save_title_template(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    name: String,
) -> Result<String, String> {
    lock(&state).save_title_template(&track, &clip, name)
}

#[tauri::command]
pub fn remove_title_template(state: State<'_, Shared>, id: String) -> Result<(), String> {
    lock(&state).remove_title_template(&id)
}

#[tauri::command]
pub fn set_title(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    title: Title,
) -> Result<(), String> {
    lock(&state).set_title(&track, &clip, title)
}

#[tauri::command]
pub fn edit(state: State<'_, Shared>, op: EditOp) -> Result<(), String> {
    lock(&state).edit(op)
}

#[tauri::command]
pub fn transport(state: State<'_, Shared>, action: TransportAction) -> Result<(), String> {
    lock(&state).transport(action)
}

/// Advance the player; the UI calls this once per animation frame and fetches
/// pixels when `changed`.
#[tauri::command]
pub fn tick(state: State<'_, Shared>) -> Result<TickDto, String> {
    lock(&state).tick()
}

#[tauri::command]
pub fn set_preview_quality(state: State<'_, Shared>, quality: PreviewQuality) {
    lock(&state).set_preview_quality(quality)
}

#[tauri::command]
pub fn scopes(state: State<'_, Shared>) -> Result<tauri::ipc::Response, String> {
    Ok(tauri::ipc::Response::new(lock(&state).scopes()?))
}

/// The current frame as `width * height * 4` bytes of RGBA8, preceded by two
/// little-endian u32 (width, height).
#[tauri::command]
pub fn frame_pixels(state: State<'_, Shared>) -> Result<tauri::ipc::Response, String> {
    let (w, h, rgba) = lock(&state).frame_pixels()?;
    let mut bytes = Vec::with_capacity(8 + rgba.len());
    bytes.extend_from_slice(&w.to_le_bytes());
    bytes.extend_from_slice(&h.to_le_bytes());
    bytes.extend_from_slice(&rgba);
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub fn waveform(
    state: State<'_, Shared>,
    media: String,
    start: f64,
    end: f64,
    buckets: usize,
) -> Result<Option<Vec<[f32; 2]>>, String> {
    lock(&state).waveform(&media, start, end, buckets)
}

#[tauri::command]
pub fn linked_selection(state: State<'_, Shared>) -> bool {
    lock(&state).linked_selection()
}

#[tauri::command]
pub fn set_linked_selection(state: State<'_, Shared>, on: bool) {
    lock(&state).set_linked_selection(on)
}

#[tauri::command]
pub fn snap_points(
    state: State<'_, Shared>,
    exclude: Option<String>,
) -> Result<Vec<SnapPointDto>, String> {
    lock(&state).snap_points(exclude)
}
