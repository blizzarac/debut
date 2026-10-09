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
          debut-engine               api::Session (the command surface), playback, session
   │  │  │  │  │  │  │  │
   project  command  media  timeline  audio  render  graphics  export  collab
   │                                                                   │
          debut-platform             six traits; no feature logic
   │                 │
platform-native   platform-web       FFmpeg/CoreAudio/OpenFX   WebCodecs/OPFS/AudioWorklet
```

Rules enforced by the crate graph:

- `debut-core` and `debut-platform` have no OS, GPU or codec dependencies and compile on every target (PLT-01).
- Engine crates depend on `debut-platform` traits only, never on an implementation crate (PLT-02). They reach files, threads, time and fonts only through `Platform`.
- The project is mutated only through `debut-command`, so every edit is undoable, journaled and syncable (TL-11, NFR-05, COL).
- Playback and export run the same `debut-render` graph; the audio clock in `debut-audio` is the master (PB-02).

These are checked on every `cargo test` and `cargo clippy` run, not just written down:

- **Layering** (`crates/debut-arch`): reads `cargo metadata` and fails when a crate depends on a higher layer, an engine-side crate depends on a platform implementation, a shell uses a feature crate instead of `debut-engine` or the other target's platform, the foundation picks up anything beyond serde/thiserror, or an OS / codec / GPU / UI-runtime crate (FFmpeg, cpal, Tauri, wasm-bindgen, wgpu, fontdue) appears outside its one home. A new crate fails until it is placed in a layer in `debut_arch::layer_of`. A second test proves each rule catches its violation.
- **No direct OS access** (`clippy.toml`): engine-side crates may not call `std::fs`, `std::path::Path::exists`/`is_file`/`is_dir`, spawn threads or read `Instant`/`SystemTime`; use `Platform::file_store`, `spawn` and `now`. The platform implementations and shells opt out with their own `clippy.toml`; test code that needs the OS says so with `#[allow(clippy::disallowed_methods, clippy::disallowed_types)]`. Clippy caches results, so after editing `clippy.toml` touch a source file (CI starts clean).

## Code layout

Where new code goes, so the shells stay thin:

- **Editing operations and their DTOs** go in `debut-engine/src/api/`, one module per area (`media`, `timeline`, `titles`, `multicam`, `playback`, `effects`, `mixer`, `export`, `markers`, `captions`), each adding methods to `Session`. Shared helpers and the session lifecycle live in `api/mod.rs`.
- **Anything OS-, codec- or device-specific** goes behind `debut_platform::Platform` (decoders, encoders, audio out, files, hardware queries). `Session` holds an `Arc<dyn Platform>`; `NativePlatform` is the desktop implementation.
- **Shells only translate.** `apps/desktop/src/ipc.rs` is one-line Tauri command forwards plus binary frame responses; it should not grow logic. The web shell will expose the same `Session` over wasm-bindgen.
- **Pure algorithms** (masks, tracking, loudness, sync, SRT, templates) live in the domain crates with their own unit tests; `Session` only wires them to the project and the player.
- **Session tests** are in `debut-engine/src/api/tests.rs`: one focused test per area over the native platform and the FFmpeg fixture, each starting from the shared `fixture()` project.

## Crate → requirement map

