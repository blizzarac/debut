import { useEffect, useState } from "react";
import type { CodecCapabilities, ExportApi, ExportPreset, ExportStatus, InterchangeFormat } from "./engine";

/** Queue exports with a delivery preset and optional loudness normalization; shows
 * progress, integrated LUFS and true peak per job (EXP-02, EXP-03, AUD-06). */
export function ExportPanel({ exporter }: { exporter: ExportApi }) {
  const [presets, setPresets] = useState<ExportPreset[]>([]);
  const [preset, setPreset] = useState("");
  const [output, setOutput] = useState("");
  const [normalize, setNormalize] = useState(true);
  const [sidecar, setSidecar] = useState(true);
  const [hardware, setHardware] = useState(true);
  const [smart, setSmart] = useState(false);
  const [caps, setCaps] = useState<CodecCapabilities | null>(null);
  const [jobs, setJobs] = useState<ExportStatus[]>([]);
  const [error, setError] = useState("");
  const [note, setNote] = useState("");

  useEffect(() => {
    exporter.presets().then((p) => {
      setPresets(p);
      if (p.length && !preset) setPreset(p[0].name);
    });
  }, [exporter]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const id = setInterval(() => exporter.status().then(setJobs).catch(() => {}), 500);
    return () => clearInterval(id);
  }, [exporter]);

  useEffect(() => {
    exporter.capabilities?.().then(setCaps).catch(() => setCaps(null));
  }, [exporter]);

  const target = presets.find((p) => p.name === preset)?.loudness_lufs ?? -14;
  const hwCount = caps?.hardware_encoders.length ?? 0;
  const hdr = presets.find((p) => p.name === preset)?.hdr ?? null;

  async function start() {
    setError("");
    try {
      await exporter.start(output, preset, normalize ? target : null, sidecar, hardware && hwCount > 0, smart);
    } catch (e) {
      setError(String(e));
    }
  }

  /** EDL/OTIO go next to the movie path: same name, the format's extension. */
  async function interchange(format: InterchangeFormat) {
    setError("");
    setNote("");
    const path = output.replace(/\.[^./\\]*$/, "") + "." + format;
    try {
      await exporter.interchange!(path, format);
      setNote(`Wrote ${path}`);
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div style={{ fontSize: 12 }}>
      <div style={{ display: "flex", gap: 4, alignItems: "center", flexWrap: "wrap" }}>
        <select value={preset} onChange={(e) => setPreset(e.target.value)}>
          {presets.map((p) => (
            <option key={p.name} value={p.name}>
              {p.name}
            </option>
          ))}
        </select>
        {hdr && (
          <span title={hdr === "pq" ? "10-bit HEVC (x265), Rec.2020, PQ with HDR10 metadata for a 1000-nit P3 grade. Scene white (1.0) is 100 nits." : "10-bit HEVC (x265), Rec.2020, hybrid log-gamma"} style={{ color: "#a60" }}>
            HDR {hdr.toUpperCase()}
          </span>
        )}
        <label>
          <input type="checkbox" checked={normalize} onChange={(e) => setNormalize(e.target.checked)} /> normalize to {target} LUFS
        </label>
        <label title={hwCount ? `Use ${caps!.hardware_encoders.map((e) => `${e.name} (${e.api})`).join(", ")}; falls back to software if it cannot open` : "No hardware encoder opens on this machine; software H.264 is used"}>
          <input type="checkbox" checked={hardware && hwCount > 0} disabled={hwCount === 0} onChange={(e) => setHardware(e.target.checked)} /> hardware encoder{hwCount ? ` (${hwCount})` : " (none)"}
        </label>
        <label title="Copy stretches that show one untouched source frame for frame, in the source's codec, instead of re-encoding them. ProRes/DNx/MJPEG sources mix copied and rendered stretches (.mov); long-GOP sources (H.264) need every stretch copied, cut on keyframes. Otherwise the export renders normally and says why.">
          <input type="checkbox" checked={smart} onChange={(e) => setSmart(e.target.checked)} /> smart render
        </label>
        <label title="Also write the captions as an .srt next to the movie">
          <input type="checkbox" checked={sidecar} onChange={(e) => setSidecar(e.target.checked)} /> .srt sidecar
        </label>
        <input value={output} onChange={(e) => setOutput(e.target.value)} placeholder="/path/to/output.mp4" style={{ flex: 1, minWidth: 160 }} />
        <button onClick={start} disabled={!output || !preset}>
          Export
        </button>
        {exporter.interchange && (
          <>
            <button onClick={() => interchange("edl")} disabled={!output} title="CMX3600 EDL of the first video and audio track">
              EDL
            </button>
            <button onClick={() => interchange("otio")} disabled={!output} title="OpenTimelineIO of the whole sequence">
              OTIO
            </button>
            <button onClick={() => interchange("fcpxml")} disabled={!output} title="Final Cut Pro XML of the whole sequence">
              FCPXML
            </button>
            <button onClick={() => interchange("aaf")} disabled={!output} title="AAF of the whole sequence, for Avid Media Composer, Pro Tools and Resolve">
              AAF
            </button>
          </>
        )}
      </div>
      {error && <p style={{ color: "#c33" }}>{error}</p>}
      {note && <p style={{ color: "#666" }}>{note}</p>}
      <ul style={{ listStyle: "none", padding: 0, margin: "6px 0 0" }}>
        {jobs.map((j) => (
          <li key={j.id} style={{ borderTop: "1px solid #eee", padding: "4px 0" }}>
            <div style={{ display: "flex", justifyContent: "space-between", gap: 6 }}>
              <span title={j.output}>
                {j.name} · {j.state}
                {j.error ? ` · ${j.error}` : ""}
              </span>
              <span>
                {j.state === "running" && <button onClick={() => exporter.pause(j.id)}>⏸</button>}
                {j.state === "paused" && <button onClick={() => exporter.resume(j.id)}>▶</button>}
                {(j.state === "running" || j.state === "paused" || j.state === "queued") && <button onClick={() => exporter.cancel(j.id)}>✕</button>}
              </span>
            </div>
            <progress value={j.frames_done} max={Math.max(1, j.frames_total)} style={{ width: "100%" }} />
            <div style={{ color: "#666" }}>
              {j.frames_done}/{j.frames_total} frames
              {j.loudness_lufs !== null ? ` · ${j.loudness_lufs.toFixed(1)} LUFS · ${j.true_peak_db.toFixed(1)} dBTP` : ""}
              {j.frames_copied ? ` · ${j.frames_copied} copied` : ""}
              {j.max_cll != null ? ` · MaxCLL ${Math.round(j.max_cll)} · MaxFALL ${Math.round(j.max_fall ?? 0)} nits` : ""}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
