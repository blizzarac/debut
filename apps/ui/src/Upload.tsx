import { useEffect, useState } from "react";
import type { ExportApi, UploadDestination, UploadStatus } from "./engine";

type Kind = UploadDestination["kind"];

/** Send a finished export somewhere (EXP-09): a folder, a pre-signed URL,
 * YouTube or Vimeo. Tokens are used for this upload and not kept. */
export function UploadForm({ exporter, file, onClose }: { exporter: ExportApi; file: string; onClose: () => void }) {
  const [kind, setKind] = useState<Kind>("folder");
  const [dir, setDir] = useState("");
  const [url, setUrl] = useState("");
  const [token, setToken] = useState("");
  const [title, setTitle] = useState(file.split("/").pop()?.replace(/\.[^.]*$/, "") ?? "");
  const [description, setDescription] = useState("");
  const [privacy, setPrivacy] = useState("private");
  const [error, setError] = useState("");

  function destination(): UploadDestination {
    switch (kind) {
      case "folder":
        return { kind, dir };
      case "http_put":
        return { kind, url, headers: url.includes(".blob.core.windows.net") ? [["x-ms-blob-type", "BlockBlob"]] : [] };
      case "youtube":
        return { kind, token, title, description, privacy };
      case "vimeo":
        return { kind, token, title, description, privacy: privacy === "private" ? "nobody" : privacy === "public" ? "anybody" : "unlisted" };
    }
  }
  const ready = kind === "folder" ? !!dir : kind === "http_put" ? /^https?:\/\//.test(url) : !!token && !!title;

  return (
    <div style={{ border: "1px solid #ddd", padding: 6, margin: "4px 0", display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{ display: "flex", gap: 4 }}>
        <select value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
          <option value="folder">folder</option>
          <option value="http_put">pre-signed URL (S3, GCS, Azure)</option>
          <option value="youtube">YouTube</option>
          <option value="vimeo">Vimeo</option>
        </select>
        <span style={{ flex: 1 }} />
        <button onClick={onClose}>×</button>
      </div>
      {kind === "folder" && <input value={dir} onChange={(e) => setDir(e.target.value)} placeholder="/path/to/folder" />}
      {kind === "http_put" && <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://bucket…/file.mp4?X-Amz-Signature=…" />}
      {(kind === "youtube" || kind === "vimeo") && (
        <>
          <input
            type="password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder={kind === "youtube" ? "OAuth access token (youtube.upload scope)" : "Vimeo access token (upload scope)"}
            title="Used for this upload only; debut does not store it"
          />
          <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="title" />
          <input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="description" />
          <select value={privacy} onChange={(e) => setPrivacy(e.target.value)}>
            <option value="private">private</option>
            <option value="unlisted">unlisted</option>
            <option value="public">public</option>
          </select>
        </>
      )}
      <button
        disabled={!ready}
        onClick={() =>
          exporter
            .upload!(file, destination())
            .then(onClose)
            .catch((e) => setError(String(e)))
        }
      >
        Upload
      </button>
      {error && <span style={{ color: "#c33" }}>{error}</span>}
    </div>
  );
}

/** Running and finished uploads. */
export function Uploads({ exporter }: { exporter: ExportApi }) {
  const [list, setList] = useState<UploadStatus[]>([]);
  useEffect(() => {
    const id = setInterval(() => exporter.uploads?.().then(setList).catch(() => {}), 500);
    return () => clearInterval(id);
  }, [exporter]);
  if (!list.length) return null;
  return (
    <ul style={{ listStyle: "none", padding: 0, margin: "6px 0 0" }}>
      {list.map((u) => (
        <li key={u.id} style={{ borderTop: "1px solid #eee", padding: "4px 0" }}>
          <div style={{ display: "flex", justifyContent: "space-between", gap: 6 }}>
            <span title={u.file}>
              ↑ {u.file.split("/").pop()} → {u.destination} · {u.state}
              {u.error ? ` · ${u.error}` : ""}
            </span>
            {u.state === "running" && <button onClick={() => exporter.cancelUpload?.(u.id)}>✕</button>}
          </div>
          <progress value={u.sent} max={Math.max(1, u.total)} style={{ width: "100%" }} />
          {u.location && (
            <div style={{ color: "#666", userSelect: "all" }} title="Where it went">
              {u.location}
            </div>
          )}
        </li>
      ))}
    </ul>
  );
}
