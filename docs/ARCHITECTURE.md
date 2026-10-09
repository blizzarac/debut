# debut architecture

One Rust engine compiles natively for desktop and to WebAssembly for the browser,
under one shared TypeScript UI. Roughly 80–90% of the code is shared; only the
platform layer differs per target. Requirements: *Video Editor Requirements*
(claude.ai/code/artifact/92b88caa-d0f5-41d4-869d-6313438ad4fb).

## Layering

```
apps/ui (TypeScript/React)          one UI for both targets
   │                 │
apps/desktop      apps/web           Tauri IPC         wasm-bindgen
   │                 │
          debut-engine               facade: api, playback, session
   │  │  │  │  │  │  │  │
   project  command  media  timeline  audio  render  graphics  export  collab
   │                                                                   │
          debut-platform             six traits; no feature logic
   │                 │
platform-native   platform-web       FFmpeg/CoreAudio/OpenFX   WebCodecs/OPFS/AudioWorklet
```

Rules enforced by the crate graph:

- `debut-core` and `debut-platform` have no OS, GPU or codec dependencies and compile on every target (PLT-01).
- Engine crates depend on `debut-platform` traits only, never on an implementation crate (PLT-02).
- The project is mutated only through `debut-command`, so every edit is undoable, journaled and syncable (TL-11, NFR-05, COL).
- Playback and export run the same `debut-render` graph; the audio clock in `debut-audio` is the master (PB-02).

## Crate → requirement map

| Crate | Owns | Requirement IDs |
| --- | --- | --- |
| `debut-core` | Rational time, timecode, frame rates, IDs, color-space tags, errors | MED-03, MED-04, AUD-01 |
| `debut-platform` | Decoder/Encoder, FileStore, AudioOut, Threads, PluginHost, Display traits; `Capabilities` | PLT-01, PLT-02, PLT-05 |
| `debut-project` | Bins, smart bins, media refs + metadata + proxies, sequences, tracks, clips, markers, schema migration | MED-04 – MED-07, MED-09, MED-10, TL-01, TL-02, TL-07, TL-10, NFR-06, PLT-03 |
| `debut-command` | Command enum, history (undo/redo), journal | TL-11, NFR-05, COL-05, COL-06 |
| `debut-media` | Ingest, proxy queue, relink, multicam sync, VFR conform, FCPXML/EDL/AAF/OTIO | MED-01 – MED-03, MED-05, MED-06, MED-11 – MED-13 |
| `debut-timeline` | Edit/trim/select/snap operations, speed, evaluation plan, shortcuts, snapshots | TL-03 – TL-06, TL-08, TL-09, TL-12 – TL-16, NFR-02 |
| `debut-audio` | RT graph, clock, mixer/buses, built-in FX, loudness, waveforms, ducking | PB-02, AUD-02 – AUD-08, AUD-10, AUD-11 |
| `debut-render` | wgpu pull graph, nodes, keyframes, OCIO color, tracking, cache, scopes, OpenFX bridge | FX-01 – FX-15, PB-01, PB-03, PB-06, PB-08, PB-10, PLT-04 |
| `debut-graphics` | Text, templates, shapes, transcription, captions, on-device AI host | GFX-01 – GFX-11, FX-14, AUD-07, TL-13 |
| `debut-export` | Presets, queue, jobs/batches/stems, smart render, HDR metadata, upload | EXP-01 – EXP-10, NFR-04 |
| `debut-collab` | Op sync, locks, presence, review comments, roles | COL-01 – COL-08 |
| `debut-engine` | Public API, transport/playback, session (autosave, recovery, workspaces) | PB-04, PB-05, PB-07, PB-11, MED-09, NFR-05, NFR-18 |
| `debut-platform-native` | FFmpeg + VideoToolbox/NVDEC/QSV/AMF, native FS, CoreAudio/WASAPI, native threads, OpenFX/VST3/AU out-of-process, SDI/HDMI | NFR-07 – NFR-09, NFR-12, PB-09, AUD-09 |
| `debut-platform-web` | WebCodecs + WASM fallback, OPFS, AudioWorklet, Web Workers, sandboxed WASM plugins, WebGPU canvas | PLT-06 – PLT-09 |
| `apps/desktop` | Tauri shell | PLT-10 |
| `apps/web` | wasm-bindgen entry | PLT-01 |
| `apps/ui` | Shared TypeScript/React app | NFR-16 – NFR-18 |
| `shaders/` | Shared WGSL | NFR-09, PLT-04 |

Not yet placed: NFR-11 scripting API (likely a `debut-scripting` crate over `debut-engine::api`),
NFR-13 – NFR-15 security/licensing/telemetry (a `debut-licensing` crate plus policy in `session`).

## Build

```
cargo check --workspace                      # native
wasm-pack build apps/web --target web        # browser (once wasm-bindgen is enabled)
```

## Suggested order of work

