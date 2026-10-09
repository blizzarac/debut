import { useEffect } from "react";
import type { PlayerApi, Tick } from "./engine";

export function timecode(t: number, fps: number): string {
  const total = Math.max(0, Math.round(t * fps));
  const f = total % Math.round(fps);
  const s = Math.floor(total / fps);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${pad(Math.floor(s / 3600))}:${pad(Math.floor(s / 60) % 60)}:${pad(s % 60)}:${pad(f)}`;
}

export function Transport({ player, tick, fps }: { player: PlayerApi; tick: Tick | null; fps: number }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "INPUT") return;
      switch (e.key) {
        case " ":
          e.preventDefault();
          player.transport({ kind: "toggle" });
          break;
        case "j":
          player.transport({ kind: "shuttle", forward: false });
          break;
        case "k":
          player.transport({ kind: "pause" });
          break;
        case "l":
          player.transport({ kind: "shuttle", forward: true });
          break;
        case "ArrowLeft":
          player.transport({ kind: "step", n: e.shiftKey ? -10 : -1 });
          break;
        case "ArrowRight":
          player.transport({ kind: "step", n: e.shiftKey ? 10 : 1 });
          break;
        case "Home":
          player.transport({ kind: "seek", t: 0 });
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [player]);

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
    </div>
  );
}
