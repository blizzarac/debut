import { useCallback, useEffect, useState } from "react";
import type { EffectInfo, EffectsApi, ParamName } from "./engine";
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
