import { invoke } from "@tauri-apps/api/core";
import type { EditOp, Engine, Frame, MediaApi, MediaInfo, PlayerApi, SequenceInfo, Tick, TransportAction } from "./index";

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

export class TauriEngine implements Engine {
  media = new TauriMedia();
  player = new TauriPlayer();

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
