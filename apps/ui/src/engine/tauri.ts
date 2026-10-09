import { invoke } from "@tauri-apps/api/core";
import type {
  EditOp,
  EffectInfo,
  EffectsApi,
  Engine,
  ExportApi,
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
  ensureSequence() {
    return invoke<SequenceInfo>("ensure_sequence");
  }
  sequence() {
    return invoke<SequenceInfo>("sequence");
  }
  addClip(track: string, media: string, at: number) {
    return invoke<void>("add_clip", { track, media, at });
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
  addEffect(track: string, clip: string, kind: "transform" | "grade") {
    return invoke<void>("add_effect", { track, clip, kind });
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
  start(output: string, preset: string, normalize: number | null) {
    return invoke<number>("export_start", { output, preset, normalize });
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

export class TauriEngine implements Engine {
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
