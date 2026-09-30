import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, ChevronDown, ChevronRight, Eye, LoaderCircle, RotateCcw, ShieldCheck, Undo2, X } from "lucide-react";
import { size } from "./ChatView";

type Tx = { id: number; scope_id: number; state: string; rationale: string; actions_count: number; created_at: number };
type Step = {
  id: number;
  action_type: string;
  source_relative: string;
  destination_relative: string | null;
  original_size: number;
  trash_location?: string | null;
  old_mode?: number | null;
  new_mode?: number | null;
  state: string;
  error: string | null;
};
type Detail = { summary: Tx; steps: Step[] };
type Approval = { token: string; tx_id: number; actions_count: number; actions: { type: string; relative_source: string; relative_dest?: string }[] };
export type HistoryScope = { id: number; path: string; name: string };

const LABEL: Record<string, string> = {
  move: "Moved", rename: "Renamed", copy: "Copied", trash: "Trashed file", trash_dir: "Trashed folder",
  create_dir: "Created folder", move_dir: "Moved folder", permissions: "Permissions", restore: "Put back",
};
const STATE: Record<string, string> = {
  verified: "Done", applied: "Done", undone: "Undone", needs_recovery: "Needs a look", applying: "Running", prepared: "Not applied", approved: "Not applied",
};
const isTrash = (t: string) => t === "trash" || t === "trash_dir";

