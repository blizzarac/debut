import { useEffect, useRef, useState, type ReactNode } from "react";
import type { PlayerApi, Tick } from "./engine";

/** Program monitor: pulls a tick per animation frame and blits changed frames.
 * `overlay` is drawn over the picture, sized to it (mask handles). */
export function Viewer({ player, onTick, refreshKey, overlay }: { player: PlayerApi; onTick: (t: Tick) => void; refreshKey: number; overlay?: ReactNode }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState<[number, number]>([0, 0]);
  const force = useRef(0);

  useEffect(() => {
    force.current = refreshKey;
  }, [refreshKey]);

  useEffect(() => {
    let alive = true;
    let busy = false;
    let lastKey = -1;
    const loop = async () => {
      if (!alive) return;
      if (!busy) {
        busy = true;
        try {
          const t = await player.tick();
          onTick(t);
          if (t.changed || force.current !== lastKey) {
            lastKey = force.current;
            const f = await player.framePixels();
            const c = canvas.current;
            if (c) {
              if (c.width !== f.width || c.height !== f.height) {
                c.width = f.width;
                c.height = f.height;
                setSize([f.width, f.height]);
              }
              c.getContext("2d")?.putImageData(new ImageData(f.rgba, f.width, f.height), 0, 0);
            }
          }
        } catch {
          // no sequence yet
        } finally {
          busy = false;
        }
      }
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
    return () => {
      alive = false;
    };
  }, [player, onTick]);

  return (
    <div style={{ background: "#111", display: "flex", alignItems: "center", justifyContent: "center", aspectRatio: "16 / 9", maxHeight: "48vh" }}>
      <div style={{ position: "relative", display: "inline-block", maxWidth: "100%", lineHeight: 0 }}>
        <canvas
          ref={canvas}
          style={{ maxWidth: "100%", maxHeight: "48vh", imageRendering: size[0] < 400 ? "pixelated" : "auto" }}
          title={`${size[0]}×${size[1]}`}
        />
        {overlay}
      </div>
    </div>
  );
}
