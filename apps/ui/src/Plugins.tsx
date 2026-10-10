import { useEffect, useState } from "react";
import type { PluginParam, PluginsApi, PluginsInfo } from "./engine";

// One scan result shared by every panel that offers plugins.
let shared: PluginsInfo | null = null;
const listeners = new Set<(p: PluginsInfo) => void>();
const publish = (p: PluginsInfo) => {
  shared = p;
  listeners.forEach((l) => l(p));
};

/** The scanned plugins (FX-15, AUD-09) and a way to scan again. */
export function usePlugins(api: PluginsApi | undefined): { info: PluginsInfo | null; scanning: boolean; scan: () => void } {
  const [info, setInfo] = useState<PluginsInfo | null>(shared);
  const [scanning, setScanning] = useState(false);
  useEffect(() => {
    listeners.add(setInfo);
    if (api && !shared) api.list().then(publish).catch(() => {});
    return () => {
      listeners.delete(setInfo);
    };
  }, [api]);
  const scan = () => {
    if (!api) return;
    setScanning(true);
    api
      .scan()
      .then(publish)
      .catch((e) => console.warn("plugin scan failed", e))
      .finally(() => setScanning(false));
  };
  return { info, scanning, scan };
}

/** A picker of scanned plugins of one kind, with a rescan entry. */
export function PluginPicker({ api, kind, onPick, label }: { api: PluginsApi; kind: "openfx" | "clap"; onPick: (path: string, index: number) => void; label: string }) {
  const { info, scanning, scan } = usePlugins(api);
  if (info && !info.available) return null;
  const list = (info?.plugins ?? []).filter((p) => p.kind === kind);
  const problems = info?.problems ?? [];
  return (
    <select
      value=""
      title={problems.length ? `Could not use:\n${problems.map(([p, why]) => `${p}: ${why}`).join("\n")}` : "Third-party plugins, run in a separate process"}
      onChange={(e) => {
        const v = e.target.value;
        if (v === "scan") return scan();
        const p = list[Number(v)];
        if (p) onPick(p.path, p.index);
      }}
    >
      <option value="">{scanning ? "scanning…" : label}</option>
      {list.map((p, i) => (
        <option key={`${p.path}#${p.index}`} value={i}>
          {p.name}
        </option>
      ))}
      <option value="scan">{info?.plugins.length ? "Rescan plugins" : "Scan for plugins"}</option>
    </select>
  );
}

/** Sliders for a plugin's parameters; `onSet` fires on release. */
export function PluginParams({ params, onSet }: { params: PluginParam[]; onSet: (name: string, value: number) => void }) {
  const [live, setLive] = useState<Record<string, number>>({});
  return (
    <>
      {params.map((p) => {
        const value = live[p.name] ?? p.value;
        const commit = (v: number) => {
          setLive((l) => {
            const { [p.name]: _, ...rest } = l;
            return rest;
          });
          onSet(p.name, v);
        };
        return (
          <label key={p.name} style={{ display: "grid", gridTemplateColumns: "80px 1fr 56px", gap: 6, alignItems: "center", marginBottom: 2 }}>
            <span style={{ color: p.animated ? "#c60" : "#333", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={p.label}>
              {p.animated ? "◆ " : ""}
              {p.label}
            </span>
            <input
              type="range"
              min={p.min}
              max={p.max}
              step={(p.max - p.min) / 200 || 0.01}
              value={value}
              onChange={(e) => setLive((l) => ({ ...l, [p.name]: Number(e.target.value) }))}
              onMouseUp={(e) => commit(Number((e.target as HTMLInputElement).value))}
              onKeyUp={(e) => commit(Number((e.target as HTMLInputElement).value))}
            />
            <code>{value.toFixed(2)}</code>
          </label>
        );
      })}
    </>
  );
}
