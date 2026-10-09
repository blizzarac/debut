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

export type TextAlign = "left" | "center" | "right";

/** Mirrors the project's `TitleStyle` (GFX-01, GFX-02); colours are straight sRGB RGBA bytes. */
export interface TitleStyle {
  font: string;
  size_px: number;
  color: [number, number, number, number];
  align: TextAlign;
  line_height: number;
  letter_spacing: number;
  stroke_px: number;
  stroke_color: [number, number, number, number];
  shadow_px: number;
  shadow_color: [number, number, number, number];
  background: [number, number, number, number];
  padding_px: number;
}

export interface TitleInfo {
  text: string;
  style: TitleStyle;
}

export interface ClipInfo {
  id: string;
  media: string | null;
  /** Set when this is a title clip. */
  title: TitleInfo | null;
  /** Set when this is a compound clip of a nested sequence. */
  nested: string | null;
  timeline_in: number;
  duration: number;
  source_in: number;
  /** Dissolve duration from the previous clip, if any. */
  transition_in: number | null;
}

export interface TrackMix {
  gain_db: number;
  pan: number;
  mute: boolean;
  solo: boolean;
}

export type InsertKind = "eq_presence" | "eq_lowcut" | "compressor" | "limiter" | "gate" | "de_esser" | "reverb";

export interface TrackInfo {
  id: string;
  kind: "video" | "audio" | "adjustment";
  clips: ClipInfo[];
  mix: TrackMix;
  inserts: string[];
}

export interface MixerApi {
  setTrackMix(track: string, mix: TrackMix): Promise<void>;
  addInsert(track: string, kind: InsertKind): Promise<void>;
  removeInsert(track: string, index: number): Promise<void>;
}

export interface ExportPreset {
  name: string;
  loudness_lufs: number;
}

export interface ExportStatus {
  id: number;
  name: string;
  output: string;
  state: "queued" | "running" | "paused" | "done" | "failed" | "cancelled";
  frames_done: number;
  frames_total: number;
  loudness_lufs: number | null;
  true_peak_db: number;
  error: string | null;
}

export interface ExportApi {
  presets(): Promise<ExportPreset[]>;
  /** Queue an export of the whole sequence; `normalize` is a LUFS target or null. */
  start(output: string, preset: string, normalize: number | null): Promise<number>;
  status(): Promise<ExportStatus[]>;
  pause(id: number): Promise<void>;
  resume(id: number): Promise<void>;
  cancel(id: number): Promise<void>;
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

export interface ScopesData {
  waveform: Uint8ClampedArray; // 256 x 128
  vectorscope: Uint8ClampedArray; // 128 x 128
  histogram: [Uint32Array, Uint32Array, Uint32Array];
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
  | { kind: "lift"; track: string; start: number; end: number }
  | { kind: "transition"; track: string; clip: string; duration: number | null }
  | { kind: "nest"; start: number; end: number };

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
  | "tint"
  | "mask_x"
  | "mask_y"
  | "mask_width"
  | "mask_height"
  | "feather"
  | "tolerance"
  | "softness"
  | "spill";

export type EffectKind = "transform" | "grade" | "lut" | "mask" | "key";

/** Non-animated knobs: mask shape/invert, key colour (straight sRGB bytes). */
export interface EffectOptions {
  shape?: "rectangle" | "ellipse";
  invert?: boolean;
  color?: [number, number, number];
}

export interface ParamInfo {
  name: ParamName;
  value: number;
  animated: boolean;
}

export interface EffectInfo {
  index: number;
  kind: EffectKind;
  params: ParamInfo[];
  options: EffectOptions;
}

export interface EffectsApi {
  clipEffects(track: string, clip: string): Promise<EffectInfo[]>;
  addEffect(track: string, clip: string, kind: EffectKind): Promise<void>;
  removeEffect(track: string, clip: string, index: number): Promise<void>;
  /** Change non-animated options; omitted fields keep their value. */
  setOptions(track: string, clip: string, effect: number, options: EffectOptions): Promise<void>;
  /** Set as a constant, or keyframe at the playhead when `keyframe` is true. */
  setParam(track: string, clip: string, effect: number, param: ParamName, value: number, keyframe: boolean): Promise<void>;
}

export interface MarkerInfo {
  id: string;
  at: number;
  duration: number;
  color: [number, number, number];
  note: string;
  clip: string | null;
}

export interface MarkerEdit {
  note?: string;
  color?: [number, number, number];
  duration?: number;
  at?: number;
}

export interface MarkersApi {
  list(): Promise<MarkerInfo[]>;
  add(at: number, note: string, clip: string | null): Promise<string>;
  update(id: string, edit: MarkerEdit): Promise<void>;
  remove(id: string): Promise<void>;
  /** Writes a tab-separated timecode list; returns the number of markers. */
  exportList(path: string): Promise<number>;
}

export interface MediaApi {
  importMedia(path: string): Promise<MediaInfo>;
  ensureSequence(): Promise<SequenceInfo>;
  sequence(): Promise<SequenceInfo>;
  addClip(track: string, media: string, at: number): Promise<void>;
  /** Adds a 5 s title at `at` on a free video track (adds a track if needed); returns the clip id. */
  addTitle(at: number, text: string): Promise<string>;
  setTitle(track: string, clip: string, title: TitleInfo): Promise<void>;
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
  /** Preview resolution divisor in use: 1, 2 or 4. */
  preview_divisor: number;
}

export type PreviewQuality = "full" | "half" | "quarter" | "auto";

export interface Frame {
  width: number;
  height: number;
  rgba: Uint8ClampedArray<ArrayBuffer>;
}

export interface PlayerApi {
  transport(action: TransportAction): Promise<void>;
  tick(): Promise<Tick>;
  framePixels(): Promise<Frame>;
  scopes?(): Promise<ScopesData>;
  setPreviewQuality?(quality: PreviewQuality): Promise<void>;
}

export interface Engine extends ProjectApi {
  media?: MediaApi;
  player?: PlayerApi;
  effects?: EffectsApi;
  mixer?: MixerApi;
  exporter?: ExportApi;
  markers?: MarkersApi;
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
