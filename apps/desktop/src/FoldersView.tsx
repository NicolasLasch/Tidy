import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  FolderPlus,
  Folder,
  LoaderCircle,
  Search,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import { size } from "./ChatView";

export type BaseScope = {
  id: number;
  path: string;
  name: string;
  files: number;
  bytes: number;
  status: string;
};
type AnyFolder = {
  path: string;
  depth: number;
  logical_bytes: number;
  file_count: number;
  scope_id: number | null;
};
type TrashView = {
  token: string;
  actions: { relative_source: string; files?: number; original_size?: number }[];
};

export default function FoldersView({
  bases,
  baseId,
  onBase,
  selected,
  onToggleBase,
  onToggleFolder,
  onAdd,
  scanning,
  refreshKey,
  onChanged,
}: {
  bases: BaseScope[];
  baseId: number | null;
  onBase: (id: number) => void;
  selected: Set<number>;
  onToggleBase: (id: number, on: boolean) => void;
  onToggleFolder: (
    baseId: number,
    path: string,
    scopeId: number | null,
    on: boolean,
  ) => Promise<void>;
  onAdd: () => void;
  scanning: boolean;
  refreshKey: number;
  onChanged: () => void;
}) {
  const [folders, setFolders] = useState<AnyFolder[]>([]);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(150);
  const [error, setError] = useState("");
  const [busyPath, setBusyPath] = useState("");
  const [sheet, setSheet] = useState<TrashView | null>(null);
  const [sheetPath, setSheetPath] = useState("");
  const [working, setWorking] = useState(false);
  const base = bases.find((b) => b.id === baseId) ?? null;

  useEffect(() => {
    if (baseId === null || scanning) return;
    let live = true;
    setLoading(true);
    invoke<AnyFolder[]>("storage_all_folders", { scopeId: baseId, limit: 1000 })
      .then((rows) => live && setFolders(rows))
      .catch((e) => live && setError(String(e)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [baseId, scanning, refreshKey]);

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return folders
      .filter((f) => !q || f.path.toLowerCase().includes(q))
      .slice(0, limit);
  }, [folders, query, limit]);
  const max = folders[0]?.logical_bytes || 1;

  async function toggle(f: AnyFolder) {
    if (!base) return;
    setBusyPath(f.path);
    setError("");
    try {
      await onToggleFolder(
        base.id,
        f.path,
        f.scope_id,
        !(f.scope_id !== null && selected.has(f.scope_id)),
      );
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
      const view = await invoke<TrashView>("request_folder_trash_approval", {
        scopeId: base.id,
        folders: [path],
      });
      setSheet(view);
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
      await invoke("execute_approved_plan", {
        scopeId: base.id,
        token: sheet.token,
      });
      setSheet(null);
      onChanged();
    } catch (e) {
      setSheet(null);
      setError(String(e));
      onChanged();
    } finally {
      setWorking(false);
    }
  }

  // Only top-level sources are shown as chips; folders switched on inside them are children.
  const roots = bases.filter(
    (b) => !bases.some((o) => o.id !== b.id && b.path.startsWith(o.path + "/")),
  );

  return (
    <div className="x-page">
      <div className="x-large-title">
        <div>
          <h1>Folders</h1>
          <p>
            Heaviest first. Switch on only what Tidy may see — everything else
            stays invisible to the assistant.
          </p>
        </div>
        <button className="x-pill" onClick={onAdd} disabled={scanning}>
          <FolderPlus size={16} /> Add
        </button>
      </div>
      {error && (
        <div className="x-error" role="alert">
          <span>{error}</span>
          <button aria-label="Dismiss" onClick={() => setError("")}>
            <X size={15} />
          </button>
        </div>
      )}
      {!roots.length ? (
        <div className="x-empty small">
          <h2>No folders yet</h2>
          <p>Add a folder like Downloads or Documents to see what takes space.</p>
          <button className="x-primary" onClick={onAdd}>
            Add a folder
          </button>
        </div>
      ) : (
        <>
          <div className="x-segment" role="tablist" aria-label="Source folder">
            {roots.map((b) => (
              <button
                key={b.id}
                role="tab"
                aria-selected={b.id === baseId}
                className={b.id === baseId ? "on" : ""}
                onClick={() => onBase(b.id)}
              >
                {b.name}
                <small>{size(b.bytes)}</small>
              </button>
            ))}
          </div>
          {base && (
            <div className="x-group">
              <label className="x-switch-row">
                <span>
                  <b>{base.name}</b>
                  <small>
                    Everything in this folder ·{" "}
                    {base.files.toLocaleString()} files · {size(base.bytes)}
                  </small>
                </span>
                <Switch
                  on={selected.has(base.id)}
                  onChange={(on) => onToggleBase(base.id, on)}
                  label={`Let Tidy see all of ${base.name}`}
                />
              </label>
            </div>
          )}
          <label className="x-search">
            <Search size={15} />
            <input
              placeholder="Filter folders"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
          {scanning ? (
            <p className="x-note">
              <LoaderCircle size={14} className="x-spin" /> Scanning… folders
              appear when it finishes.
            </p>
          ) : loading ? (
            <p className="x-note">
              <LoaderCircle size={14} className="x-spin" /> Summing folders…
            </p>
          ) : null}
          <div className="x-group">
            {rows.map((f) => {
              const covered = !!base && selected.has(base.id);
              const on = covered || (f.scope_id !== null && selected.has(f.scope_id));
              const name = f.path.split("/").at(-1);
              const parent = f.path.split("/").slice(0, -1).join("/");
              return (
                <div key={f.path} className="x-folder-row">
                  <Switch
                    on={on}
                    busy={busyPath === f.path}
                    disabled={covered}
                    onChange={() => void toggle(f)}
                    label={`Let Tidy see ${name}`}
                  />
                  <span className="x-folder-main">
                    <b title={f.path}>{name}</b>
                    <small>
                      {parent ? `${parent} · ` : ""}
                      {f.file_count.toLocaleString()} files
                    </small>
                    <i>
                      <u
                        style={{
                          width: `${Math.max(2, (100 * f.logical_bytes) / max)}%`,
                        }}
                      />
                    </i>
                  </span>
                  <em>{size(f.logical_bytes)}</em>
                  <button
                    className="x-icon-danger"
                    title="Move this whole folder to the Trash"
                    aria-label={`Move ${name} to the Trash`}
                    disabled={scanning || busyPath === f.path}
                    onClick={() => void askTrash(f.path)}
                  >
                    <Trash2 size={16} />
                  </button>
                </div>
              );
            })}
            {!rows.length && !loading && (
              <p className="x-note">
                {baseId === null
                  ? "Choose a source above."
                  : "No indexed folders here. Scan the folder first."}
              </p>
            )}
          </div>
          {folders.length > limit && !query && (
            <button className="x-more-btn" onClick={() => setLimit((n) => n + 150)}>
              Show more folders
            </button>
          )}
          <p className="x-foot">
            Sizes cover indexed files only. Moving a folder to the Trash keeps it
            recoverable; space is freed once the Trash is emptied.
          </p>
        </>
      )}
      {sheet && (
        <div className="x-sheet-backdrop" onClick={() => !working && setSheet(null)}>
          <section
            className="x-sheet"
            role="dialog"
            aria-modal="true"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="x-grabber" />
            <div className="x-sheet-icon">
              <ShieldCheck size={26} />
            </div>
            <h3>Move “{sheetPath.split("/").at(-1)}” to the Trash?</h3>
            <p>
              The whole folder — {(sheet.actions[0]?.files ?? 0).toLocaleString()} files,{" "}
              {size(sheet.actions[0]?.original_size ?? 0)} — moves as one item.
              You can put it back from Finder’s Trash.
            </p>
            <div className="x-sheet-list">
              <div>
                <Folder size={16} />
                <span>
                  <b>{sheetPath}</b>
                </span>
              </div>
            </div>
            <div className="x-sheet-buttons">
              <button className="x-secondary" disabled={working} onClick={() => setSheet(null)}>
                Cancel
              </button>
              <button className="x-danger" disabled={working} onClick={() => void confirmTrash()}>
                {working ? <LoaderCircle size={16} className="x-spin" /> : <Check size={16} />}
                Move to Trash
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}

function Switch({
  on,
  onChange,
  label,
  busy,
  disabled,
}: {
  on: boolean;
  onChange: (on: boolean) => void;
  label: string;
  busy?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={`x-switch ${on ? "on" : ""}`}
      disabled={busy || disabled}
      title={disabled ? "Already included because the whole folder is switched on" : undefined}
      onClick={(e) => {
        e.preventDefault();
        onChange(!on);
      }}
    >
      <span>{busy && <LoaderCircle size={12} className="x-spin" />}</span>
    </button>
  );
}
