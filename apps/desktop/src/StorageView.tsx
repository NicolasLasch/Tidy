import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  ChevronRight,
  FileText,
  Folder,
  FolderPlus,
  HardDrive,
  Lightbulb,
  LoaderCircle,
  Search,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import { size } from "./ChatView";

export type BaseScope = { id: number; path: string; name: string; files: number; bytes: number; status: string };
type Volume = { id: string; label: string; total_bytes: number; used_bytes: number; available_bytes: number };
type Overview = { volumes: Volume[]; indexed_bytes: number; indexed_files: number };
type Child = { path: string; logical_bytes: number; file_count: number };
type Page = {
  folders: Child[];
  folder_count: number;
  direct_files: number;
  direct_bytes: number;
  logical_bytes: number;
  file_count: number;
  omitted: number;
  status: string;
};
type Summary = {
  large_files_count: number;
  large_files_bytes: number;
  duplicate_reclaimable_bytes: number;
  duplicate_groups_count: number;
  old_installers_count: number;
  old_installers_bytes: number;
  dev_artifacts_count: number;
  dev_artifacts_bytes: number;
};
type TrashView = { token: string; actions: { relative_source: string; files?: number; original_size?: number }[] };
const PAGE = 50;

export default function StorageView({
  scopes,
  baseId,
  onBase,
  selected,
  onToggleBase,
  onToggleFolder,
  onAdd,
  onAsk,
  scanning,
  refreshKey,
  onChanged,
}: {
  scopes: BaseScope[];
  baseId: number | null;
  onBase: (id: number | null) => void;
  selected: Set<number>;
  onToggleBase: (id: number, on: boolean) => void;
  onToggleFolder: (baseId: number, path: string, scopeId: number | null, on: boolean) => Promise<void>;
  onAdd: () => void;
  onAsk: (text: string, scopeId: number) => void;
  scanning: boolean;
  refreshKey: number;
  onChanged: () => void;
}) {
  const [overview, setOverview] = useState<Overview | null>(null);
  const [parent, setParent] = useState("");
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<Page | null>(null);
  const [summary, setSummary] = useState<Summary | null>(null);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [error, setError] = useState("");
  const [busyPath, setBusyPath] = useState("");
  const [sheet, setSheet] = useState<TrashView | null>(null);
  const [sheetPath, setSheetPath] = useState("");
  const [working, setWorking] = useState(false);
  const base = scopes.find((s) => s.id === baseId) ?? null;

  useEffect(() => {
    setParent("");
    setOffset(0);
    setQuery("");
  }, [baseId]);
  useEffect(() => {
    if (scanning) return;
    let live = true;
    invoke<Overview>("storage_overview")
      .then((o) => live && setOverview(o))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [scanning, refreshKey]);
  useEffect(() => {
    if (baseId === null || scanning) return;
    let live = true;
    setLoading(true);
    invoke<Page>("storage_folder_page", { scopeId: baseId, parent, offset })
      .then((p) => live && setPage(p))
      .catch((e) => live && setError(String(e)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [baseId, parent, offset, scanning, refreshKey]);
  useEffect(() => {
    setSummary(null);
    if (baseId === null || scanning) return;
    let live = true;
    invoke<{ summary: Summary }>("analyze_storage_scope", { scopeId: baseId })
      .then((r) => live && setSummary(r.summary))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [baseId, scanning, refreshKey]);

  const scopeByPath = useMemo(() => new Map(scopes.map((s) => [s.path, s.id])), [scopes]);
  const selectedPaths = useMemo(
    () => scopes.filter((s) => selected.has(s.id)).map((s) => s.path),
    [scopes, selected],
  );
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (page?.folders ?? []).filter((f) => !q || f.path.toLowerCase().includes(q));
  }, [page, query]);
  const top = page?.folders[0];
  const max = Math.max(1, page?.logical_bytes ?? 1);

  async function toggle(f: Child) {
    if (!base) return;
    setBusyPath(f.path);
    setError("");
    try {
      const abs = `${base.path}/${f.path}`;
      const id = scopeByPath.get(abs) ?? null;
      await onToggleFolder(base.id, f.path, id, !(id !== null && selected.has(id)));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyPath("");
    }
  }
  async function askTrash(path: string) {
    if (!base) return;
    setError("");
    setBusyPath(path);
    try {
      setSheet(await invoke<TrashView>("request_folder_trash_approval", { scopeId: base.id, folders: [path] }));
      setSheetPath(path);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyPath("");
    }
  }
  async function confirmTrash() {
    if (!base || !sheet) return;
    setWorking(true);
    try {
      await invoke("execute_approved_plan", { scopeId: base.id, token: sheet.token });
      setSheet(null);
    } catch (e) {
      setSheet(null);
      setError(String(e));
    } finally {
      setWorking(false);
      onChanged();
    }
  }

  const vol = overview?.volumes[0];
  const crumbs = parent.split("/").filter(Boolean);
  const roots = [...scopes].sort((a, b) => b.bytes - a.bytes);
  const tips: { icon: string; title: string; detail: string; ask?: string; go?: string }[] = [];
  if (summary && base) {
    if (summary.dev_artifacts_count)
      tips.push({ icon: "🧹", title: `${size(summary.dev_artifacts_bytes)} of build & dependency files`, detail: `${summary.dev_artifacts_count.toLocaleString()} files (node_modules, caches, build output) that your tools can recreate.`, ask: "Clean up node_modules and build artifacts" });
    if (summary.old_installers_count)
      tips.push({ icon: "📦", title: `${size(summary.old_installers_bytes)} in old installers`, detail: `${summary.old_installers_count} installers older than 30 days.`, ask: "Delete installers older than 30 days" });
    if (summary.large_files_count)
      tips.push({ icon: "🐘", title: `${summary.large_files_count} files over 50 MB`, detail: `${size(summary.large_files_bytes)} together. Review the biggest.`, ask: "Show my biggest files" });
    if (summary.duplicate_groups_count)
      tips.push({ icon: "👯", title: `${size(summary.duplicate_reclaimable_bytes)} in duplicates`, detail: `${summary.duplicate_groups_count} identical file groups were found.`, ask: "What are my duplicate files?" });
  }
  if (top && base)
    tips.push({ icon: "📁", title: `“${top.path.split("/").at(-1)}” is the heaviest here`, detail: `${size(top.logical_bytes)} in ${top.file_count.toLocaleString()} files — open it to see what inside is big.`, go: top.path });
  if (vol && vol.available_bytes < vol.total_bytes * 0.15)
    tips.push({ icon: "⚠️", title: "Your disk is nearly full", detail: `Only ${size(vol.available_bytes)} free. Emptying the Trash after cleaning is what actually frees space.` });

  return (
    <div className="x-page">
      <div className="x-large-title">
        <div>
          <h1>Storage</h1>
          <p>Real folder sizes, heaviest first. Open a folder to see what inside is big. Switch on only what Tidy may see.</p>
        </div>
        <button className="x-pill" onClick={onAdd} disabled={scanning}>
          <FolderPlus size={16} /> Add
        </button>
      </div>
      {vol && (
        <div className="x-group x-disk">
          <HardDrive size={22} />
          <div>
            <b>{size(vol.used_bytes)} used <span>of {size(vol.total_bytes)}</span></b>
            <i><u style={{ width: `${(100 * vol.used_bytes) / Math.max(1, vol.total_bytes)}%` }} /></i>
            <small>{size(vol.available_bytes)} available · {vol.label} · Tidy indexed {size(overview?.indexed_bytes ?? 0)}</small>
          </div>
        </div>
      )}
      {error && (
        <div className="x-error" role="alert">
          <span>{error}</span>
          <button aria-label="Dismiss" onClick={() => setError("")}><X size={15} /></button>
        </div>
      )}
      {tips.length > 0 && (
        <>
          <h4 className="x-section"><Lightbulb size={14} /> Tips to free space</h4>
          <div className="x-tips">
            {tips.map((t, i) => (
              <button
                key={i}
                className="x-tip"
                disabled={!t.ask && !t.go}
                onClick={() => (t.go ? (setParent(t.go), setOffset(0)) : t.ask && base && onAsk(t.ask, base.id))}
              >
                <span>{t.icon}</span>
                <b>{t.title}</b>
                <small>{t.detail}</small>
                {(t.ask || t.go) && <em>{t.go ? "Open folder" : "Ask Tidy"} <ChevronRight size={13} /></em>}
              </button>
            ))}
          </div>
        </>
      )}
      <nav className="x-crumbs" aria-label="Location">
        <button onClick={() => { onBase(null); setParent(""); }}>Computer</button>
        {base && (
          <>
            <ChevronRight size={13} />
            <button onClick={() => { setParent(""); setOffset(0); }}>{base.name}</button>
          </>
        )}
        {crumbs.map((c, i) => (
          <span key={i}>
            <ChevronRight size={13} />
            <button onClick={() => { setParent(crumbs.slice(0, i + 1).join("/")); setOffset(0); }}>{c}</button>
          </span>
        ))}
      </nav>

      {!base ? (
        <div className="x-group">
          {roots.map((s) => (
            <div key={s.id} className="x-folder-row">
              <Switch on={selected.has(s.id)} onChange={(on) => onToggleBase(s.id, on)} label={`Let Tidy see ${s.name}`} />
              <button className="x-folder-main x-drill" onClick={() => onBase(s.id)}>
                <b>{s.name}</b>
                <small title={s.path}>{s.path} · {s.files.toLocaleString()} files</small>
                <i><u style={{ width: `${Math.max(2, (100 * s.bytes) / Math.max(1, roots[0].bytes))}%` }} /></i>
              </button>
              <em>{size(s.bytes)}</em>
              <ChevronRight size={16} className="x-chev" />
            </div>
          ))}
          {!roots.length && (
            <div className="x-empty small">
              <h2>Nothing indexed yet</h2>
              <p>Add Documents, Downloads or your home folder to see what fills your Mac.</p>
              <button className="x-primary" onClick={onAdd}>Add a folder</button>
            </div>
          )}
        </div>
      ) : (
        <>
          <label className="x-search">
            <Search size={15} />
            <input placeholder="Filter this level" value={query} onChange={(e) => setQuery(e.target.value)} />
          </label>
          {(scanning || loading) && (
            <p className="x-note"><LoaderCircle size={14} className="x-spin" /> {scanning ? "Scanning…" : "Summing folders…"}</p>
          )}
          {page && (
            <p className="x-here">
              <b>{size(page.logical_bytes)}</b> in {page.file_count.toLocaleString()} files here
              {page.omitted > 0 && page.status !== "complete" ? " · partial scan" : ""}
            </p>
          )}
          <div className="x-group">
            {shown.map((f) => {
              const abs = `${base.path}/${f.path}`;
              const covered = selectedPaths.some((p) => abs === p || abs.startsWith(p + "/"));
              const own = scopeByPath.get(abs);
              const on = covered || (own !== undefined && selected.has(own));
              const name = f.path.split("/").at(-1);
              return (
                <div key={f.path} className="x-folder-row">
                  <Switch
                    on={on}
                    busy={busyPath === f.path}
                    disabled={covered && !(own !== undefined && selected.has(own))}
                    onChange={() => void toggle(f)}
                    label={`Let Tidy see ${name}`}
                  />
                  <button className="x-folder-main x-drill" onClick={() => { setParent(f.path); setOffset(0); setQuery(""); }}>
                    <b title={f.path}><Folder size={14} /> {name}</b>
                    <small>{f.file_count.toLocaleString()} files</small>
                    <i><u style={{ width: `${Math.max(2, (100 * f.logical_bytes) / max)}%` }} /></i>
                  </button>
                  <em>{size(f.logical_bytes)}</em>
                  <button className="x-icon-danger" title="Move this whole folder to the Trash" aria-label={`Move ${name} to the Trash`} disabled={scanning || busyPath === f.path} onClick={() => void askTrash(f.path)}>
                    <Trash2 size={16} />
                  </button>
                  <ChevronRight size={16} className="x-chev" />
                </div>
              );
            })}
            {page && page.direct_files > 0 && (
              <div className="x-folder-row plain">
                <FileText size={16} className="x-chev" />
                <span className="x-folder-main"><b>Files directly in this folder</b><small>{page.direct_files.toLocaleString()} files</small></span>
                <em>{size(page.direct_bytes)}</em>
              </div>
            )}
            {page && !shown.length && !page.direct_files && !loading && <p className="x-note">Nothing indexed at this level.</p>}
          </div>
          {page && page.folder_count > PAGE && (
            <div className="x-pager">
              <button disabled={!offset} onClick={() => setOffset(Math.max(0, offset - PAGE))}>Previous</button>
              <span>{offset + 1}–{Math.min(offset + PAGE, page.folder_count)} of {page.folder_count}</span>
              <button disabled={offset + PAGE >= page.folder_count} onClick={() => setOffset(offset + PAGE)}>Next</button>
            </div>
          )}
          <p className="x-foot">Sizes are the real byte totals of every indexed file inside, subfolders included — not Finder’s estimates. The Trash frees space only once emptied.</p>
        </>
      )}
      {sheet && (
        <div className="x-sheet-backdrop" onClick={() => !working && setSheet(null)}>
          <section className="x-sheet" role="dialog" aria-modal="true" onClick={(e) => e.stopPropagation()}>
            <div className="x-grabber" />
            <div className="x-sheet-icon"><ShieldCheck size={26} /></div>
            <h3>Move “{sheetPath.split("/").at(-1)}” to the Trash?</h3>
            <p>The whole folder — {(sheet.actions[0]?.files ?? 0).toLocaleString()} files, {size(sheet.actions[0]?.original_size ?? 0)} — moves as one item. You can put it back from Finder’s Trash.</p>
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
      title={disabled ? "Already included because a parent folder is switched on" : undefined}
      onClick={() => onChange(!on)}
    >
      <span>{busy && <LoaderCircle size={12} className="x-spin" />}</span>
    </button>
  );
}
