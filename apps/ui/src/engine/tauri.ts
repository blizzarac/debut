import { invoke } from "@tauri-apps/api/core";
import type {
  AiApi,
  AiStatus,
  HwDecodeStatus,
  ProgramWindow,
  PluginsApi,
  PluginsInfo,
  TitleInfo,
  ShapeInfo,
  SnapPoint,
  CodecCapabilities,
  MulticamSync,
  SmartRule,
  CaptionSettings,
  BinInfo,
  TitleTemplate,
  SequenceListItem,
  CaptionInfo,
  CaptionEdit,
  CaptionsApi,
  EffectKind,
  EffectOptions,
  EditOp,
  EffectInfo,
  EffectsApi,
  Engine,
  Duck,
  ExportApi,
  InterchangeFormat,
  ProxyStatus,
  Shortcuts,
  KeyOverride,
  Keymap,
  SnapshotDiff,
  CollabApi,
  CollabStatus,
  SnapshotInfo,
  SnapshotsApi,
  SyncBy,
  XmlImport,
  Targeting,
  ExportPreset,
  ExportStatus,
  FileStatus,
  Frame,
  InsertKind,
  MarkerEdit,
  MarkerInfo,
  MarkersApi,
  MediaApi,
  MediaInfo,
  MixerApi,
  ParamName,
  PlayerApi,
  PreviewQuality,
  ScopesData,
  SequenceInfo,
  Tick,
  TrackMix,
  TransportAction,
} from "./index";

class TauriMedia implements MediaApi {
  importMedia(path: string) {
    return invoke<MediaInfo>("import_media", { path });
  }
  mediaList() {
    return invoke<MediaInfo[]>("media_list");
  }
  targeting() {
    return invoke<Targeting>("targeting");
  }
  setSourcePatch(kind: "video" | "audio", track: string | null) {
    return invoke<void>("set_source_patch", { kind, track });
  }
  setTrackTargeted(track: string, on: boolean) {
    return invoke<void>("set_track_targeted", { track, on });
  }
  insertMedia(media: string, at: number, overwrite: boolean) {
    return invoke<void>("insert_media", { media, at, overwrite });
  }
  bladeTargeted(at: number) {
    return invoke<void>("blade_targeted", { at });
  }
  linkedSelection() {
    return invoke<boolean>("linked_selection");
  }
  setLinkedSelection(on: boolean) {
    return invoke<void>("set_linked_selection", { on });
  }
  snapPoints(exclude: string | null) {
    return invoke<SnapPoint[]>("snap_points", { exclude });
  }
  waveform(media: string, start: number, end: number, buckets: number) {
    return invoke<[number, number][] | null>("waveform", { media, start, end, buckets });
  }
  bins() {
    return invoke<BinInfo[]>("bins");
  }
  addBin(name: string, filter?: string) {
    return invoke<string>("add_bin", { name, filter: filter ?? null });
  }
  addSmartBin(name: string, rule: SmartRule) {
    return invoke<string>("add_smart_bin", { name, rule });
  }
  setMediaTags(media: string, keywords: string[], rating: number) {
    return invoke<void>("set_media_tags", { media, keywords, rating });
  }
  renameBin(id: string, name: string) {
    return invoke<void>("rename_bin", { id, name });
  }
  removeBin(id: string) {
    return invoke<void>("remove_bin", { id });
  }
  assignMedia(media: string, bin: string | null) {
    return invoke<void>("assign_media", { media, bin });
  }
  relinkMedia(media: string, path: string) {
    return invoke<void>("relink_media", { media, path });
  }
  relinkFolder(dir: string) {
    return invoke<{ relinked: [string, string][]; ambiguous: [string, string[]][]; not_found: string[] }>("relink_folder", { dir });
  }
  ingestFolder(source: string, copyTo: string | null) {
    return invoke<{ imported: number; skipped: number; copied: number; bytes_copied: number; failed: [string, string][] }>("ingest_folder", { source, copyTo });
  }
  createProxies(media: string[], divisor: 2 | 4) {
    return invoke<void>("create_proxies", { media, divisor });
  }
  proxyStatus() {
    return invoke<ProxyStatus[]>("proxy_status");
  }
  useProxies() {
    return invoke<boolean>("use_proxies");
  }
  setUseProxies(on: boolean) {
    return invoke<void>("set_use_proxies", { on });
  }
  ensureSequence() {
    return invoke<SequenceInfo>("ensure_sequence");
  }
  sequence() {
    return invoke<SequenceInfo>("sequence");
  }
  sequences() {
    return invoke<SequenceListItem[]>("sequences");
  }
  openSequence(id: string) {
    return invoke<SequenceInfo>("open_sequence", { id });
  }
  addClip(track: string, media: string, at: number) {
    return invoke<void>("add_clip", { track, media, at });
  }
  addTitle(at: number, text: string, template?: string) {
    return invoke<string>("add_title", { at, text, template: template ?? null });
  }
  titleTemplates() {
    return invoke<TitleTemplate[]>("title_templates");
  }
  saveTitleTemplate(track: string, clip: string, name: string) {
    return invoke<string>("save_title_template", { track, clip, name });
  }
  removeTitleTemplate(id: string) {
    return invoke<void>("remove_title_template", { id });
  }
  setTitle(track: string, clip: string, title: TitleInfo) {
    return invoke<void>("set_title", { track, clip, title });
  }
  addShape(at: number, shape: ShapeInfo) {
    return invoke<string>("add_shape", { at, shape });
  }
  setShape(track: string, clip: string, shape: ShapeInfo) {
    return invoke<void>("set_shape", { track, clip, shape });
  }
  addMulticam(at: number, media: string[], by: SyncBy = "start") {
    return invoke<MulticamSync>("add_multicam", { at, media, by });
  }
  switchAngle(track: string, clip: string, angle: number, cut: boolean) {
    return invoke<string>("switch_angle", { track, clip, angle, cut });
  }
  edit(op: EditOp) {
    return invoke<void>("edit", { op });
  }
}

