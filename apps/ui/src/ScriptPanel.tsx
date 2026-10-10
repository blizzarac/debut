import { useEffect, useState } from "react";
import type { ScriptApi, ScriptOutput } from "./engine";

const STORE = "debut.script";

const EXAMPLE = `// Rhai. Ctrl+Enter runs; the whole run is one undo step.
let seq = sequence();
let v1 = seq.tracks[0].id;
for clip in seq.tracks[0].clips {
  add_marker(clip.timeline_in, "cut");
}
print(\`\${seq.tracks[0].clips.len()} clips marked\`);
`;

function load(): string {
  try {
    return localStorage.getItem(STORE) ?? EXAMPLE;
  } catch {
    return EXAMPLE;
  }
}

/** Script console (NFR-11): run a Rhai script against the session. */
export function ScriptPanel({ script, onRan }: { script: ScriptApi; onRan: () => void }) {
  const [source, setSource] = useState(load);
  const [out, setOut] = useState<ScriptOutput | null>(null);
  const [running, setRunning] = useState(false);
  const [api, setApi] = useState<[string, string][]>([]);

  useEffect(() => {
    script.api().then(setApi).catch(() => {});
  }, [script]);

  useEffect(() => {
    try {
      localStorage.setItem(STORE, source);
    } catch {
      /* per-viewer convenience only */
    }
  }, [source]);

  async function run() {
    setRunning(true);
    try {
      setOut(await script.run(source));
      onRan();
    } catch (e) {
      setOut({ log: [], value: null, error: String(e), edits: 0 });
    } finally {
      setRunning(false);
    }
  }

  return (
    <div style={{ fontSize: 12 }}>
      <textarea
        value={source}
        spellCheck={false}
        rows={8}
        onChange={(e) => setSource(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation(); // keep editor shortcuts out of the script
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            run();
          }
        }}
        style={{ width: "100%", boxSizing: "border-box", fontFamily: "ui-monospace, monospace", fontSize: 11 }}
      />
      <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
        <button onClick={run} disabled={running || !source.trim()} title="Run (Ctrl+Enter)">
          {running ? "Running…" : "Run"}
        </button>
        <button onClick={() => setSource(EXAMPLE)} title="Replace the script with the example">
          Example
        </button>
        {out && !out.error && <span style={{ color: "#666" }}>{out.edits ? `${out.edits} edit${out.edits === 1 ? "" : "s"} · one undo step` : "no edits"}</span>}
      </div>
      {out && (
        <pre style={{ background: "#f6f6f6", padding: 6, margin: "4px 0", whiteSpace: "pre-wrap", maxHeight: 160, overflowY: "auto" }}>
          {out.log.join("\n")}
          {out.value !== null ? `${out.log.length ? "\n" : ""}= ${out.value}` : ""}
          {out.error ? <span style={{ color: "#c33" }}>{`${out.log.length || out.value ? "\n" : ""}${out.error}\n(edits undone)`}</span> : ""}
          {!out.log.length && out.value === null && !out.error ? "done" : ""}
        </pre>
      )}
      <details>
        <summary style={{ cursor: "pointer", color: "#666" }}>Functions</summary>
        <ul style={{ paddingLeft: 16, margin: "4px 0" }}>
          {api.map(([sig, doc]) => (
            <li key={sig}>
              <code>{sig}</code> — {doc}
            </li>
          ))}
        </ul>
      </details>
    </div>
  );
}
