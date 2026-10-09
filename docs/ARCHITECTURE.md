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

1. `debut-core` time math + tests; `debut-project` schema with round-trip tests (PLT-03).
2. `debut-command` apply/invert for the TL-03 core edits; journal + crash recovery.
3. `debut-platform-native` decoder via FFmpeg; `debut-audio` clock + `AudioOut`; `debut-engine::playback` → PB-01/PB-02 on a single clip.
4. `debut-render` pull graph with transform/blend/dissolve nodes and the OCIO pipeline.
5. `debut-export` reusing the graph; CI gate on NFR-02 – NFR-04 numbers.
6. `debut-platform-web` + `apps/web`; browser/desktop render-match test (PLT-04).
