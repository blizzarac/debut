import { useEffect, useRef, useState } from "react";
import { useShortcut } from "./shortcuts";
import { peerColor } from "./Collab";
import type { ClipInfo, Peer, Targeting, EditOp, MarkerInfo, MediaApi, PlayerApi, SequenceInfo, ShapeInfo, ShapeKind, TitleTemplate, TrackInfo } from "./engine";

/** A fresh shape of `kind`, a quarter of the frame across (GFX-03). */
export function newShape(kind: ShapeKind["kind"], seq: { width: number; height: number }): ShapeInfo {
  const base = { width: Math.round(seq.width / 4), height: Math.round(seq.height / 4), fill: { kind: "solid" as const, color: [255, 255, 255, 255] as [number, number, number, number] }, stroke_px: 0, stroke_color: [0, 0, 0, 255] as [number, number, number, number] };
  const square = Math.round(Math.min(seq.width, seq.height) / 4);
  switch (kind) {
    case "rectangle":
      return { ...base, kind, corner_px: 0 };
    case "ellipse":
      return { ...base, kind, width: square, height: square };
    case "polygon":
      return { ...base, kind, sides: 6, width: square, height: square };
    case "star":
      return { ...base, kind, points: 5, inner: 0.45, width: square, height: square };
    case "arrow":
      return { ...base, kind, head: 0.35, shaft: 0.4 };
    case "line":
      return { ...base, kind, height: 1, stroke_px: 6, stroke_color: [255, 255, 255, 255] };
  }
}

const TRACK_H = 44;
const RULER_H = 22;
const HEADER_W = 64;
const EDGE = 8;

type Drag = {
  track: TrackInfo;
  clip: ClipInfo;
  mode: "head" | "tail" | "body";
  startX: number;
  delta: number;
  /** Snap targets in seconds, once the engine has sent them. */
  targets: number[] | null;
  /** The target the dragged edge is snapped to, if any. */
  snapAt: number | null;
};

/** How close (in pixels) an edge must come to a target to snap (TL-06). */
const SNAP_PX = 8;

