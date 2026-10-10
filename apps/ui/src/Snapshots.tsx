import { useEffect, useState } from "react";
import type { SnapshotDiff, SnapshotInfo, SnapshotsApi } from "./engine";

const KIND_COLOR: Record<string, string> = {
  added: "#16a34a",
  removed: "#dc2626",
  moved: "#2563eb",
  trimmed: "#d97706",
  retimed: "#9333ea",
  changed: "#475569",
};

/** Versions of the active sequence (TL-14): save one, see what changed since,
 * go back to it (undoable), delete it. */
export function Snapshots({ api, refreshKey, onChanged, onSeek }: { api: SnapshotsApi; refreshKey: number; onChanged: () => void; onSeek: (t: number) => void }) {
  const [list, setList] = useState<SnapshotInfo[]>([]);
  const [name, setName] = useState("");
  const [open, setOpen] = useState<{ id: string; diff: SnapshotDiff } | null>(null);
  const [error, setError] = useState("");
  const act = (p: Promise<unknown>) =>
    p
      .then(() => {
        setError("");
        onChanged();
      })
      .catch((e) => setError(String(e)));

  useEffect(() => {
    api.list().then(setList).catch(() => setList([]));
    if (open) api.compare(open.id).then((diff) => setOpen({ id: open.id, diff })).catch(() => setOpen(null));
  }, [api, refreshKey]); // eslint-disable-line react-hooks/exhaustive-deps

  const summary = (d: SnapshotDiff) => {
    const counts = new Map<string, number>();
    d.clips.forEach((c) => c.kinds.forEach((k) => counts.set(k, (counts.get(k) ?? 0) + 1)));
    const parts = [...counts].map(([k, n]) => `${n} ${k}`);
    if (d.markers_added || d.markers_removed) parts.push(`markers +${d.markers_added}/−${d.markers_removed}`);
    if (d.captions_added || d.captions_removed) parts.push(`captions +${d.captions_added}/−${d.captions_removed}`);
    return parts.length ? parts.join(" · ") : "no changes";
  };

  return (
    <div style={{ fontSize: 12 }}>
      <div style={{ display: "flex", gap: 4 }}>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="version name (optional)" style={{ flex: 1, minWidth: 0 }} onKeyDown={(e) => e.key === "Enter" && act(api.take(name).then(() => setName("")))} />
        <button onClick={() => act(api.take(name).then(() => setName("")))}>Save version</button>
      </div>
      {error && <p style={{ color: "#c33", margin: "4px 0" }}>{error}</p>}
      <ul style={{ listStyle: "none", padding: 0, margin: "4px 0 0" }}>
        {list.map((s) => (
          <li key={s.id} style={{ borderTop: "1px solid #eee", padding: "3px 0" }}>
            <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
              <span style={{ flex: 1 }}>
                {s.name} <span style={{ color: "#888" }}>· {s.clips} clips · {s.duration.toFixed(1)}s</span>
              </span>
              <button
                onClick={() =>
                  open?.id === s.id
                    ? setOpen(null)
                    : api
                        .compare(s.id)
                        .then((diff) => setOpen({ id: s.id, diff }))
                        .catch((e) => setError(String(e)))
                }
                title="What changed since this version"
              >
                {open?.id === s.id ? "Hide" : "Compare"}
              </button>
              <button onClick={() => act(api.restore(s.id))} title="Put the sequence back to this version (Undo brings the current one back)">
                Restore
              </button>
              <button onClick={() => act(api.remove(s.id))} title="Delete this version">
                ×
              </button>
            </div>
            {open?.id === s.id && (
              <div style={{ color: "#555", margin: "2px 0 0 8px" }}>
                <div>{summary(open.diff)}</div>
                {open.diff.clips.slice(0, 50).map((c) => (
                  <div key={c.clip + c.kinds.join()} style={{ cursor: "pointer" }} onClick={() => onSeek(c.at)} title="Go there">
                    <code>{c.track}</code> {c.at.toFixed(2)}s{" "}
                    {c.kinds.map((k) => (
                      <span key={k} style={{ color: KIND_COLOR[k], marginRight: 4 }}>
                        {k}
                      </span>
                    ))}
                  </div>
                ))}
              </div>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
