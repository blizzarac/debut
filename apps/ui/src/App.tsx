import { useEffect, useState } from "react";
import { detectTarget, loadEngine, type Engine } from "./engine";

export default function App() {
  const [engine, setEngine] = useState<Engine | null>(null);
  const [version, setVersion] = useState("…");
  const [status, setStatus] = useState("starting engine");
  const [json, setJson] = useState("");
  const [canUndo, setCanUndo] = useState(false);

  useEffect(() => {
    loadEngine()
      .then(async (e) => {
        setEngine(e);
        setVersion(await e.version());
        await e.newProject("Untitled");
        setJson(await e.projectJson());
        setStatus("ready");
      })
      .catch((err) => setStatus(`engine failed: ${err}`));
  }, []);

  async function refresh(e: Engine) {
    setJson(await e.projectJson());
    setCanUndo(await e.canUndo());
  }

  return (
    <main style={{ fontFamily: "system-ui", padding: 24, maxWidth: 960 }}>
      <h1 style={{ margin: 0 }}>debut</h1>
      <p style={{ color: "#666" }}>
        engine {version} · {detectTarget()} · {status}
      </p>
      <div style={{ display: "flex", gap: 8 }}>
        <button disabled={!engine || !canUndo} onClick={() => engine && engine.undo().then(() => refresh(engine))}>
          Undo
        </button>
        <button disabled={!engine} onClick={() => engine && engine.redo().then(() => refresh(engine))}>
          Redo
        </button>
      </div>
      <h2 style={{ fontSize: 14, marginTop: 24 }}>Project</h2>
      <pre style={{ background: "#f4f4f4", padding: 12, overflow: "auto", maxHeight: 480 }}>{json}</pre>
    </main>
  );
}
