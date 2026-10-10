import { useState } from "react";
import type { CollabApi, CollabStatus, HostInfo } from "./engine";
import { timecode } from "./Transport";

/** Shared editing (COL): host the open project or join someone's session,
 * see who is there and where, which tracks are locked, and what got refused. */
export function Collab({ api, status, fps, onStatus }: { api: CollabApi; status: CollabStatus | null; fps: number; onStatus: (s: string) => void }) {
  const [name, setName] = useState("Editor");
  const [addr, setAddr] = useState("127.0.0.1:7878");
  const [code, setCode] = useState("");
  const [hosted, setHosted] = useState<HostInfo | null>(null);
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
          <input value={code} onChange={(e) => setCode(e.target.value)} placeholder="invite code" title="The host's editor or reviewer code; it decides your role" style={{ flex: 1, minWidth: 0 }} />
          <button disabled={!code.trim()} onClick={() => api.join(addr, name, code).catch(fail)} title="Join a shared session; its project replaces the open one">
            Join
          </button>
          <button
            onClick={() =>
              api
                .host(port, name)
                .then((h) => {
                  setHosted(h);
                  setCode(h.editor_code);
                  onStatus(`hosting on port ${h.port}`);
                })
                .catch(fail)
            }
            title={`Share the open project on port ${port}; you get invite codes to hand out`}
          >
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
        <button
          onClick={() => {
            setHosted(null);
            api.leave().catch(fail);
          }}
        >
          Leave
        </button>
      </div>
      <div style={{ color: "#666" }}>
        v{status.version}
        {status.pending ? ` · ${status.pending} edit(s) not confirmed yet` : ""}
        {!status.joined && " · "}
        {!status.joined && (
          <button style={{ fontSize: 10 }} disabled={!code.trim()} onClick={() => api.join(status.address, status.name, code).catch(fail)}>
            Rejoin
          </button>
        )}
      </div>
      {hosted && (
        <div style={{ background: "#f6f6f6", padding: 4, margin: "4px 0" }} title="Share these with the people who should join; anyone without one is turned away">
          <div>
            editors: <code style={{ userSelect: "all" }}>{hosted.editor_code}</code>
          </div>
          {hosted.reviewer_code && (
            <div>
              reviewers: <code style={{ userSelect: "all" }}>{hosted.reviewer_code}</code>
            </div>
          )}
          <div style={{ color: "#888" }}>port {hosted.port} · traffic is not encrypted: use it on a trusted network or through a VPN/SSH tunnel</div>
        </div>
      )}
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
