import { useEffect, useState } from "react";
import type { CaptionInfo, CaptionPosition, CaptionSettings, CaptionsApi } from "./engine";
import { timecode } from "./Transport";

/** Caption list (GFX-05, GFX-06): edit text and times, jump, delete, SRT in/out.
 * Captions burn into the viewer and the export while they are active. */
export function Captions({ captions, list, fps, position, onSeek, onChanged }: { captions: CaptionsApi; list: CaptionInfo[]; fps: number; position: number; onSeek: (t: number) => void; onChanged: () => void }) {
  const [path, setPath] = useState("");
  const [status, setStatus] = useState("");
  const [settings, setSettings] = useState<CaptionSettings | null>(null);
  const act = (p: Promise<unknown>) => p.then(onChanged).catch((e) => setStatus(String(e)));
  useEffect(() => {
    captions.settings().then(setSettings).catch(() => {});
  }, [captions, list]);
  const hex = (c: [number, number, number, number]) => "#" + c.slice(0, 3).map((v) => v.toString(16).padStart(2, "0")).join("");
  const fromHex = (h: string, a: number): [number, number, number, number] => [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16), a];
  const patch = (p: Partial<CaptionSettings>) => settings && act(captions.setSettings({ ...settings, ...p }));
  const num = (v: string) => (v.includes(":") ? NaN : Number(v));
  return (
    <div style={{ fontSize: 12 }}>
      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {list.map((c) => (
          <li key={c.id} style={{ display: "grid", gridTemplateColumns: "auto 1fr auto", gap: 6, alignItems: "center", padding: "3px 0", borderBottom: "1px solid #eee" }}>
            <button onClick={() => onSeek(c.start)} title={`${c.start.toFixed(2)} – ${c.end.toFixed(2)} s; click to go there`}>
              <code>{timecode(c.start, fps)}</code>
            </button>
            <input
              defaultValue={c.text}
              placeholder="caption text"
              onBlur={(e) => e.target.value !== c.text && act(captions.update(c.id, { text: e.target.value }))}
              onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
              style={{ minWidth: 0 }}
            />
            <span style={{ whiteSpace: "nowrap" }}>
              <input
                type="number"
                step={0.1}
                min={0}
                defaultValue={(c.end - c.start).toFixed(1)}
                title="duration (s)"
                style={{ width: 48 }}
                onBlur={(e) => {
                  const d = num(e.target.value);
                  if (d > 0 && Math.abs(d - (c.end - c.start)) > 1e-6) act(captions.update(c.id, { end: c.start + d }));
                }}
              />
              <button onClick={() => act(captions.remove(c.id))} title="Delete">
                ×
              </button>
            </span>
          </li>
        ))}
      </ul>
      <div style={{ display: "flex", gap: 4, marginTop: 6, flexWrap: "wrap" }}>
        <button onClick={() => act(captions.add(position, position + 2, "Caption"))} title="2 s caption at the playhead" style={{ whiteSpace: "nowrap" }}>
          + at playhead
        </button>
        <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/path/to/subs.srt" title="Captions file: .srt, .vtt (export) or .scc (CEA-608)" style={{ flex: 1, minWidth: 90 }} />
        <button disabled={!path} onClick={() => captions.importSrt(path).then((n) => { setStatus(`imported ${n} cues`); onChanged(); }).catch((e) => setStatus(String(e)))}>
          Import
        </button>
        <button disabled={!path || list.length === 0} onClick={() => captions.exportSrt(path).then((n) => setStatus(`wrote ${n} cues`)).catch((e) => setStatus(String(e)))}>
          Export
        </button>
      </div>
      {settings && (
        <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap", marginTop: 6, color: "#444" }}>
          <label title="Render captions into the picture (viewer and export)">
            <input type="checkbox" checked={settings.burn_in} onChange={(e) => patch({ burn_in: e.target.checked })} /> burn in
          </label>
          <select value={settings.position} onChange={(e) => patch({ position: e.target.value as CaptionPosition })}>
            <option value="bottom">bottom</option>
            <option value="top">top</option>
          </select>
          <label>
            size <input type="number" min={8} max={200} value={settings.size_px} style={{ width: 48 }} onChange={(e) => patch({ size_px: Number(e.target.value) })} />
          </label>
          <input type="color" value={hex(settings.color)} title="text colour" onChange={(e) => patch({ color: fromHex(e.target.value, 255) })} />
          <label title="box behind the text">
            <input type="checkbox" checked={settings.background[3] > 0} onChange={(e) => patch({ background: [settings.background[0], settings.background[1], settings.background[2], e.target.checked ? 150 : 0] })} /> box
          </label>
        </div>
      )}
      {status && <p style={{ color: "#666", margin: "4px 0" }}>{status}</p>}
    </div>
  );
}
