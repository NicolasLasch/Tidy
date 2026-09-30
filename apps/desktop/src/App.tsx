import LocalAi from "./LocalAi";
import AssistantWorkspace from "./AssistantWorkspace";
import StorageAnalyzer from "./StorageAnalyzer";
import HistoryJournal from "./HistoryJournal";
import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  ArrowDownWideNarrow,
  ArrowUpRight,
  Check,
  ChevronLeft,
  ChevronRight,
  File,
  Folder,
  FolderPlus,
  HardDrive,
  History,
  Layers,
  LoaderCircle,
  LockKeyhole,
  RefreshCw,
  Search,
  ShieldCheck,
  Sparkles,
  Square,
  X,
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
type Page = { files: FileRow[]; total: number };
type Job = {
  running: boolean;
  scope_id: number | null;
  visited: number;
  message: string;
  omissions: { path: string; reason: string }[];
};
const emptyJob: Job = {
  running: false,
  scope_id: null,
  visited: 0,
  message: "",
  omissions: [],
};
function bytes(n: number) {
  if (n < 1024) return `${n} B`;
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), 4);
  return `${(n / 1024 ** i).toFixed(i > 1 ? 1 : 0)} ${["B", "KB", "MB", "GB", "TB"][i]}`;
}
function basename(path: string) {
  return path.split(/[\\/]/).filter(Boolean).at(-1) || path;
}
function kind(path: string) {
  const ext = basename(path).split(".").at(-1)?.toLowerCase() || "";
  return ["png", "jpg", "jpeg", "webp", "gif", "svg", "heic"].includes(ext)
    ? "image"
    : ["mp4", "mov", "mkv", "mp3", "wav"].includes(ext)
      ? "media"
      : ["pdf", "docx", "txt", "md"].includes(ext)
        ? "document"
        : ["zip", "dmg", "pkg", "tar", "gz"].includes(ext)
          ? "archive"
          : "file";
}
export default function App() {
  const [scopes, setScopes] = useState<Scope[]>([]),
    [scopeId, setScopeId] = useState<number | null>(null);
  const [page, setPage] = useState<Page>({ files: [], total: 0 }),
    [query, setQuery] = useState(""),
    [sizeSort, setSizeSort] = useState(true),
    [offset, setOffset] = useState(0);
  const [job, setJob] = useState<Job>(emptyJob),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [content, setContent] = useState(false),
    [selected, setSelected] = useState<FileRow | null>(null),
    [forget, setForget] = useState(false),
    [showOmissions, setShowOmissions] = useState(false),
    [revision, setRevision] = useState(0),
    [activeTab, setActiveTab] = useState<
      "files" | "organize" | "storage" | "history" | "ai"
    >("organize");
  const [loading, setLoading] = useState(false);
  const lastRunning = useRef(false),
    initialized = useRef(false);
  const scope = scopes.find((s) => s.id === scopeId);
  useEffect(() => {
    if (!selected && !forget) return;
    const previous = document.activeElement as HTMLElement | null;
    const dialog = document.querySelector<HTMLElement>('[aria-modal="true"]');
    const focusable = () =>
      Array.from(
        dialog?.querySelectorAll<HTMLElement>(
          'button:not(:disabled),input:not(:disabled),[tabindex="0"]',
        ) ?? [],
      );
    focusable()[0]?.focus();
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setSelected(null);
        setForget(false);
      }
      if (event.key === "Tab") {
        const nodes = focusable(),
          first = nodes[0],
          last = nodes.at(-1);
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first?.focus();
        }
      }
    };
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("keydown", key);
      previous?.focus();
    };
  }, [selected, forget]);

  async function refresh() {
    const list = await invoke<Scope[]>("list_scopes");
    setScopes(list);
    if (!initialized.current) {
      initialized.current = true;
      setScopeId(list[0]?.id ?? null);
    }
  }
  useEffect(() => {
    if (!isTauri()) {
      setError(
        "Open the Tidy desktop app to choose folders. This browser preview cannot access your files.",
      );
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
  useEffect(() => {
    setOffset(0);
    setSelected(null);
    setQuery("");
    setContent(scope?.content ?? false);
  }, [scopeId]);
  useEffect(() => {
    let active = true;
    if (scopeId === null) {
      setPage({ files: [], total: 0 });
      return;
    }
    setLoading(true);
    const timer = setTimeout(() => {
      void invoke<Page>("search_files", { scopeId, query, sizeSort, offset })
        .then((result) => {
          if (active) setPage(result);
        })
        .catch((e) => {
          if (active) setError(String(e));
        })
        .finally(() => {
          if (active) setLoading(false);
        });
    }, 180);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [scopeId, query, sizeSort, offset, revision]);
  async function run(id: number, readContent: boolean) {
    setError("");
    setSelected(null);
    await invoke("start_scan", { scopeId: id, content: readContent });
    lastRunning.current = true;
    setJob({
      ...emptyJob,
      running: true,
      scope_id: id,
      message: "Reading folder…",
    });
  }
  async function add() {
    setBusy(true);
    setError("");
    try {
      const id = await invoke<number | null>("choose_folder");
      if (id !== null) {
        await refresh();
        setScopeId(id);
        setContent(false);
        if (!job.running) await run(id, false);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function useWorkingFolder(parent: string) {
    if (scopeId === null) return;
    setBusy(true);
    setError("");
    try {
      const id = await invoke<number>("use_working_folder", {
        scopeId,
        parent,
      });
      await refresh();
      setScopeId(id);
      setContent(false);
      if (parent) await run(id, false);
      setActiveTab("organize");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function remove() {
    if (scopeId === null) return;
    setBusy(true);
    try {
      await invoke("forget_folder", { scopeId });
      const next = scopes.filter((s) => s.id !== scopeId);
      setScopes(next);
      setScopeId(next[0]?.id ?? null);
      setForget(false);
      setSelected(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const currentJob = job.scope_id === scopeId;
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-icon">
            <img src="/tidy-logo.png" alt="" />
          </span>
          tidy<span className="phase">LOCAL</span>
        </div>
        <div className="workspace-label">YOUR WORKSPACE</div>
        <button
          type="button"
          className={`nav-item ${activeTab === "organize" ? "nav-active" : ""}`}
          onClick={() => setActiveTab("organize")}
        >
          <Layers size={17} />
          <span>Ask Tidy</span>
        </button>
        <button
          type="button"
          className={`nav-item ${activeTab === "files" ? "nav-active" : ""}`}
          onClick={() => setActiveTab("files")}
        >
          <Folder size={17} />
          <span>Explore files</span>
          <span className="nav-count">{scopes.length}</span>
        </button>
        <button
          type="button"
          className={`nav-item ${activeTab === "storage" ? "nav-active" : ""}`}
          onClick={() => setActiveTab("storage")}
        >
          <HardDrive size={17} />
          <span>Storage recovery</span>
        </button>
        <button
          type="button"
          className={`nav-item ${activeTab === "ai" ? "nav-active" : ""}`}
          onClick={() => setActiveTab("ai")}
        >
          <Sparkles size={17} />
          <span>Model & local answers</span>
        </button>
        <button
          type="button"
          className={`nav-item ${activeTab === "history" ? "nav-active" : ""}`}
          onClick={() => setActiveTab("history")}
        >
          <History size={17} />
          <span>History & Undo</span>
        </button>
        <div className="sidebar-heading">
          <span>SELECTED FOLDERS</span>
          <button
            className="icon-button"
            onClick={() => void add()}
            disabled={busy || !isTauri()}
            aria-label="Add folder"
          >
            <FolderPlus size={17} />
          </button>
        </div>
        <nav aria-label="Indexed folders">
          {scopes.map((s) => (
            <button
              key={s.id}
              className={`scope-button ${scopeId === s.id ? "chosen" : ""}`}
              onClick={() => setScopeId(s.id)}
              title={s.path}
            >
              <Folder size={16} />
              <span>{basename(s.path)}</span>
              {s.status === "complete" && <Check size={13} />}
            </button>
          ))}
        </nav>
        {scopes.length === 0 && (
          <p className="sidebar-empty">
            Only folders you choose
            <br />
            will appear here.
          </p>
        )}
        <div className="sidebar-bottom">
          <div className="private-card">
            <ShieldCheck size={19} />
            <div>
              <strong>Just your computer.</strong>
              <p>
                Your files stay here.
                <br />
                No cloud. No account.
              </p>
            </div>
          </div>
          <div className="version">
            <span className="status-dot" />
            Local intelligence<span>v0.8.1</span>
          </div>
        </div>
      </aside>
      {activeTab !== "organize" && (
        <button
          className="tidy-companion"
          onClick={() => setActiveTab("organize")}
          aria-label="Open file organization assistant"
        >
          <span>
            Let’s make a little space.<small>Tell me what to organize →</small>
          </span>
          <img src="/tidy-mascot.svg" alt="Tidy leaf assistant" />
        </button>
      )}
      <main>
        <header className="topbar">
          <span>
            Workspace <span className="slash">/</span>{" "}
            <strong>{scope ? basename(scope.path) : "Overview"}</strong>
          </span>
          <span className="offline-badge">
            <span className="status-dot" />
            Offline & private
          </span>
        </header>
        <div
          className={`main-content ${activeTab === "organize" ? "assistant-view" : ""}`}
        >
          <div className="heading-row">
            <div>
              <div className="eyebrow">YOUR LOCAL WORKSPACE</div>
              <h1>
                {scope
                  ? activeTab === "organize"
                    ? "A little help. A lot more clarity."
                    : basename(scope.path)
                  : "Make space for what matters."}
              </h1>
              <p className="subtitle">
                {scope
                  ? "Your request shapes the plan. Every change stays under your control."
                  : "Start with one folder. Get to know what’s taking up space."}
              </p>
            </div>
            <button
              className="primary"
              onClick={() => void add()}
              disabled={busy || !isTauri()}
            >
              <FolderPlus size={17} />
              {busy ? "Opening…" : "Add folder"}
            </button>
          </div>
          {error && (
            <div className="error" role="alert">
              <span>{error}</span>
              <button
                className="icon-button"
                aria-label="Dismiss error"
                onClick={() => setError("")}
              >
                <X size={16} />
              </button>
            </div>
          )}
          {!scope ? (
            <section className="welcome">
              <div className="welcome-art">
                <Folder size={60} strokeWidth={1} />
                <span>
                  <Search size={25} />
                </span>
              </div>
              <h2>Your next clear-headed moment.</h2>
              <p>
                Choose Downloads, a project folder, or somewhere
                <br />
                you’ve been meaning to look through.
              </p>
              <button
                className="primary"
                onClick={() => void add()}
                disabled={busy || !isTauri()}
              >
                Choose a folder <ArrowUpRight size={17} />
              </button>
              <div className="welcome-note">
                <LockKeyhole size={14} /> Tidy operates locally. Changes require
                explicit one-use approval. Verified moves support undo; Trash
                recovery uses Finder.
              </div>
            </section>
          ) : (
            <>
              <div hidden={activeTab !== "organize"}>
                <AssistantWorkspace
                  key={`assistant-${scopeId}`}
                  scopeId={scopeId!}
                  scopeName={basename(scope.path)}
                  indexed={scope.files}
                  scanning={job.running}
                  snapshot={scope.scanned_at}
                  onScan={() => {
                    if (scopeId !== null)
                      void invoke("start_scan", { scopeId, content }).catch(
                        (e) => setError(String(e)),
                      );
                  }}
                  onModels={() => setActiveTab("ai")}
                  onRefresh={() => setRevision((r) => r + 1)}
                  onHistory={() => setActiveTab("history")}
                  onStorage={() => setActiveTab("storage")}
                />
              </div>
              {activeTab === "storage" && (
                <StorageAnalyzer
                  key={`storage-${scopeId}`}
                  scopeId={scopeId}
                  scopeName={basename(scope.path)}
                  snapshot={scope.scanned_at}
                  refreshKey={revision}
                  scanning={job.running}
                  onSelectScope={setScopeId}
                  onAddFolder={() => void add()}
                  onUseWorkingFolder={(path) => void useWorkingFolder(path)}
                  working={busy}
                  onRefreshNeeded={() => setRevision((r) => r + 1)}
                />
              )}
              {activeTab === "history" && (
                <HistoryJournal
                  key={`history-${scopeId}`}
                  scopeId={scopeId}
                  scopeName={basename(scope.path)}
                  onRefreshNeeded={() => setRevision((r) => r + 1)}
                />
              )}
              {activeTab === "ai" && <LocalAi scopeId={scopeId} />}
              {activeTab === "files" && (
                <>
                  <section className="metrics" aria-label="Folder summary">
                    <div>
                      <span className="metric-label">
                        <File size={15} />
                        FILES INDEXED
                      </span>
                      <strong>{scope.files.toLocaleString()}</strong>
                      <small>In the latest scan</small>
                    </div>
                    <div>
                      <span className="metric-label">
                        <HardDrive size={15} />
                        LOGICAL SIZE
                      </span>
                      <strong>{bytes(scope.bytes)}</strong>
                      <small>Not an estimate of recoverable space</small>
                    </div>
                    <div>
                      <span className="metric-label">
                        <ShieldCheck size={15} />
                        SCAN COVERAGE
                      </span>
                      <strong className="coverage">
                        {scope.status === "complete"
                          ? "Finished"
                          : scope.status === "not scanned"
                            ? "Not scanned"
                            : "Partial"}
                      </strong>
                      <small>
                        {scope.omitted
                          ? `${scope.omitted.toLocaleString()} excluded entries`
                          : scope.scanned_at
                            ? "No exclusions reported"
                            : "Ready when you are"}
                      </small>
                    </div>
                  </section>
                  <section className="scan-panel">
                    <div className="scan-title">
                      <span
                        className={`scan-icon ${currentJob && job.running ? "spinning" : ""}`}
                      >
                        <RefreshCw size={16} />
                      </span>
                      <div>
                        <strong>
                          {currentJob && job.running
                            ? `Scanning · ${job.visited.toLocaleString()} entries checked`
                            : currentJob && job.message
                              ? job.message
                              : scope.scanned_at
                                ? `Last scanned ${new Date(scope.scanned_at * 1000).toLocaleString()}`
                                : "Ready to scan"}
                        </strong>
                        <p title={scope.path}>{scope.path}</p>
                      </div>
                    </div>
                    <button
                      className="secondary"
                      disabled={busy || (job.running && !currentJob)}
                      onClick={() =>
                        void (
                          currentJob && job.running
                            ? invoke("cancel_scan")
                            : run(scope.id, content)
                        ).catch((e) => setError(String(e)))
                      }
                    >
                      {currentJob && job.running ? (
                        <>
                          <Square size={13} />
                          Stop scan
                        </>
                      ) : (
                        <>
                          <RefreshCw size={14} />
                          Rescan
                        </>
                      )}
                    </button>
                  </section>
                  <div className="scan-options">
                    <label>
                      <input
                        type="checkbox"
                        checked={content}
                        disabled={job.running}
                        onChange={(e) => setContent(e.target.checked)}
                      />
                      Read file contents for text search and duplicate checks
                    </label>
                    <span>Optional · stored locally</span>
                  </div>
                  {currentJob && job.omissions.length > 0 && (
                    <details
                      className="omissions"
                      open={showOmissions}
                      onToggle={(e) => setShowOmissions(e.currentTarget.open)}
                    >
                      <summary>Why were some entries excluded?</summary>
                      <p>
                        Links, app bundles, protected folders, and unreadable
                        entries are skipped. Showing up to 100 exclusions.
                      </p>
                      <ul>
                        {job.omissions.map((o, i) => (
                          <li key={i}>
                            <code>{o.path}</code>
                            <span>{o.reason}</span>
                          </li>
                        ))}
                      </ul>
                    </details>
                  )}
                  <section className="file-panel">
                    <div className="file-toolbar">
                      <h2>
                        Your files <span>{page.total.toLocaleString()}</span>
                      </h2>
                      <div className="toolbar-controls">
                        <label className="search-box">
                          <Search size={16} />
                          <input
                            aria-label="Search filenames and indexed text"
                            placeholder="Search files or text…"
                            value={query}
                            onChange={(e) => {
                              setQuery(e.target.value);
                              setOffset(0);
                            }}
                            maxLength={100}
                          />
                          {query && (
                            <button
                              className="icon-button"
                              onClick={() => setQuery("")}
                              aria-label="Clear search"
                            >
                              <X size={14} />
                            </button>
                          )}
                        </label>
                        <button
                          className="sort-button"
                          onClick={() => {
                            setSizeSort(!sizeSort);
                            setOffset(0);
                          }}
                        >
                          <ArrowDownWideNarrow size={16} />
                          {sizeSort ? "Largest first" : "Name A–Z"}
                        </button>
                      </div>
                    </div>
                    <div className="table-wrap" aria-busy={loading}>
                      <table>
                        <thead>
                          <tr>
                            <th>Name</th>
                            <th>Size</th>
                            <th>Modified</th>
                          </tr>
                        </thead>
                        <tbody>
                          {page.files.map((f) => (
                            <tr
                              key={f.id}
                              className={
                                selected?.id === f.id ? "selected-row" : ""
                              }
                            >
                              <td>
                                <button
                                  className="file-name"
                                  onClick={() => setSelected(f)}
                                >
                                  <span className={`file-icon ${kind(f.path)}`}>
                                    <File size={17} />
                                  </span>
                                  <span>
                                    <strong>{basename(f.path)}</strong>
                                    <small>
                                      {f.path.includes("/")
                                        ? f.path.slice(
                                            0,
                                            f.path.lastIndexOf("/"),
                                          )
                                        : "In this folder"}
                                    </small>
                                  </span>
                                </button>
                              </td>
                              <td className="size-cell">{bytes(f.size)}</td>
                              <td>
                                {f.modified
                                  ? new Date(
                                      f.modified * 1000,
                                    ).toLocaleDateString(undefined, {
                                      month: "short",
                                      day: "numeric",
                                      year: "numeric",
                                    })
                                  : "—"}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                      {page.files.length === 0 && (
                        <div className="table-empty">
                          {loading ? (
                            <>
                              <LoaderCircle size={20} className="spinning" />
                              Loading your index…
                            </>
                          ) : query ? (
                            "No files match this search."
                          ) : job.running ? (
                            "Your results will appear when the scan finishes."
                          ) : (
                            "Run a scan to see your files here."
                          )}
                        </div>
                      )}
                    </div>
                    <footer className="table-footer">
                      <span>
                        {page.total
                          ? `${offset + 1}–${Math.min(offset + 100, page.total)} of ${page.total.toLocaleString()} files`
                          : "No indexed files"}
                        {loading ? " · Updating…" : ""}
                      </span>
                      <div>
                        <button
                          className="icon-button"
                          aria-label="Previous page"
                          disabled={offset === 0}
                          onClick={() => setOffset(Math.max(0, offset - 100))}
                        >
                          <ChevronLeft size={17} />
                        </button>
                        <button
                          className="icon-button"
                          aria-label="Next page"
                          disabled={offset + 100 >= page.total}
                          onClick={() => setOffset(offset + 100)}
                        >
                          <ChevronRight size={17} />
                        </button>
                      </div>
                    </footer>
                  </section>
                  <div className="bottom-note">
                    <span>
                      <LockKeyhole size={13} />
                      No model needed. Search and scanning work entirely
                      offline.
                    </span>
                    <button onClick={() => setForget(true)}>
                      Forget this folder
                    </button>
                  </div>
                </>
              )}
            </>
          )}
        </div>
      </main>
      {selected && (
        <div className="modal-backdrop" onClick={() => setSelected(null)}>
          <section
            className="dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="file-title"
            onClick={(e) => e.stopPropagation()}
          >
            <button
              className="dialog-close icon-button"
              onClick={() => setSelected(null)}
              aria-label="Close file details"
            >
              <X size={20} />
            </button>
            <div className="eyebrow">INDEXED FILE</div>
            <h2 id="file-title">{basename(selected.path)}</h2>
            <p className="detail-path">{selected.path}</p>
            <div className="detail-meta">
              <span>{bytes(selected.size)}</span>
              <span>{selected.hashed ? "SHA-256 indexed" : "Not hashed"}</span>
            </div>
            {selected.excerpt ? (
              <>
                <h3>Text excerpt</h3>
                <pre>{selected.excerpt}</pre>
              </>
            ) : (
              <p>
                No text excerpt is indexed for this file. Optional text indexing
                supports small plain-text files.
              </p>
            )}
            <p className="muted">
              This is saved index information. Tidy has not changed the file.
            </p>
          </section>
        </div>
      )}
      {forget && (
        <div className="modal-backdrop">
          <section
            className="dialog"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="forget-title"
          >
            <h2 id="forget-title">Forget this folder?</h2>
            <p>
              This removes its saved index and text excerpts from Tidy and
              cancels its running scan. Your original files stay untouched.
            </p>
            <div className="dialog-actions">
              <button className="secondary" onClick={() => setForget(false)}>
                Keep folder
              </button>
              <button
                className="primary"
                disabled={busy}
                onClick={() => void remove()}
              >
                Forget folder
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
