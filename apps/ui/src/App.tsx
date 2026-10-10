import { useCallback, useEffect, useRef, useState } from "react";
import { chordLabel, installShortcuts, setKeymap, useShortcut } from "./shortcuts";
import { detectTarget, loadEngine, type BinInfo, type CaptionInfo, type Engine, type FileStatus, type MarkerInfo, type MediaInfo, type SequenceInfo, type SequenceListItem, type Shortcuts, type SyncBy, type Tick } from "./engine";
import { Captions } from "./Captions";
import { MediaPanel } from "./MediaPanel";
import { ExportPanel } from "./ExportPanel";
import { Inspector, TitleEditor } from "./Inspector";
import { Markers } from "./Markers";
import { MaskOverlay } from "./MaskOverlay";
import { Snapshots } from "./Snapshots";
import { Mixer } from "./Mixer";
import { Scopes } from "./Scopes";
import { Timeline, type Selection } from "./Timeline";
import { Transport } from "./Transport";
import { Viewer } from "./Viewer";

export default function App() {
  const [engine, setEngine] = useState<Engine | null>(null);
  const [version, setVersion] = useState("…");
  const [status, setStatus] = useState("starting engine");
  const [seq, setSeq] = useState<SequenceInfo | null>(null);
  const [mediaList, setMediaList] = useState<MediaInfo[]>([]);
  const [binList, setBinList] = useState<BinInfo[]>([]);
  const [tick, setTick] = useState<Tick | null>(null);
  const [canUndo, setCanUndo] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const [selected, setSelected] = useState<Selection>(null);
  const [file, setFile] = useState<FileStatus | null>(null);
  const [markerList, setMarkerList] = useState<MarkerInfo[]>([]);
  const [captionList, setCaptionList] = useState<CaptionInfo[]>([]);
  const [sequences, setSequences] = useState<SequenceListItem[]>([]);
  const [filePath, setFilePath] = useState("");
  const [shortcuts, setShortcuts] = useState<Shortcuts | null>(null);
  const [keymapId, setKeymapId] = useState(() => {
    try {
      return localStorage.getItem("debut.keymap") ?? "debut";
    } catch {
      return "debut";
    }
  });
  const [showKeys, setShowKeys] = useState(false);
  useEffect(() => installShortcuts(), []);
  useEffect(() => {
    const maps = shortcuts?.keymaps ?? [];
    setKeymap(maps.find((m) => m.id === keymapId) ?? maps[0] ?? null);
    try {
      localStorage.setItem("debut.keymap", keymapId);
    } catch {
      // Private window: the choice lasts for this session only.
    }
  }, [shortcuts, keymapId]);

  const refresh = useCallback(async (e: Engine) => {
    if (e.media) {
      setSeq(await e.media.sequence().catch(() => null));
      setSequences(await e.media.sequences().catch(() => []));
      setMediaList(await e.media.mediaList().catch(() => []));
      setBinList(await e.media.bins().catch(() => []));
    }
    if (e.fileStatus) setFile(await e.fileStatus().catch(() => null));
    if (e.markers) setMarkerList(await e.markers.list().catch(() => []));
    if (e.captions) setCaptionList(await e.captions.list().catch(() => []));
    setCanUndo(await e.canUndo());
    setRefreshKey((k) => k + 1);
  }, []);

  useEffect(() => {
    loadEngine()
      .then(async (e) => {
        setEngine(e);
        setVersion(await e.version());
        e.shortcuts?.().then(setShortcuts).catch(() => {});
        await e.newProject("Untitled");
        if (e.media) setSeq(await e.media.ensureSequence());
        if (e.fileStatus) setFile(await e.fileStatus().catch(() => null));
        setStatus("ready");
      })
      .catch((err) => setStatus(`engine failed: ${err}`));
  }, []);

  const onTick = useCallback((t: Tick) => setTick(t), []);
  const tickRef = useRef<Tick | null>(null);
  tickRef.current = tick;
  const addMarkerAtPlayhead = useCallback(() => {
    if (!engine?.markers) return;
    engine.markers.add(tickRef.current?.position ?? 0, "", null).then(() => refresh(engine)).catch((err) => setStatus(`marker failed: ${err}`));
  }, [engine, refresh]);

  const selectedRef = useRef<Selection>(null);
  selectedRef.current = selected;
  const seqRef = useRef<SequenceInfo | null>(null);
  seqRef.current = seq;
  const switchAngle = useCallback(
    (angle: number) => {
      const sel = selectedRef.current;
      const clip = sel && seqRef.current?.tracks.find((t) => t.id === sel.track)?.clips.find((c) => c.id === sel.clip);
      if (!engine?.media || !sel || !clip || clip.angles == null || angle >= clip.angles) return;
      engine.media
        .switchAngle(sel.track, sel.clip, angle, true)
        .then((id) => {
          setSelected({ track: sel.track, clip: id });
          return refresh(engine);
        })
        .catch((err) => setStatus(`switch failed: ${err}`));
    },
    [engine, refresh],
  );

  async function openSequence(id: string) {
    if (!engine?.media) return;
    try {
      await engine.media.openSequence(id);
      setSelected(null);
      await refresh(engine);
    } catch (err) {
      setStatus(`open failed: ${err}`);
    }
  }

  async function addMulticam(ids: string[], by: SyncBy) {
    if (!engine?.media || ids.length < 2) return;
    try {
      const r = await engine.media.addMulticam(tick?.position ?? 0, ids, by);
      if (by === "timecode") setStatus(`multicam by timecode: offsets ${r.offsets.map((o) => o.toFixed(2) + "s").join(", ")}`);
      if (by === "audio") {
        const low = r.confidences.filter((c) => c < 0.3).length;
        setStatus(`multicam synced: offsets ${r.offsets.map((o) => o.toFixed(2) + "s").join(", ")}${low ? ` · ${low} angle(s) with weak audio match` : ""}`);
      }
    } catch (err) {
      setStatus(`multicam failed: ${err}`);
    }
    await refresh(engine);
  }

  async function addToTimeline(m: MediaInfo) {
    if (!engine?.media || !seq) return;
    const at = tick?.position ?? 0;
    for (const track of seq.tracks) {
      if (track.kind === "video" || (track.kind === "audio" && m.has_audio)) {
        await engine.media.addClip(track.id, m.id, at).catch((err) => setStatus(`add failed: ${err}`));
      }
    }
    await refresh(engine);
  }

  const fps = seq ? seq.frame_rate[0] / seq.frame_rate[1] : 25;
  const selectedTitle = (selected && seq?.tracks.find((t) => t.id === selected.track)?.clips.find((c) => c.id === selected.clip)?.title) ?? null;

  async function save() {
    if (!engine?.saveProject) return;
    try {
      setFile(await engine.saveProject(filePath || null));
      setStatus("saved");
    } catch (err) {
      setStatus(`save failed: ${err}`);
    }
  }

  useShortcut("undo", () => engine?.undo().then(() => refresh(engine)));
  useShortcut("redo", () => engine?.redo().then(() => refresh(engine)));
  useShortcut("save", () => void save());

  async function openFile() {
    if (!engine?.openProjectFile || !filePath) return;
    try {
      const f = await engine.openProjectFile(filePath);
      setFile(f);
      setSelected(null);
      await refresh(engine);
      setStatus(f.recovered > 0 ? `opened, recovered ${f.recovered} unsaved edits` : "opened");
    } catch (err) {
      setStatus(`open failed: ${err}`);
    }
  }

  return (
    <main style={{ fontFamily: "system-ui", padding: 16, display: "grid", gridTemplateColumns: "300px 1fr", gap: 16, height: "100vh", boxSizing: "border-box", overflow: "hidden" }}>
      <aside style={{ display: "flex", flexDirection: "column", gap: 8, overflowY: "auto", minHeight: 0, paddingRight: 4 }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>debut</h1>
        <p style={{ color: "#666", fontSize: 12, margin: 0 }}>
          engine {version} · {detectTarget()} · {status}
        </p>
        <div style={{ display: "flex", gap: 6 }}>
          <button disabled={!engine || !canUndo} onClick={() => engine && engine.undo().then(() => refresh(engine))}>
            Undo
          </button>
          <button disabled={!engine} onClick={() => engine && engine.redo().then(() => refresh(engine))}>
            Redo
          </button>
          {shortcuts && (
            <>
              <select value={keymapId} onChange={(e) => setKeymapId(e.target.value)} title="Keyboard shortcut preset" style={{ fontSize: 12, width: 120, minWidth: 0 }}>
                {shortcuts.keymaps.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name} keys
                  </option>
                ))}
              </select>
              <button onClick={() => setShowKeys((v) => !v)} title="Show the shortcuts of this preset">
                ?
              </button>
            </>
          )}
        </div>
        {showKeys && shortcuts && (
          <table style={{ fontSize: 11, borderCollapse: "collapse" }}>
            <tbody>
              {shortcuts.actions
                .filter((a) => !a.id.startsWith("angle_") || a.id === "angle_1")
                .map((a) => {
                  const map = shortcuts.keymaps.find((m) => m.id === keymapId) ?? shortcuts.keymaps[0];
                  const keys = map.bindings.filter((b) => b.action === a.id).map(chordLabel).join(", ");
                  return (
                    <tr key={a.id}>
                      <td style={{ padding: "1px 6px 1px 0", color: "#555" }}>{a.id === "angle_1" ? "Multicam angle 1–9" : a.label}</td>
                      <td>
                        <code>{a.id === "angle_1" ? "1 … 9" : keys}</code>
                      </td>
                    </tr>
                  );
                })}
            </tbody>
          </table>
        )}
        {engine?.saveProject && (
          <div style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12 }}>
            <div style={{ display: "flex", gap: 4 }}>
              <input value={filePath} onChange={(e) => setFilePath(e.target.value)} placeholder={file?.path ?? "/path/to/project.debut"} style={{ flex: 1 }} />
              <button onClick={save} title="Save (autosaves every 2 min while dirty)" style={{ whiteSpace: "nowrap" }}>
                Save{file?.dirty ? " •" : ""}
              </button>
              <button onClick={openFile} disabled={!filePath}>
                Open
              </button>
            </div>
            {file?.path && (
              <span style={{ color: "#888" }} title={file.path}>
                {file.path.split("/").pop()}
                {file.dirty ? " (unsaved changes)" : ""}
              </span>
            )}
          </div>
        )}
        {engine?.media ? (
          <>
            <h2 style={{ fontSize: 13, margin: "12px 0 4px" }}>Media</h2>
            <MediaPanel media={engine.media} list={mediaList} bins={binList} onChanged={() => refresh(engine)} onInsert={addToTimeline} onMulticam={addMulticam} onStatus={setStatus} />
          </>
        ) : (
          <p style={{ fontSize: 12, color: "#999" }}>Media import and playback are not available on this target yet.</p>
        )}
        {engine?.effects && (
          <>
            <h2 style={{ fontSize: 13, margin: "12px 0 4px" }}>Inspector</h2>
            {engine.media && selectedTitle && (
              <TitleEditor media={engine.media} selected={selected} title={selectedTitle} onChanged={() => refresh(engine)} />
            )}
            <Inspector effects={engine.effects} selected={selected} position={Math.round((tick?.position ?? 0) * 25) / 25} refreshKey={refreshKey} onChanged={() => refresh(engine)} />
          </>
        )}
        {engine?.markers && engine.player && (
          <>
            <h2 style={{ fontSize: 13, margin: "12px 0 4px" }}>Markers</h2>
            <Markers markers={engine.markers} list={markerList} fps={fps} onSeek={(t) => engine.player!.transport({ kind: "seek", t })} onChanged={() => refresh(engine)} />
          </>
        )}
        {engine?.snapshots && engine.player && (
          <>
            <h2 style={{ fontSize: 13, margin: "12px 0 4px" }}>Versions</h2>
            <Snapshots api={engine.snapshots} refreshKey={refreshKey} onChanged={() => refresh(engine)} onSeek={(t) => engine.player!.transport({ kind: "seek", t })} />
          </>
        )}
        {engine?.captions && engine.player && (
          <>
            <h2 style={{ fontSize: 13, margin: "12px 0 4px" }}>Captions</h2>
            <Captions captions={engine.captions} list={captionList} fps={fps} position={tick?.position ?? 0} onSeek={(t) => engine.player!.transport({ kind: "seek", t })} onChanged={() => refresh(engine)} />
          </>
        )}
        {engine?.mixer && seq && seq.tracks.some((t) => t.kind === "audio") && (
          <>
            <h2 style={{ fontSize: 13, margin: "12px 0 4px" }}>Mixer</h2>
            <Mixer seq={seq} mixer={engine.mixer} onChanged={() => refresh(engine)} />
          </>
        )}
        {engine?.exporter && (
          <>
            <h2 style={{ fontSize: 13, margin: "12px 0 4px" }}>Export</h2>
            <ExportPanel exporter={engine.exporter} />
          </>
        )}
      </aside>
      <section style={{ display: "flex", flexDirection: "column", minWidth: 0, minHeight: 0, overflowY: "auto" }}>
        {engine?.player ? (
          <>
            <Viewer
              player={engine.player}
              onTick={onTick}
              refreshKey={refreshKey}
              overlay={
                engine.effects && seq ? (
                  <MaskOverlay effects={engine.effects} selected={selected} width={seq.width} height={seq.height} position={Math.round((tick?.position ?? 0) * 25) / 25} refreshKey={refreshKey} onChanged={() => refresh(engine)} />
                ) : null
              }
            />
            <Transport player={engine.player} tick={tick} fps={fps} onMarker={addMarkerAtPlayhead} onAngle={switchAngle} />
            <Scopes player={engine.player} frameKey={(tick?.frame ?? 0) * 1000 + refreshKey} />
          </>
        ) : null}
        {engine?.media && engine.player && seq ? (
          <>
          {sequences.length > 1 && (
            <div style={{ display: "flex", gap: 4, alignItems: "center", fontSize: 12, padding: "4px 0" }}>
              <span style={{ color: "#666" }}>sequence</span>
              {sequences.map((s) => (
                <button key={s.id} disabled={s.active} onClick={() => openSequence(s.id)} title={`${s.duration.toFixed(2)} s`} style={{ fontWeight: s.active ? 600 : 400 }}>
                  {s.name}
                </button>
              ))}
            </div>
          )}
          <Timeline
            seq={seq}
            position={tick?.position ?? 0}
            media={engine.media}
            player={engine.player}
            selected={selected}
            onSelect={setSelected}
            onEdited={() => refresh(engine)}
            onOpenNested={openSequence}
            markers={markerList}
          />
          </>
        ) : null}
      </section>
    </main>
  );
}
