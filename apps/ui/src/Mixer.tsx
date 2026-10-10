import type { Duck, InsertKind, MixerApi, PluginsApi, SequenceInfo, TrackInfo } from "./engine";
import { PluginParams, PluginPicker } from "./Plugins";

const INSERTS: [InsertKind, string][] = [
  ["eq_lowcut", "Low cut"],
  ["eq_presence", "Presence EQ"],
  ["compressor", "Compressor"],
  ["limiter", "Limiter"],
  ["gate", "Gate"],
  ["de_esser", "De-esser"],
  ["reverb", "Reverb"],
];

const DUCK_DEFAULTS = { amount_db: -12, threshold_db: -40, attack_ms: 80, release_ms: 500 };

/** One strip per audio track: fader, pan, mute/solo, inserts, and ducking
 * under another track (AUD-02, AUD-05, AUD-08). */
export function Mixer({ seq, mixer, plugins, onChanged }: { seq: SequenceInfo; mixer: MixerApi; plugins?: PluginsApi; onChanged: () => void }) {
  const tracks = seq.tracks.filter((t) => t.kind === "audio");
  const act = (p: Promise<void>) => p.then(onChanged).catch((e) => console.warn("mixer rejected", e));
  const strip = (t: TrackInfo, n: number) => (
    <div key={t.id} style={{ border: "1px solid #e5e5e5", borderRadius: 4, padding: 8, minWidth: 120, flex: 1, fontSize: 12 }}>
      <strong>A{n}</strong>
      <label style={{ display: "block", marginTop: 6 }}>
        gain <code>{t.mix.gain_db.toFixed(1)} dB</code>
        <input type="range" min={-60} max={12} step={0.5} defaultValue={t.mix.gain_db} style={{ width: "100%" }} onMouseUp={(e) => act(mixer.setTrackMix(t.id, { ...t.mix, gain_db: Number((e.target as HTMLInputElement).value) }))} />
      </label>
      <label style={{ display: "block" }}>
        pan <code>{t.mix.pan.toFixed(2)}</code>
        <input type="range" min={-1} max={1} step={0.05} defaultValue={t.mix.pan} style={{ width: "100%" }} onMouseUp={(e) => act(mixer.setTrackMix(t.id, { ...t.mix, pan: Number((e.target as HTMLInputElement).value) }))} />
      </label>
      <div style={{ display: "flex", gap: 4, margin: "4px 0" }}>
        <button style={{ background: t.mix.mute ? "#fbbf24" : undefined }} onClick={() => act(mixer.setTrackMix(t.id, { ...t.mix, mute: !t.mix.mute }))}>
          M
        </button>
        <button style={{ background: t.mix.solo ? "#86efac" : undefined }} onClick={() => act(mixer.setTrackMix(t.id, { ...t.mix, solo: !t.mix.solo }))}>
          S
        </button>
        <select defaultValue="" onChange={(e) => e.target.value && act(mixer.addInsert(t.id, e.target.value as InsertKind)).then(() => (e.target.value = ""))} title="Add insert">
          <option value="">+ insert</option>
          {INSERTS.map(([k, label]) => (
            <option key={k} value={k}>
              {label}
            </option>
          ))}
        </select>
        {plugins && mixer.addPluginInsert && <PluginPicker api={plugins} kind="clap" label="+ plugin" onPick={(path, index) => act(mixer.addPluginInsert!(t.id, path, index))} />}
      </div>
      {mixer.setTrackDuck && (
        <div style={{ marginBottom: 4 }} title="Lower this track while the chosen track (e.g. dialogue) plays">
          <label>
            duck under{" "}
            <select
              value={t.duck?.key ?? ""}
              onChange={(e) => {
                const key = e.target.value;
                const duck: Duck | null = key ? { ...DUCK_DEFAULTS, ...(t.duck ?? {}), key } : null;
                act(mixer.setTrackDuck!(t.id, duck));
              }}
            >
              <option value="">—</option>
              {tracks.map((o, i) =>
                o.id === t.id ? null : (
                  <option key={o.id} value={o.id}>
                    A{i + 1}
                  </option>
                ),
              )}
            </select>
          </label>
          {t.duck && (
            <label style={{ display: "block" }}>
              by <code>{t.duck.amount_db.toFixed(0)} dB</code>
              <input
                type="range"
                min={-40}
                max={-1}
                step={1}
                defaultValue={t.duck.amount_db}
                style={{ width: "100%" }}
                onMouseUp={(e) => act(mixer.setTrackDuck!(t.id, { ...t.duck!, amount_db: Number((e.target as HTMLInputElement).value) }))}
              />
            </label>
          )}
        </div>
      )}
      <ol style={{ margin: 0, paddingLeft: 16 }}>
        {t.inserts.map((name, i) => (
          <li key={i}>
            {name}{" "}
            <button onClick={() => act(mixer.removeInsert(t.id, i))} title="Remove" style={{ fontSize: 10 }}>
              ×
            </button>
            {!!t.insert_params?.[i]?.length && mixer.setInsertParam && <PluginParams params={t.insert_params[i]} onSet={(name, value) => act(mixer.setInsertParam!(t.id, i, name, value))} />}
          </li>
        ))}
      </ol>
    </div>
  );
  return <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>{tracks.map((t, i) => strip(t, i + 1))}</div>;
}
