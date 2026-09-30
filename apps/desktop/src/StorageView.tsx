import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  ChevronRight,
  Eye,
  File as FileIcon,
  Folder,
  FolderPlus,
  HardDrive,
  Lightbulb,
  Lock,
  LoaderCircle,
  RefreshCw,
  Search,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import { size } from "./ChatView";
import FileSheet, { type FileTarget } from "./FileSheet";

export type BaseScope = { id: number; path: string; name: string; files: number; bytes: number };
type Entry = { name: string; path: string; bytes: number; files: number; modified: number; is_dir: boolean; denied: boolean; mount: boolean; done: boolean };
type DiskView = {
  path: string;
  running: boolean;
  refreshing: boolean;
  updated_at: number | null;
  entries: Entry[];
  measured_bytes: number;
  volume: { total_bytes: number; used_bytes: number; available_bytes: number } | null;
};
type TrashView = { token: string; actions: { relative_source: string; files?: number; original_size?: number }[] };
type Summary = {
  large_files_count: number; large_files_bytes: number; old_installers_count: number; old_installers_bytes: number;
  dev_artifacts_count: number; dev_artifacts_bytes: number; duplicate_groups_count: number; duplicate_reclaimable_bytes: number;
};
const DATA = "/System/Volumes/Data";
// Folder rows live under the Data volume mount; scopes and the assistant use the plain path.
const norm = (p: string) => (p.startsWith(DATA + "/") ? p.slice(DATA.length) : p);
// Remembered between visits so returning to Storage shows the last folder instantly.
let lastPath: string | null = null;
function ago(t: number | null) {
  if (!t) return "";
  const s = Math.max(0, Math.floor(Date.now() / 1000) - t);
  return s < 60 ? "just now" : s < 3600 ? `${Math.floor(s / 60)} min ago` : s < 86400 ? `${Math.floor(s / 3600)} h ago` : `${Math.floor(s / 86400)} d ago`;
}

const ADVICE: [RegExp, string][] = [
  [/^\.Trash$/i, "Your Trash still holds these files. Emptying it in Finder is what actually frees the space."],
  [/^Downloads$/i, "Usually old installers, zips and things you already opened. Sort by size and clear what you don’t need."],
  [/^(Caches|\.cache)$/i, "Caches are rebuilt automatically, so most of this is safe to clear."],
  [/DerivedData|CoreSimulator|Xcode/i, "Xcode build data and simulators. Safe to delete; Xcode recreates what it needs."],
  [/^MobileSync$/i, "Old iPhone/iPad backups. Delete the ones for devices you no longer use."],
  [/docker|com\.docker/i, "Docker images and volumes. `docker system prune` reclaims most of this."],
  [/^node_modules$/i, "Dependencies your package manager can reinstall."],
  [/^Movies$/i, "Video files are the usual culprits — look for old exports and screen recordings."],
  [/^Library$/i, "App data, caches and logs live here. Open it to see which app is heaviest."],
  [/^Application Support$/i, "Per-app data. The biggest app here is the one to look at first."],
  [/^Applications$/i, "Installed apps. Remove the ones you never open."],
  [/Mobile Documents|iCloud/i, "iCloud Drive copies. Use Optimize Storage in System Settings to keep only recent files."],
  [/Photos Library|\.photoslibrary/i, "Your Photos library. Managed by the Photos app — avoid editing it directly."],
  [/^(private|var|vm)$/i, "System files and swap. Managed by macOS."],
];

