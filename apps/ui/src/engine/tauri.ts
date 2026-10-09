import { invoke } from "@tauri-apps/api/core";
import type { EditOp, EffectInfo, EffectsApi, Engine, FileStatus, Frame, MediaApi, MediaInfo, ParamName, PlayerApi, SequenceInfo, Tick, TransportAction } from "./index";

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

export class TauriEngine implements Engine {
  media = new TauriMedia();
  player = new TauriPlayer();
  effects = new TauriEffects();

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
