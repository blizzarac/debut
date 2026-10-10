import { useEffect, useRef, useState } from "react";
import type { EffectInfo, EffectsApi } from "./engine";
import type { Selection } from "./Timeline";

type Pt = [number, number];

/** Polygon masks of the selected clip drawn over the viewer (FX-04): drag a
 * point to move it, double-click the outline to add one, Alt+click a point to
 * remove it. Coordinates are sequence pixels, like the mask itself; the SVG's
 * viewBox makes them line up with the canvas at any preview size. */
export function MaskOverlay({
  effects,
  selected,
  width,
  height,
  position,
  refreshKey,
  onChanged,
}: {
  effects: EffectsApi;
  selected: Selection;
  width: number;
  height: number;
  position: number;
  refreshKey: number;
  onChanged: () => void;
}) {
  const [masks, setMasks] = useState<EffectInfo[]>([]);
  const [drag, setDrag] = useState<{ fx: number; i: number; pts: Pt[] } | null>(null);
  const svg = useRef<SVGSVGElement>(null);

  useEffect(() => {
    if (!selected) return setMasks([]);
    effects
      .clipEffects(selected.track, selected.clip)
      .then((all) => setMasks(all.filter((e) => e.kind === "mask" && e.options.shape === "polygon")))
      .catch(() => setMasks([]));
  }, [effects, selected, position, refreshKey]);

  if (!selected || masks.length === 0 || !width) return null;

  // Sequence pixels under the pointer.
  const toSeq = (e: { clientX: number; clientY: number }): Pt => {
    const r = svg.current!.getBoundingClientRect();
    return [((e.clientX - r.left) / r.width) * width, ((e.clientY - r.top) / r.height) * height];
  };
  const commit = (fx: EffectInfo, pts: Pt[]) =>
    effects
      .setOptions(selected.track, selected.clip, fx.index, { points: pts.map(([x, y]) => [Math.round(x * 10) / 10, Math.round(y * 10) / 10] as Pt) })
      .then(onChanged)
      .catch(() => {});

  return (
    <svg
      ref={svg}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      style={{ position: "absolute", inset: 0, width: "100%", height: "100%", overflow: "visible" }}
      onMouseMove={(e) => {
        if (!drag) return;
        const fx = masks.find((m) => m.index === drag.fx)!;
        const [ox, oy] = origin(fx, width, height);
        const [x, y] = toSeq(e);
        const pts = drag.pts.slice();
        pts[drag.i] = [x - ox, y - oy];
        setDrag({ ...drag, pts });
      }}
      onMouseUp={() => {
        if (!drag) return;
        const fx = masks.find((m) => m.index === drag.fx)!;
        commit(fx, drag.pts);
        setDrag(null);
      }}
      onMouseLeave={() => setDrag(null)}
    >
      {masks.map((fx) => {
        const [ox, oy] = origin(fx, width, height);
        const pts: Pt[] = drag?.fx === fx.index ? drag.pts : ((fx.options.points ?? []) as Pt[]);
        const smooth = (fx.options.handles ?? []).length > 0 && !drag;
        const abs = pts.map(([x, y]) => [x + ox, y + oy] as Pt);
        const r = Math.max(width, height) / 160;
        return (
          <g key={fx.index}>
            <path
              d={outline(abs, smooth ? (fx.options.handles as [number, number, number, number][]) : [])}
              fill="rgba(250,204,21,0.08)"
              stroke="#facc15"
              strokeWidth={r / 3}
              vectorEffect="non-scaling-stroke"
              style={{ cursor: "copy" }}
              onDoubleClick={(e) => {
                // Insert after the nearest edge's first point.
                const [x, y] = toSeq(e);
                const at = nearestEdge(abs, [x, y]);
                const next = pts.slice();
                next.splice(at + 1, 0, [x - ox, y - oy]);
                commit(fx, next);
              }}
            />
            {abs.map(([x, y], i) => (
              <circle
                key={i}
                cx={x}
                cy={y}
                r={r}
                fill="#facc15"
                stroke="#111"
                strokeWidth={r / 4}
                style={{ cursor: "move" }}
                onMouseDown={(e) => {
                  e.preventDefault();
                  if (e.altKey) {
                    if (pts.length > 3) commit(fx, pts.filter((_, j) => j !== i));
                    return;
                  }
                  setDrag({ fx: fx.index, i, pts: pts.slice() });
                }}
              >
                <title>Drag to move · Alt+click to remove · double-click the outline to add</title>
              </circle>
            ))}
          </g>
        );
      })}
    </svg>
  );
}

/** Where a mask's points are measured from: the frame centre moved by mask_x / mask_y. */
function origin(fx: EffectInfo, width: number, height: number): Pt {
  const v = (n: string) => fx.params.find((p) => p.name === n)?.value ?? 0;
  return [width / 2 + v("mask_x"), height / 2 + v("mask_y")];
}

/** SVG path of a closed outline; with handles, cubic segments like the renderer. */
function outline(pts: Pt[], handles: [number, number, number, number][]): string {
  if (pts.length === 0) return "";
  let d = `M ${pts[0][0]} ${pts[0][1]}`;
  for (let i = 0; i < pts.length; i++) {
    const a = pts[i];
    const b = pts[(i + 1) % pts.length];
    const ha = handles[i];
    const hb = handles[(i + 1) % pts.length];
    if (ha && hb) d += ` C ${a[0] + ha[2]} ${a[1] + ha[3]} ${b[0] + hb[0]} ${b[1] + hb[1]} ${b[0]} ${b[1]}`;
    else d += ` L ${b[0]} ${b[1]}`;
  }
  return d + " Z";
}

/** Index of the edge (from point i to i+1) closest to `p`. */
function nearestEdge(pts: Pt[], p: Pt): number {
  let best = 0;
  let bestD = Infinity;
  pts.forEach((a, i) => {
    const b = pts[(i + 1) % pts.length];
    const [ex, ey] = [b[0] - a[0], b[1] - a[1]];
    const t = Math.max(0, Math.min(1, ((p[0] - a[0]) * ex + (p[1] - a[1]) * ey) / Math.max(ex * ex + ey * ey, 1e-9)));
    const d = Math.hypot(a[0] + t * ex - p[0], a[1] + t * ey - p[1]);
    if (d < bestD) {
      bestD = d;
      best = i;
    }
  });
  return best;
}
