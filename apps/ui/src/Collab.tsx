import { useState } from "react";
import type { CollabApi, CollabStatus } from "./engine";
import { timecode } from "./Transport";

/** Shared editing (COL): host the open project or join someone's session,
 * see who is there and where, which tracks are locked, and what got refused. */
export function Collab({ api, status, fps, onStatus }: { api: CollabApi; status: CollabStatus | null; fps: number; onStatus: (s: string) => void }) {
  const [name, setName] = useState("Editor");
  const [addr, setAddr] = useState("127.0.0.1:7878");
  const [role, setRole] = useState<"editor" | "reviewer">("editor");
  const fail = (e: unknown) => onStatus(String(e));
  const port = Number(addr.split(":").pop()) || 7878;

  if (!status)
    return (
      <div style={{ fontSize: 12, display: "flex", flexDirection: "column", gap: 4 }}>
        <div style={{ display: "flex", gap: 4 }}>
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="your name" style={{ width: 80 }} />
          <input value={addr} onChange={(e) => setAddr(e.target.value)} placeholder="host:port" style={{ flex: 1, minWidth: 0 }} />
        </div>
        <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
          <select value={role} onChange={(e) => setRole(e.target.value as "editor" | "reviewer")} title="Reviewers can only leave comments (markers)">
            <option value="editor">editor</option>
            <option value="reviewer">reviewer</option>
          </select>
          <button onClick={() => api.join(addr, name, role).catch(fail)} title="Join a shared session; its project replaces the open one">
            Join
          </button>
          <button onClick={() => api.host(port, name).then((a) => onStatus(`hosting on port ${port} (${a})`)).catch(fail)} title={`Share the open project on port ${port}`}>
            Host
          </button>
        </div>
      </div>
    );

  return (
    <div style={{ fontSize: 12 }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <span>
          <strong style={{ color: status.joined ? "#16a34a" : "#d97706" }}>{status.joined ? "●" : "○"}</strong> {status.name} ({status.role}) · {status.address}
        </span>
        <button onClick={() => api.leave().catch(fail)}>Leave</button>
      </div>
      <div style={{ color: "#666" }}>
        v{status.version}
        {status.pending ? ` · ${status.pending} edit(s) not confirmed yet` : ""}
        {!status.joined && " · "}
        {!status.joined && (
          <button style={{ fontSize: 10 }} onClick={() => api.join(status.address, status.name, status.role).catch(fail)}>
            Rejoin
          </button>
        )}
      </div>
      <ul style={{ listStyle: "none", padding: 0, margin: "4px 0" }}>
        {status.peers.map((p) => (
          <li key={p.client}>
            <span style={{ color: peerColor(p.client) }}>■</span> {p.name} <span style={{ color: "#888" }}>({p.role}) at {timecode(p.playhead, fps)}</span>
          </li>
        ))}
        {status.peers.length === 0 && <li style={{ color: "#888" }}>nobody else here yet</li>}
      </ul>
      {status.locks.length > 0 && <div style={{ color: "#555" }}>locked: {status.locks.map((l) => `${l.owner}`).join(", ")}</div>}
      {status.notes.slice(-4).map((n, i) => (
        <div key={i} style={{ color: "#b45309" }}>
          {n}
        </div>
      ))}
    </div>
  );
}

/** A stable colour per collaborator. */
export function peerColor(client: number): string {
  const hues = [200, 330, 140, 30, 270, 90];
  return `hsl(${hues[client % hues.length]} 70% 45%)`;
}
