// The UI talks to the engine only through these interfaces (PLT-01). The Tauri
// transport forwards to native Rust over IPC; the wasm transport calls the same
// Rust compiled to WebAssembly. Optional parts (`media`, `player`) are capability
// flags: a target that lacks them shows the difference instead of failing (PLT-05).

export interface FileStatus {
  path: string | null;
  dirty: boolean;
  /** Commands recovered from the journal when the file was last opened. */
  recovered: number;
}

export interface ProjectApi {
  version(): Promise<string>;
  newProject(name: string): Promise<void>;
  openProject(json: string): Promise<void>;
  projectJson(): Promise<string>;
  /** Desktop only: project files on disk with autosave and crash recovery. */
  saveProject?(path: string | null): Promise<FileStatus>;
  openProjectFile?(path: string): Promise<FileStatus>;
  fileStatus?(): Promise<FileStatus>;
  /** Apply a `Command` (serde JSON form) through the undo history. */
  execute(commandJson: string): Promise<void>;
  undo(): Promise<boolean>;
  redo(): Promise<boolean>;
  canUndo(): Promise<boolean>;
}

export interface MediaInfo {
  id: string;
  path: string;
  width: number;
  height: number;
  duration: number;
  frame_rate: [number, number];
  has_audio: boolean;
}

export interface ClipInfo {
  id: string;
  media: string | null;
  timeline_in: number;
  duration: number;
  source_in: number;
}

export interface TrackInfo {
  id: string;
  kind: "video" | "audio" | "adjustment";
  clips: ClipInfo[];
}

export interface SequenceInfo {
  id: string;
  name: string;
  width: number;
  height: number;
  frame_rate: [number, number];
  duration: number;
  tracks: TrackInfo[];
}

export type EditOp =
  | { kind: "ripple_head"; track: string; clip: string; delta: number }
  | { kind: "ripple_tail"; track: string; clip: string; delta: number }
  | { kind: "roll"; track: string; clip: string; delta: number }
  | { kind: "slip"; track: string; clip: string; delta: number }
  | { kind: "slide"; track: string; clip: string; delta: number }
  | { kind: "move"; track: string; clip: string; delta: number }
  | { kind: "blade"; track: string; at: number }
  | { kind: "extract"; track: string; start: number; end: number }
  | { kind: "lift"; track: string; start: number; end: number };

export type ParamName =
  | "scale"
  | "rotation"
  | "x"
  | "y"
  | "opacity"
  | "exposure"
  | "contrast"
  | "saturation"
  | "temperature"
  | "tint";

export interface ParamInfo {
  name: ParamName;
  value: number;
  animated: boolean;
}

export interface EffectInfo {
  index: number;
  kind: "transform" | "grade" | "lut";
  params: ParamInfo[];
}

export interface EffectsApi {
  clipEffects(track: string, clip: string): Promise<EffectInfo[]>;
  addEffect(track: string, clip: string, kind: "transform" | "grade"): Promise<void>;
  removeEffect(track: string, clip: string, index: number): Promise<void>;
  /** Set as a constant, or keyframe at the playhead when `keyframe` is true. */
  setParam(track: string, clip: string, effect: number, param: ParamName, value: number, keyframe: boolean): Promise<void>;
}

export interface MediaApi {
  importMedia(path: string): Promise<MediaInfo>;
  ensureSequence(): Promise<SequenceInfo>;
  sequence(): Promise<SequenceInfo>;
  addClip(track: string, media: string, at: number): Promise<void>;
  edit(op: EditOp): Promise<void>;
}

export type TransportAction =
  | { kind: "play" }
  | { kind: "pause" }
  | { kind: "toggle" }
  | { kind: "seek"; t: number }
  | { kind: "step"; n: number }
  | { kind: "shuttle"; forward: boolean };

export interface Tick {
  frame: number;
  position: number;
  playing: boolean;
  changed: boolean;
  dropped: number;
}

export interface Frame {
  width: number;
  height: number;
  rgba: Uint8ClampedArray<ArrayBuffer>;
}

export interface PlayerApi {
  transport(action: TransportAction): Promise<void>;
  tick(): Promise<Tick>;
  framePixels(): Promise<Frame>;
}

export interface Engine extends ProjectApi {
  media?: MediaApi;
  player?: PlayerApi;
  effects?: EffectsApi;
}

export type Target = "desktop" | "browser";

export function detectTarget(): Target {
  return "__TAURI_INTERNALS__" in window ? "desktop" : "browser";
}

export async function loadEngine(): Promise<Engine> {
  if (detectTarget() === "desktop") {
    const { TauriEngine } = await import("./tauri");
    return new TauriEngine();
  }
  const { WasmEngine } = await import("./wasm");
  return WasmEngine.load();
}
