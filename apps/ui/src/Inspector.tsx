import { useCallback, useEffect, useState } from "react";
import type { EffectInfo, EffectsApi, MediaApi, ParamName, PluginsApi, ShapeFill, ShapeInfo, TextAlign, TitleInfo } from "./engine";
import { PluginParams, PluginPicker } from "./Plugins";
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
  mask_x: [-2000, 2000, 1],
  mask_y: [-2000, 2000, 1],
  mask_width: [0, 4000, 1],
  mask_height: [0, 4000, 1],
  feather: [0, 400, 1],
  tolerance: [0, 1, 0.005],
  softness: [0, 1, 0.005],
  spill: [0, 1, 0.01],
};

/** Effect stack of the selected clip. Sliders set constants, or keyframes at
 * the playhead when the key toggle is on (FX-01). */
export function Inspector({ effects, plugins, selected, position, refreshKey, onChanged }: { effects: EffectsApi; plugins?: PluginsApi; selected: Selection; position: number; refreshKey?: number; onChanged: () => void }) {
  const [trackNote, setTrackNote] = useState("");
  const [pluginError, setPluginError] = useState<string | null>(null);
  const [stack, setStack] = useState<EffectInfo[]>([]);
  const [keyframe, setKeyframe] = useState(false);

  const reload = useCallback(async () => {
    if (!selected) return setStack([]);
    setStack(await effects.clipEffects(selected.track, selected.clip).catch(() => []));
  }, [effects, selected]);

  useEffect(() => {
    reload();
  }, [reload, position, refreshKey]);

  const hasPlugin = stack.some((f) => f.kind === "plugin");
  useEffect(() => {
    if (!plugins || !hasPlugin) return setPluginError(null);
    plugins.lastError().then(setPluginError).catch(() => {});
  }, [plugins, hasPlugin, position, refreshKey, stack]);

  if (!selected) return <p style={{ fontSize: 12, color: "#999" }}>Select a clip to edit its effects.</p>;

  const act = (p: Promise<void>) => p.then(reload).then(onChanged).catch((e) => console.warn("effect rejected", e));

  return (
    <div style={{ fontSize: 12 }}>
      <div style={{ display: "flex", gap: 6, alignItems: "center", marginBottom: 8 }}>
        <button onClick={() => act(effects.addEffect(selected.track, selected.clip, "transform"))}>+ Transform</button>
        <button onClick={() => act(effects.addEffect(selected.track, selected.clip, "grade"))}>+ Grade</button>
        <button onClick={() => act(effects.addEffect(selected.track, selected.clip, "mask"))}>+ Mask</button>
        <button onClick={() => act(effects.addEffect(selected.track, selected.clip, "key"))} title="Chroma key">+ Key</button>
        {plugins && effects.addPluginEffect && (
          <PluginPicker api={plugins} kind="openfx" label="+ Plugin" onPick={(path, index) => act(effects.addPluginEffect!(selected.track, selected.clip, path, index))} />
        )}
        <label style={{ marginLeft: "auto" }} title="Changes add a keyframe at the playhead">
          <input type="checkbox" checked={keyframe} onChange={(e) => setKeyframe(e.target.checked)} /> key
        </label>
      </div>
      {stack.map((fx) => (
        <div key={fx.index} style={{ border: "1px solid #e5e5e5", borderRadius: 4, padding: 8, marginBottom: 8 }}>
          <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 4 }}>
            <strong title={fx.options.plugin?.id}>{fx.options.plugin?.name ?? fx.kind}</strong>
            <button onClick={() => act(effects.removeEffect(selected.track, selected.clip, fx.index))} title="Remove">
              ×
            </button>
          </div>
          {fx.kind === "mask" && (
            <div style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 4 }}>
              <select value={fx.options.shape ?? "rectangle"} onChange={(e) => act(effects.setOptions(selected.track, selected.clip, fx.index, { shape: e.target.value as "rectangle" | "ellipse" | "polygon" }))}>
                <option value="rectangle">rectangle</option>
                <option value="ellipse">ellipse</option>
                <option value="polygon">polygon</option>
              </select>
              <label>
                <input type="checkbox" checked={fx.options.invert ?? false} onChange={(e) => act(effects.setOptions(selected.track, selected.clip, fx.index, { invert: e.target.checked }))} /> invert
              </label>
              {effects.trackMask && (
                <button
                  title="Follow the picture under the mask from the playhead for 5 s, keyframing mask_x / mask_y"
                  onClick={() =>
                    effects
                      .trackMask!(selected.track, selected.clip, fx.index, 5)
                      .then((r) => {
                        setTrackNote(`${r.keys} keys, weakest match ${r.weakest_match.toFixed(2)}`);
                        onChanged();
                      })
                      .catch((e) => setTrackNote(String(e)))
                  }
                >
                  Track 5 s
                </button>
              )}
              {effects.trackPlanar && (
                <button
                  title="Track the surface under the mask for 5 s, perspective included; the outline sticks to it (a rectangle becomes a polygon)"
                  onClick={() => {
                    setTrackNote("tracking plane…");
                    effects
                      .trackPlanar!(selected.track, selected.clip, fx.index, 5)
                      .then((r) => {
                        setTrackNote(`${r.keys} frames, ${Math.round(r.weakest_match * 100)}% points agreeing at worst`);
                        onChanged();
                        reload();
                      })
                      .catch((e) => setTrackNote(String(e)));
                  }}
                >
                  Track plane
                </button>
              )}
              {!!fx.options.planar_keys && effects.clearPlanar && (
                <button title="Forget the planar track" onClick={() => act(effects.clearPlanar!(selected.track, selected.clip, fx.index))}>
                  clear plane
                </button>
              )}
              {trackNote && <span style={{ color: "#666" }}>{trackNote}</span>}
            </div>
          )}
          {fx.kind === "mask" && fx.options.shape === "polygon" && (
            <label style={{ display: "block", marginBottom: 4 }} title="Vertices as x,y pairs in sequence pixels from the frame centre, one per line or separated by semicolons; mask_x / mask_y move the whole shape">
              points
              <textarea
                key={(fx.options.points ?? []).map((p) => p.join(",")).join(";")}
                defaultValue={(fx.options.points ?? []).map((p) => `${p[0]}, ${p[1]}`).join("\n")}
                rows={4}
                style={{ width: "100%", boxSizing: "border-box", fontFamily: "monospace", fontSize: 11 }}
                onBlur={(e) => {
                  const pts = e.target.value
                    .split(/[\n;]/)
                    .map((l) => l.split(",").map((v) => Number(v.trim())))
                    .filter((p) => p.length === 2 && p.every((v) => Number.isFinite(v))) as [number, number][];
                  if (pts.length >= 3) act(effects.setOptions(selected.track, selected.clip, fx.index, { points: pts }));
                }}
              />
              <span style={{ display: "inline-flex", gap: 4, alignItems: "center" }} title="Curve the outline smoothly through the points instead of joining them with straight edges">
                <input
                  type="checkbox"
                  checked={(fx.options.handles ?? []).length > 0}
                  onChange={(e) => act(effects.setOptions(selected.track, selected.clip, fx.index, { smooth: e.target.checked }))}
                />
                smooth curve
              </span>
            </label>
          )}
          {fx.kind === "key" && (
            <label style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 4 }}>
              key colour
              <input
                type="color"
                value={hex([...(fx.options.color ?? [0, 255, 0]), 255] as Rgba)}
                onChange={(e) => {
                  const c = fromHex(e.target.value, 255);
                  act(effects.setOptions(selected.track, selected.clip, fx.index, { color: [c[0], c[1], c[2]] }));
                }}
              />
            </label>
          )}
          {fx.kind === "plugin" && fx.options.plugin && (
            <>
              <PluginParams params={fx.options.plugin.params} onSet={(name, value) => act(effects.setPluginParam!(selected.track, selected.clip, fx.index, name, value, keyframe))} />
              {pluginError && <p style={{ color: "#b91c1c", margin: "4px 0 0" }}>Plugin failed, showing the clip without it: {pluginError}</p>}
              {pluginError && /not approved|changed since/.test(pluginError) && plugins?.approve && (
                <button
                  title={`Run ${fx.options.plugin.path}: plugins named by a project only run once you approve the binary (it is pinned by its SHA-256)`}
                  onClick={() => act(plugins.approve!(fx.options.plugin!.path))}
                >
                  Approve this plugin
                </button>
              )}
            </>
          )}
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
  const [templateName, setTemplateName] = useState("");
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
      <div style={row}>
        <input value={templateName} onChange={(e) => setTemplateName(e.target.value)} placeholder="template name" style={{ flex: 1, minWidth: 0 }} />
        <button
          disabled={!templateName || dirty}
          title="Save this title's style and animation as a project template (apply pending edits first)"
          onClick={() => media.saveTitleTemplate(selected.track, selected.clip, templateName).then(() => { setTemplateName(""); onChanged(); })}
        >
          Save as template
        </button>
      </div>
    </div>
  );
}

