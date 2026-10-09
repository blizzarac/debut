import { useState } from "react";
import type { MarkerInfo, MarkersApi } from "./engine";
import { timecode } from "./Transport";

const COLORS: [number, number, number][] = [
  [59, 130, 246],
  [239, 68, 68],
  [34, 197, 94],
  [234, 179, 8],
  [168, 85, 247],
];

/** Marker list for the sequence (TL-10): edit notes and colours, jump, delete, export. */
export function Markers({ markers, list, fps, onSeek, onChanged }: { markers: MarkersApi; list: MarkerInfo[]; fps: number; onSeek: (t: number) => void; onChanged: () => void }) {
  const [path, setPath] = useState("");
  const [status, setStatus] = useState("");
  const act = (p: Promise<unknown>) => p.then(onChanged).catch((e) => setStatus(String(e)));
  return (
    <div style={{ fontSize: 12 }}>
      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {list.map((m) => (
          <li key={m.id} style={{ display: "grid", gridTemplateColumns: "12px 1fr auto", gap: 6, alignItems: "center", padding: "3px 0", borderBottom: "1px solid #eee" }}>
            <button
              title="Cycle colour"
              style={{ width: 12, height: 12, padding: 0, border: "none", borderRadius: 6, background: `rgb(${m.color.join(",")})` }}
              onClick={() => {
                const i = COLORS.findIndex((c) => c.join() === m.color.join());
                act(markers.update(m.id, { color: COLORS[(i + 1) % COLORS.length] }));
              }}
            />
            <input
              defaultValue={m.note}
              placeholder={m.clip ? "clip marker" : "marker"}
              onBlur={(e) => e.target.value !== m.note && act(markers.update(m.id, { note: e.target.value }))}
              onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
              style={{ minWidth: 0 }}
            />
            <span style={{ whiteSpace: "nowrap" }}>
              <button onClick={() => onSeek(m.at)} title="Go to marker">
                <code>{timecode(m.at, fps)}</code>
              </button>
              <button onClick={() => act(markers.remove(m.id))} title="Delete">
                ×
              </button>
            </span>
          </li>
        ))}
      </ul>
      {list.length === 0 && <p style={{ color: "#999", margin: "4px 0" }}>Press M to add a marker at the playhead.</p>}
      <div style={{ display: "flex", gap: 4, marginTop: 6 }}>
        <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/path/to/markers.tsv" style={{ flex: 1 }} />
        <button disabled={!path || list.length === 0} onClick={() => markers.exportList(path).then((n) => setStatus(`wrote ${n} markers`)).catch((e) => setStatus(String(e)))}>
          Export list
        </button>
      </div>
      {status && <p style={{ color: "#666", margin: "4px 0" }}>{status}</p>}
    </div>
  );
}
