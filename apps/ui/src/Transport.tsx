import { useEffect, useState } from "react";
import { useShortcut } from "./shortcuts";
import type { HwDecodeStatus, PlayerApi, PreviewQuality, ProgramWindow, Tick } from "./engine";

export function timecode(t: number, fps: number): string {
  const total = Math.max(0, Math.round(t * fps));
  const f = total % Math.round(fps);
  const s = Math.floor(total / fps);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${pad(Math.floor(s / 3600))}:${pad(Math.floor(s / 60) % 60)}:${pad(s % 60)}:${pad(f)}`;
}

export function Transport({ player, tick, fps, onMarker, onAngle }: { player: PlayerApi; tick: Tick | null; fps: number; onMarker?: () => void; onAngle?: (angle: number) => void }) {
  const [quality, setQuality] = useState<PreviewQuality>("auto");
  const [hw, setHw] = useState<HwDecodeStatus | null>(null);
  const [program, setProgram] = useState<ProgramWindow | null>(null);
  const [programNote, setProgramNote] = useState("");
  const [screens, setScreens] = useState<{ index: number; name: string }[]>([]);
  const [screen, setScreen] = useState<number | null>(null);
  useEffect(() => {
    player.monitors?.().then(setScreens).catch(() => {});
    player.programWindow?.().then(setProgram).catch(() => {});
  }, [player]);
  // The window can be closed from its own title bar.
  useEffect(() => {
    if (!program?.open || !player.programWindow) return;
    const id = setInterval(() => player.programWindow!().then(setProgram).catch(() => {}), 1000);
    return () => clearInterval(id);
  }, [player, program?.open]);
  const refreshHw = () => player.hardwareDecodeStatus?.().then(setHw).catch(() => {});
  useEffect(() => {
    refreshHw();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per player
  }, [player]);
  // Transport, marker and angle keys come from the active keymap (TL-12).
  const t = (a: Parameters<PlayerApi["transport"]>[0]) => () => void player.transport(a);
  useShortcut("play_pause", t({ kind: "toggle" }));
  useShortcut("shuttle_back", t({ kind: "shuttle", forward: false }));
  useShortcut("pause", t({ kind: "pause" }));
  useShortcut("shuttle_forward", t({ kind: "shuttle", forward: true }));
  useShortcut("step_back", t({ kind: "step", n: -1 }));
  useShortcut("step_forward", t({ kind: "step", n: 1 }));
  useShortcut("step_back_10", t({ kind: "step", n: -10 }));
  useShortcut("step_forward_10", t({ kind: "step", n: 10 }));
  useShortcut("go_to_start", t({ kind: "seek", t: 0 }));
  useShortcut("add_marker", onMarker);
  for (let n = 1; n <= 9; n++) {
    // eslint-disable-next-line react-hooks/rules-of-hooks -- fixed count
    useShortcut(`angle_${n}`, onAngle ? () => onAngle(n - 1) : null);
  }

  const btn = (label: string, action: () => void, title?: string) => (
    <button onClick={action} title={title} style={{ minWidth: 36 }}>
      {label}
    </button>
  );
  return (
    <div style={{ display: "flex", gap: 6, alignItems: "center", padding: "8px 0" }}>
      {btn("⏮", () => player.transport({ kind: "seek", t: 0 }), "Home")}
      {btn("◀◀", () => player.transport({ kind: "shuttle", forward: false }), "J")}
      {btn("◀", () => player.transport({ kind: "step", n: -1 }), "←")}
      {btn(tick?.playing ? "⏸" : "▶", () => player.transport({ kind: "toggle" }), "Space")}
      {btn("▶", () => player.transport({ kind: "step", n: 1 }), "→")}
      {btn("▶▶", () => player.transport({ kind: "shuttle", forward: true }), "L")}
      <code style={{ marginLeft: 12, fontSize: 16 }}>{timecode(tick?.position ?? 0, fps)}</code>
      {tick && tick.dropped > 0 && <span style={{ color: "#c33", fontSize: 12 }}>{tick.dropped} dropped</span>}
      <span style={{ flex: 1 }} />
      {player.openProgramWindow && (
        <span style={{ fontSize: 12, display: "inline-flex", gap: 4, alignItems: "center" }} title={program?.error ?? programNote ?? ""}>
          {screens.length > 1 && !program?.open && (
            <select value={screen ?? ""} onChange={(e) => setScreen(e.target.value === "" ? null : Number(e.target.value))} title="Display for the program window">
              <option value="">last display</option>
              {screens.map((s) => (
                <option key={s.index} value={s.index}>
                  {s.name}
                </option>
              ))}
            </select>
          )}
          <button
            title="Show the program in its own window, straight from the GPU (put it on a second display, full screen)"
            onClick={() =>
              (program?.open
                ? player.closeProgramWindow!().then(() => setProgram({ open: false, size: null, error: null }))
                : player.openProgramWindow!(screen, false).then((p) => {
                    setProgram(p);
                    setProgramNote("");
                  })
              ).catch((e) => setProgramNote(String(e)))
            }
          >
            {program?.open ? "Close program" : "Program"}
          </button>
          {program?.open && player.programFullscreen && (
            <label>
              <input type="checkbox" onChange={(e) => player.programFullscreen!(e.target.checked).catch((err) => setProgramNote(String(err)))} /> full screen
            </label>
          )}
          {(program?.error || programNote) && <span style={{ color: "#c33" }}>{program?.error ?? programNote}</span>}
        </span>
      )}
      {hw?.available && player.setHardwareDecode && (
        <label
          style={{ fontSize: 12, color: hw.enabled && hw.media.some((m) => m.mode === "fallback") ? "#b45309" : undefined }}
          title={
            hw.media.length
              ? hw.media.map((m) => `${m.name}: ${m.mode}${m.detail ? ` (${m.detail})` : ""}`).join("\n")
              : "Decode video on the GPU where a device takes it; falls back to software otherwise"
          }
          onMouseEnter={refreshHw}
        >
          <input
            type="checkbox"
            checked={hw.enabled}
            onChange={(e) =>
              player
                .setHardwareDecode!(e.target.checked)
                // Decode a frame so the status says what the driver did.
                .then(() => player.framePixels())
                .then(refreshHw)
            }
          />{" "}
          HW decode{hw.enabled && hw.media.length ? ` (${hw.media.filter((m) => m.mode === "hardware").length}/${hw.media.length})` : ""}
        </label>
      )}
      {player.setPreviewQuality && (
        <label style={{ fontSize: 12 }} title="Playback resolution">
          preview{" "}
          <select
            value={quality}
            onChange={(e) => {
              const q = e.target.value as PreviewQuality;
              setQuality(q);
              player.setPreviewQuality!(q);
            }}
          >
            <option value="auto">Auto{tick ? ` (1/${tick.preview_divisor})` : ""}</option>
            <option value="full">Full</option>
            <option value="half">1/2</option>
            <option value="quarter">1/4</option>
          </select>
        </label>
      )}
    </div>
  );
}
