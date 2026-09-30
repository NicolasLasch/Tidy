import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  History,
  RotateCcw,
  CheckCircle2,
  ArrowRight,
  ShieldCheck,
  Clock,
  LoaderCircle,
  ChevronDown,
  ChevronRight,
  AlertCircle,
} from "lucide-react";

export type JournalState =
  | "prepared"
  | "approved"
  | "applying"
  | "applied"
  | "verified"
  | "needs_recovery"
  | "undone";

export type TransactionSummary = {
  id: number;
  tx_uuid: string;
  scope_id: number;
  state: JournalState;
  rationale: string;
  actions_count: number;
  created_at: number;
  applied_at: number | null;
  verified_at: number | null;
  undone_at: number | null;
};

export type JournalStepRecord = {
  id: number;
  transaction_id: number;
  step_order: number;
  action_type: string;
  source_relative: string;
  destination_relative: string | null;
  original_size: number;
  original_modified: number;
  trash_location?: string | null;
  old_mode?: number | null;
  new_mode?: number | null;
  state: string;
  error: string | null;
};

export type TransactionDetail = {
  summary: TransactionSummary;
  steps: JournalStepRecord[];
};

export type ApprovalView = {
  approval_id: string;
  token: string;
  tx_id: number;
  scope_id: number;
  expires_at: number;
  actions_count: number;
  actions: any[];
};

export type ExecutionReport = {
  transaction_id: number;
  tx_uuid: string;
  actions_applied: number;
  verified: boolean;
  duration_ms: number;
  rationale: string;
};

function formatTimestamp(epochSecs: number) {
  if (!epochSecs) return "—";
  const date = new Date(epochSecs * 1000);
  return date.toLocaleString();
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 4);
  return `${(bytes / 1024 ** i).toFixed(i > 1 ? 1 : 0)} ${["B", "KB", "MB", "GB", "TB"][i]}`;
}