/** Geometry, fill and outline of the selected shape clip (GFX-03). Edits
 * apply with Apply so each change is one undoable command; position, scale
 * and rotation are the clip's Transform like any layer. */
export function ShapeEditor({ media, selected, shape, onChanged }: { media: MediaApi; selected: Selection; shape: ShapeInfo; onChanged: () => void }) {
  const [draft, setDraft] = useState<ShapeInfo>(shape);
  const [error, setError] = useState("");
  useEffect(() => setDraft(shape), [shape]);
  if (!selected || !media.setShape) return null;
  const set = (patch: Partial<ShapeInfo>) => setDraft({ ...draft, ...patch } as ShapeInfo);
  const dirty = JSON.stringify(draft) !== JSON.stringify(shape);
  const apply = () =>
    media.setShape!(selected.track, selected.clip, draft)
      .then(() => {
        setError("");
        onChanged();
      })
      .catch((e) => setError(String(e)));
  const row = { display: "flex", gap: 6, alignItems: "center", fontSize: 12, marginTop: 4, flexWrap: "wrap" } as const;
  const num = (label: string, value: number, onChange: (v: number) => void, opts: { min?: number; max?: number; step?: number; title?: string } = {}) => (
    <label title={opts.title}>
      {label} <input type="number" min={opts.min} max={opts.max} step={opts.step ?? 1} value={value} style={{ width: 56 }} onChange={(e) => onChange(Number(e.target.value))} />
    </label>
  );
  const fill = draft.fill;
  const setFill = (f: ShapeFill) => set({ fill: f });
  return (
    <div style={{ marginBottom: 8 }}>
      <h3 style={{ fontSize: 12, margin: "8px 0 4px" }}>Shape · {draft.kind}</h3>
      <div style={row}>
        {num("w", draft.width, (width) => set({ width }), { min: 1, max: 16384 })}
        {draft.kind !== "line" && num("h", draft.height, (height) => set({ height }), { min: 1, max: 16384 })}
        {draft.kind === "rectangle" && num("corner", draft.corner_px, (corner_px) => set({ corner_px } as Partial<ShapeInfo>), { min: 0 })}
        {draft.kind === "polygon" && num("sides", draft.sides, (sides) => set({ sides } as Partial<ShapeInfo>), { min: 3, max: 64 })}
        {draft.kind === "star" && (
          <>
            {num("points", draft.points, (points) => set({ points } as Partial<ShapeInfo>), { min: 2, max: 64 })}
            {num("inner", draft.inner, (inner) => set({ inner } as Partial<ShapeInfo>), { min: 0.05, max: 1, step: 0.05, title: "Inner radius as a fraction of the outer" })}
          </>
        )}
        {draft.kind === "arrow" && (
          <>
            {num("head", draft.head, (head) => set({ head } as Partial<ShapeInfo>), { min: 0.05, max: 1, step: 0.05, title: "Head length as a fraction of the width" })}
            {num("shaft", draft.shaft, (shaft) => set({ shaft } as Partial<ShapeInfo>), { min: 0.05, max: 1, step: 0.05, title: "Shaft thickness as a fraction of the height" })}
          </>
        )}
      </div>
      {draft.kind !== "line" && (
        <div style={row}>
          <select
            value={fill.kind}
            onChange={(e) => {
              const base: Rgba = fill.kind === "solid" ? fill.color : fill.kind === "linear" ? fill.from : [255, 255, 255, 255];
              const k = e.target.value;
              setFill(k === "none" ? { kind: "none" } : k === "solid" ? { kind: "solid", color: base } : { kind: "linear", from: base, to: [0, 0, 0, 255], angle_deg: 0 });
            }}
          >
            <option value="solid">solid fill</option>
            <option value="linear">gradient</option>
            <option value="none">no fill</option>
          </select>
          {fill.kind === "solid" && <input type="color" value={hex(fill.color)} onChange={(e) => setFill({ ...fill, color: fromHex(e.target.value, fill.color[3]) })} />}
          {fill.kind === "linear" && (
            <>
              <input type="color" value={hex(fill.from)} onChange={(e) => setFill({ ...fill, from: fromHex(e.target.value, fill.from[3]) })} />
              <input type="color" value={hex(fill.to)} onChange={(e) => setFill({ ...fill, to: fromHex(e.target.value, fill.to[3]) })} />
              {num("angle", fill.angle_deg, (angle_deg) => setFill({ ...fill, angle_deg }), { step: 15, title: "0 = left to right, 90 = top to bottom" })}
            </>
          )}
        </div>
      )}
      <div style={row}>
        {num("stroke", draft.stroke_px, (stroke_px) => set({ stroke_px }), { min: 0, max: 1000, step: 0.5 })}
        <input type="color" value={hex(draft.stroke_color)} onChange={(e) => set({ stroke_color: fromHex(e.target.value, 255) })} />
      </div>
      <div style={row}>
        <button disabled={!dirty} onClick={apply}>Apply</button>
        <button disabled={!dirty} onClick={() => setDraft(shape)}>Revert</button>
      </div>
      {error && <p style={{ color: "#c33", fontSize: 12 }}>{error}</p>}
    </div>
  );
}
