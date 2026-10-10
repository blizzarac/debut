import { useState } from "react";
import type { AboutInfo } from "./engine";

/** Version, the codec library's license, and third-party notices (NFR-14). */
export function About({ load }: { load: () => Promise<AboutInfo> }) {
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
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="filter notices (package or license)" />
          <pre style={{ flex: 1, overflow: "auto", background: "#f6f6f6", padding: 6, margin: 0, whiteSpace: "pre-wrap" }}>{shown}</pre>
        </div>
      )}
    </>
  );
}
