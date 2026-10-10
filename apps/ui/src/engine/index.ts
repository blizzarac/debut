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
  /** Bring a Final Cut Pro XML file in as new sequences (opens the first). */
  importFcpxml?(path: string): Promise<XmlImport>;
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
  /** Variable frame rate: playback holds frames across its gaps. */
  vfr?: boolean;
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

type Rgba8 = [number, number, number, number];

/** Mirrors the project's `ShapeKind` (GFX-03), flattened into the shape. */
export type ShapeKind =
  | { kind: "rectangle"; corner_px: number }
  | { kind: "ellipse" }
  | { kind: "polygon"; sides: number }
  | { kind: "star"; points: number; inner: number }
  | { kind: "arrow"; head: number; shaft: number }
  | { kind: "line" };

export type ShapeFill = { kind: "none" } | { kind: "solid"; color: Rgba8 } | { kind: "linear"; from: Rgba8; to: Rgba8; angle_deg: number };

/** A shape clip (GFX-03), authored in sequence pixels; colours are straight sRGB RGBA bytes. */
export type ShapeInfo = ShapeKind & {
  width: number;
  height: number;
  fill: ShapeFill;
  stroke_px: number;
  stroke_color: Rgba8;
};

export interface ClipInfo {
  id: string;
  media: string | null;
  /** Set when this is a title clip. */
  title: TitleInfo | null;
  /** Set when this is a shape clip. */
  shape?: ShapeInfo | null;
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
  /** Each insert's parameters: a plugin's, empty for built-ins. */
  insert_params?: PluginParam[][];
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
  /** A scanned CLAP effect as an insert (desktop only). */
  addPluginInsert?(track: string, path: string, index: number): Promise<void>;
  setInsertParam?(track: string, insert: number, name: string, value: number): Promise<void>;
}

/** A third-party plugin: an OpenFX filter or a CLAP effect (FX-15, AUD-09). */
export interface PluginInfo {
  kind: "openfx" | "clap";
  path: string;
  index: number;
  id: string;
  name: string;
  params: PluginParam[];
}

export interface PluginParam {
  name: string;
  label: string;
  min: number;
  max: number;
  value: number;
  animated: boolean;
}

export interface PluginsInfo {
  available: boolean;
  plugins: PluginInfo[];
  /** Binaries that could not be used: [path, why]. */
  problems: [string, string][];
}

export interface PluginsApi {
  /** Load every plugin on the search path (in the helper process) and list them. */
  scan(): Promise<PluginsInfo>;
  /** The last scan's result. */
  list(): Promise<PluginsInfo>;
  /** The last error a plugin effect reported while rendering. */
  lastError(): Promise<string | null>;
}

export interface ExportPreset {
  name: string;
  loudness_lufs: number;
  /** "pq" or "hlg" for HDR presets (10-bit HEVC, Rec.2020). */
  hdr?: "pq" | "hlg" | null;
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
  /** Measured light levels in nits (HDR exports). */
  max_cll?: number | null;
  max_fall?: number | null;
  error: string | null;
}

export type InterchangeFormat = "edl" | "otio" | "fcpxml" | "aaf";

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

export type EffectKind = "transform" | "grade" | "lut" | "mask" | "key" | "plugin";

