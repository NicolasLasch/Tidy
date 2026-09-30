import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { File as FileIcon, Folder, LoaderCircle, Trash2 } from "lucide-react";
import { size } from "./ChatView";

export type FileTarget = {
  /** Absolute path as it exists on disk. */
  path: string;
  name: string;
  bytes: number;
  modified: number;
  isDir: boolean;
  /** The authorized folder containing it, when there is one (needed to move it to the Trash). */
  scope: { id: number; path: string } | null;
};

export default function FileSheet({
  file,
  onClose,
  onTrashed,
}: {
  file: FileTarget;
  onClose: () => void;
  onTrashed: () => void;
}) {
  const [confirm, setConfirm] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const shown = file.path.replace(/^\/System\/Volumes\/Data(?=\/)/, "");
  const relative = file.scope ? shown.slice(file.scope.path.length + 1) : "";

  async function trash() {
    if (!file.scope) return;
    setWorking(true);
    setError("");
    try {
      const scopeId = file.scope.id;
      const view = file.isDir
        ? await invoke<{ token: string }>("request_folder_trash_approval", { scopeId, folders: [relative] })
        : await (async () => {
            const id = await invoke<number>("file_id_for_path", { scopeId, relative });
            return invoke<{ token: string }>("request_plan_approval", {
              scopeId,
              rationale: `Move ${relative} to the Trash`,
              actions: [{ action: "trash", source: id }],
            });
          })();
      await invoke("execute_approved_plan", { scopeId, token: view.token });
      onTrashed();
    } catch (e) {
      setError(String(e));
      setConfirm(false);
    } finally {
      setWorking(false);
    }
  }
  return (
    <div className="x-sheet-backdrop" onClick={() => !working && onClose()}>
      <section className="x-sheet" role="dialog" aria-modal="true" onClick={(e) => e.stopPropagation()}>
        <div className="x-grabber" />
        <h3>{file.name}</h3>
        <p>
          {size(file.bytes)}
          {file.modified ? ` · modified ${new Date(file.modified * 1000).toLocaleDateString()}` : ""}
        </p>
        <div className="x-sheet-list">
          <div>
            {file.isDir ? <Folder size={16} /> : <FileIcon size={16} />}
            <span>
              <b>Location</b>
              <small style={{ overflowWrap: "anywhere", userSelect: "text" }}>{shown}</small>
            </span>
          </div>
        </div>
        {!file.scope && (
          <p className="x-foot">This item is outside the folders Tidy manages. You can show it in Finder; switch its folder on in Storage to move it to the Trash from here.</p>
        )}
        {error && <div className="x-error" role="alert"><span>{error}</span></div>}
        <div className="x-sheet-buttons">
          <button className="x-secondary" onClick={() => void invoke("reveal_path", { path: file.path }).catch((e) => setError(String(e)))}>
            Show in Finder
          </button>
          {file.scope &&
            (confirm ? (
              <button className="x-danger" disabled={working} onClick={() => void trash()}>
                {working ? <LoaderCircle size={16} className="x-spin" /> : <Trash2 size={16} />} Confirm: move to Trash
              </button>
            ) : (
              <button className="x-secondary" onClick={() => setConfirm(true)}>
                <Trash2 size={16} /> Move to Trash
              </button>
            ))}
        </div>
        <div className="x-sheet-buttons">
          <button className="x-secondary" disabled={working} onClick={onClose}>Close</button>
        </div>
      </section>
    </div>
  );
}