export default function StorageView({
  scopes,
  selected,
  onToggleScope,
  onAuthorize,
  onPick,
  onAsk,
  onChanged,
  scanning,
  refreshKey,
}: {
  scopes: BaseScope[];
  selected: Set<number>;
  onToggleScope: (id: number, on: boolean) => Promise<void>;
  onAuthorize: (path: string) => Promise<void>;
  onPick: () => Promise<void>;
  onAsk: (text: string, scopeId: number) => void;
  onChanged: () => void;
  scanning: boolean;
  refreshKey: number;
}) {
  const [root, setRoot] = useState(DATA);
  const [cur, setCur] = useState<string | null>(null);
  const [home, setHome] = useState("");
  const [view, setView] = useState<DiskView | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("");
  const [sheet, setSheet] = useState<TrashView | null>(null);
  const [sheetTarget, setSheetTarget] = useState<{ path: string; scopeId: number } | null>(null);
  const [working, setWorking] = useState(false);
  const [summary, setSummary] = useState<Summary | null>(null);
  const [sort, setSort] = useState<"size" | "name" | "date">("size");
  const [query, setQuery] = useState("");
  const [openFile, setOpenFile] = useState<FileTarget | null>(null);
  const alive = useRef(true);
  useEffect(() => () => void (alive.current = false), []);

  const go = useCallback(
    async (path: string, force = false) => {
      setError("");
      try {
        await invoke("disk_start", { path, force });
        setCur(path);
        setQuery("");
        lastPath = path;
        setView(await invoke<DiskView>("disk_status"));
      } catch (e) {
        if (path === DATA) {
          // Older systems have no separate Data volume; the disk root is the same thing.
          setRoot("/");
          void go("/", force);
        } else setError(String(e));
      }
    },
    [],
  );
  useEffect(() => {
    void invoke<string | null>("disk_home").then((h) => setHome(h ?? ""));
    // Cached sizes appear at once; a background pass refreshes and saves them.
    void go(lastPath ?? DATA);
  }, [go]);
  useEffect(() => {
    if (!view?.running) return;
    const t = setInterval(() => {
      void invoke<DiskView>("disk_status").then((v) => alive.current && setView(v));
    }, 700);
    return () => clearInterval(t);
  }, [view?.running]);

  const scopeFor = useCallback(
    (path: string) =>
      scopes
        .filter((s) => norm(path) === s.path || norm(path).startsWith(s.path + "/"))
        .sort((a, b) => b.path.length - a.path.length)[0] ?? null,
    [scopes],
  );
  const selectedScopeFor = useCallback(
    (path: string) =>
      scopes.some((s) => selected.has(s.id) && (norm(path) === s.path || norm(path).startsWith(s.path + "/"))),
    [scopes, selected],
  );
  const curScope = cur ? scopeFor(cur) : null;
  useEffect(() => {
    setSummary(null);
    if (!curScope || !selected.has(curScope.id) || scanning) return;
    let live = true;
    invoke<{ summary: Summary }>("analyze_storage_scope", { scopeId: curScope.id })
      .then((r) => live && setSummary(r.summary))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [curScope?.id, selected, scanning, refreshKey]);

  const entries = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rows = (view?.entries ?? []).filter((e) => !q || e.name.toLowerCase().includes(q));
    const by = {
      size: (a: Entry, b: Entry) => b.bytes - a.bytes,
      name: (a: Entry, b: Entry) => a.name.localeCompare(b.name, undefined, { numeric: true }),
      date: (a: Entry, b: Entry) => b.modified - a.modified,
    }[sort];
    return [...rows].sort((a, b) => by(a, b) || a.name.localeCompare(b.name));
  }, [view?.entries, sort, query]);
  const biggest = Math.max(1, ...(view?.entries ?? []).map((e) => e.bytes));
  const denied = entries.filter((e) => e.denied).length;
  const vol = view?.volume;
  const atTop = cur === root;
  const other =
    atTop && vol && view && !view.running ? Math.max(0, vol.used_bytes - view.measured_bytes) : 0;
  const crumbs = useMemo(() => {
    if (!cur) return [];
    const rel = cur.startsWith(root) ? cur.slice(root.length) : cur;
    const parts = rel.split("/").filter(Boolean);
    return parts.map((p, i) => ({ label: p, path: (root === "/" ? "" : root) + "/" + parts.slice(0, i + 1).join("/") }));
  }, [cur, root]);

  const tips: { icon: string; title: string; detail: string; path?: string; ask?: string }[] = [];
  if (view && !view.running) {
    for (const e of entries.slice(0, 6)) {
      const advice = ADVICE.find(([re]) => re.test(e.name))?.[1];
      if (advice && e.bytes > 500 * 1024 * 1024)
        tips.push({ icon: "💡", title: `${e.name} · ${size(e.bytes)}`, detail: advice, path: e.path });
    }
    if (other > 20 * 1024 ** 3)
      tips.push({ icon: "🔒", title: `${size(other)} isn’t visible to Tidy`, detail: "That’s system files, APFS snapshots, purgeable space and folders macOS protects. Grant Full Disk Access to see more.", });
  }
  if (summary && curScope) {
    if (summary.dev_artifacts_count) tips.push({ icon: "🧹", title: `${size(summary.dev_artifacts_bytes)} of build & dependency files`, detail: `${summary.dev_artifacts_count.toLocaleString()} files your tools can recreate.`, ask: "Clean up node_modules and build artifacts" });
    if (summary.old_installers_count) tips.push({ icon: "📦", title: `${size(summary.old_installers_bytes)} in old installers`, detail: `${summary.old_installers_count} installers older than 30 days.`, ask: "Delete installers older than 30 days" });
    if (summary.large_files_count) tips.push({ icon: "🐘", title: `${summary.large_files_count} files over 50 MB`, detail: `${size(summary.large_files_bytes)} together.`, ask: "Show my biggest files" });
  }

  async function guard(key: string, fn: () => Promise<void>) {
    setBusy(key);
    setError("");
    try {
      await fn();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy("");
    }
  }
  async function askTrash(e: Entry) {
    const scope = scopeFor(e.path);
    if (!scope || norm(e.path) === scope.path) return;
    await guard(e.path, async () => {
      const rel = norm(e.path).slice(scope.path.length + 1);
      setSheet(await invoke<TrashView>("request_folder_trash_approval", { scopeId: scope.id, folders: [rel] }));
      setSheetTarget({ path: e.path, scopeId: scope.id });
    });
  }
  async function confirmTrash() {
    if (!sheet || !sheetTarget) return;
    setWorking(true);
    try {
      await invoke("execute_approved_plan", { scopeId: sheetTarget.scopeId, token: sheet.token });
      setSheet(null);
      if (cur) await go(cur, true);
    } catch (e) {
      setSheet(null);
      setError(String(e));
    } finally {
      setWorking(false);
      onChanged();
    }
  }

  return (
    <div className="x-page">
      <div className="x-large-title">
        <div>
          <h1>Storage</h1>
          <p>Every folder on your Mac with its real size, biggest first. Open one to see what’s inside.</p>
        </div>
        <div className="x-title-actions">
          <button className="x-pill" onClick={() => void guard("pick", onPick)}>
            <FolderPlus size={15} /> Add folder
          </button>
          <button className="x-pill" disabled={!cur || !!view?.running} onClick={() => cur && void go(cur, true)}>
            <RefreshCw size={15} /> Rescan
          </button>
        </div>
      </div>
      {vol && (
        <div className="x-group x-disk">
          <HardDrive size={22} />
          <div>
            <b>
              {size(vol.used_bytes)} used <span>of {size(vol.total_bytes)}</span>
            </b>
            <i><u style={{ width: `${(100 * vol.used_bytes) / Math.max(1, vol.total_bytes)}%` }} /></i>
            <small>{size(vol.available_bytes)} available</small>
          </div>
        </div>
      )}
      {error && (
        <div className="x-error" role="alert">
          <span>{error}</span>
          <button aria-label="Dismiss" onClick={() => setError("")}><X size={15} /></button>
        </div>
      )}
      {denied > 0 && !view?.running && (
        <div className="x-error" role="status">
          <Lock size={16} />
          <span>macOS blocks {denied} folder{denied === 1 ? "" : "s"} here, so their sizes are incomplete. Allow Tidy under Privacy &amp; Security → Full Disk Access to see everything, then Rescan.</span>
          <button onClick={() => void invoke("open_full_disk_access")}><b>Open Settings</b></button>
        </div>
      )}
      <nav className="x-crumbs" aria-label="Location">
        <button onClick={() => void go(root)}>Macintosh HD</button>
        {home && (
          <span><button onClick={() => void go(root === "/" ? home : root + home)} title="Jump to your home folder">· Home</button></span>
        )}
        {crumbs.map((c) => (
          <span key={c.path}>
            <ChevronRight size={13} />
            <button onClick={() => void go(c.path)}>{c.label}</button>
          </span>
        ))}
      </nav>
      {view && (
        <p className="x-here">
          <b>{size(view.measured_bytes)}</b> in this folder
          {view.running ? (
            <>
              {" "}· <LoaderCircle size={12} className="x-spin" style={{ verticalAlign: "-2px" }} />{" "}
              {view.refreshing ? `updating… showing the last scan (${ago(view.updated_at)})` : "measuring… sizes grow as folders finish"}
            </>
          ) : view.updated_at ? (
            <> · updated {ago(view.updated_at)}</>
          ) : null}
        </p>
      )}
      <div className="x-controls">
        <label className="x-search">
          <Search size={15} />
          <input placeholder="Filter this folder" value={query} onChange={(e) => setQuery(e.target.value)} />
        </label>
        <div className="x-segment" role="group" aria-label="Sort by">
          {(["size", "name", "date"] as const).map((k) => (
            <button key={k} className={sort === k ? "on" : ""} onClick={() => setSort(k)}>
              {k === "size" ? "Size" : k === "name" ? "Name" : "Date"}
            </button>
          ))}
        </div>
      </div>
      <div className="x-group">
        {entries.map((e) => {
          const scope = scopeFor(e.path);
          const isScope = scope?.path === norm(e.path);
          const covered = selectedScopeFor(e.path);
          const inside = !!scope && !isScope;
          const name = e.name;
          return (
            <div key={e.path} className="x-folder-row">
              {e.is_dir && !e.mount ? (
                <Switch
                  on={covered}
                  busy={busy === e.path}
                  disabled={covered && !isScope}
                  label={`Let Tidy see ${name}`}
                  onChange={(on) =>
                    void guard(e.path, async () => {
                      if (isScope && scope) await onToggleScope(scope.id, on);
                      else if (on) await onAuthorize(e.path);
                    })
                  }
                />
              ) : (
                <span style={{ width: 44 }} />
              )}
              <button
                className="x-folder-main"
                disabled={e.mount}
                onClick={() =>
                  e.is_dir
                    ? void go(e.path)
                    : setOpenFile({ path: e.path, name: e.name, bytes: e.bytes, modified: e.modified, isDir: false, scope: scopeFor(e.path) ? { id: scopeFor(e.path)!.id, path: scopeFor(e.path)!.path } : null })
                }
              >
                <b title={e.path}>
                  {e.is_dir ? <Folder size={14} /> : <FileIcon size={14} />} {name}
                  {e.denied && <span className="x-badge warn">Protected</span>}
                  {e.mount && <span className="x-badge">Other volume</span>}
                  {!e.done && <LoaderCircle size={12} className="x-spin" />}
                </b>
                <small>
                  {e.is_dir ? `${e.files.toLocaleString()} files` : "file"}
                  {e.modified ? ` · ${new Date(e.modified * 1000).toLocaleDateString()}` : ""}
                </small>
                <i><u style={{ width: `${Math.max(1, (100 * e.bytes) / biggest)}%` }} /></i>
              </button>
              <em>{e.mount ? "—" : size(e.bytes)}</em>
              <button className="x-icon-btn" title="Show in Finder" aria-label={`Show ${name} in Finder`} onClick={() => void invoke("reveal_path", { path: e.path }).catch((x) => setError(String(x)))}>
                <Eye size={16} />
              </button>
              {e.is_dir && inside && (
                <button className="x-icon-btn danger" title="Move this whole folder to the Trash" aria-label={`Move ${name} to the Trash`} disabled={!e.done || busy === e.path} onClick={() => void askTrash(e)}>
                  <Trash2 size={16} />
                </button>
              )}
              {e.is_dir && !e.mount && <ChevronRight size={16} className="x-chev" />}
            </div>
          );
        })}
        {other > 0 && (
          <div className="x-folder-row plain">
            <span style={{ width: 44 }} />
            <span className="x-folder-main">
              <b><Lock size={14} /> System, snapshots &amp; protected data</b>
              <small>Space macOS uses that can’t be read folder by folder</small>
              <i><u style={{ width: `${Math.max(1, (100 * other) / biggest)}%`, opacity: 0.45 }} /></i>
            </span>
            <em>{size(other)}</em>
          </div>
        )}
        {!entries.length && (
          <p className="x-note">{view?.running ? "Reading…" : "This folder is empty or unreadable."}</p>
        )}
      </div>
      {tips.length > 0 && (
        <>
          <h4 className="x-section"><Lightbulb size={14} /> Tips to free space</h4>
          <div className="x-tips">
            {tips.slice(0, 6).map((t, i) => (
              <button
                key={i}
                className="x-tip"
                disabled={!t.path && !t.ask}
                onClick={() => (t.path ? void go(t.path) : t.ask && curScope && onAsk(t.ask, curScope.id))}
              >
                <span>{t.icon}</span>
                <b>{t.title}</b>
                <small>{t.detail}</small>
                {(t.path || t.ask) && <em>{t.path ? "Open" : "Ask Tidy"} <ChevronRight size={13} /></em>}
              </button>
            ))}
          </div>
        </>
      )}
      <p className="x-foot">
        Sizes are allocated bytes from the disk itself, the same basis as Finder, including hidden files and subfolders. The switch lets Tidy’s assistant see and change a folder; leave it off and Tidy never touches it. Trash frees space only once emptied.
      </p>
      {scopes.length === 0 && (
        <button className="x-pill" onClick={() => void guard("add", () => onAuthorize(home || "/Users"))}>
          <FolderPlus size={15} /> Let Tidy manage my home folder
        </button>
      )}
      {openFile && (
        <FileSheet
          file={openFile}
          onClose={() => setOpenFile(null)}
          onTrashed={() => {
            setOpenFile(null);
            if (cur) void go(cur, true);
            onChanged();
          }}
        />
      )}
      {sheet && sheetTarget && (
        <div className="x-sheet-backdrop" onClick={() => !working && setSheet(null)}>
          <section className="x-sheet" role="dialog" aria-modal="true" onClick={(e) => e.stopPropagation()}>
            <div className="x-grabber" />
            <div className="x-sheet-icon"><ShieldCheck size={26} /></div>
            <h3>Move “{sheetTarget.path.split("/").at(-1)}” to the Trash?</h3>
            <p>
              The whole folder — {(sheet.actions[0]?.files ?? 0).toLocaleString()} files, {size(sheet.actions[0]?.original_size ?? 0)} — moves as one item. Put it back from Finder’s Trash any time.
            </p>
            <div className="x-sheet-buttons">
              <button className="x-secondary" disabled={working} onClick={() => setSheet(null)}>Cancel</button>
              <button className="x-danger" disabled={working} onClick={() => void confirmTrash()}>
                {working ? <LoaderCircle size={16} className="x-spin" /> : <Check size={16} />} Move to Trash
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}

function Switch({ on, onChange, label, busy, disabled }: { on: boolean; onChange: (on: boolean) => void; label: string; busy?: boolean; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={`x-switch ${on ? "on" : ""}`}
      disabled={busy || disabled}
      title={disabled ? "Included because a parent folder is switched on" : "Let Tidy’s assistant see and organize this folder"}
      onClick={() => onChange(!on)}
    >
      <span>{busy && <LoaderCircle size={12} className="x-spin" />}</span>
    </button>
  );
}
