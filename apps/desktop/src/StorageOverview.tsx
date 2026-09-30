import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FolderOpen,
  HardDrive,
  ChevronRight,
  ArrowLeft,
  RefreshCw,
  LoaderCircle,
} from "lucide-react";
type Root = {
  id: number;
  path: string;
  logical_bytes: number;
  files: number;
  scanned_at: number | null;
  status: string;
  omitted: number;
  volume_id: string | null;
  error: string | null;
};
type Volume = {
  id: string;
  label: string;
  total_bytes: number;
  used_bytes: number;
  available_bytes: number;
};
type Overview = {
  roots: Root[];
  volumes: Volume[];
  indexed_bytes: number;
  indexed_files: number;
};
type Folder = {
  path: string;
  logical_bytes: number;
  file_count: number;
  direct_bytes: number;
  direct_files: number;
};
type Page = {
  parent: string;
  folders: Folder[];
  folder_count: number;
  logical_bytes: number;
  file_count: number;
  direct_files: number;
  direct_bytes: number;
  omitted: number;
  status: string;
  scanned_at: number | null;
};
function bytes(n: number) {
  if (n === 0) return "0 B";
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), 4);
  return `${(n / 1024 ** i).toFixed(i > 1 ? 1 : 0)} ${["B", "KB", "MB", "GB", "TB"][i]}`;
}
export default function StorageOverview({
  scopeId,
  refreshKey,
  scanning,
  onSelectScope,
  onAddFolder,
  onSelectContents,
  canSelect,
  onUseWorkingFolder,
  working,
}: {
  scopeId: number;
  refreshKey: string;
  scanning: boolean;
  onSelectScope: (id: number) => void;
  onAddFolder: () => void;
  onSelectContents: (path: string) => void;
  canSelect: boolean;
  onUseWorkingFolder: (path: string) => void;
  working: boolean;
}) {
  const [overview, setOverview] = useState<Overview | null>(null),
    [page, setPage] = useState<Page | null>(null),
    [parent, setParent] = useState(""),
    [offset, setOffset] = useState(0),
    [error, setError] = useState(""),
    [refresh, setRefresh] = useState(0),
    [loading, setLoading] = useState(false);
  useEffect(() => {
    setParent("");
    setOffset(0);
  }, [scopeId]);
  useEffect(() => {
    let current = true;
    setOverview(null);
    setError("");
    if (scanning) return;
    invoke<Overview>("storage_overview")
      .then((v) => {
        if (current) setOverview(v);
      })
      .catch((e) => {
        if (current) setError(String(e));
      });
    return () => {
      current = false;
    };
  }, [scopeId, refreshKey, refresh, scanning]);
  useEffect(() => {
    let current = true;
    setPage(null);
    setLoading(true);
    if (scanning) {
      setLoading(false);
      return;
    }
    invoke<Page>("storage_folder_page", { scopeId, parent, offset })
      .then((v) => {
        if (current) setPage(v);
      })
      .catch((e) => {
        if (current) setError(String(e));
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [scopeId, parent, offset, refreshKey, refresh, scanning]);
  function open(path: string) {
    setParent(path);
    setOffset(0);
    setError("");
  }
  return (
    <section className="storage-overview">
      <div className="storage-overview-title">
        <div>
          <span className="assistant-kicker">ON YOUR DEVICE · READ ONLY</span>
          <h2>Storage by folder</h2>
        </div>
        <button
          className="secondary"
          disabled={scanning}
          onClick={() => setRefresh((n) => n + 1)}
        >
          <RefreshCw size={15} />
          Refresh overview
        </button>
      </div>
      {scanning && <p>Waiting for the scan to finish…</p>}
      {error && (
        <p className="planner-error" role="alert">
          {error}
        </p>
      )}
      {overview ? (
        <>
          <details className="storage-capacity">
            <summary>
              Disk capacity · {bytes(overview.indexed_bytes)} indexed contents
            </summary>
            <div className="volume-grid">
              {overview.volumes.map((v) => (
                <article className="volume-card" key={v.id}>
                  <HardDrive size={20} />
                  <strong>{bytes(v.available_bytes)} available</strong>
                  <small>
                    {v.label} · {bytes(v.used_bytes)} used of{" "}
                    {bytes(v.total_bytes)}
                  </small>
                  <div
                    className="volume-meter"
                    role="meter"
                    aria-label="Volume used bytes"
                    aria-valuemin={0}
                    aria-valuemax={v.total_bytes}
                    aria-valuenow={v.used_bytes}
                  >
                    <span
                      style={{
                        width: `${v.total_bytes ? Math.min(100, (100 * v.used_bytes) / v.total_bytes) : 0}%`,
                      }}
                    />
                  </div>
                </article>
              ))}
              <article className="volume-card">
                <FolderOpen size={20} />
                <strong>
                  {bytes(overview.indexed_bytes)} indexed contents
                </strong>
                <small>
                  {overview.indexed_files.toLocaleString()} distinct indexed
                  paths · overlapping roots counted once
                </small>
              </article>
            </div>
            <p className="storage-coverage">
              Volume figures come from the OS. File analysis covers authorized,
              scanned folders only. Other areas, scan exclusions, APFS snapshots
              and physical allocation are not included in file totals.
            </p>
          </details>
          <div className="storage-overview-title">
            <h3>Base folders you authorized · largest first</h3>
            <button
              className="secondary"
              onClick={onAddFolder}
              disabled={scanning}
            >
              Add a folder
            </button>
          </div>
          <div className="storage-root-grid">
            {overview.roots.map((r) => (
              <button
                key={r.id}
                className={`storage-root ${r.id === scopeId ? "active" : ""}`}
                onClick={() => onSelectScope(r.id)}
              >
                <FolderOpen size={20} />
                <span>
                  <strong>{r.path}</strong>
                  <small>
                    {r.files.toLocaleString()} files · {r.status} · {r.omitted}{" "}
                    exclusions
                    {r.scanned_at
                      ? ` · ${new Date(r.scanned_at * 1000).toLocaleString()}`
                      : " · not scanned"}
                  </small>
                  {r.error && (
                    <small className="assistant-trash">{r.error}</small>
                  )}
                </span>
                <b>{bytes(r.logical_bytes)}</b>
              </button>
            ))}
          </div>
        </>
      ) : (
        !scanning && (
          <p>
            <LoaderCircle size={15} className="spinning" /> Reading authorized
            index totals…
          </p>
        )
      )}
      <div className="folder-usage-header">
        <div>
          <h3>Folder contents</h3>
          <p>
            Recursive file totals, including small files. Empty directories are
            not indexed. Root cards may overlap; do not add their sizes
            together.
          </p>
        </div>
        {parent && (
          <button
            className="secondary"
            onClick={() => open(parent.split("/").slice(0, -1).join("/"))}
          >
            <ArrowLeft size={15} />
            Up one level
          </button>
        )}
      </div>
      <nav className="folder-breadcrumbs" aria-label="Folder path">
        <button onClick={() => open("")}>
          {overview?.roots
            .find((r) => r.id === scopeId)
            ?.path.split("/")
            .at(-1) || "Root"}
        </button>
        {parent
          .split("/")
          .filter(Boolean)
          .map((part, i) => (
            <span key={i}>
              <ChevronRight size={14} />
              <button
                onClick={() =>
                  open(
                    parent
                      .split("/")
                      .slice(0, i + 1)
                      .join("/"),
                  )
                }
              >
                {part}
              </button>
            </span>
          ))}
      </nav>
      {loading && (
        <p>
          <LoaderCircle size={15} className="spinning" /> Summing indexed
          contents…
        </p>
      )}
      {page && (
        <>
          <div className="folder-current">
            <div>
              <strong>{parent || "Selected folder root"}</strong>
              <span>
                {bytes(page.logical_bytes)} in{" "}
                {page.file_count.toLocaleString()} indexed files
              </span>
              <small>
                {page.direct_files.toLocaleString()} files directly here (
                {bytes(page.direct_bytes)}) · {page.status} · {page.omitted}{" "}
                scan exclusions
                {page.scanned_at
                  ? ` · snapshot ${new Date(page.scanned_at * 1000).toLocaleString()}`
                  : ""}
              </small>
            </div>
            <button
              className="secondary"
              disabled={working || scanning}
              onClick={() => onUseWorkingFolder(parent)}
            >
              Use this folder in Ask Tidy
            </button>
            <button
              className="secondary"
              disabled={!canSelect || scanning || !page.file_count}
              title="Select individual indexed files for a reviewed Trash proposal; the directory itself is never deleted"
              onClick={() => onSelectContents(parent)}
            >
              Select contents for review · up to 500
            </button>
          </div>
          {(page.omitted > 0 || page.status !== "complete") && (
            <p className="storage-coverage">
              Known indexed contents only. Excluded or unscanned files may make
              the real folder larger. Refresh the index to update these totals.
            </p>
          )}
          <div className="folder-usage-list">
            {page.folders.map((f) => (
              <div key={f.path} className="folder-usage-row">
                <button className="folder-drill" onClick={() => open(f.path)}>
                  <FolderOpen size={19} />
                  <span>
                    <strong>{f.path.split("/").at(-1)}</strong>
                    <span className="folder-content-meter">
                      <span
                        style={{
                          width: `${page.logical_bytes ? Math.min(100, (100 * f.logical_bytes) / page.logical_bytes) : 0}%`,
                        }}
                      />
                    </span>
                    <small>
                      {f.file_count.toLocaleString()} files recursively
                    </small>
                  </span>
                  <b>{bytes(f.logical_bytes)}</b>
                  <ChevronRight size={17} />
                </button>
                <button
                  className="secondary"
                  disabled={working || scanning}
                  onClick={() => onUseWorkingFolder(f.path)}
                >
                  Use folder
                </button>
                <button
                  className="secondary"
                  disabled={!canSelect || scanning}
                  onClick={() => onSelectContents(f.path)}
                >
                  Select contents
                </button>
              </div>
            ))}
          </div>
          {!page.folders.length && (
            <p>
              No indexed subfolders here. Direct file contents:{" "}
              {bytes(page.direct_bytes)}.
            </p>
          )}
          {page.folder_count > 50 && (
            <div className="folder-pagination">
              <button
                disabled={!offset}
                onClick={() => setOffset((n) => Math.max(0, n - 50))}
              >
                Previous
              </button>
              <span>
                {offset + 1}–{Math.min(offset + 50, page.folder_count)} of{" "}
                {page.folder_count}
              </span>
              <button
                disabled={offset + 50 >= page.folder_count}
                onClick={() => setOffset((n) => n + 50)}
              >
                Next
              </button>
            </div>
          )}
          <p className="storage-coverage">
            Selection replaces the current cleanup selection with the largest
            eligible files in that folder, up to 500. Duplicate keepers and
            protected bundles are skipped. Nothing changes until you review
            exact files and approve. Trash does not immediately free disk space.
          </p>
        </>
      )}
    </section>
  );
}
