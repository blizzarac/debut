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
  /** Ids of the bins this media is in (manual and smart). */
  bins: string[];
  keywords: string[];
  /** 0 = unrated, else 1..5. */
  rating: number;
  /** False when the file is missing; clips show a slate until relinked. */
  online: boolean;
  /** Start timecode and reel from the file's tags. */
  timecode: string | null;
  reel: string | null;
}

/** How a multicam clip lines up its angles. */
export type SyncBy = "start" | "audio" | "timecode";

export type RuleField = "name" | "path" | "keyword" | "rating" | "reel" | "camera" | "audio";

export interface SmartRule {
  field: RuleField;
  op: "contains" | "eq" | "starts" | "gte" | "lte";
  value: string;
}

export interface MulticamSync {
  offsets: number[];
  confidences: number[];
}

export interface BinInfo {
  id: string;
  name: string;
  smart: boolean;
  filter: string | null;
  count: number;
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
  /** Widen the raster to at least this many pixels (lower-third bars). */
  min_width_px?: number;
}

export interface TitleTemplate {
  id: string;
  name: string;
  description: string;
  /** Project-saved (removable) rather than built-in. */
  saved: boolean;
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
  /** Multicam clips: angle count and the active angle. */
  angles: number | null;
  angle: number | null;
  /** Per-angle head offsets in seconds (audio sync). */
  angle_offsets: number[] | null;
  timeline_in: number;
  duration: number;
  source_in: number;
  /** Dissolve duration from the previous clip, if any. */
  transition_in: number | null;
  /** Constant speed: 1 normal, 0 freeze, negative reverse. */
  speed: number;
  /** Speed ramp in clip-local seconds; overrides `speed` when non-empty. */
  ramp: SpeedKey[];
}

export interface SpeedKey {
  at: number;
  speed: number;
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
  /** Auto-ducking under another audio track, if set. */
  duck: Duck | null;
}

/** Lower a track while the `key` track (dialogue) is active (AUD-08). */
export interface Duck {
  key: string;
  amount_db: number;
  threshold_db: number;
  attack_ms: number;
  release_ms: number;
}

export interface MixerApi {
  setTrackMix(track: string, mix: TrackMix): Promise<void>;
  setTrackDuck?(track: string, duck: Duck | null): Promise<void>;
  addInsert(track: string, kind: InsertKind): Promise<void>;
  removeInsert(track: string, index: number): Promise<void>;
}

export interface ExportPreset {
  name: string;
  loudness_lufs: number;
}

