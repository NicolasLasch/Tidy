import LocalAi from "./LocalAi";
import HistoryJournal from "./HistoryJournal";
import ChatView, { size } from "./ChatView";
import StorageView from "./StorageView";
import { useEffect, useMemo, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  ArrowDownWideNarrow,
  Cpu,
  File,
  Folder,
  History,
  LoaderCircle,
  MessageCircle,
  Minimize2,
  Maximize2,
  RefreshCw,
  Search,
  Sparkles,
  X,
  HardDrive,
} from "lucide-react";

type Scope = {
  id: number;
  path: string;
  status: string;
  scanned_at: number | null;
  files: number;
  bytes: number;
  omitted: number;
  content: boolean;
};
type FileRow = {
  id: number;
  path: string;
  size: number;
  modified: number;
  excerpt: string | null;
  hashed: boolean;
};
type Job = {
  running: boolean;
  scope_id: number | null;
  visited: number;
  message: string;
};
type Tab = "chat" | "storage" | "files" | "history" | "ai";
const emptyJob: Job = { running: false, scope_id: null, visited: 0, message: "" };
const basename = (path: string) =>
  path.split(/[\\/]/).filter(Boolean).at(-1) || path;

export default function App() {
  const [scopes, setScopes] = useState<Scope[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [tab, setTab] = useState<Tab>("chat");
  const [compact, setCompact] = useState(false);
  const [baseId, setBaseId] = useState<number | null>(null);
  const [chatId, setChatId] = useState<number | null>(null);
  const [job, setJob] = useState<Job>(emptyJob);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const [prompt, setPrompt] = useState<string | null>(null);
  const lastRunning = useRef(false);

  async function refresh() {
    const [list, ids] = await Promise.all([
      invoke<Scope[]>("list_scopes"),
      invoke<number[]>("ai_selection"),
    ]);
    setScopes(list);
    setSelected(new Set(ids));
    setBaseId((b) =>
      b !== null && list.some((s) => s.id === b)
        ? b
        : (list
            .filter((s) => !list.some((o) => o.id !== s.id && s.path.startsWith(o.path + "/")))
            .sort((a, b) => b.bytes - a.bytes)[0]?.id ?? null),
    );
  }
  useEffect(() => {
    if (!isTauri()) {
      setError("Open the Tidy desktop app to choose folders.");
      return;
    }
    void refresh().catch((e) => setError(String(e)));
    const timer = setInterval(() => {
      void invoke<Job>("scan_status")
        .then((next) => {
          setJob(next);
          if (lastRunning.current && !next.running) {
            void refresh().catch((e) => setError(String(e)));
            setRevision((n) => n + 1);
          }
          lastRunning.current = next.running;
        })
        .catch((e) => setError(String(e)));
    }, 650);
    return () => clearInterval(timer);
  }, []);

  const chatScopes = useMemo(
    () =>
      scopes
        .filter((s) => selected.has(s.id))
        .map((s) => ({ id: s.id, name: basename(s.path), files: s.files })),
    [scopes, selected],
  );
  useEffect(() => {
    setChatId((c) =>
      c !== null && chatScopes.some((s) => s.id === c)
        ? c
        : (chatScopes[0]?.id ?? null),
    );
  }, [chatScopes]);

  async function setCompactMode(next: boolean) {
    try {
      await invoke("set_compact_mode", { compact: next });
      setCompact(next);
      if (next) setTab("chat");
    } catch (e) {
      setError(String(e));
    }
  }
  async function scan(id: number, content = false) {
    setError("");
    await invoke("start_scan", { scopeId: id, content });
    lastRunning.current = true;
    setJob({ ...emptyJob, running: true, scope_id: id, message: "Reading folder…" });
  }
  async function add() {
    try {
      const id = await invoke<number | null>("choose_folder");
      if (id !== null) {
        await refresh();
        setBaseId(id);
        setTab("storage");
        await scan(id);
      }
    } catch (e) {
      setError(String(e));
    }
  }
  async function toggleBase(id: number, on: boolean) {
    try {
      setSelected(new Set(await invoke<number[]>("set_ai_selected", { scopeId: id, selected: on })));
    } catch (e) {
      setError(String(e));
    }
  }
  async function toggleFolder(
    base: number,
    path: string,
    scopeId: number | null,
    on: boolean,
  ) {
    if (scopeId !== null) {
      setSelected(new Set(await invoke<number[]>("set_ai_selected", { scopeId, selected: on })));
      return;
    }
    if (!on) return;
    if (job.running) throw new Error("Wait for the current scan to finish, then try again.");
    const id = await invoke<number>("use_working_folder", { scopeId: base, parent: path });
    await invoke<number[]>("set_ai_selected", { scopeId: id, selected: true });
    await refresh();
    await scan(id);
  }

  const tabs: { id: Tab; label: string; icon: React.ReactNode }[] = [
    { id: "chat", label: "Chat", icon: <MessageCircle size={22} /> },
    { id: "storage", label: "Storage", icon: <HardDrive size={22} /> },
    { id: "files", label: "Files", icon: <File size={22} /> },
    { id: "history", label: "History", icon: <History size={22} /> },
    { id: "ai", label: "AI", icon: <Cpu size={22} /> },
  ];
  const chatScope = scopes.find((s) => s.id === chatId);
  const base = scopes.find((s) => s.id === baseId);
  const scanningNow = job.running;

  return (
    <div className={`x-app ${compact ? "compact" : ""}`}>
      <header className="x-top" data-tauri-drag-region>
        <div className="x-top-left" data-tauri-drag-region>
          <span className="x-logo">
            <Sparkles size={15} />
          </span>
          <strong data-tauri-drag-region>Tidy</strong>
        </div>
        {tab === "chat" && chatScopes.length > 0 && (
          <label className="x-scope-pick">
            <Folder size={14} />
            <select
              value={chatId ?? ""}
              onChange={(e) => setChatId(Number(e.target.value))}
              aria-label="Folder Tidy is working in"
            >
              {chatScopes.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
        )}
        <div className="x-top-right">
          {scanningNow && (
            <span className="x-scanning">
              <LoaderCircle size={13} className="x-spin" />
              {job.visited.toLocaleString()}
            </span>
          )}
          <button
            className="x-round"
            onClick={() => void setCompactMode(!compact)}
            aria-label={compact ? "Expand to full window" : "Shrink to a small assistant"}
            title={compact ? "Full window" : "Small assistant"}
          >
            {compact ? <Maximize2 size={15} /> : <Minimize2 size={15} />}
          </button>
        </div>
      </header>
      {error && (
        <div className="x-error banner" role="alert">
          <span>{error}</span>
          <button aria-label="Dismiss" onClick={() => setError("")}>
            <X size={15} />
          </button>
        </div>
      )}
      <main className="x-main">
        <div hidden={tab !== "chat"} className="x-fill">
          <ChatView
            scopes={chatScopes}
            scopeId={chatId}
            scanning={scanningNow}
            prompt={prompt}
            onPromptSent={() => setPrompt(null)}
            compact={compact}
            onRefresh={() => setRevision((r) => r + 1)}
            onOpenFolders={() => {
              if (compact) void setCompactMode(false);
              setTab("storage");
            }}
            onScan={() => chatId !== null && void scan(chatId).catch((e) => setError(String(e)))}
          />
        </div>
        {tab === "storage" && (
          <StorageView
            scopes={scopes.map((s) => ({
              id: s.id,
              path: s.path,
              name: basename(s.path),
              files: s.files,
              bytes: s.bytes,
              status: s.status,
            }))}
            baseId={baseId}
            onBase={setBaseId}
            selected={selected}
            onToggleBase={(id: number, on: boolean) => void toggleBase(id, on)}
            onToggleFolder={toggleFolder}
            onAdd={() => void add()}
            onAsk={(text: string, id: number) => {
              if (!selected.has(id)) {
                setError("Switch this folder on first so Tidy is allowed to work in it.");
                return;
              }
              setChatId(id);
              setPrompt(text);
              setTab("chat");
            }}
            scanning={scanningNow}
            refreshKey={revision}
            onChanged={() => {
              setRevision((r) => r + 1);
              void refresh();
            }}
          />
        )}
        {tab === "files" && (
          <FilesView
            scope={base ?? null}
            scopes={scopes.filter(
              (s) => !scopes.some((o) => o.id !== s.id && s.path.startsWith(o.path + "/")),
            )}
            onBase={setBaseId}
            revision={revision}
            scanning={scanningNow}
            onRescan={(content) => base && void scan(base.id, content).catch((e) => setError(String(e)))}
          />
        )}
        {tab === "history" && (
          <div className="x-page legacy">
            {base ? (
              <HistoryJournal
                key={`history-${baseId}`}
                scopeId={base.id}
                scopeName={basename(base.path)}
                onRefreshNeeded={() => setRevision((r) => r + 1)}
              />
            ) : (
              <p className="x-note">Nothing here yet.</p>
            )}
          </div>
        )}
        {tab === "ai" && (
          <div className="x-page legacy">
            <LocalAi scopeId={chatId} />
          </div>
        )}
      </main>
      {!compact && (
        <nav className="x-tabbar" aria-label="Sections">
          {tabs.map((t) => (
            <button
              key={t.id}
              className={tab === t.id ? "on" : ""}
              onClick={() => setTab(t.id)}
              aria-current={tab === t.id}
            >
              {t.icon}
              <span>{t.label}</span>
            </button>
          ))}
          <span className="x-version">v0.8.2{chatScope ? ` · ${basename(chatScope.path)}` : ""}</span>
        </nav>
      )}
    </div>
  );
}

function FilesView({
  scope,
  scopes,
  onBase,
  revision,
  scanning,
  onRescan,
}: {
  scope: Scope | null;
  scopes: Scope[];
  onBase: (id: number) => void;
  revision: number;
  scanning: boolean;
  onRescan: (content: boolean) => void;
}) {
  const [query, setQuery] = useState("");
  const [bySize, setBySize] = useState(true);
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<{ files: FileRow[]; total: number }>({ files: [], total: 0 });
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState<FileRow | null>(null);
  const [content, setContent] = useState(false);
  const id = scope?.id ?? null;
  useEffect(() => setOffset(0), [id, query, bySize]);
  useEffect(() => {
    if (id === null) return;
    let live = true;
    setLoading(true);
    const t = setTimeout(() => {
      invoke<{ files: FileRow[]; total: number }>("search_files", {
        scopeId: id,
        query,
        sizeSort: bySize,
        offset,
      })
        .then((r) => live && setPage(r))
        .catch(() => {})
        .finally(() => live && setLoading(false));
    }, 180);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [id, query, bySize, offset, revision]);
  if (!scope) return <div className="x-page"><p className="x-note">Add a folder in Folders first.</p></div>;
  return (
    <div className="x-page">
      <div className="x-large-title">
        <div>
          <h1>Files</h1>
          <p>
            {scope.files.toLocaleString()} indexed · {size(scope.bytes)} ·{" "}
            {scope.scanned_at ? `scanned ${new Date(scope.scanned_at * 1000).toLocaleString()}` : "not scanned"}
          </p>
        </div>
        <button className="x-pill" disabled={scanning} onClick={() => onRescan(content)}>
          <RefreshCw size={15} /> Rescan
        </button>
      </div>
      <div className="x-segment">
        {scopes.map((s) => (
          <button key={s.id} className={s.id === id ? "on" : ""} onClick={() => onBase(s.id)}>
            {basename(s.path)}
          </button>
        ))}
      </div>
      <label className="x-search">
        <Search size={15} />
        <input placeholder="Search names and text" value={query} maxLength={100} onChange={(e) => setQuery(e.target.value)} />
        <button type="button" className="x-sort" onClick={() => setBySize(!bySize)}>
          <ArrowDownWideNarrow size={14} /> {bySize ? "Largest" : "A–Z"}
        </button>
      </label>
      <label className="x-check">
        <input type="checkbox" checked={content} disabled={scanning} onChange={(e) => setContent(e.target.checked)} />
        Also read text inside files on the next rescan (stored locally)
      </label>
      <div className="x-group">
        {page.files.map((f) => (
          <button key={f.id} className="x-file-row" onClick={() => setOpen(f)}>
            <File size={18} />
            <span>
              <b>{basename(f.path)}</b>
              <small>{f.path.includes("/") ? f.path.slice(0, f.path.lastIndexOf("/")) : "top level"}</small>
            </span>
            <em>{size(f.size)}</em>
          </button>
        ))}
        {!page.files.length && (
          <p className="x-note">{loading ? "Loading…" : query ? "No matches." : "Nothing indexed yet."}</p>
        )}
      </div>
      {page.total > 100 && (
        <div className="x-pager">
          <button disabled={!offset} onClick={() => setOffset(Math.max(0, offset - 100))}>Previous</button>
          <span>{offset + 1}–{Math.min(offset + 100, page.total)} of {page.total.toLocaleString()}</span>
          <button disabled={offset + 100 >= page.total} onClick={() => setOffset(offset + 100)}>Next</button>
        </div>
      )}
      {open && (
        <div className="x-sheet-backdrop" onClick={() => setOpen(null)}>
          <section className="x-sheet" role="dialog" aria-modal="true" onClick={(e) => e.stopPropagation()}>
            <div className="x-grabber" />
            <h3>{basename(open.path)}</h3>
            <p>{open.path}</p>
            <div className="x-sheet-list">
              <div><span><b>{size(open.size)}</b><small>{open.hashed ? "SHA-256 indexed" : "Not hashed"}</small></span></div>
              {open.excerpt && <pre>{open.excerpt}</pre>}
            </div>
            <div className="x-sheet-buttons">
              <button className="x-secondary" onClick={() => setOpen(null)}>Close</button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