export default function HistoryView({
  scopes,
  refreshKey,
  onChanged,
  onRescan,
}: {
  scopes: HistoryScope[];
  refreshKey: number;
  onChanged: () => void;
  onRescan: (scopeId: number) => void;
}) {
  const [items, setItems] = useState<Tx[]>([]);
  const [open, setOpen] = useState<number | null>(null);
  const [details, setDetails] = useState<Map<number, Detail>>(new Map());
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [undo, setUndo] = useState<{ tx: Tx; approval: Approval } | null>(null);
  const [putBack, setPutBack] = useState<{ tx: Tx; step: Step } | null>(null);
  const [working, setWorking] = useState(false);
  const scopeOf = (id: number) => scopes.find((s) => s.id === id);
  const abs = (tx: Tx, rel: string) => `${scopeOf(tx.scope_id)?.path ?? ""}/${rel}`;

  const load = useCallback(async () => {
    setError("");
    try {
      setItems((await invoke<Tx[]>("list_journal_history", { scopeId: null })).sort((a, b) => b.id - a.id));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => void load(), [load, refreshKey]);

  const loadDetail = useCallback(async (id: number) => {
    const d = await invoke<Detail | null>("get_transaction_detail", { txId: id }).catch(() => null);
    if (d) setDetails((m) => new Map(m).set(id, d));
  }, []);
  async function toggle(id: number) {
    if (open === id) return setOpen(null);
    setOpen(id);
    if (!details.has(id)) await loadDetail(id);
  }
  async function askUndo(tx: Tx) {
    setError("");
    setWorking(true);
    try {
      setUndo({ tx, approval: await invoke<Approval>("request_undo_approval", { scopeId: tx.scope_id, txId: tx.id }) });
    } catch (e) {
      setError(String(e));
    } finally {
      setWorking(false);
    }
  }
  async function runUndo() {
    if (!undo) return;
    setWorking(true);
    try {
      await invoke("execute_approved_undo", { scopeId: undo.tx.scope_id, originalTxId: undo.tx.id, undoToken: undo.approval.token });
      setUndo(null);
      await load();
      onChanged();
    } catch (e) {
      setUndo(null);
      setError(String(e));
    } finally {
      setWorking(false);
    }
  }
  async function runPutBack() {
    if (!putBack) return;
    setWorking(true);
    try {
      await invoke("restore_from_trash", { scopeId: putBack.tx.scope_id, txId: putBack.tx.id, stepId: putBack.step.id });
      const { tx } = putBack;
      setPutBack(null);
      await loadDetail(tx.id);
      await load();
      onChanged();
      onRescan(tx.scope_id);
    } catch (e) {
      setPutBack(null);
      setError(String(e));
    } finally {
      setWorking(false);
    }
  }
  const undoable = (d?: Detail) =>
    !!d && ["verified", "applied", "needs_recovery"].includes(d.summary.state) && !d.steps.some((s) => isTrash(s.action_type) || s.action_type === "restore") && d.steps.some((s) => s.action_type !== "create_dir");
  const reveal = (path: string) => void invoke("reveal_path", { path }).catch((e) => setError(String(e)));

  return (
    <div className="x-page">
      <div className="x-large-title">
        <div>
          <h1>History</h1>
          <p>Everything Tidy has changed, across all your folders. Open an item to see exactly where things were and where they went. Trashed items can be put back in one click.</p>
        </div>
      </div>
      {error && (
        <div className="x-error" role="alert"><span>{error}</span><button aria-label="Dismiss" onClick={() => setError("")}><X size={15} /></button></div>
      )}
      <div className="x-group">
        {items.map((tx) => {
          const d = details.get(tx.id);
          return (
            <div key={tx.id} className="x-tx">
              <button className="x-tx-head" onClick={() => void toggle(tx.id)} aria-expanded={open === tx.id}>
                {open === tx.id ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                <span>
                  <b>{tx.rationale || `Change #${tx.id}`}</b>
                  <small>
                    {scopeOf(tx.scope_id)?.name ?? "Folder"} · {tx.actions_count} {tx.actions_count === 1 ? "item" : "items"} · {new Date(tx.created_at * 1000).toLocaleString()}
                  </small>
                </span>
                <span className={`x-badge ${tx.state === "needs_recovery" ? "warn" : ""}`}>{STATE[tx.state] ?? tx.state}</span>
              </button>
              {open === tx.id && (
                <>
                  <div className="x-steps">
                    {!d && <LoaderCircle size={14} className="x-spin" />}
                    {d?.steps.slice(0, 200).map((s) => {
                      const from = abs(tx, s.source_relative);
                      const to = s.destination_relative ? abs(tx, s.destination_relative) : null;
                      const restored = s.state === "restored";
                      return (
                        <div key={s.id} className="x-step-block">
                          <div className="x-step">
                            <b>{LABEL[s.action_type] ?? s.action_type}</b>
                            <span>
                              <code className="x-path">{from}</code>
                              {to && <code className="x-path">→ {to}</code>}
                              {isTrash(s.action_type) && (
                                <code className="x-path">
                                  {restored ? "Put back to the original place" : s.trash_location ? `Now in the Trash: ${s.trash_location}` : "Trash location was not recorded — check Finder’s Trash"}
                                </code>
                              )}
                              <small>
                                {s.action_type === "permissions" ? `${s.old_mode?.toString(8)} → ${s.new_mode?.toString(8)} · ` : ""}
                                {size(s.original_size)} · {restored ? "restored" : s.state}
                                {s.error ? ` · ${s.error}` : ""}
                              </small>
                            </span>
                          </div>
                          <div className="x-tx-actions">
                            {isTrash(s.action_type) && s.state === "verified" && (
                              <>
                                <button className="x-pill" onClick={() => setPutBack({ tx, step: s })}>
                                  <RotateCcw size={14} /> Put back
                                </button>
                                {s.trash_location && (
                                  <button className="x-secondary small" onClick={() => void invoke("reveal_trash_file", { scopeId: tx.scope_id, txId: tx.id, stepId: s.id }).catch((e) => setError(String(e)))}>
                                    <Eye size={14} /> Show in Trash
                                  </button>
                                )}
                              </>
                            )}
                            {!isTrash(s.action_type) && s.state !== "pending" && (
                              <button className="x-secondary small" onClick={() => reveal(to ?? from)}>
                                <Eye size={14} /> Show in Finder
                              </button>
                            )}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                  <div className="x-tx-actions">
                    {undoable(d) && (
                      <button className="x-pill" disabled={working} onClick={() => void askUndo(tx)}>
                        <Undo2 size={15} /> Undo all
                      </button>
                    )}
                  </div>
                </>
              )}
            </div>
          );
        })}
        {!items.length && <p className="x-note">{loading ? "Loading…" : "Nothing has been changed yet. Anything Tidy does will be listed here."}</p>}
      </div>
      {undo && (
        <div className="x-sheet-backdrop" onClick={() => !working && setUndo(null)}>
          <section className="x-sheet" role="dialog" aria-modal="true" onClick={(e) => e.stopPropagation()}>
            <div className="x-grabber" />
            <div className="x-sheet-icon"><ShieldCheck size={26} /></div>
            <h3>Undo this change?</h3>
            <p>Tidy checks nothing was edited since, then puts everything back. Folders it created stay (they may hold newer files).</p>
            <div className="x-sheet-list">
              {undo.approval.actions.slice(0, 30).map((a, i) => (
                <div key={i}><span><b>{a.relative_source}</b><small>{a.relative_dest ? `→ ${a.relative_dest}` : a.type}</small></span></div>
              ))}
            </div>
            <div className="x-sheet-buttons">
              <button className="x-secondary" disabled={working} onClick={() => setUndo(null)}>Cancel</button>
              <button className="x-primary" disabled={working} onClick={() => void runUndo()}>
                {working ? <LoaderCircle size={16} className="x-spin" /> : <Check size={16} />} Undo
              </button>
            </div>
          </section>
        </div>
      )}
      {putBack && (
        <div className="x-sheet-backdrop" onClick={() => !working && setPutBack(null)}>
          <section className="x-sheet" role="dialog" aria-modal="true" onClick={(e) => e.stopPropagation()}>
            <div className="x-grabber" />
            <div className="x-sheet-icon"><RotateCcw size={26} /></div>
            <h3>Put it back?</h3>
            <p>It moves from the Trash back to its original place. Tidy refuses if something already exists there, and never overwrites.</p>
            <div className="x-sheet-list">
              <div><span><b>To</b><small className="x-path">{abs(putBack.tx, putBack.step.source_relative)}</small></span></div>
              {putBack.step.trash_location && <div><span><b>From the Trash</b><small className="x-path">{putBack.step.trash_location}</small></span></div>}
            </div>
            <div className="x-sheet-buttons">
              <button className="x-secondary" disabled={working} onClick={() => setPutBack(null)}>Cancel</button>
              <button className="x-primary" disabled={working} onClick={() => void runPutBack()}>
                {working ? <LoaderCircle size={16} className="x-spin" /> : <Check size={16} />} Put back
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
