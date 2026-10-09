import { useRef, useState } from "react";
import type { ClipInfo, EditOp, MarkerInfo, MediaApi, PlayerApi, SequenceInfo, TrackInfo } from "./engine";

const TRACK_H = 44;
const RULER_H = 22;
const HEADER_W = 64;
const EDGE = 8;

type Drag = { track: TrackInfo; clip: ClipInfo; mode: "head" | "tail" | "body"; startX: number; delta: number };

/** Tracks as rows, clips as blocks. Click the ruler to seek; drag a clip body to
 * slide it, drag an edge to ripple-trim; buttons blade at the playhead and
 * ripple-delete the selected clip. */
export type Selection = { track: string; clip: string } | null;

export function Timeline({
  seq,
  position,
  media,
  player,
  selected,
  onSelect,
  onEdited,
  markers = [],
  onOpenNested,
}: {
  seq: SequenceInfo;
  position: number;
  media: MediaApi;
  player: PlayerApi;
  selected: Selection;
  onSelect: (s: Selection) => void;
  onEdited: () => void;
  markers?: MarkerInfo[];
  /** Double-click on a compound clip opens its nested sequence. */
  onOpenNested?: (sequence: string) => void;
}) {
  const [pxPerSec, setPxPerSec] = useState(120);
  const setSelected = onSelect;
  const [drag, setDrag] = useState<Drag | null>(null);
  const svg = useRef<SVGSVGElement>(null);
  const fps = seq.frame_rate[0] / seq.frame_rate[1];
  const length = Math.max(seq.duration + 5, 10);
  const width = HEADER_W + length * pxPerSec;
  const height = RULER_H + seq.tracks.length * TRACK_H;

  const xToTime = (clientX: number) => {
    const rect = svg.current!.getBoundingClientRect();
    return Math.max(0, (clientX - rect.left - HEADER_W) / pxPerSec);
  };
  const snap = (t: number) => Math.round(t * fps) / fps;

  const run = async (op: EditOp) => {
    try {
      await media.edit(op);
    } catch (e) {
      console.warn("edit rejected", e);
    }
    onEdited();
  };

  const onMouseMove = (e: React.MouseEvent) => {
    if (!drag) return;
    setDrag({ ...drag, delta: snap((e.clientX - drag.startX) / pxPerSec) });
  };
  const onMouseUp = () => {
    if (!drag) return;
    const { track, clip, mode, delta } = drag;
    setDrag(null);
    if (delta === 0) return;
    if (mode === "head") run({ kind: "ripple_head", track: track.id, clip: clip.id, delta });
    else if (mode === "tail") run({ kind: "ripple_tail", track: track.id, clip: clip.id, delta });
    else run({ kind: "slide", track: track.id, clip: clip.id, delta });
  };

  const selectedClip = selected && seq.tracks.find((t) => t.id === selected.track)?.clips.find((c) => c.id === selected.clip);

  return (
    <div>
      <div style={{ display: "flex", gap: 6, alignItems: "center", padding: "6px 0" }}>
        <button
          onClick={() => {
            const t = selected ? seq.tracks.find((t) => t.id === selected.track) : seq.tracks[0];
            if (t) run({ kind: "blade", track: t.id, at: snap(position) });
          }}
        >
          Blade at playhead
        </button>
        <button
          disabled={!selectedClip}
          onClick={() =>
            selectedClip && run({ kind: "extract", track: selected!.track, start: selectedClip.timeline_in, end: selectedClip.timeline_in + selectedClip.duration })
          }
        >
          Ripple delete
        </button>
        <button
          disabled={!selectedClip}
          onClick={() =>
            selectedClip && run({ kind: "lift", track: selected!.track, start: selectedClip.timeline_in, end: selectedClip.timeline_in + selectedClip.duration })
          }
        >
          Lift
        </button>
        <button
          disabled={!selectedClip}
          title="Dissolve from the previous clip into the selected one (needs a head handle)"
          onClick={() => selectedClip && run({ kind: "transition", track: selected!.track, clip: selectedClip.id, duration: selectedClip.transition_in ? null : 0.5 })}
        >
          {selectedClip?.transition_in ? "Remove dissolve" : "Dissolve in"}
        </button>
        <button
          disabled={!selectedClip}
          title="Collapse the selected clip's span on every track into a nested sequence"
          onClick={() => selectedClip && run({ kind: "nest", start: selectedClip.timeline_in, end: selectedClip.timeline_in + selectedClip.duration })}
        >
          Nest
        </button>
        <button
          title="Add a 5 s title at the playhead on a free video track"
          onClick={async () => {
            const id = await media.addTitle(snap(position), "Title").catch(() => null);
            if (id) onEdited();
          }}
        >
          + Title
        </button>
        <span style={{ flex: 1 }} />
        <label style={{ fontSize: 12 }}>
          zoom <input type="range" min={20} max={600} value={pxPerSec} onChange={(e) => setPxPerSec(Number(e.target.value))} />
        </label>
      </div>
      <div style={{ overflowX: "auto", border: "1px solid #ddd", userSelect: "none" }}>
        <svg ref={svg} width={width} height={height} onMouseMove={onMouseMove} onMouseUp={onMouseUp} onMouseLeave={onMouseUp} style={{ display: "block" }}>
          {/* ruler */}
          <rect x={HEADER_W} y={0} width={width - HEADER_W} height={RULER_H} fill="#f0f0f0" onMouseDown={(e) => player.transport({ kind: "seek", t: snap(xToTime(e.clientX)) })} />
          {Array.from({ length: Math.ceil(length) + 1 }, (_, s) => (
            <g key={s}>
              <line x1={HEADER_W + s * pxPerSec} y1={RULER_H - 8} x2={HEADER_W + s * pxPerSec} y2={RULER_H} stroke="#888" />
              <text x={HEADER_W + s * pxPerSec + 3} y={12} fontSize={10} fill="#666" pointerEvents="none">
                {s}s
              </text>
            </g>
          ))}
          {/* markers */}
          {markers.map((m) => {
            const mx = HEADER_W + m.at * pxPerSec;
            const mw = Math.max(0, m.duration * pxPerSec);
            const c = `rgb(${m.color.join(",")})`;
            return (
              <g key={m.id} onMouseDown={() => player.transport({ kind: "seek", t: m.at })} style={{ cursor: "pointer" }}>
                {mw > 0 && <rect x={mx} y={2} width={mw} height={RULER_H - 4} fill={c} opacity={0.35} />}
                <polygon points={`${mx - 5},2 ${mx + 5},2 ${mx},${RULER_H - 2}`} fill={c} />
                <title>{m.note || (m.clip ? "clip marker" : "marker")}</title>
              </g>
            );
          })}
          {/* tracks */}
          {seq.tracks.map((track, i) => {
            const y = RULER_H + i * TRACK_H;
            return (
              <g key={track.id}>
                <rect x={0} y={y} width={HEADER_W} height={TRACK_H} fill="#e8e8e8" stroke="#ccc" />
                <text x={8} y={y + TRACK_H / 2 + 4} fontSize={12} fill="#333">
                  {track.kind === "video" ? "V" : "A"}
                  {seq.tracks.filter((t) => t.kind === track.kind).indexOf(track) + 1}
                </text>
                <rect x={HEADER_W} y={y} width={width - HEADER_W} height={TRACK_H} fill={track.kind === "video" ? "#fafafa" : "#f4f8f4"} stroke="#e0e0e0" onMouseDown={() => setSelected(null)} />
                {track.clips.map((clip) => {
                  const dragging = drag && drag.clip.id === clip.id ? drag : null;
                  let tin = clip.timeline_in;
                  let dur = clip.duration;
                  if (dragging?.mode === "body") tin += dragging.delta;
                  if (dragging?.mode === "head") {
                    tin += Math.min(dragging.delta, dur - 1 / fps);
                    dur -= Math.min(dragging.delta, dur - 1 / fps);
                  }
                  if (dragging?.mode === "tail") dur = Math.max(1 / fps, dur + dragging.delta);
                  const x = HEADER_W + tin * pxPerSec;
                  const w = Math.max(2, dur * pxPerSec);
                  const sel = selected?.clip === clip.id;
                  const color = clip.title ? (sel ? "#9333ea" : "#a855f7") : clip.nested ? (sel ? "#b45309" : "#f59e0b") : track.kind === "video" ? (sel ? "#3b82f6" : "#60a5fa") : sel ? "#16a34a" : "#4ade80";
                  const start = (mode: Drag["mode"]) => (e: React.MouseEvent) => {
                    e.stopPropagation();
                    setSelected({ track: track.id, clip: clip.id });
                    setDrag({ track, clip, mode, startX: e.clientX, delta: 0 });
                  };
                  return (
                    <g key={clip.id}>
                      <rect x={x} y={y + 4} width={w} height={TRACK_H - 8} rx={3} fill={color} stroke={sel ? "#111" : "none"} onMouseDown={start("body")} onDoubleClick={() => clip.nested && onOpenNested?.(clip.nested)} style={{ cursor: "grab" }} />
                      <rect x={x} y={y + 4} width={EDGE} height={TRACK_H - 8} fill="rgba(0,0,0,0.15)" onMouseDown={start("head")} style={{ cursor: "ew-resize" }} />
                      <rect x={x + w - EDGE} y={y + 4} width={EDGE} height={TRACK_H - 8} fill="rgba(0,0,0,0.15)" onMouseDown={start("tail")} style={{ cursor: "ew-resize" }} />
                      {clip.transition_in && (
                        <polygon
                          points={`${x - (clip.transition_in / 2) * pxPerSec},${y + TRACK_H - 4} ${x + (clip.transition_in / 2) * pxPerSec},${y + 4} ${x + (clip.transition_in / 2) * pxPerSec},${y + TRACK_H - 4}`}
                          fill="rgba(255,255,255,0.55)"
                          pointerEvents="none"
                        />
                      )}
                      <text x={x + EDGE + 2} y={y + TRACK_H / 2 + 4} fontSize={11} fill="#fff" pointerEvents="none">
                        {clip.title ? `T “${clip.title.text.slice(0, 18)}”` : clip.angles != null ? `MC ${(clip.angle ?? 0) + 1}/${clip.angles}` : clip.media ? `media ${clip.media.slice(-4)}` : "nested"} · {dur.toFixed(2)}s
                      </text>
                    </g>
                  );
                })}
              </g>
            );
          })}
          {/* playhead */}
          <line x1={HEADER_W + position * pxPerSec} y1={0} x2={HEADER_W + position * pxPerSec} y2={height} stroke="#e11" strokeWidth={2} pointerEvents="none" />
        </svg>
      </div>
    </div>
  );
}