/** Non-animated knobs: mask shape/invert, key colour (straight sRGB bytes). */
export interface EffectOptions {
  shape?: "rectangle" | "ellipse" | "polygon";
  invert?: boolean;
  color?: [number, number, number];
  /** Polygon mask vertices in sequence pixels from the frame centre. */
  points?: [number, number][];
  /** Bézier handles per point: [in_x, in_y, out_x, out_y] relative to it; empty = straight edges. */
  handles?: [number, number, number, number][];
  /** Setting only: true makes a smooth curve through the points, false makes corners. */
  smooth?: boolean;
  /** Read only: frames keyed by a planar track. */
  planar_keys?: number;
  /** Read only: the planar homography at the playhead (row-major 3x3). */
  planar_now?: number[];
  /** Read only: a plugin effect's identity and parameters at the playhead. */
  plugin?: PluginInfo;
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
  /** Track the surface under a mask through perspective changes; the outline follows it. */
  trackPlanar?(track: string, clip: string, effect: number, seconds: number): Promise<{ keys: number; weakest_match: number }>;
  clearPlanar?(track: string, clip: string, effect: number): Promise<void>;
  /** Set as a constant, or keyframe at the playhead when `keyframe` is true. */
  setParam(track: string, clip: string, effect: number, param: ParamName, value: number, keyframe: boolean): Promise<void>;
  /** Add a scanned OpenFX filter (desktop only). */
  addPluginEffect?(track: string, clip: string, path: string, index: number): Promise<void>;
  setPluginParam?(track: string, clip: string, effect: number, name: string, value: number, keyframe: boolean): Promise<void>;
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
  /** Captions whose text contains `query`, in time order. */
  search?(query: string): Promise<{ id: string; start: number; text: string }[]>;
}

/** On-device AI (GFX-04): off until opted in; models run locally. */
export interface AiStatus {
  enabled: boolean;
  /** A local transcription engine and model are set up. */
  available: boolean;
  backend: string | null;
}

export interface AiApi {
  status(): Promise<AiStatus>;
  setEnabled(on: boolean): Promise<AiStatus>;
  /** The whisper.cpp command-line tool and a ggml model file. */
  configure(engine: string, model: string): Promise<AiStatus>;
  /** Transcribe a clip's audio into captions over it (one undo step). */
  transcribeClip(track: string, clip: string, language: string | null): Promise<{ segments: number; captions_added: number; backend: string }>;
}

export interface MarkerEdit {
  note?: string;
  color?: [number, number, number];
  duration?: number;
  at?: number;
}

/** A saved version of the active sequence (TL-14). */
export interface SnapshotInfo {
  id: string;
  name: string;
  clips: number;
  duration: number;
}

export interface SnapshotDiff {
  clips: { clip: string; track: string; at: number; kinds: ("added" | "removed" | "moved" | "trimmed" | "retimed" | "changed")[] }[];
  markers_added: number;
  markers_removed: number;
  captions_added: number;
  captions_removed: number;
}