class TauriPlayer implements PlayerApi {
  transport(action: TransportAction) {
    return invoke<void>("transport", { action });
  }
  tick() {
    return invoke<Tick>("tick");
  }
  async framePixels(): Promise<Frame> {
    const buf = await invoke<ArrayBuffer>("frame_pixels");
    const view = new DataView(buf);
    const width = view.getUint32(0, true);
    const height = view.getUint32(4, true);
    return { width, height, rgba: new Uint8ClampedArray(buf, 8, width * height * 4) };
  }
  setPreviewQuality(quality: PreviewQuality) {
    return invoke<void>("set_preview_quality", { quality });
  }
  setHardwareDecode(on: boolean) {
    return invoke<void>("set_hardware_decode", { on });
  }
  hardwareDecodeStatus() {
    return invoke<HwDecodeStatus>("hardware_decode_status");
  }
  monitors() {
    return invoke<{ index: number; name: string; width: number; height: number }[]>("monitors");
  }
  openProgramWindow(monitor: number | null, fullscreen: boolean) {
    return invoke<ProgramWindow>("open_program_window", { monitor, fullscreen });
  }
  closeProgramWindow() {
    return invoke<void>("close_program_window");
  }
  programFullscreen(on: boolean) {
    return invoke<void>("program_fullscreen", { on });
  }
  programWindow() {
    return invoke<ProgramWindow>("program_window");
  }
  async scopes(): Promise<ScopesData> {
    const buf = await invoke<ArrayBuffer>("scopes");
    const wave = 256 * 128;
    const vec = 128 * 128;
    const h = (i: number) => new Uint32Array(buf.slice(wave + vec + i * 1024, wave + vec + (i + 1) * 1024));
    return { waveform: new Uint8ClampedArray(buf, 0, wave), vectorscope: new Uint8ClampedArray(buf, wave, vec), histogram: [h(0), h(1), h(2)] };
  }
}

