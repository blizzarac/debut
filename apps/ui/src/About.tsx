import { useEffect, useState } from "react";
import type { AboutInfo, TelemetryApi, TelemetryStatus } from "./engine";

/** Version, the codec library's license, and third-party notices (NFR-14). */
export function About({ load, telemetry }: { load: () => Promise<AboutInfo>; telemetry?: TelemetryApi }) {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState("");

  async function toggle() {
    if (!open && !info) setInfo(await load());
    setOpen((v) => !v);
  }

  const notices = info?.notices ?? "";
  const shown = filter
    ? notices
        .split("\n")
        .filter((l) => l.toLowerCase().includes(filter.toLowerCase()))
        .join("\n")
    : notices;

  return (
    <>
      <button onClick={toggle} title="Version, licenses and third-party notices">
        About
      </button>
      {open && info && (
        <div style={{ position: "fixed", inset: "40px 40px", background: "white", border: "1px solid #ccc", boxShadow: "0 8px 30px rgba(0,0,0,.2)", zIndex: 50, display: "flex", flexDirection: "column", fontSize: 12, padding: 12, gap: 6 }}>
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <strong>debut {info.version}</strong>
            <button onClick={() => setOpen(false)}>Close</button>
          </div>
          <div>debut is MIT OR Apache-2.0.</div>
          <div>
            FFmpeg (linked from this system): <strong>{info.codec_license ?? "not available"}</strong>
            {info.codec_gpl && <span style={{ color: "#b45309" }}> — a GPL build: bundling it with the app makes that distribution GPL. Ship an LGPL build to keep the app permissive.</span>}
          </div>
          {info.codec_configuration && (
            <details>
              <summary style={{ cursor: "pointer" }}>FFmpeg build configuration</summary>
              <code style={{ wordBreak: "break-all" }}>{info.codec_configuration}</code>
            </details>
          )}
          {telemetry && <Telemetry api={telemetry} />}
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="filter notices (package or license)" />
          <pre style={{ flex: 1, overflow: "auto", background: "#f6f6f6", padding: 6, margin: 0, whiteSpace: "pre-wrap" }}>{shown}</pre>
        </div>
      )}
    </>
  );
}

/** Opt-in usage statistics: off by default, anonymous, local until sent. */
function Telemetry({ api }: { api: TelemetryApi }) {
  const [st, setSt] = useState<TelemetryStatus | null>(null);
  const [endpoint, setEndpoint] = useState("");
  const [note, setNote] = useState("");
  useEffect(() => {
    api
      .status()
      .then((s) => {
        setSt(s);
        setEndpoint(s.endpoint ?? "");
      })
      .catch(() => {});
  }, [api]);
  if (!st) return null;
  return (
    <details>
      <summary style={{ cursor: "pointer" }}>Usage statistics: {st.enabled ? "on" : "off"}</summary>
      <p style={{ margin: "4px 0", color: "#555" }}>
        When on, debut counts which kinds of edits and features you use, how many sessions, and crash messages (paths removed). No file names, project content or media. It stays on this computer; Send posts it to the address below, only when you press it. Turning it off erases what was collected.
      </p>
      <label>
        <input type="checkbox" checked={st.enabled} onChange={(e) => api.set(e.target.checked).then(setSt).catch((err) => setNote(String(err)))} /> collect usage statistics
      </label>
      {st.enabled && (
        <>
          <div style={{ display: "flex", gap: 4, marginTop: 4 }}>
            <input value={endpoint} onChange={(e) => setEndpoint(e.target.value)} onBlur={() => api.setEndpoint(endpoint || null).catch((err) => setNote(String(err)))} placeholder="https://… endpoint (none: nothing is sent)" style={{ flex: 1 }} />
            <button
              disabled={!endpoint}
              onClick={() =>
                api
                  .setEndpoint(endpoint)
                  .then(() => api.send())
                  .then((code) => setNote(`sent (${code})`))
                  .catch((err) => setNote(String(err)))
              }
            >
              Send
            </button>
          </div>
          {note && <div style={{ color: "#666" }}>{note}</div>}
          <pre style={{ maxHeight: 160, overflow: "auto", background: "#f6f6f6", padding: 4 }}>{st.report}</pre>
        </>
      )}
    </details>
  );
}