export interface SnapshotsApi {
  list(): Promise<SnapshotInfo[]>;
  /** Save the active sequence as it is now; an empty name becomes "Version n". */
  take(name: string): Promise<string>;
  /** Put the sequence back to a snapshot (undoable). */
  restore(id: string): Promise<void>;
  remove(id: string): Promise<void>;
  /** What changed from the snapshot to now. */
  compare(id: string): Promise<SnapshotDiff>;
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

export interface Targeting {
  video_source: string | null;
  audio_source: string | null;
  targeted: string[];
}

export interface MediaApi {
  importMedia(path: string): Promise<MediaInfo>;
  /** Every media in the project (also after opening a file). */
  mediaList(): Promise<MediaInfo[]>;
  /** Source patching and track targeting of the active sequence. */
  targeting?(): Promise<Targeting>;
  /** Send inserted media's picture/sound to `track`, or nowhere (null). */
  setSourcePatch?(kind: "video" | "audio", track: string | null): Promise<void>;
  setTrackTargeted?(track: string, on: boolean): Promise<void>;
  /** Put media on the patched tracks at `at`: insert (pushes later clips) or overwrite. */
  insertMedia?(media: string, at: number, overwrite: boolean): Promise<void>;
  /** Blade every targeted track at `at`. */
  bladeTargeted?(at: number): Promise<void>;
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
  /** Find every missing file by name under `dir` and relink single matches. */
  relinkFolder?(dir: string): Promise<{ relinked: [string, string][]; ambiguous: [string, string[]][]; not_found: string[] }>;
  /** Import a folder of media; with `copyTo`, copy (and verify) it there first. */
  ingestFolder?(source: string, copyTo: string | null): Promise<{ imported: number; skipped: number; copied: number; bytes_copied: number; failed: [string, string][] }>;
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
  /** Adds a 5 s shape at `at` on a free video track, centred; returns the clip id. */
  addShape?(at: number, shape: ShapeInfo): Promise<string>;
  setShape?(track: string, clip: string, shape: ShapeInfo): Promise<void>;
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
  /** Decode video on the GPU where a device takes it (desktop). */
  setHardwareDecode?(on: boolean): Promise<void>;
  hardwareDecodeStatus?(): Promise<HwDecodeStatus>;
  /** A native window showing the program straight from the GPU (desktop). */
  monitors?(): Promise<{ index: number; name: string; width: number; height: number }[]>;
  openProgramWindow?(monitor: number | null, fullscreen: boolean): Promise<ProgramWindow>;
  closeProgramWindow?(): Promise<void>;
  programFullscreen?(on: boolean): Promise<void>;
  programWindow?(): Promise<ProgramWindow>;
}

export interface ProgramWindow {
  open: boolean;
  size: [number, number] | null;
  error: string | null;
}

export interface HwDecodeStatus {
  /** A hardware device opens on this machine (it may still turn streams down). */
  available: boolean;
  enabled: boolean;
  /** Per open media: "software", "requested" (no frame yet), "hardware" (detail = device API) or "fallback" (detail = why). */
  media: { media: string; name: string; mode: "software" | "requested" | "hardware" | "fallback"; detail: string }[];
}

/** A key chord bound to a shortcut action (TL-12). */
export interface Binding {
  action: string;
  /** KeyboardEvent.key, lowercased ("j", " ", "arrowleft", "delete"). */
  key: string;
  /** Ctrl, or ⌘ on a Mac. */
  cmd: boolean;
  shift: boolean;
  alt: boolean;
}

export interface Keymap {
  id: string;
  name: string;
  bindings: Binding[];
}

export type Chord = Omit<Binding, "action">;

export interface KeyOverride {
  action: string;
  /** Empty unbinds the action. */
  keys: Chord[];
}

export interface Shortcuts {
  actions: { id: string; label: string }[];
  /** Presets, the default first. */
  keymaps: Keymap[];
}

export interface XmlImport {
  sequence: string;
  name: string;
  sequences: number;
  clips: number;
  media_added: number;
  missing: string[];
  skipped: string[];
}

/** A collaborator in the shared session (COL-02). */
export interface Peer {
  client: number;
  name: string;
  role: "editor" | "reviewer";
  sequence: string | null;
  playhead: number;
  clip: string | null;
}

export interface CollabStatus {
  connected: boolean;
  joined: boolean;
  address: string;
  name: string;
  role: "editor" | "reviewer";
  client: number;
  version: number;
  pending: number;
  peers: Peer[];
  locks: { track: string; client: number; owner: string }[];
  notes: string[];
}

export interface CollabApi {
  /** Serve the open project on `port` and join it; returns the local address. */
  host(port: number, name: string): Promise<string>;
  join(addr: string, name: string, role: "editor" | "reviewer"): Promise<void>;
  leave(): Promise<void>;
  /** Read the network; [project changed, session state]. */
  poll(clip: string | null): Promise<[boolean, CollabStatus | null]>;
  lock(track: string, on: boolean): Promise<void>;
}

export interface Engine extends ProjectApi {
  collab?: CollabApi;
  /** Keyboard shortcut presets (debut, Premiere Pro, Final Cut Pro, Avid). */
  shortcuts?(): Promise<Shortcuts>;
  /** A preset with the user's own keys on top: each overridden action gets exactly its keys. */
  resolveKeymap?(id: string, overrides: KeyOverride[]): Promise<Keymap>;
  media?: MediaApi;
  player?: PlayerApi;
  effects?: EffectsApi;
  mixer?: MixerApi;
  exporter?: ExportApi;
  markers?: MarkersApi;
  snapshots?: SnapshotsApi;
  captions?: CaptionsApi;
  plugins?: PluginsApi;
  ai?: AiApi;
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