class TauriEffects implements EffectsApi {
  clipEffects(track: string, clip: string) {
    return invoke<EffectInfo[]>("clip_effects", { track, clip });
  }
  addEffect(track: string, clip: string, kind: EffectKind) {
    return invoke<void>("add_effect", { track, clip, kind });
  }
  setOptions(track: string, clip: string, index: number, options: EffectOptions) {
    return invoke<void>("set_effect_options", { track, clip, index, options });
  }
  trackMask(track: string, clip: string, effect: number, seconds: number) {
    return invoke<{ keys: number; weakest_match: number }>("track_mask", { track, clip, effect, seconds });
  }
  trackPlanar(track: string, clip: string, effect: number, seconds: number) {
    return invoke<{ keys: number; weakest_match: number }>("track_planar", { track, clip, effect, seconds });
  }
  clearPlanar(track: string, clip: string, effect: number) {
    return invoke<void>("clear_planar", { track, clip, effect });
  }
  removeEffect(track: string, clip: string, index: number) {
    return invoke<void>("remove_effect", { track, clip, index });
  }
  setParam(track: string, clip: string, effect: number, param: ParamName, value: number, keyframe: boolean) {
    return invoke<void>("set_param", { track, clip, effect, param, value, keyframe });
  }
  addPluginEffect(track: string, clip: string, path: string, index: number) {
    return invoke<void>("add_plugin_effect", { track, clip, path, index });
  }
  setPluginParam(track: string, clip: string, effect: number, name: string, value: number, keyframe: boolean) {
    return invoke<void>("set_plugin_param", { track, clip, effect, name, value, keyframe });
  }
}

class TauriPlugins implements PluginsApi {
  scan() {
    return invoke<PluginsInfo>("scan_plugins");
  }
  list() {
    return invoke<PluginsInfo>("plugins");
  }
  lastError() {
    return invoke<string | null>("plugin_error");
  }
}

class TauriMixer implements MixerApi {
  setTrackMix(track: string, mix: TrackMix) {
    return invoke<void>("set_track_mix", { track, mix });
  }
  addInsert(track: string, kind: InsertKind) {
    return invoke<void>("add_insert", { track, kind });
  }
  setTrackDuck(track: string, duck: Duck | null) {
    return invoke<void>("set_track_duck", { track, duck });
  }
  removeInsert(track: string, index: number) {
    return invoke<void>("remove_insert", { track, index });
  }
  addPluginInsert(track: string, path: string, index: number) {
    return invoke<void>("add_plugin_insert", { track, path, index });
  }
  setInsertParam(track: string, insert: number, name: string, value: number) {
    return invoke<void>("set_insert_param", { track, insert, name, value });
  }
}

class TauriExport implements ExportApi {
  presets() {
    return invoke<ExportPreset[]>("export_presets");
  }
  start(output: string, preset: string, normalize: number | null, captionSidecar = false, hardware = false, smart = false) {
    return invoke<number>("export_start", { output, preset, normalize, captionSidecar, hardware, smart });
  }
  interchange(path: string, format: InterchangeFormat) {
    return invoke<void>("export_interchange", { path, format });
  }
  capabilities() {
    return invoke<CodecCapabilities>("codec_capabilities");
  }
  status() {
    return invoke<ExportStatus[]>("export_status");
  }
  pause(id: number) {
    return invoke<void>("export_pause", { id });
  }
  resume(id: number) {
    return invoke<void>("export_resume", { id });
  }
  cancel(id: number) {
    return invoke<void>("export_cancel", { id });
  }
}

class TauriCollab implements CollabApi {
  host(port: number, name: string) {
    return invoke<string>("collab_host", { port, name });
  }
  join(addr: string, name: string, role: "editor" | "reviewer") {
    return invoke<void>("collab_join", { addr, name, role });
  }
  leave() {
    return invoke<void>("collab_leave");
  }
  poll(clip: string | null) {
    return invoke<[boolean, CollabStatus | null]>("collab_poll", { clip });
  }
  lock(track: string, on: boolean) {
    return invoke<void>("collab_lock", { track, on });
  }
}