/** Tracks as rows, clips as blocks. Click the ruler to seek; drag a clip body to
 * slide it, drag an edge to ripple-trim, with edges snapping to the playhead,
 * other clips and markers (S toggles); buttons blade at the playhead,
 * ripple-delete the selected clip and close gaps. */
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
  peers = [],
  locks = [],
  onLock,
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
  /** Collaborators (their playheads are drawn) and track locks (COL). */
  peers?: Peer[];
  locks?: { track: string; owner: string }[];
  /** Click a track name to lock or unlock it for yourself. */
  onLock?: (track: string, on: boolean) => void;
}) {
  const [pxPerSec, setPxPerSec] = useState(120);
  const [templates, setTemplates] = useState<TitleTemplate[]>([{ id: "title", name: "Title", description: "", saved: false }]);
  const [template, setTemplate] = useState("title");
  const [shapeKind, setShapeKind] = useState<ShapeKind["kind"]>("rectangle");
  useEffect(() => {
    // Re-read after edits so templates saved in the Inspector show up.
    media.titleTemplates().then(setTemplates).catch(() => {});
  }, [media, seq]);
  // Source patching and track targeting (TL-05).
  const [targeting, setTargeting] = useState<Targeting | null>(null);
  const loadTargeting = () => media.targeting?.().then(setTargeting).catch(() => setTargeting(null));
  useEffect(() => {
    loadTargeting();
  }, [media, seq]); // eslint-disable-line react-hooks/exhaustive-deps
  const bladeAtPlayhead = () => {
    // A selected clip: its track (and linked partners); otherwise every targeted track.
    if (!selected && media.bladeTargeted) {
      media.bladeTargeted(toFrame(position)).then(onEdited).catch(() => {});
      return;
    }
    const t = selected ? seq.tracks.find((t) => t.id === selected.track) : seq.tracks[0];
    if (t) run({ kind: "blade", track: t.id, at: toFrame(position) });
  };
  const setSelected = onSelect;
  const [drag, setDrag] = useState<Drag | null>(null);
  const [snapOn, setSnapOn] = useState(true);
  const [linkedOn, setLinkedOn] = useState(true);
  useEffect(() => {
    media.linkedSelection?.().then(setLinkedOn).catch(() => {});
  }, [media]);
  const toggleLinked = () => {
    const on = !linkedOn;
    media.setLinkedSelection?.(on).then(() => setLinkedOn(on)).catch(() => {});
  };
  useShortcut("toggle_snap", () => setSnapOn((v) => !v));
  useShortcut("toggle_linked", () => toggleLinked());
  const svg = useRef<SVGSVGElement>(null);
  const fps = seq.frame_rate[0] / seq.frame_rate[1];
  const length = Math.max(seq.duration + 5, 10);
  const width = HEADER_W + length * pxPerSec;
  const height = RULER_H + seq.tracks.length * TRACK_H;

  const xToTime = (clientX: number) => {
    const rect = svg.current!.getBoundingClientRect();
    return Math.max(0, (clientX - rect.left - HEADER_W) / pxPerSec);
  };
  const toFrame = (t: number) => Math.round(t * fps) / fps;

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
    const raw = (e.clientX - drag.startX) / pxPerSec;
    let delta = toFrame(raw);
    let snapAt: number | null = null;
    if (snapOn && drag.targets) {
      const { clip, mode } = drag;
      const edges = mode === "head" ? [clip.timeline_in] : mode === "tail" ? [clip.timeline_in + clip.duration] : [clip.timeline_in, clip.timeline_in + clip.duration];
      let best = SNAP_PX / pxPerSec;
      for (const edge of edges) {
        for (const t of drag.targets) {
          const d = Math.abs(edge + raw - t);
          if (d <= best) {
            best = d;
            delta = toFrame(t - edge);
            snapAt = t;
          }
        }
      }
    }
    setDrag({ ...drag, delta, snapAt });
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
  useShortcut("blade", bladeAtPlayhead);
  useShortcut("ripple_delete", () => {
    if (selectedClip) run({ kind: "extract", track: selected!.track, start: selectedClip.timeline_in, end: selectedClip.timeline_in + selectedClip.duration });
  });
  useShortcut("lift", () => {
    if (selectedClip) run({ kind: "lift", track: selected!.track, start: selectedClip.timeline_in, end: selectedClip.timeline_in + selectedClip.duration });
  });

  return (
    <div>
      <div style={{ display: "flex", gap: 6, alignItems: "center", padding: "6px 0" }}>
        <button onClick={bladeAtPlayhead} title="Blade the selected clip, or every targeted track (T) when nothing is selected">
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
          title="Close the gaps between clips on the selected clip's track (or V1)"
          onClick={() => {
            const t = selected ? seq.tracks.find((t) => t.id === selected.track) : seq.tracks[0];
            if (t) run({ kind: "close_gaps", track: t.id });
          }}
        >
          Close gaps
        </button>
        {selectedClip && selectedClip.media && <SpeedControl clip={selectedClip} track={selected!.track} run={run} />}
        <button
          disabled={!selectedClip}
          title="Collapse the selected clip's span on every track into a nested sequence"
          onClick={() => selectedClip && run({ kind: "nest", start: selectedClip.timeline_in, end: selectedClip.timeline_in + selectedClip.duration })}
        >
          Nest
        </button>
        <button
          title="Add a 5 s title of the chosen template at the playhead on a free video track"
          onClick={async () => {
            const id = await media.addTitle(toFrame(position), template === "lower_third" ? "Name" : "Title", template).catch(() => null);
            if (id) onEdited();
          }}
        >
          + Title
        </button>
        <select value={template} onChange={(e) => setTemplate(e.target.value)} title="Title template">
          {templates.map((t) => (
            <option key={t.id} value={t.id} title={t.description}>
              {t.saved ? "★ " : ""}
              {t.name}
            </option>
          ))}
        </select>
        {templates.find((t) => t.id === template)?.saved && (
          <button
            title="Delete this saved template"
            onClick={async () => {
              await media.removeTitleTemplate(template).catch(() => {});
              setTemplate("title");
              onEdited();
            }}
          >
            ×
          </button>
        )}
        {media.addShape && (
          <>
            <button
              title="Add a 5 s shape at the playhead on a free video track, centred"
              onClick={async () => {
                const id = await media.addShape!(toFrame(position), newShape(shapeKind, seq)).catch(() => null);
                if (id) onEdited();
              }}
            >
              + Shape
            </button>
            <select value={shapeKind} onChange={(e) => setShapeKind(e.target.value as ShapeKind["kind"])} title="Shape">
              {(["rectangle", "ellipse", "polygon", "star", "arrow", "line"] as const).map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </>
        )}
        <span style={{ flex: 1 }} />
        {media.setLinkedSelection && (
          <button onClick={toggleLinked} title="Edit a clip together with its linked picture/sound on other tracks" style={{ fontWeight: linkedOn ? 600 : 400, background: linkedOn ? "#bfdbfe" : undefined }}>
            Linked {linkedOn ? "on" : "off"}
          </button>
        )}
        <button onClick={() => setSnapOn((v) => !v)} title="Snap edges to the playhead, clip edges and markers while dragging (S)" style={{ fontWeight: snapOn ? 600 : 400, background: snapOn ? "#fde68a" : undefined }}>
          Snap {snapOn ? "on" : "off"}
        </button>
        <label style={{ fontSize: 12 }}>
          zoom <input type="range" min={20} max={600} value={pxPerSec} onChange={(e) => setPxPerSec(Number(e.target.value))} />
        </label>
      </div>
      <div style={{ overflowX: "auto", border: "1px solid #ddd", userSelect: "none" }}>
        <svg ref={svg} width={width} height={height} onMouseMove={onMouseMove} onMouseUp={onMouseUp} onMouseLeave={onMouseUp} style={{ display: "block" }}>
          {/* ruler */}
          <rect x={HEADER_W} y={0} width={width - HEADER_W} height={RULER_H} fill="#f0f0f0" onMouseDown={(e) => player.transport({ kind: "seek", t: toFrame(xToTime(e.clientX)) })} />
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
                {(() => {
                  const lock = locks.find((l) => l.track === track.id);
                  return (
                    <>
                      <rect x={0} y={y} width={HEADER_W} height={TRACK_H} fill={lock ? "#fee2e2" : "#e8e8e8"} stroke="#ccc" />
                      <text
                        x={8}
                        y={y + TRACK_H / 2 + 4}
                        fontSize={12}
                        fill="#333"
                        style={{ cursor: onLock ? "pointer" : undefined }}
                        onMouseDown={() => onLock?.(track.id, !(lock?.owner === "you"))}
                      >
                        <title>{lock ? `Locked by ${lock.owner}` : onLock ? "Click to lock this track for yourself" : ""}</title>
                        {track.kind === "video" ? "V" : "A"}
                        {seq.tracks.filter((t) => t.kind === track.kind).indexOf(track) + 1}
                        {lock ? " 🔒" : ""}
                      </text>
                    </>
                  );
                })()}
                {targeting && (
                  <>
                    {(() => {
                      const kind = track.kind === "audio" ? "audio" : "video";
                      const patched = (kind === "video" ? targeting.video_source : targeting.audio_source) === track.id;
                      const targeted = targeting.targeted.includes(track.id);
                      const chip = (x: number, label: string, on: boolean, color: string, title: string, act: () => Promise<void> | undefined) => (
                        <g style={{ cursor: "pointer" }} onMouseDown={(e) => { e.stopPropagation(); act()?.then(loadTargeting).catch(() => {}); }}>
                          <title>{title}</title>
                          <rect x={x} y={y + TRACK_H / 2 - 7} width={13} height={14} rx={2} fill={on ? color : "#fff"} stroke={color} />
                          <text x={x + 6.5} y={y + TRACK_H / 2 + 4} fontSize={9} textAnchor="middle" fill={on ? "#fff" : color}>
                            {label}
                          </text>
                        </g>
                      );
                      return (
                        <>
                          {track.kind !== "adjustment" &&
                            chip(30, "S", patched, "#ea580c", patched ? "Inserted media goes here; click to switch off" : "Send inserted media here", () => media.setSourcePatch?.(kind, patched ? null : track.id))}
                          {chip(46, "T", targeted, "#2563eb", targeted ? "Targeted: playhead edits act here" : "Not targeted", () => media.setTrackTargeted?.(track.id, !targeted))}
                        </>
                      );
                    })()}
                  </>
                )}
                <rect x={HEADER_W} y={y} width={width - HEADER_W} height={TRACK_H} fill={track.kind === "video" ? "#fafafa" : "#f4f8f4"} stroke="#e0e0e0" onMouseDown={() => setSelected(null)} />
                {track.clips.map((clip) => {
                  const dragging = drag && (drag.clip.id === clip.id || (linkedOn && partners(drag.clip, clip))) ? drag : null;
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
                  const sel = selected?.clip === clip.id || (linkedOn && !!selectedClip && partners(selectedClip, clip));
                  const color = clip.title ? (sel ? "#9333ea" : "#a855f7") : clip.shape ? (sel ? "#db2777" : "#f472b6") : clip.nested ? (sel ? "#b45309" : "#f59e0b") : track.kind === "video" ? (sel ? "#3b82f6" : "#60a5fa") : sel ? "#16a34a" : "#4ade80";
                  const start = (mode: Drag["mode"]) => (e: React.MouseEvent) => {
                    e.stopPropagation();
                    setSelected({ track: track.id, clip: clip.id });
                    setDrag({ track, clip, mode, startX: e.clientX, delta: 0, targets: null, snapAt: null });
                    media
                      .snapPoints?.(clip.id)
                      .then((pts) => setDrag((d) => (d && d.clip.id === clip.id ? { ...d, targets: pts.map((p) => p.t) } : d)))
                      .catch(() => {});
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
                      {track.kind === "audio" && clip.media && media.waveform && clip.ramp.length === 0 && clip.speed !== 0 && (
                        <ClipWaveform
                          media={media}
                          mediaId={clip.media}
                          start={Math.min(clip.source_in, clip.source_in + clip.duration * clip.speed)}
                          duration={clip.duration * Math.abs(clip.speed)}
                          x={x}
                          y={y + 4}
                          w={w}
                          h={TRACK_H - 8}
                        />
                      )}
                      <text x={x + EDGE + 2} y={y + TRACK_H / 2 + 4} fontSize={11} fill="#fff" pointerEvents="none">
                        {clip.title ? `T “${clip.title.text.slice(0, 18)}”` : clip.shape ? `◆ ${clip.shape.kind}` : clip.angles != null ? `MC ${(clip.angle ?? 0) + 1}/${clip.angles}` : clip.media ? `media ${clip.media.slice(-4)}` : "nested"} · {dur.toFixed(2)}s{speedLabel(clip)}
                      </text>
                    </g>
                  );
                })}
              </g>
            );
          })}
          {drag?.snapAt != null && (
            <line x1={HEADER_W + drag.snapAt * pxPerSec} y1={RULER_H} x2={HEADER_W + drag.snapAt * pxPerSec} y2={height} stroke="#f59e0b" strokeWidth={1.5} strokeDasharray="4 3" pointerEvents="none" />
          )}
          {/* collaborators' playheads */}
          {peers
            .filter((p) => p.sequence === seq.id)
            .map((p) => (
              <g key={p.client} pointerEvents="none">
                <line x1={HEADER_W + p.playhead * pxPerSec} y1={0} x2={HEADER_W + p.playhead * pxPerSec} y2={height} stroke={peerColor(p.client)} strokeWidth={1.5} strokeDasharray="3 2" />
                <text x={HEADER_W + p.playhead * pxPerSec + 3} y={10} fontSize={9} fill={peerColor(p.client)}>
                  {p.name}
                </text>
              </g>
            ))}
          {/* playhead */}
          <line x1={HEADER_W + position * pxPerSec} y1={0} x2={HEADER_W + position * pxPerSec} y2={height} stroke="#e11" strokeWidth={2} pointerEvents="none" />
        </svg>
      </div>
    </div>
  );
}

