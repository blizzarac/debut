import { invoke } from "@tauri-apps/api/core";
import type {
  TitleInfo,
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
  ExportApi,
  InterchangeFormat,
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
  addMulticam(at: number, media: string[], sync = false) {
    return invoke<MulticamSync>("add_multicam", { at, media, sync });
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
  removeEffect(track: string, clip: string, index: number) {
    return invoke<void>("remove_effect", { track, clip, index });
  }
  setParam(track: string, clip: string, effect: number, param: ParamName, value: number, keyframe: boolean) {
    return invoke<void>("set_param", { track, clip, effect, param, value, keyframe });
  }
}

class TauriMixer implements MixerApi {
  setTrackMix(track: string, mix: TrackMix) {
    return invoke<void>("set_track_mix", { track, mix });
  }
  addInsert(track: string, kind: InsertKind) {
    return invoke<void>("add_insert", { track, kind });
  }
  removeInsert(track: string, index: number) {
    return invoke<void>("remove_insert", { track, index });
  }
}

class TauriExport implements ExportApi {
  presets() {
    return invoke<ExportPreset[]>("export_presets");
  }
  start(output: string, preset: string, normalize: number | null, captionSidecar = false, hardware = false) {
    return invoke<number>("export_start", { output, preset, normalize, captionSidecar, hardware });
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

class TauriCaptions implements CaptionsApi {
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
  media = new TauriMedia();
  player = new TauriPlayer();
  effects = new TauriEffects();
  mixer = new TauriMixer();
  exporter = new TauriExport();

  version() {
    return invoke<string>("version");
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