| Crate | Owns | Requirement IDs |
| --- | --- | --- |
| `debut-core` | Rational time, timecode, frame rates, IDs, color-space tags, errors | MED-03, MED-04, AUD-01 |
| `debut-platform` | Object-safe `Platform` (decoders, encoders, audio out, files, hardware queries); Decoder/Encoder, FileStore, AudioOut, Threads, PluginHost, Display traits; `Capabilities` | PLT-01, PLT-02, PLT-05 |
| `debut-project` | Bins, smart bins, media refs + metadata + proxies, sequences, tracks, clips, markers, schema migration | MED-04 – MED-07, MED-09, MED-10, TL-01, TL-02, TL-07, TL-10, NFR-06, PLT-03 |
| `debut-command` | Command enum, history (undo/redo), journal | TL-11, NFR-05, COL-05, COL-06 |
| `debut-media` | Ingest, proxy queue, relink, multicam sync, VFR conform, FCPXML/EDL/AAF/OTIO | MED-01 – MED-03, MED-05, MED-06, MED-11 – MED-13 |
| `debut-timeline` | Edit/trim/select/snap operations, speed, evaluation plan, shortcuts, snapshots | TL-03 – TL-06, TL-08, TL-09, TL-12 – TL-16, NFR-02 |
| `debut-audio` | RT graph, clock, mixer/buses, built-in FX, loudness, waveforms, ducking | PB-02, AUD-02 – AUD-08, AUD-10, AUD-11 |
| `debut-render` | wgpu pull graph, nodes, keyframes, OCIO color, tracking, cache, scopes, OpenFX bridge | FX-01 – FX-15, PB-01, PB-03, PB-06, PB-08, PB-10, PLT-04 |
| `debut-graphics` | Text, templates, shapes, transcription, captions, on-device AI host | GFX-01 – GFX-11, FX-14, AUD-07, TL-13 |
| `debut-export` | Presets, queue, jobs/batches/stems, smart render, HDR metadata, upload | EXP-01 – EXP-10, NFR-04 |
| `debut-collab` | Op sync, locks, presence, review comments, roles | COL-01 – COL-08 |
| `debut-engine` | `api::Session` command surface used by both shells, transport/playback, workspace (autosave, recovery) | PB-04, PB-05, PB-07, PB-11, MED-09, NFR-05, NFR-18 |
| `debut-platform-native` | FFmpeg + VideoToolbox/NVDEC/QSV/AMF, native FS, CoreAudio/WASAPI, native threads, OpenFX/VST3/AU out-of-process, SDI/HDMI | NFR-07 – NFR-09, NFR-12, PB-09, AUD-09 |
| `debut-platform-web` | WebCodecs + WASM fallback, OPFS, AudioWorklet, Web Workers, sandboxed WASM plugins, WebGPU canvas | PLT-06 – PLT-09 |
| `apps/desktop` | Tauri shell: command forwards to `Session` over `NativePlatform` | PLT-10 |
| `crates/debut-arch` | Architecture tests: the layering rules over `cargo metadata` | PLT-01, PLT-02 |
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
7. Timeline: ~~trim tools and three-point editing~~ Done. Still open: track targeting/patching, shortcuts, snapshots.
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
22. ~~Keywords and ratings~~ Done: `MediaRef.keywords` / `rating` with `SetMediaTags`; smart-bin rules on keyword and rating (`add_smart_bin` with any field); stars and keyword editing per media, rule picker for smart bins, search also matches keywords.
23. ~~Offline media and relink~~ Done: a missing or unreadable file no longer breaks loading or playback; its clips show a grey slate (`offline_frame`) and silence, the media panel flags it and offers relink (`SetMediaPath`, invertible, player reloads the file).
24. ~~Multicam sync by audio~~ Done: `debut_audio::align` (400 Hz log-RMS envelopes, normalised cross-correlation over a lag window, confidence score); `ClipSource::Multicam.offsets` per angle applied by `Clip::media_at` in both renderer and mixer; "Sync by audio" in the media panel reports offsets and weak matches.
25. ~~User-saved title templates~~ Done: `Project.title_templates` (`SavedTitleTemplate`: style + effect stack) with Add/Remove commands; saved templates appear in the picker as `saved:<id>`, "Save as template" in the Inspector's title editor.
26. ~~Polygon masks~~ Done: `PolyMask` node (even-odd inside test, signed distance to the outline, feather, invert) on CPU and GPU (`shaders/polymask.wgsl`, up to 32 vertices in a uniform), conformant to 1e-4; `MaskShape::Polygon` with `MaskFx.points` in sequence pixels, moved by mask_x/mask_y; vertex editing in the Inspector.
27. ~~Point tracking~~ Done: `debut_render::Tracker` (luma patch, normalised cross-correlation over a search window, parabolic sub-pixel refinement, match score); `track_mask` keyframes a mask's position from the playhead until the match drops below 0.5; "Track 5 s" on mask effects.
28. ~~Hardware codecs~~ Done: `hardware_encoders()` probes NVENC / VideoToolbox / Quick Sync / AMF encoders by actually opening them, `hardware_decoders()` lists what FFmpeg carries; `EncodeSettings.encoder` with per-family rate control and software H.264 fallback (`used_fallback`); export picks a hardware encoder for the preset's codec on request and notes a fallback in the job name; capability-aware checkbox in the export panel. VAAPI waits on hardware frame upload.
29. ~~Waveforms~~ Done: `debut_audio::Peaks` (min/max per 256 samples plus halving levels, exact segment-tree range queries), built per media on a background job via `Platform::spawn`, `Session::waveform` range query, drawn on a decibel scale inside audio clips.
30. ~~Snapping and gap removal~~ Done: `debut_timeline::snap_targets` (start, playhead, clip edges, markers; the dragged clip excluded) and `close_gaps`; the timeline snaps dragged edges within 8 px with a snap line, S toggles, "Close gaps" ripples a track together.
31. ~~EDL and OpenTimelineIO export~~ Done: `debut_media::interchange` writes CMX3600 (first video and audio track, dissolves as C/D pairs, reel from metadata or file name, titles and nests as comments) and OTIO JSON (all tracks, gaps, dissolves, titles as generator references, nested stacks, markers, offline media as missing references); "EDL" / "OTIO" buttons in the export panel. Still open: FCPXML, AAF, import.
32. ~~Speed changes~~ Done: `Clip::speed` (0 = freeze, negative = reverse) plus an optional piecewise-linear `ramp` integrated into source time (`debut_project::retime`); `debut_timeline::set_speed` keeps the clip's material and ripples or stops at the next clip, `set_ramp` keeps the clip's length; audio resamples retimed clips (varispeed, freeze is silent); EDL `M2` lines and OTIO `LinearTimeWarp`/`FreezeFrame`; a speed control on the timeline toolbar. Still open: pitch-preserving time stretch, a graphical ramp editor, optical-flow frame blending.
33. ~~Auto-ducking~~ Done: `Track::duck` names a key track, amount, threshold and fade lengths; `debut_audio::Ducker` follows the key's post-fader level, holds 250 ms through pauses and fades in dB; `render_span` renders every track before ducking so playback and export agree; "duck under" and amount on each mixer strip. Still open: several key tracks, a gain-reduction meter.
34. ~~Proxies~~ Done: `debut_media::proxy` box-downscales to 1/2 or 1/4 and re-times to a constant-rate stream whose frame times match the original's source times; a background job encodes video-only H.264 next to the project with a `.done` marker naming the source (stale after a relink, never half-written); "use proxies" switches the player's video decoders, while audio and export stay on the originals. `Decoder::select` stops a reader of one stream from buffering the other. Still open: proxies listed in `MediaRef::proxies` from cameras, a proxy folder setting, deleting proxies.
35. ~~Linked selection~~ Done: `debut_timeline::linked` derives partners (same source, span, source in and speed on another track), so blades and undo keep them matched and a solo edit unlinks; with linked selection on (default) clip edits, speed, blades and ripple delete/lift apply to partners in one undo step and snapping ignores them; a "Linked" toggle, partner highlight and drag preview in the timeline. Still open: explicit link/unlink, track targeting and patching.
36. ~~Timecode metadata and multicam sync by timecode~~ Done: `Decoder::tags` reads timecode, reel and camera from the container and every stream (the `tmcd` track included); import stores them in `MediaMetadata` (`Timecode::parse`), the media panel shows TC and reel, EDLs pick them up; `SyncBy::Timecode` lines multicam angles up where their start timecodes overlap. Still open: timecode across midnight, jam-sync drift, LTC from audio.
37. ~~SCC captions~~ Done: `debut_graphics::scc` writes CEA-608 pop-on captions on CC1 at 29.97 DF (load timed so End Of Caption lands on the cue start, doubled control codes, 32-column word wrap into up to 4 bottom rows, accented letters from the basic set) and reads them back with a small 608 decoder; roll-up and paint-on files are refused. Caption import/export picks the format by extension. Still open: roll-up/paint-on, CC2–CC4, extended character sets, 708.
38. ~~FCPXML export~~ Done: `debut_media::fcpxml` writes FCPXML 1.10: V1 as the primary storyline with gaps, other video tracks as connected clips in lanes above and unlinked audio below (linked audio rides in its storyline clip, unlinked video is `srcEnable="video"`), Cross Dissolve transitions, Basic Titles with text styles, nested sequences as compound clips, `timeMap`s for speed, timecode-based asset starts and markers; an "FCPXML" button in the export panel. Not yet checked against Apple's DTD or an import into Final Cut. Still open: multicam as `mc-clip`, volume and effects, FCPXML import, AAF.
39. ~~Curved polygon masks~~ Done: `MaskFx::handles` gives each point Bézier in/out handles; `debut_render::flatten_outline` turns curved segments into polygon vertices (straight ones stay as they are) within a 64-vertex budget, so the CPU reference and GPU shader need no new maths; `smooth_handles` computes Catmull-Rom handles; a "smooth curve" switch on polygon masks. Still open: dragging points and handles in the viewer.
40. Browser: `debut-platform-web` (WebCodecs, OPFS, AudioWorklet) so the wasm engine can decode and play.
41. Still open from the requirements: planar/multi-point tracking, transcription (GFX-04) and the on-device AI host, VAAPI and hardware decode in the player, OpenFX/VST hosts, collaboration, GPU-surface viewer.
