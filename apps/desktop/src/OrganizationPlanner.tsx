import { useEffect, useMemo, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  Sparkles,
  Layers,
  Calendar,
  FolderGit2,
  ArrowRight,
  ShieldCheck,
  Check,
  AlertCircle,
  LoaderCircle,
  FileText,
  SlidersHorizontal,
  FolderOpen,
  RotateCcw,
  CheckCircle2,
  Play,
  CheckSquare,
  ChevronLeft,
  ChevronRight,
} from "lucide-react";

export type ProposedAction =
  | { action: "move"; source: number; destination_relative: string }
  | { action: "rename"; source: number; new_name: string }
  | { action: "trash"; source: number };

export type Proposal = {
  actions: ProposedAction[];
  rationale: string;
};

export type ScopeFile = {
  id: number;
  path: string;
  display: string;
  size: number;
  modified: number;
  excerpt: string | null;
  hash: string | null;
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

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 4);
  return `${(bytes / 1024 ** i).toFixed(i > 1 ? 1 : 0)} ${["B", "KB", "MB", "GB", "TB"][i]}`;
}

export default function OrganizationPlanner({
  scopeId,
  scopeName,
  onRefreshNeeded,
}: {
  scopeId: number | null;
  scopeName?: string;
  onRefreshNeeded?: () => void;
}) {
  const [mode, setMode] = useState<"category" | "date" | "project" | "custom">(
    "custom",
  );
  const [projectName, setProjectName] = useState("");
  const [customInstruction, setCustomInstruction] = useState("");
  const [useAi, setUseAi] = useState(true);
  const [planning, setPlanning] = useState(false);
  const [error, setError] = useState("");
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const destinationFolders = useMemo(() => {
    const folders = new Map<string, number>();
    for (const action of proposal?.actions ?? []) {
      if (action.action !== "move") continue;
      const folder = action.destination_relative.split("/").slice(0, -1).join("/");
      folders.set(folder, (folders.get(folder) ?? 0) + 1);
    }
    return [...folders].sort(([a], [b]) => a.localeCompare(b));
  }, [proposal]);
  const [filesMap, setFilesMap] = useState<Map<number, ScopeFile>>(new Map());
  const [loadingFiles, setLoadingFiles] = useState(false);

  // Phase 5 Safety Execution & Approval state
  const [approvalView, setApprovalView] = useState<ApprovalView | null>(null);
  const [requestingApproval, setRequestingApproval] = useState(false);
  const [executing, setExecuting] = useState(false);
  const [executionReport, setExecutionReport] =
    useState<ExecutionReport | null>(null);

  // Immediate Undo state
  const [undoApproval, setUndoApproval] = useState<ApprovalView | null>(null);
  const [undoing, setUndoing] = useState(false);
  const [undoReport, setUndoReport] = useState<ExecutionReport | null>(null);

  // Pagination & category filter for proposal preview
  const [page, setPage] = useState(0);
  const [actionCategoryFilter, setActionCategoryFilter] =
    useState<string>("all");

  const categories = useMemo(() => {
    if (!proposal) return [];
    const map = new Map<string, number>();
    for (const act of proposal.actions) {
      if (act.action === "move") {
        const cat = act.destination_relative.split(/[/\\]/)[0] || "Other";
        map.set(cat, (map.get(cat) || 0) + 1);
      } else if (act.action === "rename") {
        map.set("Rename", (map.get("Rename") || 0) + 1);
      } else if (act.action === "trash") {
        map.set("Trash", (map.get("Trash") || 0) + 1);
      }
    }
    return Array.from(map.entries()).map(([name, count]) => ({ name, count }));
  }, [proposal]);

  const filteredActions = useMemo(() => {
    if (!proposal) return [];
    if (actionCategoryFilter === "all") return proposal.actions;
    return proposal.actions.filter((act) => {
      if (act.action === "move") {
        const cat = act.destination_relative.split(/[/\\]/)[0] || "Other";
        return cat === actionCategoryFilter;
      }
      if (act.action === "rename") return actionCategoryFilter === "Rename";
      if (act.action === "trash") return actionCategoryFilter === "Trash";
      return false;
    });
  }, [proposal, actionCategoryFilter]);

  const pageSize = 50;
  const totalPages = Math.max(1, Math.ceil(filteredActions.length / pageSize));
  const paginatedActions = useMemo(() => {
    const start = page * pageSize;
    return filteredActions.slice(start, start + pageSize);
  }, [filteredActions, page, pageSize]);

  useEffect(() => {
    setProposal(null);
    setApprovalView(null);
    setExecutionReport(null);
    setUndoApproval(null);
    setUndoReport(null);
    setError("");
    setPage(0);
    setActionCategoryFilter("all");

    if (scopeId === null || !isTauri()) {
      setFilesMap(new Map());
      return;
    }

    loadFiles();
  }, [scopeId]);

  function loadFiles() {
    if (scopeId === null) return;
    setLoadingFiles(true);
    invoke<ScopeFile[]>("list_scope_files", { scopeId })
      .then((records) => {
        const map = new Map<number, ScopeFile>();
        for (const file of records) {
          map.set(file.id, file);
        }
        setFilesMap(map);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoadingFiles(false));
  }

  async function generatePlan() {
    if (scopeId === null) return;
    if (mode === "project" && !projectName.trim()) {
      setError("Please specify a project name to group files.");
      return;
    }
    if (mode === "custom" && !customInstruction.trim()) {
      setError("Please specify how you want to group your files.");
      return;
    }

    setError("");
    setPlanning(true);
    setApprovalView(null);
    setExecutionReport(null);
    setUndoReport(null);

    try {
      const res = await invoke<Proposal>("propose_organization", {
        scopeId,
        mode,
        projectName: mode === "project" ? projectName.trim() : null,
        customInstruction: mode === "custom" ? customInstruction.trim() : null,
        useAi,
      });
      setProposal(res);
    } catch (e) {
      setError(String(e));
    } finally {
      setPlanning(false);
    }
  }

  async function requestApproval() {
    if (scopeId === null || !proposal || proposal.actions.length === 0) return;
    setError("");
    setRequestingApproval(true);
    try {
      const view = await invoke<ApprovalView>("request_plan_approval", {
        scopeId,
        rationale: proposal.rationale,
        actions: proposal.actions,
      });
      setApprovalView(view);
    } catch (e) {
      setError(String(e));
    } finally {
      setRequestingApproval(false);
    }
  }

  async function executePlan() {
    if (scopeId === null || !approvalView) return;
    setError("");
    setExecuting(true);
    try {
      const report = await invoke<ExecutionReport>("execute_approved_plan", {
        scopeId,
        token: approvalView.token,
      });
      setExecutionReport(report);
      setApprovalView(null);
      setProposal(null); // Clear proposal since it has been executed
      loadFiles();
      if (onRefreshNeeded) onRefreshNeeded();
    } catch (e) {
      setApprovalView(null);
      setProposal(null);
      loadFiles();
      if (onRefreshNeeded) onRefreshNeeded();
      setError(String(e));
    } finally {
      setExecuting(false);
    }
  }

  async function requestUndoForPlan(txId: number) {
    if (scopeId === null) return;
    setError("");
    setUndoing(true);
    try {
      const undoView = await invoke<ApprovalView>("request_undo_approval", {
        scopeId,
        txId,
      });
      setUndoApproval(undoView);
    } catch (e) {
      setError(String(e));
    } finally {
      setUndoing(false);
    }
  }

  async function executeUndoForPlan() {
    if (scopeId === null || !executionReport || !undoApproval) return;
    setError("");
    setUndoing(true);
    try {
      const report = await invoke<ExecutionReport>("execute_approved_undo", {
        scopeId,
        originalTxId: executionReport.transaction_id,
        undoToken: undoApproval.token,
      });
      setUndoReport(report);
      setUndoApproval(null);
      setExecutionReport(null);
      loadFiles();
      if (onRefreshNeeded) onRefreshNeeded();
    } catch (e) {
      setError(String(e));
    } finally {
      setUndoing(false);
    }
  }

  if (scopeId === null) {
    return (
      <div className="planner-empty-state">
        <FolderOpen size={48} strokeWidth={1.2} />
        <h3>Choose a folder to organize</h3>
        <p>
          Select a folder in the sidebar to propose category, date, project, or
          custom grouping.
        </p>
      </div>
    );
  }

  return (
    <div className="planner-container">
      <div className="planner-header">
        <div>
          <div className="planner-badge">
            <Sparkles size={13} />
            <span>Phase 5 Plan & Safe Execution</span>
          </div>
          <h2>Organize & Group: {scopeName || "Folder"}</h2>
          <p className="planner-subtitle">
            Plan, preview, explicitly approve, and execute file reorganizations
            with collision checks and approved undo for unchanged moved files.
          </p>
        </div>
      </div>

      {planning && useAi && (
        <button
          type="button"
          className="btn-secondary"
          onClick={() =>
            void invoke("cancel_ai").catch((e) => setError(String(e)))
          }
        >
          Cancel planning
        </button>
      )}
      {error && (
        <div className="planner-error" role="alert">
          <AlertCircle size={16} />
          <span>{error}</span>
        </div>
      )}

      {/* Execution Success Banner */}
      {executionReport && (
        <div className="execution-success-banner">
          <CheckCircle2 size={24} className="success-icon" />
          <div className="banner-content">
            <strong>Plan Executed & Verified on Disk!</strong>
            <p>
              Applied {executionReport.actions_applied} action(s) in{" "}
              {executionReport.duration_ms}ms. All file moves are logged in the
              durable SQLite journal.
            </p>
          </div>
          <button
            type="button"
            className="btn-undo-now"
            onClick={() =>
              void requestUndoForPlan(executionReport.transaction_id)
            }
            disabled={undoing}
          >
            <RotateCcw size={14} />
            <span>Undo Plan</span>
          </button>
        </div>
      )}

      {/* Undo Success Banner */}
      {undoReport && (
        <div className="execution-success-banner">
          <CheckCircle2 size={24} className="success-icon" />
          <div className="banner-content">
            <strong>Plan Undone Successfully!</strong>
            <p>
              Restored {undoReport.actions_applied} file(s) back to original
              locations in {undoReport.duration_ms}ms.
            </p>
          </div>
        </div>
      )}

      {/* One-Use Approval Confirmation Modal */}
      {approvalView && (
        <div className="approval-modal-backdrop">
          <div className="approval-modal">
            <div className="approval-modal-header">
              <ShieldCheck size={22} className="shield-icon" />
              <div>
                <h3>Confirm Execution Approval</h3>
                <p>
                  One-use safety approval for Transaction #{approvalView.tx_id}
                </p>
              </div>
            </div>

            <div className="approval-notice">
              <CheckSquare size={16} />
              <span>
                Review before approval: {approvalView.actions_count} atomic
                move(s). Destinations have been checked for collisions (no
                overwrite permitted). Every action is journaled and reversible.
              </span>
            </div>

            <div className="approval-modal-actions">
              <button
                type="button"
                className="btn-secondary"
                onClick={() => setApprovalView(null)}
                disabled={executing}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn-primary-apply"
                onClick={() => void executePlan()}
                disabled={executing}
              >
                {executing ? (
                  <>
                    <LoaderCircle size={15} className="spinning" />
                    <span>Executing Moves…</span>
                  </>
                ) : (
                  <>
                    <Play size={14} />
                    <span>Confirm & Execute Now</span>
                  </>
                )}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Undo Confirmation Modal */}
      {undoApproval && (
        <div className="approval-modal-backdrop">
          <div className="approval-modal">
            <div className="approval-modal-header">
              <RotateCcw size={22} className="undo-modal-icon" />
              <div>
                <h3>Confirm Reversible Undo</h3>
                <p>Reverse moves for Transaction #{undoApproval.tx_id}</p>
              </div>
            </div>

            <div className="approval-notice">
              <ShieldCheck size={16} />
              <span>
                Review before approval: Inverting {undoApproval.actions_count}{" "}
                move(s) back to original locations. Original destinations
                checked for collisions.
              </span>
            </div>

            <div className="approval-modal-actions">
              <button
                type="button"
                className="btn-secondary"
                onClick={() => setUndoApproval(null)}
                disabled={undoing}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn-primary-undo"
                onClick={() => void executeUndoForPlan()}
                disabled={undoing}
              >
                {undoing ? (
                  <>
                    <LoaderCircle size={15} className="spinning" />
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

      {/* Mode selection cards */}
      <div className="planner-modes">
        <button
          type="button"
          className={`mode-card ${mode === "category" ? "selected" : ""}`}
          onClick={() => setMode("category")}
        >
          <div className="mode-icon">
            <Layers size={20} />
          </div>
          <div className="mode-content">
            <strong>By Category (Downloads)</strong>
            <span>
              Sort into Documents, Images, Code, Audio, Video, Archives...
            </span>
          </div>
          {mode === "category" && <Check className="mode-check" size={16} />}
        </button>

        <button
          type="button"
          className={`mode-card ${mode === "date" ? "selected" : ""}`}
          onClick={() => setMode("date")}
        >
          <div className="mode-icon">
            <Calendar size={20} />
          </div>
          <div className="mode-content">
            <strong>By Modification Date</strong>
            <span>Sort chronologically into Year/Month (YYYY/MM) folders.</span>
          </div>
          {mode === "date" && <Check className="mode-check" size={16} />}
        </button>

        <button
          type="button"
          className={`mode-card ${mode === "project" ? "selected" : ""}`}
          onClick={() => setMode("project")}
        >
          <div className="mode-icon">
            <FolderGit2 size={20} />
          </div>
          <div className="mode-content">
            <strong>Group by Project</strong>
            <span>
              Match names, subfolders, and text excerpts to a project name.
            </span>
          </div>
          {mode === "project" && <Check className="mode-check" size={16} />}
        </button>

        <button
          type="button"
          className={`mode-card ${mode === "custom" ? "selected" : ""}`}
          onClick={() => { setMode("custom"); setUseAi(true); }}
        >
          <div className="mode-icon">
            <SlidersHorizontal size={20} />
          </div>
          <div className="mode-content">
            <strong>Custom Grouping (Prompt / Rules)</strong>
            <span>
              Specify your own rules, tags, or natural language prompt.
            </span>
          </div>
          {mode === "custom" && <Check className="mode-check" size={16} />}
        </button>
      </div>

      {/* Project name input when in Project mode */}
      {mode === "project" && (
        <div className="project-input-group">
          <label htmlFor="project-name-input">Project Name or Tag:</label>
          <div className="input-with-button">
            <input
              id="project-name-input"
              type="text"
              placeholder="e.g. Tidy, Thesis, ClientPortal, Apollo..."
              value={projectName}
              onChange={(e) => setProjectName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void generatePlan()}
              disabled={planning}
            />
          </div>
          <small className="input-hint">
            Existing Git repositories (`.git`) are automatically excluded and
            protected.
          </small>
        </div>
      )}

      {/* Custom instructions input when in Custom mode */}
      {mode === "custom" && (
        <div className="project-input-group">
          <label htmlFor="custom-instruction-input">
            What would you like to organize?
          </label>
          <div className="custom-input-wrap">
            <textarea
              id="custom-instruction-input"
              rows={3}
              placeholder='e.g. "Move all invoices and receipts into Invoices/, photos into Photos/", "Group by file extension", or rule mapping like "Receipts: pdf | Photos: png, jpg"'
              value={customInstruction}
              onChange={(e) => { setCustomInstruction(e.target.value); setUseAi(true); }}
              disabled={planning}
            />
          </div>
          <div className="custom-presets">
            <span className="preset-label">Quick presets:</span>
            <button
              type="button"
              className="preset-chip"
              onClick={() => { setCustomInstruction("Group by file extension"); setUseAi(false); }}
            >
              By file extension
            </button>
            <button
              type="button"
              className="preset-chip"
              onClick={() => { setCustomInstruction("Group by year (YYYY)"); setUseAi(false); }}
            >
              By year (YYYY)
            </button>
            <button
              type="button"
              className="preset-chip"
              onClick={() =>
                setCustomInstruction(
                  "Receipts: pdf | Photos: png, jpg | Audio: mp3, wav",
                )
              }
            >
              Receipts / Photos / Audio
            </button>
            <button
              type="button"
              className="preset-chip"
              onClick={() =>
                setCustomInstruction("Invoices, Taxes, Personal, Work")
              }
            >
              Tags (Invoices, Taxes...)
            </button>
          </div>
          <small className="input-hint">
            Local AI translates your request into filename/type rules checked against the full index. Only top-level files are moved; nested folders stay intact. Unsupported requests show an error instead of a different plan.
          </small>
        </div>
      )}

      {/* Controls & Generate Button */}
      <div className="planner-actions-bar">
        <label className="ai-toggle-label">
          <input
            type="checkbox"
            checked={useAi}
            onChange={(e) => setUseAi(e.target.checked)}
            disabled={planning || mode !== "custom"}
          />
          <Sparkles size={15} />
          <span>
            Interpret custom request with local AI (no substitute plan)
          </span>
        </label>

        <button
          type="button"
          className="planner-generate-btn"
          onClick={() => void generatePlan()}
          disabled={
            planning ||
            loadingFiles ||
            (mode === "project" && !projectName.trim())
          }
        >
          {planning ? (
            <>
              <LoaderCircle size={16} className="spinning" />
              <span>Analyzing & Planning…</span>
            </>
          ) : (
            <>
              <SlidersHorizontal size={16} />
              <span>Propose Organization Plan</span>
            </>
          )}
        </button>
      </div>

      {/* Proposal Results Section */}
      {proposal && (
        <div className="proposal-result-panel">
          <div className="rationale-card">
            <div className="rationale-header">
              <ShieldCheck size={22} className="shield-icon" />
              <div>
                <h4>Plan Proposal Summary</h4>
                <p>{proposal.rationale}</p>
              </div>
            </div>
            <div className="rationale-actions">
              <button
                type="button"
                className="btn-accept-proposal"
                onClick={() => void requestApproval()}
                disabled={requestingApproval || proposal.actions.length === 0}
              >
                {requestingApproval ? (
                  <>
                    <LoaderCircle size={15} className="spinning" />
                    <span>Verifying Paths…</span>
                  </>
                ) : (
                  <>
                    <Play size={15} />
                    <span>
                      Review & approve ({proposal.actions.length})
                    </span>
                  </>
                )}
              </button>
            </div>
          </div>

          {destinationFolders.length > 0 && <section className="destination-preview" aria-label="Proposed folder structure">
            <div className="destination-heading"><Layers size={18} /><strong>Your new folder structure</strong><span>{proposal.actions.length} files · preview only</span></div>
            <div className="destination-grid">{destinationFolders.map(([folder, count]) => <div className="destination-folder" key={folder}><Layers size={18}/><code>{folder}/</code><span>{count} files</span></div>)}</div>
          </section>}
          <div className="proposal-actions-table-wrap">
            <div className="table-header-title">
              <span>Proposed File Movements</span>
              <span className="safe-tag">Preview only · Reversible</span>
            </div>

            {categories.length > 1 && (
              <div className="category-filter-tabs">
                <button
                  type="button"
                  className={`category-tab ${actionCategoryFilter === "all" ? "active" : ""}`}
                  onClick={() => {
                    setActionCategoryFilter("all");
                    setPage(0);
                  }}
                >
                  All Categories ({proposal.actions.length})
                </button>
                {categories.map((c) => (
                  <button
                    key={c.name}
                    type="button"
                    className={`category-tab ${actionCategoryFilter === c.name ? "active" : ""}`}
                    onClick={() => {
                      setActionCategoryFilter(c.name);
                      setPage(0);
                    }}
                  >
                    {c.name} ({c.count})
                  </button>
                ))}
              </div>
            )}

            {filteredActions.length === 0 ? (
              <div className="no-actions-notice">
                <Check size={20} />
                <span>
                  All files are already in place, or no files matched the
                  criteria. No moves required.
                </span>
              </div>
            ) : (
              <>
                <table className="proposal-table">
                  <thead>
                    <tr>
                      <th style={{ width: "80px" }}>Action</th>
                      <th>Current File</th>
                      <th style={{ width: "30px" }}></th>
                      <th>Proposed Destination</th>
                      <th style={{ width: "100px", textAlign: "right" }}>
                        Size
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {paginatedActions.map((act, index) => {
                      const file = filesMap.get(act.source);
                      const sourceName = file
                        ? file.display
                        : `File #${act.source}`;
                      const sizeStr = file ? formatBytes(file.size) : "";

                      return (
                        <tr key={index}>
                          <td>
                            <span className={`action-badge ${act.action}`}>
                              {act.action.toUpperCase()}
                            </span>
                          </td>
                          <td className="source-path" title={sourceName}>
                            <FileText size={14} className="file-icon" />
                            <button
                              type="button"
                              className="btn-link"
                              onClick={() =>
                                void invoke("reveal_indexed_file", {
                                  scopeId,
                                  fileId: act.source,
                                }).catch((e) => setError(String(e)))
                              }
                            >
                              {sourceName}
                            </button>
                          </td>
                          <td className="arrow-cell">
                            <ArrowRight size={14} />
                          </td>
                          <td className="dest-path">
                            {act.action === "move" && (
                              <strong className="dest-highlight">
                                {act.destination_relative}
                              </strong>
                            )}
                            {act.action === "rename" && (
                              <strong className="dest-highlight">
                                {act.new_name}
                              </strong>
                            )}
                            {act.action === "trash" && (
                              <span className="trash-dest">
                                Native Trash (Reversible)
                              </span>
                            )}
                          </td>
                          <td className="size-cell">{sizeStr}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>

                {filteredActions.length > pageSize && (
                  <div className="table-pagination-bar">
                    <span className="pagination-info">
                      Showing {page * pageSize + 1}–
                      {Math.min((page + 1) * pageSize, filteredActions.length)}{" "}
                      of {filteredActions.length} actions
                    </span>
                    <div className="pagination-buttons">
                      <button
                        type="button"
                        className="btn-page-nav"
                        disabled={page === 0}
                        onClick={() => setPage((p) => Math.max(0, p - 1))}
                      >
                        <ChevronLeft size={16} />
                        <span>Previous</span>
                      </button>
                      <span className="page-indicator">
                        Page {page + 1} of {totalPages}
                      </span>
                      <button
                        type="button"
                        className="btn-page-nav"
                        disabled={page >= totalPages - 1}
                        onClick={() =>
                          setPage((p) => Math.min(totalPages - 1, p + 1))
                        }
                      >
                        <span>Next</span>
                        <ChevronRight size={16} />
                      </button>
                    </div>
                  </div>
                )}

                {/* Execution Approval Bar */}
                <div className="execution-approval-bar">
                  <div className="approval-info">
                    <ShieldCheck size={16} />
                    <span>
                      Ready for safe execution. You will receive an explicit
                      confirmation request with a one-use token.
                    </span>
                  </div>
                  <button
                    type="button"
                    className="btn-request-approval"
                    onClick={() => void requestApproval()}
                    disabled={
                      requestingApproval || proposal.actions.length === 0
                    }
                  >
                    {requestingApproval ? (
                      <>
                        <LoaderCircle size={15} className="spinning" />
                        <span>Verifying Paths…</span>
                      </>
                    ) : (
                      <>
                        <Play size={14} />
                        <span>
                          Approve & Execute Plan ({proposal.actions.length})
                        </span>
                      </>
                    )}
                  </button>
                </div>
              </>
            )}
          </div>

          <div className="safety-disclaimer">
            <ShieldCheck size={14} />
            <span>
              <strong>Safety policy:</strong> Tidy strictly uses atomic,
              no-replace moves. Collisions are never overwritten. Every action
              is recorded in the durable SQLite transaction journal and can be
              restored while their recorded files remain unchanged.
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
