import { useCallback, useEffect, useState } from "react";
import type { EffectInfo, EffectsApi, MediaApi, ParamName, TextAlign, TitleInfo } from "./engine";
import type { Selection } from "./Timeline";

const RANGES: Record<ParamName, [number, number, number]> = {
  scale: [0, 4, 0.01],
  rotation: [-180, 180, 0.5],
  x: [-2000, 2000, 1],
  y: [-2000, 2000, 1],
  opacity: [0, 1, 0.01],
  exposure: [-4, 4, 0.05],
  contrast: [0, 3, 0.01],
  saturation: [0, 3, 0.01],
  temperature: [-1, 1, 0.01],
  tint: [-1, 1, 0.01],
};

/** Effect stack of the selected clip. Sliders set constants, or keyframes at
 * the playhead when the key toggle is on (FX-01). */
export function Inspector({ effects, selected, position, onChanged }: { effects: EffectsApi; selected: Selection; position: number; onChanged: () => void }) {
  const [stack, setStack] = useState<EffectInfo[]>([]);
  const [keyframe, setKeyframe] = useState(false);

  const reload = useCallback(async () => {
    if (!selected) return setStack([]);
    setStack(await effects.clipEffects(selected.track, selected.clip).catch(() => []));
  }, [effects, selected]);

  useEffect(() => {
    reload();
  }, [reload, position]);

  if (!selected) return <p style={{ fontSize: 12, color: "#999" }}>Select a clip to edit its effects.</p>;

  const act = (p: Promise<void>) => p.then(reload).then(onChanged).catch((e) => console.warn("effect rejected", e));

  return (
    <div style={{ fontSize: 12 }}>
      <div style={{ display: "flex", gap: 6, alignItems: "center", marginBottom: 8 }}>
        <button onClick={() => act(effects.addEffect(selected.track, selected.clip, "transform"))}>+ Transform</button>
        <button onClick={() => act(effects.addEffect(selected.track, selected.clip, "grade"))}>+ Grade</button>
        <label style={{ marginLeft: "auto" }} title="Changes add a keyframe at the playhead">
          <input type="checkbox" checked={keyframe} onChange={(e) => setKeyframe(e.target.checked)} /> key
        </label>
      </div>
      {stack.map((fx) => (
        <div key={fx.index} style={{ border: "1px solid #e5e5e5", borderRadius: 4, padding: 8, marginBottom: 8 }}>
          <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 4 }}>
            <strong>{fx.kind}</strong>
            <button onClick={() => act(effects.removeEffect(selected.track, selected.clip, fx.index))} title="Remove">
              ×
            </button>
          </div>
          {fx.params.map((p) => {
            const [min, max, step] = RANGES[p.name];
            return (
              <label key={p.name} style={{ display: "grid", gridTemplateColumns: "80px 1fr 56px", gap: 6, alignItems: "center", marginBottom: 2 }}>
                <span style={{ color: p.animated ? "#c60" : "#333" }} title={p.animated ? "animated" : "constant"}>
                  {p.animated ? "◆ " : ""}
                  {p.name}
                </span>
                <input
                  type="range"
                  min={min}
                  max={max}
                  step={step}
                  value={p.value}
                  onChange={(e) => {
                    const value = Number(e.target.value);
                    setStack((s) => s.map((f) => (f.index === fx.index ? { ...f, params: f.params.map((q) => (q.name === p.name ? { ...q, value } : q)) } : f)));
                  }}
                  onMouseUp={(e) => act(effects.setParam(selected.track, selected.clip, fx.index, p.name, Number((e.target as HTMLInputElement).value), keyframe))}
                  onKeyUp={(e) => act(effects.setParam(selected.track, selected.clip, fx.index, p.name, Number((e.target as HTMLInputElement).value), keyframe))}
                />
                <code>{p.value.toFixed(2)}</code>
              </label>
            );
          })}
        </div>
      ))}
    </div>
  );
}

type Rgba = [number, number, number, number];

function hex(c: Rgba): string {
  return "#" + c.slice(0, 3).map((v) => v.toString(16).padStart(2, "0")).join("");
}

function fromHex(h: string, alpha: number): Rgba {
  return [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16), alpha];
}

/** Text and style of the selected title clip (GFX-01, GFX-02). Edits apply on
 * blur / Apply so each change is one undoable command. */
export function TitleEditor({ media, selected, title, onChanged }: { media: MediaApi; selected: Selection; title: TitleInfo; onChanged: () => void }) {
  const [draft, setDraft] = useState<TitleInfo>(title);
  useEffect(() => setDraft(title), [title]);
  if (!selected) return null;
  const style = draft.style;
  const setStyle = (patch: Partial<TitleInfo["style"]>) => setDraft({ ...draft, style: { ...style, ...patch } });
  const dirty = JSON.stringify(draft) !== JSON.stringify(title);
  const apply = () => media.setTitle(selected.track, selected.clip, draft).then(onChanged);
  const row = { display: "flex", gap: 6, alignItems: "center", fontSize: 12, marginTop: 4 } as const;
  return (
    <div style={{ marginBottom: 8 }}>
      <h3 style={{ fontSize: 12, margin: "8px 0 4px" }}>Title</h3>
      <textarea
        value={draft.text}
        rows={3}
        style={{ width: "100%", boxSizing: "border-box", fontSize: 12 }}
        onChange={(e) => setDraft({ ...draft, text: e.target.value })}
      />
      <div style={row}>
        <label>
          size <input type="number" min={4} max={600} step={1} value={style.size_px} style={{ width: 56 }} onChange={(e) => setStyle({ size_px: Number(e.target.value) })} />
        </label>
        <label>
          color <input type="color" value={hex(style.color)} onChange={(e) => setStyle({ color: fromHex(e.target.value, style.color[3]) })} />
        </label>
        <select value={style.align} onChange={(e) => setStyle({ align: e.target.value as TextAlign })}>
          <option value="left">left</option>
          <option value="center">centre</option>
          <option value="right">right</option>
        </select>
      </div>
      <div style={row}>
        <label>
          stroke <input type="number" min={0} max={40} step={0.5} value={style.stroke_px} style={{ width: 48 }} onChange={(e) => setStyle({ stroke_px: Number(e.target.value) })} />
        </label>
        <input type="color" value={hex(style.stroke_color)} onChange={(e) => setStyle({ stroke_color: fromHex(e.target.value, 255) })} />
        <label>
          shadow <input type="number" min={0} max={40} step={0.5} value={style.shadow_px} style={{ width: 48 }} onChange={(e) => setStyle({ shadow_px: Number(e.target.value) })} />
        </label>
      </div>
      <div style={row}>
        <label>
          box <input type="color" value={hex(style.background)} onChange={(e) => setStyle({ background: fromHex(e.target.value, Math.max(style.background[3], 160)) })} />
        </label>
        <label>
          <input type="checkbox" checked={style.background[3] > 0} onChange={(e) => setStyle({ background: [style.background[0], style.background[1], style.background[2], e.target.checked ? 160 : 0] })} /> on
        </label>
        <label>
          font <input value={style.font} style={{ width: 90 }} onChange={(e) => setStyle({ font: e.target.value })} />
        </label>
      </div>
      <div style={row}>
        <button disabled={!dirty} onClick={apply}>Apply</button>
        <button disabled={!dirty} onClick={() => setDraft(title)}>Revert</button>
      </div>
    </div>
  );
}