/** Linked partners (TL-05): other clips playing the same source over the same span. */
function partners(a: ClipInfo, b: ClipInfo): boolean {
  return (
    a.id !== b.id &&
    a.media === b.media &&
    a.nested === b.nested &&
    JSON.stringify(a.title) === JSON.stringify(b.title) &&
    a.timeline_in === b.timeline_in &&
    a.duration === b.duration &&
    a.source_in === b.source_in &&
    a.speed === b.speed &&
    JSON.stringify(a.ramp) === JSON.stringify(b.ramp)
  );
}

/** " · 50%", " · ◀ 100%", " · freeze", " · ramp" or nothing at normal speed. */
function speedLabel(clip: ClipInfo): string {
  if (clip.ramp.length) return " · ramp";
  if (clip.speed === 1) return "";
  if (clip.speed === 0) return " · freeze";
  return ` · ${clip.speed < 0 ? "◀ " : ""}${Math.round(Math.abs(clip.speed) * 100)}%`;
}

/** Speed of the selected clip (TL-09): a percentage applied on Enter or blur,
 * reverse, freeze, ripple on or off, and a ramp from 100% at the head to the
 * entered speed at the tail. */
function SpeedControl({ clip, track, run }: { clip: ClipInfo; track: string; run: (op: EditOp) => void }) {
  const [pct, setPct] = useState(String(Math.round(Math.abs(clip.speed || 1) * 100)));
  const [ripple, setRipple] = useState(true);
  useEffect(() => setPct(String(Math.round(Math.abs(clip.speed || 1) * 100))), [clip.id, clip.speed]);
  const value = () => Math.max(1, Math.min(10000, Number(pct) || 100)) / 100;
  const sign = clip.speed < 0 ? -1 : 1;
  const apply = (speed: number) => run({ kind: "speed", track, clip: clip.id, speed, ripple });
  return (
    <span style={{ display: "inline-flex", gap: 4, alignItems: "center", border: "1px solid #ddd", borderRadius: 4, padding: "0 4px" }} title="Clip speed: keeps the material, so the clip gets longer or shorter">
      Speed
      <input
        value={pct}
        onChange={(e) => setPct(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && apply(sign * value())}
        onBlur={() => value() !== Math.abs(clip.speed) && clip.speed !== 0 && apply(sign * value())}
        style={{ width: 44 }}
      />
      %
      <label title="Play backwards">
        <input type="checkbox" checked={clip.speed < 0} onChange={(e) => apply((e.target.checked ? -1 : 1) * value())} /> reverse
      </label>
      <button onClick={() => apply(clip.speed === 0 ? 1 : 0)} title="Hold the clip's first frame for its whole length">
        {clip.speed === 0 ? "Unfreeze" : "Freeze"}
      </button>
      <button
        onClick={() => run({ kind: "ramp", track, clip: clip.id, keys: clip.ramp.length ? [] : [{ at: 0, speed: 1 }, { at: clip.duration, speed: sign * value() }] })}
        title="Ramp from 100% at the head to the entered speed at the tail; the clip keeps its length"
      >
        {clip.ramp.length ? "Clear ramp" : "Ramp"}
      </button>
      <label title="Move later clips on the track when the clip changes length">
        <input type="checkbox" checked={ripple} onChange={(e) => setRipple(e.target.checked)} /> ripple
      </label>
    </span>
  );
}

/** Audio peaks drawn inside a clip (AUD-04): one vertical stroke per pixel
 * column, on a decibel scale. The engine builds peaks in the background, so this polls briefly
 * until they arrive. */
function ClipWaveform({ media, mediaId, start, duration, x, y, w, h }: { media: MediaApi; mediaId: string; start: number; duration: number; x: number; y: number; w: number; h: number }) {
  const buckets = Math.max(1, Math.min(4096, Math.round(w)));
  const [peaks, setPeaks] = useState<[number, number][] | null>(null);
  useEffect(() => {
    let alive = true;
    let tries = 0;
    const fetchPeaks = () => {
      media
        .waveform?.(mediaId, start, start + duration, buckets)
        .then((p) => {
          if (!alive) return;
          if (p) setPeaks(p);
          else if (tries++ < 50) setTimeout(fetchPeaks, 200);
        })
        .catch(() => {});
    };
    fetchPeaks();
    return () => {
      alive = false;
    };
  }, [media, mediaId, start, duration, buckets]);
  if (!peaks) return null;
  const mid = y + h / 2;
  // Decibel scale over a 60 dB range, so quiet dialogue stays readable.
  const db = (a: number) => (a <= 0.001 ? 0 : Math.min(1, (20 * Math.log10(a) + 60) / 60));
  const d = peaks
    .map(([lo, hi], i) => {
      const half = (db(Math.max(-lo, hi)) * h) / 2;
      return `M${(x + (i * w) / peaks.length).toFixed(1)} ${(mid - half).toFixed(1)}V${(mid + half + 0.5).toFixed(1)}`;
    })
    .join("");
  return <path d={d} stroke="rgba(0,60,20,0.55)" strokeWidth={1} fill="none" pointerEvents="none" />;
}