export default function HistoryJournal({
  scopeId,
  scopeName,
  onRefreshNeeded,
}: {
  scopeId: number | null;
  scopeName?: string;
  onRefreshNeeded?: () => void;
}) {
  const [history, setHistory] = useState<TransactionSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [expandedTxId, setExpandedTxId] = useState<number | null>(null);
  const [detailsMap, setDetailsMap] = useState<Map<number, TransactionDetail>>(
    new Map(),
  );

  // Undo approval state
  const [undoTxId, setUndoTxId] = useState<number | null>(null);
  const [undoApproval, setUndoApproval] = useState<ApprovalView | null>(null);
  const [undoing, setUndoing] = useState(false);
  const [undoSuccess, setUndoSuccess] = useState<ExecutionReport | null>(null);

  useEffect(() => {
    loadHistory();
  }, [scopeId]);

  async function loadHistory() {
    if (!isTauri()) return;
    setLoading(true);
    setError("");
    try {
      const res = await invoke<TransactionSummary[]>("list_journal_history", {
        scopeId,
      });
      setHistory(res);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function toggleExpand(txId: number) {
    if (expandedTxId === txId) {
      setExpandedTxId(null);
      return;
    }
    setExpandedTxId(txId);
    if (!detailsMap.has(txId)) {
      try {
        const detail = await invoke<TransactionDetail | null>(
          "get_transaction_detail",
          {
            txId,
          },
        );
        if (detail) {
          setDetailsMap((prev) => new Map(prev).set(txId, detail));
        }
      } catch (e) {
        setError(String(e));
      }
    }
  }

  async function requestUndo(txId: number) {
    if (scopeId === null) return;
    setError("");
    setUndoTxId(txId);
    setUndoApproval(null);
    setUndoSuccess(null);
    setUndoing(true);
    try {
      const approval = await invoke<ApprovalView>("request_undo_approval", {
        scopeId,
        txId,
      });
      setUndoApproval(approval);
    } catch (e) {
      setError(String(e));
      setUndoTxId(null);
    } finally {
      setUndoing(false);
    }
  }

  async function executeUndo() {
    if (scopeId === null || undoTxId === null || !undoApproval) return;
    setError("");
    setUndoing(true);
    try {
      const report = await invoke<ExecutionReport>("execute_approved_undo", {
        scopeId,
        originalTxId: undoTxId,
        undoToken: undoApproval.token,
      });
      setUndoSuccess(report);
      setUndoApproval(null);
      await loadHistory();
      if (onRefreshNeeded) onRefreshNeeded();
    } catch (e) {
      setError(String(e));
    } finally {
      setUndoing(false);
    }
  }

  return (
    <div className="planner-container">
      <div className="planner-header">
        <div>
          <div className="planner-badge">
            <History size={13} />
            <span>Phase 5 Safety Journal & History</span>
          </div>
          <h2>Transaction History: {scopeName || "All Scopes"}</h2>
          <p className="planner-subtitle">
            Durable SQLite audit journal of all executed organization and
            storage operations. Verified actions can be safely undone.
          </p>
        </div>
        <button
          type="button"
          className="btn-secondary-sm"
          onClick={() => void loadHistory()}
          disabled={loading}
          style={{ padding: "8px 14px", fontSize: "12px" }}
        >
          {loading ? (
            <LoaderCircle size={14} className="spinning" />
          ) : (
            <Clock size={14} />
          )}
          <span>Refresh History</span>
        </button>
      </div>

      {error && (
        <div className="planner-error" role="alert">
          <AlertCircle size={16} />
          <span>{error}</span>
        </div>
      )}

      {undoSuccess && (
        <div className="execution-success-banner">
          <CheckCircle2 size={20} className="success-icon" />
          <div>
            <strong>Undo Completed Successfully!</strong>
            <p>
              Reversed {undoSuccess.actions_applied} action(s) in{" "}
              {undoSuccess.duration_ms}ms. Files have been restored to their
              original locations.
            </p>
          </div>
        </div>
      )}

      {/* Undo confirmation modal */}
      {undoApproval && (
        <div className="approval-modal-backdrop">
          <div className="approval-modal">
            <div className="approval-modal-header">
              <RotateCcw size={20} className="undo-modal-icon" />
              <div>
                <h3>Confirm Reversible Undo</h3>
                <p>Transaction #{undoApproval.tx_id} will be reversed.</p>
              </div>
            </div>

            <div className="approval-notice">
              <ShieldCheck size={16} />
              <span>
                Review {undoApproval.actions_count} inverse actions below.
                Copies go to Trash; permissions restore their recorded mode;
                moves return to their original paths. No overwrites.
              </span>
            </div>

            <div className="assistant-approved-moves">
              {undoApproval.actions.map((a, i) => (
                <div key={i}>
                  <span>{a.relative_source}</span>
                  <strong>
                    {a.type === "trash"
                      ? "Move copied file to Trash"
                      : a.type === "permissions"
                        ? `Permissions ${a.old_mode.toString(8)} → ${a.new_mode.toString(8)}`
                        : a.relative_dest}
                  </strong>
                </div>
              ))}
            </div>
            <div className="approval-modal-actions">
              <button
                type="button"
                className="btn-secondary"
                onClick={() => {
                  setUndoApproval(null);
                  setUndoTxId(null);
                }}
                disabled={undoing}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn-primary-undo"
                onClick={() => void executeUndo()}
                disabled={undoing}
              >
                {undoing ? (
                  <>
                    <LoaderCircle size={14} className="spinning" />
                    <span>Executing Undo…</span>
                  </>
                ) : (
                  <>
                    <RotateCcw size={14} />
                    <span>Confirm & Execute Undo</span>
                  </>
                )}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* History table */}
      <div className="history-table-wrap">
        {history.length === 0 ? (
          <div className="no-actions-notice" style={{ padding: "40px" }}>
            <History size={28} />
            <span>
              No transactions have been recorded in this scope yet. Plan and
              execute an organization or cleanup operation to see it audited
              here.
            </span>
          </div>
        ) : (
          <div className="history-list">
            {history.map((tx) => {
              const isExpanded = expandedTxId === tx.id;
              const detail = detailsMap.get(tx.id);
              const canUndo =
                (tx.state === "verified" ||
                  tx.state === "applied" ||
                  tx.state === "needs_recovery") &&
                tx.actions_count > 0 &&
                !!detail &&
                !detail.steps.some((s) => s.action_type === "trash");

              return (
                <div key={tx.id} className="history-card">
                  <div
                    className="history-card-header"
                    onClick={() => void toggleExpand(tx.id)}
                  >
                    <button type="button" className="expand-chevron-btn">
                      {isExpanded ? (
                        <ChevronDown size={18} />
                      ) : (
                        <ChevronRight size={18} />
                      )}
                    </button>

                    <div className="tx-meta-col">
                      <div className="tx-top-line">
                        <span className="tx-id-badge">TX #{tx.id}</span>
                        <span className={`tx-state-badge ${tx.state}`}>
                          {tx.state.replace("_", " ").toUpperCase()}
                        </span>
                        <span className="tx-date">
                          {formatTimestamp(tx.created_at)}
                        </span>
                      </div>
                      <p className="tx-rationale">{tx.rationale}</p>
                    </div>

                    <div className="tx-actions-col">
                      <span className="tx-count-chip">
                        {tx.actions_count} action
                        {tx.actions_count === 1 ? "" : "s"}
                      </span>

                      {canUndo && (
                        <button
                          type="button"
                          className="undo-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            void requestUndo(tx.id);
                          }}
                          disabled={undoing && undoTxId === tx.id}
                        >
                          <RotateCcw size={13} />
                          <span>Undo</span>
                        </button>
                      )}
                    </div>
                  </div>

                  {/* Expanded step details */}
                  {isExpanded && (
                    <div className="history-detail-panel">
                      {!detail ? (
                        <div className="loading-steps">
                          <LoaderCircle size={16} className="spinning" />
                          <span>Loading step details…</span>
                        </div>
                      ) : (
                        <table className="history-steps-table">
                          <thead>
                            <tr>
                              <th style={{ width: "60px" }}>Type</th>
                              <th>Source File</th>
                              <th style={{ width: "30px" }}></th>
                              <th>Destination</th>
                              <th style={{ width: "80px", textAlign: "right" }}>
                                Size
                              </th>
                              <th
                                style={{ width: "90px", textAlign: "center" }}
                              >
                                Status
                              </th>
                            </tr>
                          </thead>
                          <tbody>
                            {detail.steps.map((step) => (
                              <tr key={step.id}>
                                <td>
                                  <span
                                    className={`action-badge ${step.action_type}`}
                                  >
                                    {step.action_type.toUpperCase()}
                                  </span>
                                </td>
                                <td className="source-path">
                                  {step.source_relative}
                                </td>
                                <td className="arrow-cell">
                                  <ArrowRight size={14} />
                                </td>
                                <td className="dest-path">
                                  {step.action_type === "trash" ? (
                                    <button
                                      type="button"
                                      className="btn-link"
                                      title={
                                        step.trash_location ||
                                        "No recorded receipt"
                                      }
                                      onClick={() =>
                                        void invoke("reveal_trash_file", {
                                          scopeId: tx.scope_id,
                                          txId: tx.id,
                                          stepId: step.id,
                                        }).catch((e) => setError(String(e)))
                                      }
                                    >
                                      Reveal in Finder Trash
                                    </button>
                                  ) : (
                                    <code>
                                      {step.action_type === "permissions"
                                        ? `${step.old_mode?.toString(8)} → ${step.new_mode?.toString(8)}`
                                        : step.destination_relative || "—"}
                                    </code>
                                  )}
                                </td>
                                <td className="size-cell">
                                  {formatBytes(step.original_size)}
                                </td>
                                <td style={{ textAlign: "center" }}>
                                  <span
                                    className={`step-status-chip ${step.state}`}
                                  >
                                    {step.state.toUpperCase()}
                                  </span>
                                </td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
