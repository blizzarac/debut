import { useEffect, useState } from "react";
import type { BinInfo, MediaApi, MediaInfo, ProxyStatus, RuleField } from "./engine";

/** Media and bins (MED-07): import, filter by bin or search, assign media to
 * manual bins, create manual or smart (file-name) bins; build proxies and
 * switch playback to them (MED-05). */
export function MediaPanel({
  media,
  list,
  bins,
  onChanged,
  onInsert,
  onMulticam,
  onStatus,
}: {
  media: MediaApi;
  list: MediaInfo[];
  bins: BinInfo[];
  onChanged: () => void;
  onInsert: (m: MediaInfo) => void;
  onMulticam: (ids: string[], sync: boolean) => void;
  onStatus: (s: string) => void;
}) {
  const [path, setPath] = useState("");
  const [bin, setBin] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [newBin, setNewBin] = useState("");
  const [ruleField, setRuleField] = useState<RuleField>("name");
  const [ruleValue, setRuleValue] = useState("");
  const [tagging, setTagging] = useState<string | null>(null);
  const [relink, setRelink] = useState<{ id: string; path: string } | null>(null);
  const [proxies, setProxies] = useState<Record<string, ProxyStatus>>({});
  const [proxyDiv, setProxyDiv] = useState<2 | 4>(2);
  const [useProxies, setUseProxies] = useState(false);
  const act = (p: Promise<unknown>) => p.then(onChanged).catch((e) => onStatus(String(e)));

  // Poll proxy jobs; quickly while one runs, slowly otherwise.
  const busy = Object.values(proxies).some((p) => p.state === "queued" || p.state === "running");
  useEffect(() => {
    if (!media.proxyStatus) return;
    const load = () =>
      media
        .proxyStatus!()
        .then((all) => setProxies(Object.fromEntries(all.map((p) => [p.media, p]))))
        .catch(() => {});
    load();
    const id = setInterval(load, busy ? 500 : 3000);
    return () => clearInterval(id);
  }, [media, busy, list.length]);
  useEffect(() => {
    media.useProxies?.().then(setUseProxies).catch(() => {});
  }, [media]);
  const name = (m: MediaInfo) => m.path.split("/").pop() ?? m.path;
  const q = search.toLowerCase();
  const shown = list.filter((m) => (!bin || m.bins.includes(bin)) && (!q || name(m).toLowerCase().includes(q) || m.keywords.some((k) => k.toLowerCase().includes(q))));
  const manual = bins.filter((b) => !b.smart);
  const chip = (active: boolean): React.CSSProperties => ({
    padding: "2px 8px",
    borderRadius: 10,
    border: "1px solid #ccc",
    background: active ? "#333" : "#fff",
    color: active ? "#fff" : "#333",
    fontSize: 11,
    cursor: "pointer",
  });

  async function importMedia() {
    if (!path) return;
    try {
      await media.importMedia(path);
      setPath("");
      onChanged();
    } catch (err) {
      onStatus(`import failed: ${err}`);
    }
  }

  return (
    <div style={{ fontSize: 12 }}>
      <div style={{ display: "flex", gap: 4 }}>
        <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/path/to/clip.mp4" style={{ flex: 1, minWidth: 0 }} onKeyDown={(e) => e.key === "Enter" && importMedia()} />
        <button onClick={importMedia}>Import</button>
      </div>
      <div style={{ display: "flex", gap: 4, flexWrap: "wrap", margin: "6px 0" }}>
        <button style={chip(bin === null)} onClick={() => setBin(null)}>
          All ({list.length})
        </button>
        {bins.map((b) => (
          <button key={b.id} style={chip(bin === b.id)} onClick={() => setBin(bin === b.id ? null : b.id)} title={b.smart ? `smart: name contains “${b.filter}”` : "manual bin"}>
            {b.smart ? "◇ " : ""}
            {b.name} ({b.count})
          </button>
        ))}
      </div>
      {bin && (
        <div style={{ display: "flex", gap: 4, marginBottom: 6 }}>
          <input
            defaultValue={bins.find((b) => b.id === bin)?.name}
            onBlur={(e) => {
              const b = bins.find((x) => x.id === bin);
              if (b && e.target.value && e.target.value !== b.name) act(media.renameBin(bin, e.target.value));
            }}
            style={{ flex: 1, minWidth: 0 }}
            title="Rename bin"
          />
          <button
            onClick={() => {
              const id = bin;
              setBin(null);
              act(media.removeBin(id));
            }}
            title="Delete bin (media stay in the project)"
          >
            Delete bin
          </button>
        </div>
      )}
      <div style={{ display: "flex", gap: 4, marginBottom: 6 }}>
        <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="search names" style={{ flex: 1, minWidth: 0 }} />
      </div>
      {media.createProxies && (
        <div style={{ display: "flex", gap: 4, alignItems: "center", margin: "4px 0", color: "#555" }} title="Small copies for smooth editing; export always uses the original files">
          proxies
          <select value={proxyDiv} onChange={(e) => setProxyDiv(Number(e.target.value) as 2 | 4)}>
            <option value={2}>½ size</option>
            <option value={4}>¼ size</option>
          </select>
          <button
            disabled={shown.length === 0}
            onClick={() =>
              media
                .createProxies!(
                  shown.filter((m) => m.online && proxies[m.id]?.state !== "ready").map((m) => m.id),
                  proxyDiv,
                )
                .then(() => media.proxyStatus?.().then((all) => setProxies(Object.fromEntries(all.map((p) => [p.media, p])))))
                .catch((e) => onStatus(String(e)))
            }
          >
            Create for listed
          </button>
          <label>
            <input
              type="checkbox"
              checked={useProxies}
              onChange={(e) => {
                const on = e.target.checked;
                media
                  .setUseProxies!(on)
                  .then(() => setUseProxies(on))
                  .then(onChanged)
                  .catch((err) => onStatus(String(err)));
              }}
            />{" "}
            use proxies
          </label>
        </div>
      )}
      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {shown.map((m) => (
          <li key={m.id} style={{ padding: "4px 2px", borderBottom: "1px solid #eee", display: "grid", gridTemplateColumns: "1fr auto auto", gap: 4, alignItems: "center" }}>
            <span title={m.path} style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: m.online ? undefined : "#c33" }}>
              {!m.online && (
                <button onClick={() => setRelink({ id: m.id, path: m.path })} title="File missing: click to relink" style={{ marginRight: 4, color: "#c33", borderColor: "#c33" }}>
                  offline
                </button>
              )}
              {name(m)} · {m.width}×{m.height} · {m.duration.toFixed(2)}s
              <ProxyBadge status={proxies[m.id]} />
            </span>
            {manual.length > 0 ? (
              <select value={m.bins.find((b) => manual.some((x) => x.id === b)) ?? ""} onChange={(e) => act(media.assignMedia(m.id, e.target.value || null))} title="Bin">
                <option value="">no bin</option>
                {manual.map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.name}
                  </option>
                ))}
              </select>
            ) : (
              <span />
            )}
            <button onClick={() => onInsert(m)} title="Insert at playhead">
              +
            </button>
            {relink?.id === m.id && (
              <div style={{ gridColumn: "1 / -1", display: "flex", gap: 4 }}>
                <input value={relink.path} onChange={(e) => setRelink({ id: m.id, path: e.target.value })} placeholder="/new/path/to/file.mp4" style={{ flex: 1, minWidth: 0 }} />
                <button onClick={() => { act(media.relinkMedia(m.id, relink.path)); setRelink(null); }}>Relink</button>
                <button onClick={() => setRelink(null)}>×</button>
              </div>
            )}
            <div style={{ gridColumn: "1 / -1", display: "flex", gap: 4, alignItems: "center", color: "#666" }}>
              <span title="rating" style={{ letterSpacing: 1, cursor: "pointer" }}>
                {[1, 2, 3, 4, 5].map((n) => (
                  <span key={n} onClick={() => act(media.setMediaTags(m.id, m.keywords, m.rating === n ? 0 : n))} style={{ color: n <= m.rating ? "#f59e0b" : "#ccc" }}>
                    ★
                  </span>
                ))}
              </span>
              {tagging === m.id ? (
                <input
                  autoFocus
                  defaultValue={m.keywords.join(", ")}
                  placeholder="keywords, comma separated"
                  style={{ flex: 1, minWidth: 0 }}
                  onBlur={(e) => {
                    setTagging(null);
                    const kws = e.target.value.split(",").map((k) => k.trim()).filter(Boolean);
                    if (kws.join("|") !== m.keywords.join("|")) act(media.setMediaTags(m.id, kws, m.rating));
                  }}
                  onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
                />
              ) : (
                <span onClick={() => setTagging(m.id)} title="click to edit keywords" style={{ flex: 1, cursor: "text", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {m.keywords.length ? m.keywords.join(", ") : "add keywords…"}
                </span>
              )}
            </div>
          </li>
        ))}
      </ul>
      {shown.length === 0 && list.length > 0 && <p style={{ color: "#999", margin: "4px 0" }}>Nothing matches.</p>}
      <div style={{ display: "flex", gap: 4, marginTop: 6, flexWrap: "wrap" }}>
        <input value={newBin} onChange={(e) => setNewBin(e.target.value)} placeholder="new bin name" style={{ flex: 1, minWidth: 80 }} />
        <button disabled={!newBin} onClick={() => { act(media.addBin(newBin)); setNewBin(""); }}>
          Bin
        </button>
        <select value={ruleField} onChange={(e) => setRuleField(e.target.value as RuleField)} title="Smart bin rule">
          <option value="name">name contains</option>
          <option value="keyword">keyword is</option>
          <option value="rating">rating ≥</option>
          <option value="path">path contains</option>
        </select>
        <input value={ruleValue} onChange={(e) => setRuleValue(e.target.value)} placeholder={ruleField === "rating" ? "1–5" : "value"} style={{ width: 70 }} />
        <button
          disabled={!newBin || !ruleValue}
          title="Smart bin: members are the media matching the rule"
          onClick={() => {
            const op = ruleField === "rating" ? "gte" : ruleField === "keyword" ? "eq" : "contains";
            act(media.addSmartBin(newBin, { field: ruleField, op, value: ruleValue }));
            setNewBin("");
            setRuleValue("");
          }}
        >
          Smart
        </button>
        {shown.length >= 2 && (
          <>
            <button onClick={() => onMulticam(shown.map((m) => m.id), false)} title="Insert the listed media as one multicam clip at the playhead; keys 1–9 switch angles">
              Multicam ({shown.length})
            </button>
            <button onClick={() => onMulticam(shown.map((m) => m.id), true)} title="Same, aligning the angles to the first one by their audio">
              Sync by audio
            </button>
          </>
        )}
      </div>
    </div>
  );
}

/** "proxy" when ready, a percentage while building, "proxy failed" with the reason. */
function ProxyBadge({ status }: { status: ProxyStatus | undefined }) {
  if (!status || status.state === "none") return null;
  const style: React.CSSProperties = { marginLeft: 6, padding: "0 4px", borderRadius: 3, fontSize: 10, border: "1px solid #ccc" };
  if (status.state === "ready")
    return (
      <span style={{ ...style, color: "#16a34a", borderColor: "#16a34a" }} title={status.path ?? ""}>
        proxy
      </span>
    );
  if (status.state === "failed")
    return (
      <span style={{ ...style, color: "#c33", borderColor: "#c33" }} title={status.error ?? ""}>
        proxy failed
      </span>
    );
  return <span style={style}>proxy {Math.round(status.progress * 100)}%</span>;
}
