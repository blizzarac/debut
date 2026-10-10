import { useEffect, useState } from "react";
import type { AiApi, AiStatus, CaptionInfo, CaptionPosition, CaptionSettings, CaptionsApi } from "./engine";
import type { Selection } from "./Timeline";
import { timecode } from "./Transport";

/** Caption list (GFX-05, GFX-06): edit text and times, jump, delete, SRT in/out.
 * Captions burn into the viewer and the export while they are active. */
// Per-viewer memory of the AI opt-in and the engine paths.
const remember = (key: string, value?: string) => {
  try {
    if (value === undefined) return localStorage.getItem(key) ?? "";
    localStorage.setItem(key, value);
  } catch {
    // storage unavailable: the choice lasts for this session only
  }
  return value ?? "";
};

export function Captions({ captions, ai, selected, list, fps, position, onSeek, onChanged }: { captions: CaptionsApi; ai?: AiApi; selected?: Selection; list: CaptionInfo[]; fps: number; position: number; onSeek: (t: number) => void; onChanged: () => void }) {
  const [path, setPath] = useState("");
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<{ id: string; start: number; text: string }[]>([]);
  const [aiStatus, setAiStatus] = useState<AiStatus | null>(null);
  const [engine, setEngine] = useState(() => remember("debut.ai.engine"));
  const [model, setModel] = useState(() => remember("debut.ai.model"));
  const [language, setLanguage] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!ai) return;
    // Re-apply this viewer's opt-in and engine choice.
    (async () => {
      if (engine && model) await ai.configure(engine, model).catch(() => {});
      const s = remember("debut.ai.enabled") === "1" ? await ai.setEnabled(true) : await ai.status();
      setAiStatus(s);
    })().catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per engine
  }, [ai]);
  useEffect(() => {
    if (!captions.search || !query.trim()) return setHits([]);
    captions.search(query).then(setHits).catch(() => setHits([]));
  }, [captions, query, list]);
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
      {captions.search && (
        <div style={{ marginTop: 6 }}>
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="search captions / transcript" style={{ width: "100%", boxSizing: "border-box" }} />
          {hits.map((h) => (
            <button key={h.id} onClick={() => onSeek(h.start)} style={{ display: "block", width: "100%", textAlign: "left", marginTop: 2 }} title="Go there">
              <code>{timecode(h.start, fps)}</code> {h.text}
            </button>
          ))}
        </div>
      )}
      {ai && aiStatus && (
        <div style={{ marginTop: 8, paddingTop: 6, borderTop: "1px solid #eee" }}>
          <label title="Transcription runs on this machine with a local model; no audio is uploaded">
            <input
              type="checkbox"
              checked={aiStatus.enabled}
              onChange={(e) => {
                remember("debut.ai.enabled", e.target.checked ? "1" : "0");
                ai.setEnabled(e.target.checked).then(setAiStatus).catch((err) => setStatus(String(err)));
              }}
            />{" "}
            on-device AI (audio stays on this machine)
          </label>
          {aiStatus.enabled && (
            <>
              <div style={{ color: "#666", margin: "2px 0" }}>{aiStatus.available ? aiStatus.backend : "No local transcription engine: give the whisper.cpp tool and a ggml model file."}</div>
              {!aiStatus.available && (
                <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                  <input value={engine} onChange={(e) => setEngine(e.target.value)} placeholder="/path/to/whisper-cli" style={{ flex: 1, minWidth: 90 }} />
                  <input value={model} onChange={(e) => setModel(e.target.value)} placeholder="/path/to/ggml-base.en.bin" style={{ flex: 1, minWidth: 90 }} />
                  <button
                    disabled={!engine || !model}
                    onClick={() =>
                      ai
                        .configure(engine, model)
                        .then((s) => {
                          remember("debut.ai.engine", engine);
                          remember("debut.ai.model", model);
                          setAiStatus(s);
                        })
                        .catch((e) => setStatus(String(e)))
                    }
                  >
                    Use
                  </button>
                </div>
              )}
              {aiStatus.available && (
                <div style={{ display: "flex", gap: 4, alignItems: "center", marginTop: 2 }}>
                  <button
                    disabled={!selected || busy}
                    title={selected ? "Transcribe the selected clip's audio into captions over it" : "Select a clip first"}
                    onClick={() => {
                      if (!selected) return;
                      setBusy(true);
                      setStatus("transcribing…");
                      ai
                        .transcribeClip(selected.track, selected.clip, language || null)
                        .then((r) => {
                          setStatus(`${r.captions_added} captions from ${r.segments} segments`);
                          onChanged();
                        })
                        .catch((e) => setStatus(String(e)))
                        .finally(() => setBusy(false));
                    }}
                  >
                    Transcribe clip
                  </button>
                  <select value={language} onChange={(e) => setLanguage(e.target.value)} title="Spoken language">
                    <option value="">detect language</option>
                    {["en", "de", "fr", "es", "it", "pt", "nl", "ja", "zh"].map((l) => (
                      <option key={l} value={l}>
                        {l}
                      </option>
                    ))}
                  </select>
                </div>
              )}
            </>
          )}
        </div>
      )}
      {status && <p style={{ color: "#666", margin: "4px 0" }}>{status}</p>}
    </div>
  );
}