class TauriSnapshots implements SnapshotsApi {
  list() {
    return invoke<SnapshotInfo[]>("snapshots");
  }
  take(name: string) {
    return invoke<string>("take_snapshot", { name });
  }
  restore(id: string) {
    return invoke<void>("restore_snapshot", { id });
  }
  remove(id: string) {
    return invoke<void>("remove_snapshot", { id });
  }
  compare(id: string) {
    return invoke<SnapshotDiff>("compare_snapshot", { id });
  }
}

class TauriMarkers implements MarkersApi {
  list() {
    return invoke<MarkerInfo[]>("markers");
  }
  add(at: number, note: string, clip: string | null) {
    return invoke<string>("add_marker", { at, note, clip });
  }
  update(id: string, edit: MarkerEdit) {
    return invoke<void>("update_marker", { id, edit });
  }
  remove(id: string) {
    return invoke<void>("remove_marker", { id });
  }
  exportList(path: string) {
    return invoke<number>("export_markers", { path });
  }
}

class TauriAi implements AiApi {
  status() {
    return invoke<AiStatus>("ai_status");
  }
  setEnabled(on: boolean) {
    return invoke<AiStatus>("set_ai_enabled", { on });
  }
  configure(engine: string, model: string) {
    return invoke<AiStatus>("configure_transcriber", { engine, model });
  }
  transcribeClip(track: string, clip: string, language: string | null) {
    return invoke<{ segments: number; captions_added: number; backend: string }>("transcribe_clip", { track, clip, language });
  }
}

class TauriCaptions implements CaptionsApi {
  search(query: string) {
    return invoke<{ id: string; start: number; text: string }[]>("search_captions", { query });
  }
  list() {
    return invoke<CaptionInfo[]>("captions");
  }
  settings() {
    return invoke<CaptionSettings>("caption_settings");
  }
  setSettings(settings: CaptionSettings) {
    return invoke<void>("set_caption_settings", { settings });
  }
  add(start: number, end: number, text: string) {
    return invoke<string>("add_caption", { start, end, text });
  }
  update(id: string, edit: CaptionEdit) {
    return invoke<void>("update_caption", { id, edit });
  }
  remove(id: string) {
    return invoke<void>("remove_caption", { id });
  }
  importSrt(path: string) {
    return invoke<number>("import_srt", { path });
  }
  exportSrt(path: string) {
    return invoke<number>("export_srt", { path });
  }
}

export class TauriEngine implements Engine {
  captions = new TauriCaptions();
  markers = new TauriMarkers();
  snapshots = new TauriSnapshots();
  collab = new TauriCollab();
  media = new TauriMedia();
  player = new TauriPlayer();
  effects = new TauriEffects();
  mixer = new TauriMixer();
  exporter = new TauriExport();
  plugins = new TauriPlugins();
  ai = new TauriAi();

  version() {
    return invoke<string>("version");
  }
  shortcuts() {
    return invoke<Shortcuts>("shortcuts");
  }
  resolveKeymap(id: string, overrides: KeyOverride[]) {
    return invoke<Keymap>("resolve_keymap", { id, overrides });
  }
  newProject(name: string) {
    return invoke<void>("new_project", { name });
  }
  openProject(json: string) {
    return invoke<void>("open_project", { json });
  }
  projectJson() {
    return invoke<string>("project_json");
  }
  saveProject(path: string | null) {
    return invoke<FileStatus>("save_project", { path });
  }
  importFcpxml(path: string) {
    return invoke<XmlImport>("import_fcpxml", { path });
  }
  openProjectFile(path: string) {
    return invoke<FileStatus>("open_project_file", { path });
  }
  fileStatus() {
    return invoke<FileStatus>("file_status");
  }
  execute(commandJson: string) {
    return invoke<void>("execute", { commandJson });
  }
  undo() {
    return invoke<boolean>("undo");
  }
  redo() {
    return invoke<boolean>("redo");
  }
  canUndo() {
    return invoke<boolean>("can_undo");
  }
}
