import { useCallback, useEffect, useState } from "react";
import { detectTarget, loadEngine, type Engine, type MediaInfo, type SequenceInfo, type Tick } from "./engine";
import { Timeline } from "./Timeline";
import { Transport } from "./Transport";
import { Viewer } from "./Viewer";

export default function App() {
  const [engine, setEngine] = useState<Engine | null>(null);
  const [version, setVersion] = useState("…");
  const [status, setStatus] = useState("starting engine");
  const [seq, setSeq] = useState<SequenceInfo | null>(null);
  const [mediaList, setMediaList] = useState<MediaInfo[]>([]);
  const [tick, setTick] = useState<Tick | null>(null);
  const [path, setPath] = useState("");
  const [canUndo, setCanUndo] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  const refresh = useCallback(async (e: Engine) => {
    if (e.media) setSeq(await e.media.sequence().catch(() => null));
    setCanUndo(await e.canUndo());
    setRefreshKey((k) => k + 1);
  }, []);

  useEffect(() => {
    loadEngine()
      .then(async (e) => {
        setEngine(e);
        setVersion(await e.version());
        await e.newProject("Untitled");
        if (e.media) setSeq(await e.media.ensureSequence());
        setStatus("ready");
      })
      .catch((err) => setStatus(`engine failed: ${err}`));
  }, []);

  const onTick = useCallback((t: Tick) => setTick(t), []);

  async function importMedia() {
    if (!engine?.media || !path) return;
    try {
      const m = await engine.media.importMedia(path);
      setMediaList((l) => [...l, m]);
      setPath("");
      await refresh(engine);
    } catch (err) {
      setStatus(`import failed: ${err}`);
    }
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

  return (
    <main style={{ fontFamily: "system-ui", padding: 16, display: "grid", gridTemplateColumns: "260px 1fr", gap: 16, height: "100vh", boxSizing: "border-box" }}>
      <aside style={{ display: "flex", flexDirection: "column", gap: 8 }}>
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
        </div>
        {engine?.media ? (
          <>
            <h2 style={{ fontSize: 13, margin: "12px 0 4px" }}>Media</h2>
            <div style={{ display: "flex", gap: 4 }}>
              <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/path/to/clip.mp4" style={{ flex: 1 }} onKeyDown={(e) => e.key === "Enter" && importMedia()} />
              <button onClick={importMedia}>Import</button>
            </div>
            <ul style={{ listStyle: "none", padding: 0, margin: 0, fontSize: 12 }}>
              {mediaList.map((m) => (
                <li key={m.id} style={{ padding: "6px 4px", borderBottom: "1px solid #eee", display: "flex", justifyContent: "space-between", gap: 6 }}>
                  <span title={m.path}>
                    {m.path.split("/").pop()} · {m.width}×{m.height} · {m.duration.toFixed(2)}s
                  </span>
                  <button onClick={() => addToTimeline(m)} title="Insert at playhead">
                    +
                  </button>
                </li>
              ))}
            </ul>
          </>
        ) : (
          <p style={{ fontSize: 12, color: "#999" }}>Media import and playback are not available on this target yet.</p>
        )}
      </aside>
      <section style={{ display: "flex", flexDirection: "column", minWidth: 0 }}>
        {engine?.player ? (
          <>
            <Viewer player={engine.player} onTick={onTick} refreshKey={refreshKey} />
            <Transport player={engine.player} tick={tick} fps={fps} />
          </>
        ) : null}
        {engine?.media && engine.player && seq ? (
          <Timeline seq={seq} position={tick?.position ?? 0} media={engine.media} player={engine.player} onEdited={() => refresh(engine)} />
        ) : null}
      </section>
    </main>
  );
}