1. ~~`debut-core` time math + tests; `debut-project` schema with round-trip tests (PLT-03).~~ Done.
2. ~~`debut-command` apply/invert for the TL-03 core edits; journal + crash recovery.~~ Done: `Replace`/`Shift`/`Blade`/`Join` primitives, `insert`/`overwrite`/`lift`/`extract` constructors, `History` undo/redo, JSON-lines `Journal`.
3. ~~`debut-platform-native` decoder via FFmpeg; `debut-audio` clock + `AudioOut`; `debut-engine::playback`~~ Done: `FfmpegDecoder` (video + audio, seek), lock-free audio `Clock`, `Transport` (tick/drop stats, JKL, loop, step), `FrameSource` decoder→graph bridge, `NativePlatform` with cpal `AudioOut`, file store, thread pool. Still open: audio mixing into `AudioOut`, hardware decode.
4. ~~`debut-render` pull graph with transform/blend/dissolve nodes~~ Done, CPU reference + wgpu backend with conformance test. Still open: the OCIO pipeline, masks/keys/LUTs, scopes.
5. ~~`debut-export` reusing the graph~~ Done: `FfmpegEncoder` (H.264/AAC), `export()` job, `ExportQueue`, presets. Still open: CI gate on NFR-02 – NFR-04 numbers, smart render, HDR.
6. ~~`apps/web` builds for wasm32~~ Done (wasm-bindgen API over the command log). Still open: `debut-platform-web` implementations (WebCodecs, OPFS, AudioWorklet), browser render-match test.
7. Timeline: ~~trim tools and three-point editing~~ Done. Still open: selection/patching, snapping, speed ramps, shortcuts, snapshots.
8. ~~Shells: Tauri desktop app and the shared TypeScript UI~~ Done: Tauri 2 shell with media/edit/transport IPC, React UI with viewer, transport (JKL) and timeline (ripple trim, slide, blade, extract, lift); verified end to end headless (Rust test) and visually under Xvfb. Known gap: frames cross IPC as RGBA8 per tick, fine for proxies but not full-res 4K — the viewer needs a shared GPU surface next.
9. ~~Color (transfer functions, primaries, LUTs, primary grade) and audio effects/loudness~~ Done: managed pipeline on both backends with conformance tests; per-clip effect stack with keyframes and an Inspector; audio inserts per track, BS.1770 loudness, export normalization.
10. ~~Project files~~ Done: `Workspace` with JSON-lines journal, autosave, rotating backups, crash recovery; Save/Open in the app.
11. ~~Export + mixer UI~~ Done: queue worker thread, presets, loudness normalization, per-job progress; mixer strips with inserts.
12. ~~Dissolves and scopes~~ Done: `Clip.transition_in` centred on the cut with handle checks, `Track::layer_at`, Dissolve node in `compose`; CPU waveform/vectorscope/histogram.
13. ~~Preview quality, markers, titles~~ Done: Full/Half/Quarter/Auto preview with frame-cost control; timeline and clip markers with TSV export; `ClipSource::Title` rasterized by `debut-graphics` (fontdue) into an `Image` node, cached by content hash, edited in the Inspector.
14. ~~Nested sequences~~ Done: `SourceInfo::sequence` / `SampleSource::sequence` resolve compound clips; `compose` recurses with the nested canvas scaled to the preview, `render_span` mixes nested audio through its own track inserts; depth-limited against self-nesting; "Nest" edit in the app builds the compound from a time range across all tracks.
15. ~~Masks and chroma key~~ Done: `nodes.rs` holds the shared maths (rectangle/ellipse with feather and invert; Cb/Cr-distance keyer with despill), CPU and GPU passes conform to 1e-5/1e-4; `Effect::Mask`/`Effect::ChromaKey` with keyframable geometry and knobs, `ReplaceEffect` for the non-animated options; Inspector controls.
16. ~~Multicam switching~~ Done: `add_multicam` builds a `ClipSource::Multicam` clip from two or more media (shortest angle sets the length, audio track gets one too), `switch_angle` blades at the playhead and switches the tail as one undoable group; keys 1–9 in the app.
17. ~~Captions~~ Done: `Sequence.captions` with Add/Update/Remove commands, SRT import/export in `debut-graphics::captions`, burn-in in `compose` (style and margin scale with the sequence height, so the same look at any size), Captions panel in the app.
18. ~~Open nested sequences~~ Done: the session has an active sequence; `open_sequence` rebuilds the player for it, the app shows a sequence switcher and opens a compound clip on double-click; undoing a nest while inside it falls back to the main sequence.
19. ~~Title templates~~ Done: `debut-graphics::templates` builds a title plus a keyframed Transform sized for the sequence (centred, fade, lower third sliding in on a half-width bar via `TitleStyle::min_width_px`, subtitle); template picker next to "+ Title".
20. ~~Bins~~ Done: manual bins with `AssignMedia`, smart bins by file-name rule (`SmartRule::matches` also knows path/reel/camera/audio), Add/Remove/Rename commands, `media_list` that re-probes after opening a project; Media panel with bin chips, search, per-media bin select.
21. ~~Caption styling, VTT, sidecars~~ Done: `CaptionSettings` on the sequence (burn-in toggle, top/bottom, font, size, colours) with `SetCaptionSettings`; WebVTT export and a parser that accepts both; export writes an `.srt` sidecar next to the movie on request.
22. Browser: `debut-platform-web` (WebCodecs, OPFS, AudioWorklet) so the wasm engine can decode and play.
23. Still open from the requirements: bezier masks and tracking, user-saved title templates, SCC captions, transcription, multicam switching UI, bins, opening a nested sequence in the timeline, hardware decode/encode, OpenFX/VST hosts, collaboration, GPU-surface viewer.
