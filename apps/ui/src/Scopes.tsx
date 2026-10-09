import { useEffect, useRef } from "react";
import type { PlayerApi, ScopesData } from "./engine";

function drawDensity(canvas: HTMLCanvasElement, data: Uint8ClampedArray, w: number, h: number, rgb: [number, number, number]) {
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const img = ctx.createImageData(w, h);
  for (let i = 0; i < w * h; i++) {
    const v = data[i];
    img.data[i * 4] = Math.min(255, rgb[0] * v);
    img.data[i * 4 + 1] = Math.min(255, rgb[1] * v);
    img.data[i * 4 + 2] = Math.min(255, rgb[2] * v);
    img.data[i * 4 + 3] = 255;
  }
  ctx.putImageData(img, 0, 0);
}

function drawHistogram(canvas: HTMLCanvasElement, hist: ScopesData["histogram"]) {
  canvas.width = 256;
  canvas.height = 96;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.fillStyle = "#111";
  ctx.fillRect(0, 0, 256, 96);
  const max = Math.max(1, ...hist.map((h) => Math.max(...h)));
  const colors = ["rgba(255,80,80,0.7)", "rgba(80,255,80,0.7)", "rgba(80,120,255,0.7)"];
  hist.forEach((h, c) => {
    ctx.fillStyle = colors[c];
    for (let x = 0; x < 256; x++) {
      const v = (Math.log1p(h[x]) / Math.log1p(max)) * 96;
      ctx.fillRect(x, 96 - v, 1, v);
    }
  });
}

/** Waveform, vectorscope and histogram of the frame on screen (PB-08). */
export function Scopes({ player, frameKey }: { player: PlayerApi; frameKey: number }) {
  const wave = useRef<HTMLCanvasElement>(null);
  const vec = useRef<HTMLCanvasElement>(null);
  const hist = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    if (!player.scopes) return;
    let alive = true;
    player
      .scopes()
      .then((s) => {
        if (!alive) return;
        if (wave.current) drawDensity(wave.current, s.waveform, 256, 128, [0.4, 1.0, 0.5]);
        if (vec.current) drawDensity(vec.current, s.vectorscope, 128, 128, [0.5, 0.9, 1.0]);
        if (hist.current) drawHistogram(hist.current, s.histogram);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [player, frameKey]);
  return (
    <div style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
      <canvas ref={wave} title="Luma waveform" style={{ width: 256, height: 128, background: "#111", imageRendering: "pixelated" }} />
      <canvas ref={vec} title="Vectorscope" style={{ width: 128, height: 128, background: "#111", borderRadius: 64, imageRendering: "pixelated" }} />
      <canvas ref={hist} title="RGB histogram" style={{ width: 256, height: 96, background: "#111" }} />
    </div>
  );
}