export interface CodecCapabilities {
  hardware_encoders: { name: string; codec: string; api: string }[];
  hardware_decoders: string[];
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

export type InterchangeFormat = "edl" | "otio";

export interface ExportApi {
  presets(): Promise<ExportPreset[]>;
  /** Queue an export of the whole sequence; `normalize` is a LUFS target or null. */
  start(output: string, preset: string, normalize: number | null, captionSidecar?: boolean, hardware?: boolean): Promise<number>;
  /** Hardware encoders that open on this machine and hardware decoders compiled in. */
  capabilities?(): Promise<CodecCapabilities>;
  /** Write the active sequence as an EDL or OpenTimelineIO file for another application. */
  interchange?(path: string, format: InterchangeFormat): Promise<void>;
  status(): Promise<ExportStatus[]>;
  pause(id: number): Promise<void>;
  resume(id: number): Promise<void>;
  cancel(id: number): Promise<void>;
}

export interface SequenceListItem {
  id: string;
  name: string;
  duration: number;
  active: boolean;
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
  | { kind: "nest"; start: number; end: number }
  | { kind: "close_gaps"; track: string }
  | { kind: "speed"; track: string; clip: string; speed: number; ripple: boolean }
  | { kind: "ramp"; track: string; clip: string; keys: SpeedKey[] };

/** A place a drag may snap to: sequence start, playhead, clip edge or marker. */
export interface SnapPoint {
  t: number;
  kind: "start" | "playhead" | "edge" | "marker";
}

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
  shape?: "rectangle" | "ellipse" | "polygon";
  invert?: boolean;
  color?: [number, number, number];
  /** Polygon mask vertices in sequence pixels from the frame centre. */
  points?: [number, number][];
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
  /** Track the picture under a mask from the playhead for `seconds`, keyframing its position. */
  trackMask?(track: string, clip: string, effect: number, seconds: number): Promise<{ keys: number; weakest_match: number }>;
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

export interface CaptionInfo {
  id: string;
  start: number;
  end: number;
  text: string;
}

export interface CaptionEdit {
  start?: number;
  end?: number;
  text?: string;
}

export type CaptionPosition = "bottom" | "top";

/** Burn-in look of a sequence's captions; sizes are for 1080 lines and scale. */
export interface CaptionSettings {
  burn_in: boolean;
  position: CaptionPosition;
  font: string;
  size_px: number;
  color: [number, number, number, number];
  background: [number, number, number, number];
}

export interface CaptionsApi {
  list(): Promise<CaptionInfo[]>;
  settings(): Promise<CaptionSettings>;
  setSettings(settings: CaptionSettings): Promise<void>;
  add(start: number, end: number, text: string): Promise<string>;
  update(id: string, edit: CaptionEdit): Promise<void>;
  remove(id: string): Promise<void>;
  /** Read an .srt file into the sequence; returns the cue count. */
  importSrt(path: string): Promise<number>;
  exportSrt(path: string): Promise<number>;
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

export interface ProxyStatus {
  media: string;
  state: "none" | "queued" | "running" | "ready" | "failed";
  progress: number;
  error: string | null;
  path: string | null;
}

export interface MediaApi {
  importMedia(path: string): Promise<MediaInfo>;
  /** Every media in the project (also after opening a file). */
  mediaList(): Promise<MediaInfo[]>;
  /** Whether edits follow a clip's linked partners (same source and span) on other tracks. */
  linkedSelection?(): Promise<boolean>;
  setLinkedSelection?(on: boolean): Promise<void>;
  /** Snap targets for a drag, leaving out the clip being dragged (and its partners). */
  snapPoints?(exclude: string | null): Promise<SnapPoint[]>;
  /** [min, max] audio peaks of a media between source times; null while still being built. */
  waveform?(media: string, start: number, end: number, buckets: number): Promise<[number, number][] | null>;
  bins(): Promise<BinInfo[]>;
  /** With `filter`, a smart bin matching file names containing it. */
  addBin(name: string, filter?: string): Promise<string>;
  addSmartBin(name: string, rule: SmartRule): Promise<string>;
  setMediaTags(media: string, keywords: string[], rating: number): Promise<void>;
  renameBin(id: string, name: string): Promise<void>;
  removeBin(id: string): Promise<void>;
  assignMedia(media: string, bin: string | null): Promise<void>;
  /** Point a media at another file; the player reloads it. */
  relinkMedia(media: string, path: string): Promise<void>;
  /** Build 1/2 or 1/4 resolution proxies in the background (MED-05). */
  createProxies?(media: string[], divisor: 2 | 4): Promise<void>;
  proxyStatus?(): Promise<ProxyStatus[]>;
  useProxies?(): Promise<boolean>;
  /** Play video from proxies where they exist; export always uses originals. */
  setUseProxies?(on: boolean): Promise<void>;
  ensureSequence(): Promise<SequenceInfo>;
  sequence(): Promise<SequenceInfo>;
  /** Every sequence in the project; one is active (shown in the timeline). */
  sequences(): Promise<SequenceListItem[]>;
  /** Show a sequence (a nested one, or the main one again) in the timeline. */
  openSequence(id: string): Promise<SequenceInfo>;
  addClip(track: string, media: string, at: number): Promise<void>;
  /** Adds a 5 s title at `at` on a free video track (adds a track if needed); returns the clip id. */
  addTitle(at: number, text: string, template?: string): Promise<string>;
  titleTemplates(): Promise<TitleTemplate[]>;
  /** Save a title clip's style and effects as a project template; returns its id. */
  saveTitleTemplate(track: string, clip: string, name: string): Promise<string>;
  removeTitleTemplate(id: string): Promise<void>;
  setTitle(track: string, clip: string, title: TitleInfo): Promise<void>;
  /** Inserts a multicam clip of two or more media at `at` (shortest angle sets the length);
   * with `sync`, angles are aligned to the first by audio. */
  addMulticam(at: number, media: string[], by?: SyncBy): Promise<MulticamSync>;
  /** Switch a multicam clip's angle; with `cut`, blade at the playhead first and switch the tail. Returns the switched clip id. */
  switchAngle(track: string, clip: string, angle: number, cut: boolean): Promise<string>;
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
  captions?: CaptionsApi;
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
