import { useEffect, useMemo, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  HardDrive,
  Trash2,
  Copy,
  Package,
  Wrench,
  ShieldCheck,
  AlertCircle,
  LoaderCircle,
  FolderOpen,
  ArrowRight,
  CheckCircle2,
  CheckSquare,
  Square,
  MinusSquare,
  Check,
  FileText,
  RotateCcw,
  Play,
  ChevronLeft,
  ChevronRight,
} from "lucide-react";
import {
  ApprovalView,
  ExecutionReport,
  Proposal,
  ScopeFile,
} from "./OrganizationPlanner";

export type FindingKind =
  "large_file" | "exact_duplicate" | "old_installer" | "development_artifact";

export type Finding = {
  file_ids: number[];
  kind: FindingKind;
  evidence: string;
  estimated_reclaimable_bytes: number | null;
};

export type StorageSummary = {
  total_findings: number;
  potential_reclaimable_bytes: number;
  large_files_count: number;
  large_files_bytes: number;
  duplicate_groups_count: number;
  duplicate_redundant_files_count: number;
  duplicate_reclaimable_bytes: number;
  old_installers_count: number;
  old_installers_bytes: number;
  dev_artifacts_count: number;
  dev_artifacts_bytes: number;
};

export type StorageAnalysisResult = {
  findings: Finding[];
  summary: StorageSummary;
};

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 4);
  return `${(bytes / 1024 ** i).toFixed(i > 1 ? 1 : 0)} ${["B", "KB", "MB", "GB", "TB"][i]}`;
}

export default function StorageAnalyzer({
  scopeId,
  scopeName,
  onRefreshNeeded,
  snapshot,
  scanning,
}: {
  scopeId: number | null;
  scopeName?: string;
  onRefreshNeeded?: () => void;
  snapshot: number | null;
  scanning: boolean;
}) {
  const analysisVersion = useRef(0);
  const [analyzing, setAnalyzing] = useState(false);
  const [result, setResult] = useState<StorageAnalysisResult | null>(null);
  const [error, setError] = useState("");
  const [filesMap, setFilesMap] = useState<Map<number, ScopeFile>>(new Map());
  const [cleanupProposal, setCleanupProposal] = useState<Proposal | null>(null);
  const [proposingCleanup, setProposingCleanup] = useState(false);

  const [visibleLimit, setVisibleLimit] = useState(100);
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

  // Granular selection state
  // selectedIds: Set of file IDs chosen for trashing
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  // keptDuplicateByGroup: Map of group index -> kept file ID
  const [keptDuplicateByGroup, setKeptDuplicateByGroup] = useState<
    Map<number, number>
  >(new Map());
  // Active category filter tab: "all" | FindingKind
  const [categoryFilter, setCategoryFilter] = useState<"all" | FindingKind>(
    "all",
  );

  // Pagination for cleanup proposal
  const [cleanupPage, setCleanupPage] = useState(0);
  const CLEANUP_PAGE_SIZE = 50;

  const totalCleanupPages = useMemo(() => {
    if (!cleanupProposal) return 1;
    return Math.max(
      1,
      Math.ceil(cleanupProposal.actions.length / CLEANUP_PAGE_SIZE),
    );
  }, [cleanupProposal]);

  const paginatedCleanupActions = useMemo(() => {
    if (!cleanupProposal) return [];
    const start = cleanupPage * CLEANUP_PAGE_SIZE;
    return cleanupProposal.actions.slice(start, start + CLEANUP_PAGE_SIZE);
  }, [cleanupProposal, cleanupPage]);

  useEffect(() => {
    setResult(null);
    setCleanupProposal(null);
    setCleanupPage(0);
    setApprovalView(null);
    setExecutionReport(null);
    setUndoApproval(null);
    setUndoReport(null);
    setError("");
    setSelectedIds(new Set());
    setKeptDuplicateByGroup(new Map());

    if (scopeId === null || !isTauri()) {
      setFilesMap(new Map());
      return;
    }

    setFilesMap(new Map());
  }, [scopeId]);

  useEffect(() => {
    if (scanning) {
      analysisVersion.current++;
      setAnalyzing(false);
      setApprovalView(null);
      setCleanupProposal(null);
    } else if (scopeId !== null && isTauri()) void runAnalysis();
  }, [scopeId, snapshot, scanning]);

  async function runAnalysis() {
    if (scopeId === null || scanning) return;
    const version = ++analysisVersion.current;
    setError("");
    setAnalyzing(true);
    setCleanupProposal(null);
    setApprovalView(null);
    try {
      const res = await invoke<StorageAnalysisResult>("analyze_storage_scope", {
        scopeId,
      });
      const records = await invoke<ScopeFile[]>("list_scope_files", {
        scopeId,
      });
      if (version !== analysisVersion.current) return;
      setResult(res);
      setFilesMap(new Map(records.map((file) => [file.id, file])));

      // Initialize default selections:
      // 1. Redundant duplicate copies (preserve the first copy)
      // 2. Old installers
      // 3. Development artifacts
      // (Large files are unchecked by default for safety, but can be checked individually)
      const initialSelected = new Set<number>();
      const initialKept = new Map<number, number>();

      let dupGroupIndex = 0;
      for (const finding of res.findings) {
        if (finding.kind === "exact_duplicate" && finding.file_ids.length > 1) {
          const keptId = finding.file_ids[0];
          initialKept.set(dupGroupIndex, keptId);
          for (let i = 1; i < finding.file_ids.length; i++) {
            initialSelected.add(finding.file_ids[i]);
          }
          dupGroupIndex++;
        } else if (
          finding.kind === "old_installer" ||
          finding.kind === "development_artifact"
        ) {
          for (const id of finding.file_ids) {
            initialSelected.add(id);
          }
        }
      }

      setKeptDuplicateByGroup(initialKept);
      setSelectedIds(new Set()); // Explicit selection starts empty.
    } catch (e) {
      if (version === analysisVersion.current) setError(String(e));
    } finally {
      if (version === analysisVersion.current) setAnalyzing(false);
    }
  }

  // Duplicate groups structured for easy rendering and selection
  const duplicateGroups = useMemo(() => {
    if (!result) return [];
    return result.findings
      .filter((f) => f.kind === "exact_duplicate" && f.file_ids.length > 1)
      .map((f, groupIdx) => {
        const keptId = keptDuplicateByGroup.get(groupIdx) ?? f.file_ids[0];
        return {
          groupIdx,
          fileIds: f.file_ids,
          keptId,
          evidence: f.evidence,
          redundantIds: f.file_ids.filter((id) => id !== keptId),
        };
      });
  }, [result, keptDuplicateByGroup]);

  // All individual items grouped by category
  const categorizedItems = useMemo(() => {
    if (!result) {
      return {
        exact_duplicate: [] as {
          id: number;
          file?: ScopeFile;
          groupIdx: number;
          isKept: boolean;
        }[],
        old_installer: [] as {
          id: number;
          file?: ScopeFile;
          evidence: string;
        }[],
        development_artifact: [] as {
          id: number;
          file?: ScopeFile;
          evidence: string;
        }[],
        large_file: [] as { id: number; file?: ScopeFile; evidence: string }[],
      };
    }

    const dups: {
      id: number;
      file?: ScopeFile;
      groupIdx: number;
      isKept: boolean;
    }[] = [];
    duplicateGroups.forEach((g) => {
      for (const id of g.fileIds) {
        dups.push({
          id,
          file: filesMap.get(id),
          groupIdx: g.groupIdx,
          isKept: id === g.keptId,
        });
      }
    });

    const installers: { id: number; file?: ScopeFile; evidence: string }[] = [];
    const artifacts: { id: number; file?: ScopeFile; evidence: string }[] = [];
    const largeFiles: { id: number; file?: ScopeFile; evidence: string }[] = [];

    for (const f of result.findings) {
      if (f.kind === "old_installer") {
        for (const id of f.file_ids) {
          installers.push({ id, file: filesMap.get(id), evidence: f.evidence });
        }
      } else if (f.kind === "development_artifact") {
        for (const id of f.file_ids) {
          artifacts.push({ id, file: filesMap.get(id), evidence: f.evidence });
        }
      } else if (f.kind === "large_file") {
        for (const id of f.file_ids) {
          largeFiles.push({ id, file: filesMap.get(id), evidence: f.evidence });
        }
      }
    }

    return {
      exact_duplicate: dups,
      old_installer: installers,
      development_artifact: artifacts,
      large_file: largeFiles,
    };
  }, [result, duplicateGroups, filesMap]);

  // Compute selected bytes and counts
  const { selectedBytes, selectedCount } = useMemo(() => {
    let bytes = 0;
    let count = 0;
    for (const id of selectedIds) {
      const file = filesMap.get(id);
      if (file) {
        bytes += file.size;
        count++;
      }
    }
    return { selectedBytes: bytes, selectedCount: count };
  }, [selectedIds, filesMap]);

  // Toggle individual item
  function toggleItem(id: number) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  // Change kept copy in a duplicate group
  function setKeptDuplicate(groupIdx: number, newKeptId: number) {
    const group = duplicateGroups.find((g) => g.groupIdx === groupIdx);
    if (!group) return;

    setKeptDuplicateByGroup((prev) => {
      const next = new Map(prev);
      next.set(groupIdx, newKeptId);
      return next;
    });

    setSelectedIds((prev) => {
      const next = new Set(prev);
      // Ensure the new kept file is NOT marked for deletion
      next.delete(newKeptId);
      // Mark all other files in this group as selected for deletion
      for (const id of group.fileIds) {
        if (id !== newKeptId) {
          next.add(id);
        }
      }
      return next;
    });
  }

  // Category selection status and toggle
  function getCategoryState(kind: FindingKind): {
    total: number;
    selected: number;
    state: "checked" | "unchecked" | "indeterminate";
  } {
    if (kind === "exact_duplicate") {
      // In duplicates, only redundant copies can be checked
      const redundantIds = duplicateGroups.flatMap((g) => g.redundantIds);
      const selected = redundantIds.filter((id) => selectedIds.has(id)).length;
      const total = redundantIds.length;
      return {
        total,
        selected,
        state:
          total === 0
            ? "unchecked"
            : selected === total
              ? "checked"
              : selected > 0
                ? "indeterminate"
                : "unchecked",
      };
    }

    const items = categorizedItems[kind];
    const total = items.length;
    const selected = items.filter((item) => selectedIds.has(item.id)).length;
    return {
      total,
      selected,
      state:
        total === 0
          ? "unchecked"
          : selected === total
            ? "checked"
            : selected > 0
              ? "indeterminate"
              : "unchecked",
    };
  }

  function toggleCategory(kind: FindingKind) {
    const catState = getCategoryState(kind);
    const shouldSelect = catState.state !== "checked";

    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (kind === "exact_duplicate") {
        const redundantIds = duplicateGroups.flatMap((g) => g.redundantIds);
        for (const id of redundantIds) {
          if (shouldSelect) {
            next.add(id);
          } else {
            next.delete(id);
          }
        }
      } else {
        const items = categorizedItems[kind];
        for (const item of items) {
          if (shouldSelect) {
            next.add(item.id);
          } else {
            next.delete(item.id);
          }
        }
      }
      return next;
    });
  }

  function selectAll() {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      // Redundant duplicates
      for (const g of duplicateGroups) {
        for (const id of g.redundantIds) {
          next.add(id);
        }
      }
      // Old installers
      for (const item of categorizedItems.old_installer) {
        next.add(item.id);
      }
      // Dev artifacts
      for (const item of categorizedItems.development_artifact) {
        next.add(item.id);
      }
      // Large files
      for (const item of categorizedItems.large_file) {
        next.add(item.id);
      }
      return next;
    });
  }

  function deselectAll() {
    setSelectedIds(new Set());
  }

  async function generateCleanup() {
    if (scopeId === null) return;
    setError("");
    setProposingCleanup(true);
    try {
      const keepDuplicateIds = Array.from(keptDuplicateByGroup.values());
      const selectedFileIds = Array.from(selectedIds);

      const prop = await invoke<Proposal>("propose_storage_cleanup", {
        scopeId,
        keepDuplicateIds,
        selectedFileIds,
      });
      setCleanupProposal(prop);
      setCleanupPage(0);
    } catch (e) {
      setError(String(e));
    } finally {
      setProposingCleanup(false);
    }
  }

  async function startDirectCleanup() {
    if (scopeId === null || selectedCount === 0) return;
    setError("");
    setProposingCleanup(true);
    try {
      const keepDuplicateIds = Array.from(keptDuplicateByGroup.values());
      const selectedFileIds = Array.from(selectedIds);

      const prop = await invoke<Proposal>("propose_storage_cleanup", {
        scopeId,
        keepDuplicateIds,
        selectedFileIds,
      });
      setCleanupProposal(prop);
      setCleanupPage(0);

      if (prop.actions.length > 0) {
        setRequestingApproval(true);
        const view = await invoke<ApprovalView>("request_plan_approval", {
          scopeId,
          rationale: prop.rationale,
          actions: prop.actions,
        });
        setApprovalView(view);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setProposingCleanup(false);
      setRequestingApproval(false);
    }
  }

  async function requestCleanupApproval() {
    if (
      scopeId === null ||
      !cleanupProposal ||
      cleanupProposal.actions.length === 0
    )
      return;
    setError("");
    setRequestingApproval(true);
    try {
      const view = await invoke<ApprovalView>("request_plan_approval", {
        scopeId,
        rationale: cleanupProposal.rationale,
        actions: cleanupProposal.actions,
      });
      setApprovalView(view);
    } catch (e) {
      setError(String(e));
    } finally {
      setRequestingApproval(false);
    }
  }

  async function executeCleanup() {
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
      setCleanupProposal(null);
      setSelectedIds(new Set());
      await runAnalysis();
      if (onRefreshNeeded) onRefreshNeeded();
    } catch (e) {
      setApprovalView(null);
      setCleanupProposal(null);
      setSelectedIds(new Set());
      if (onRefreshNeeded) onRefreshNeeded();
      setError(String(e));
    } finally {
      setExecuting(false);
    }
  }

  async function executeUndoForCleanup() {
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
      await runAnalysis();
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
        <h3>Choose a folder for storage analysis</h3>
        <p>
          Select a folder in the sidebar to inspect large files, duplicates, and
          reclaimable space.
        </p>
      </div>
    );
  }

  const dupState = getCategoryState("exact_duplicate");
  const installerState = getCategoryState("old_installer");
  const devState = getCategoryState("development_artifact");
  const largeState = getCategoryState("large_file");

  return (
    <div className="planner-container">
      <p className="ai-footnote">
        Duplicate findings cover saved hashes from optional content indexing
        only. Files without hashes are not confirmed unique. Sizes are logical
        estimates; moving files to Trash does not immediately free disk space.
      </p>
      <div className="planner-header">
        <div>
          <div className="planner-badge">
            <HardDrive size={13} />
            <span>Storage Recovery & Deduplication</span>
          </div>
          <h2>Storage Analysis: {scopeName || "Folder"}</h2>
          <p className="planner-subtitle">
            Detect large files, exact duplicates, aging installers, and build
            artifacts. Select categories or items one-by-one to safely reclaim
            space.
          </p>
        </div>
        <button
          type="button"
          className="planner-generate-btn"
          onClick={() => void runAnalysis()}
          disabled={analyzing || scanning || executing || requestingApproval}
        >
          {analyzing ? (
            <>
              <LoaderCircle size={16} className="spinning" />
              <span>Analyzing storage…</span>
            </>
          ) : (
            <>
              <HardDrive size={16} />
              <span>Analyze Storage Savings</span>
            </>
          )}
        </button>
      </div>

      {error && (
        <div className="planner-error" role="alert">
          <AlertCircle size={16} />
          <span>{error}</span>
          <button
            className="secondary"
            disabled={analyzing || executing}
            onClick={() => void runAnalysis()}
          >
            Refresh results
          </button>
        </div>
      )}

      {/* Execution Success Banner */}
      {executionReport && (
        <div className="execution-success-banner">
          <CheckCircle2 size={24} className="success-icon" />
          <div className="banner-content">
            <strong>Cleanup Completed: Files Moved to OS Trash!</strong>
            <p>
              Safely moved {executionReport.actions_applied} item(s) to native
              Trash in {executionReport.duration_ms}ms. Recovery locations are
              recorded in History. Restore files using Finder Trash. Moving to
              Trash does not immediately free disk space.
            </p>
          </div>
          <button
            type="button"
            className="btn-undo-now"
            onClick={() =>
              setError(
                "Restore these files through Finder Trash. History records their actual Trash locations. Automatic Trash undo is not available.",
              )
            }
            disabled={undoing}
          >
            <RotateCcw size={14} />
            <span>How to restore</span>
          </button>
        </div>
      )}

      {/* Undo Success Banner */}
      {undoReport && (
        <div className="execution-success-banner">
          <CheckCircle2 size={24} className="success-icon" />
          <div className="banner-content">
            <strong>Cleanup Undone Successfully!</strong>
            <p>
              Restored {undoReport.actions_applied} item(s) back to original
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
                <h3>Confirm Storage Cleanup</h3>
                <p>
                  One-use safety approval for Transaction #{approvalView.tx_id}
                </p>
              </div>
            </div>

            <div className="approval-notice">
              <Trash2 size={16} />
              <span>
                Review before approval: {approvalView.actions_count} file(s)
                will be moved to the <strong>native OS Trash</strong>. Permanent
                deletion (`rm -rf`) is strictly prohibited. Every action is
                recorded in the transaction journal. Restore trashed files
                manually using Finder.
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
                onClick={() => void executeCleanup()}
                disabled={executing}
              >
                {executing ? (
                  <>
                    <LoaderCircle size={15} className="spinning" />
                    <span>Moving to Trash…</span>
                  </>
                ) : (
                  <>
                    <Trash2 size={14} />
                    <span>Confirm & Move to Trash Now</span>
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
                <p>Reverse cleanup for Transaction #{undoApproval.tx_id}</p>
              </div>
            </div>

            <div className="approval-notice">
              <ShieldCheck size={16} />
              <span>
                Review before approval: Restoring {undoApproval.actions_count}{" "}
                file(s) back from native Trash to original locations.
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
                onClick={() => void executeUndoForCleanup()}
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

      {result && (
        <div className="storage-summary-section">
          {/* Top highlight card */}
          <div className="storage-savings-card">
            <div className="savings-content">
              <span className="savings-eyebrow">
                SPACE SELECTED FOR RECOVERY
              </span>
              <strong className="savings-amount">
                {formatBytes(selectedBytes)}
              </strong>
              <p>
                {selectedCount} item{selectedCount === 1 ? "" : "s"} selected
                out of {formatBytes(result.summary.potential_reclaimable_bytes)}{" "}
                logical candidate bytes, with overlapping categories. Not
                measured physical savings; Trash does not immediately free
                space.
              </p>
            </div>
            <div className="savings-actions">
              <button
                type="button"
                className="cleanup-execute-btn"
                onClick={() => void startDirectCleanup()}
                disabled={
                  proposingCleanup || requestingApproval || selectedCount === 0
                }
              >
                {proposingCleanup || requestingApproval ? (
                  <>
                    <LoaderCircle size={15} className="spinning" />
                    <span>Preparing Cleanup…</span>
                  </>
                ) : (
                  <>
                    <Trash2 size={16} />
                    <span>Clean Up Selected Files ({selectedCount})</span>
                  </>
                )}
              </button>
              <button
                type="button"
                className="cleanup-preview-link-btn"
                onClick={() => void generateCleanup()}
                disabled={proposingCleanup || selectedCount === 0}
              >
                <span>Preview Actions List</span>
              </button>
            </div>
          </div>

          {/* Breakdown / Category metric chips */}
          <div className="storage-metrics-grid">
            <div
              className={`metric-chip interactive ${categoryFilter === "exact_duplicate" ? "active-filter" : ""}`}
              onClick={() =>
                setCategoryFilter(
                  categoryFilter === "exact_duplicate"
                    ? "all"
                    : "exact_duplicate",
                )
              }
            >
              <div className="chip-icon">
                <Copy size={16} />
              </div>
              <div className="chip-data">
                <span>Exact Duplicates</span>
                <strong>{result.summary.duplicate_groups_count} groups</strong>
                <small>
                  {formatBytes(result.summary.duplicate_reclaimable_bytes)}{" "}
                  reclaimable
                </small>
              </div>
            </div>

            <div
              className={`metric-chip interactive ${categoryFilter === "old_installer" ? "active-filter" : ""}`}
              onClick={() =>
                setCategoryFilter(
                  categoryFilter === "old_installer" ? "all" : "old_installer",
                )
              }
            >
              <div className="chip-icon">
                <Package size={16} />
              </div>
              <div className="chip-data">
                <span>Old Installers</span>
                <strong>{result.summary.old_installers_count} packages</strong>
                <small>
                  {formatBytes(result.summary.old_installers_bytes)} total
                </small>
              </div>
            </div>

            <div
              className={`metric-chip interactive ${categoryFilter === "development_artifact" ? "active-filter" : ""}`}
              onClick={() =>
                setCategoryFilter(
                  categoryFilter === "development_artifact"
                    ? "all"
                    : "development_artifact",
                )
              }
            >
              <div className="chip-icon">
                <Wrench size={16} />
              </div>
              <div className="chip-data">
                <span>Dev Build Artifacts</span>
                <strong>{result.summary.dev_artifacts_count} artifacts</strong>
                <small>
                  {formatBytes(result.summary.dev_artifacts_bytes)} total
                </small>
              </div>
            </div>

            <div
              className={`metric-chip interactive ${categoryFilter === "large_file" ? "active-filter" : ""}`}
              onClick={() =>
                setCategoryFilter(
                  categoryFilter === "large_file" ? "all" : "large_file",
                )
              }
            >
              <div className="chip-icon">
                <HardDrive size={16} />
              </div>
              <div className="chip-data">
                <span>Large Files (&gt;50MB)</span>
                <strong>{result.summary.large_files_count} files</strong>
                <small>
                  {formatBytes(result.summary.large_files_bytes)} total
                </small>
              </div>
            </div>
          </div>

          <p>
            Showing up to {visibleLimit} groups/items per category.{" "}
            <button
              className="btn-link"
              onClick={() => setVisibleLimit((n) => n + 100)}
            >
              Show 100 more
            </button>
          </p>

          {/* Granular Selection & Category Controls */}
          <div className="findings-container">
            <div className="findings-toolbar">
              <div className="toolbar-left">
                <span className="toolbar-title">
                  Flagged Removable Elements
                </span>
                <span className="findings-count">
                  {selectedCount} selected ({formatBytes(selectedBytes)})
                </span>
              </div>
              <div className="toolbar-actions">
                <button
                  type="button"
                  className="btn-secondary-sm"
                  onClick={selectAll}
                >
                  Select All
                </button>
                <button
                  type="button"
                  className="btn-secondary-sm"
                  onClick={deselectAll}
                >
                  Deselect All
                </button>
              </div>
            </div>

            {/* Category tabs */}
            <div className="category-filter-tabs">
              <button
                type="button"
                className={`category-tab ${categoryFilter === "all" ? "active" : ""}`}
                onClick={() => setCategoryFilter("all")}
              >
                All Categories
              </button>
              <button
                type="button"
                className={`category-tab ${categoryFilter === "exact_duplicate" ? "active" : ""}`}
                onClick={() => setCategoryFilter("exact_duplicate")}
              >
                Exact Duplicates ({dupState.selected}/{dupState.total})
              </button>
              <button
                type="button"
                className={`category-tab ${categoryFilter === "old_installer" ? "active" : ""}`}
                onClick={() => setCategoryFilter("old_installer")}
              >
                Old Installers ({installerState.selected}/{installerState.total}
                )
              </button>
              <button
                type="button"
                className={`category-tab ${categoryFilter === "development_artifact" ? "active" : ""}`}
                onClick={() => setCategoryFilter("development_artifact")}
              >
                Build Artifacts ({devState.selected}/{devState.total})
              </button>
              <button
                type="button"
                className={`category-tab ${categoryFilter === "large_file" ? "active" : ""}`}
                onClick={() => setCategoryFilter("large_file")}
              >
                Large Files ({largeState.selected}/{largeState.total})
              </button>
            </div>

            {/* Category 1: Exact Duplicates */}
            {(categoryFilter === "all" ||
              categoryFilter === "exact_duplicate") &&
              duplicateGroups.length > 0 && (
                <div className="category-section">
                  <div className="category-header">
                    <button
                      type="button"
                      className="checkbox-btn"
                      onClick={() => toggleCategory("exact_duplicate")}
                      title="Toggle all exact duplicates"
                    >
                      {dupState.state === "checked" ? (
                        <CheckSquare size={18} className="check-icon checked" />
                      ) : dupState.state === "indeterminate" ? (
                        <MinusSquare
                          size={18}
                          className="check-icon indeterminate"
                        />
                      ) : (
                        <Square size={18} className="check-icon" />
                      )}
                    </button>
                    <div className="category-header-title">
                      <span className="finding-kind-badge exact_duplicate">
                        EXACT DUPLICATES
                      </span>
                      <strong>Redundant Duplicate Copies</strong>
                      <span className="category-stats-chip">
                        {dupState.selected} of {dupState.total} redundant copies
                        selected
                      </span>
                    </div>
                  </div>

                  <div className="category-items-list">
                    {duplicateGroups.slice(0, visibleLimit).map((group) => (
                      <div
                        key={group.groupIdx}
                        className="duplicate-group-card"
                      >
                        <div className="group-evidence">
                          <Copy size={14} />
                          <span>{group.evidence}</span>
                        </div>
                        <div className="duplicate-group-files">
                          {group.fileIds.slice(0, visibleLimit).map((id) => {
                            const file = filesMap.get(id);
                            const isKept = id === group.keptId;
                            const isSelected = selectedIds.has(id);

                            return (
                              <div
                                key={id}
                                className={`duplicate-file-row ${isKept ? "kept-row" : isSelected ? "selected-row" : ""}`}
                              >
                                <div className="file-select-side">
                                  {isKept ? (
                                    <span className="kept-badge">
                                      <Check size={12} />
                                      <span>KEPT (PRESERVED)</span>
                                    </span>
                                  ) : (
                                    <button
                                      type="button"
                                      className="checkbox-btn"
                                      onClick={() => toggleItem(id)}
                                    >
                                      {isSelected ? (
                                        <CheckSquare
                                          size={17}
                                          className="check-icon checked"
                                        />
                                      ) : (
                                        <Square
                                          size={17}
                                          className="check-icon"
                                        />
                                      )}
                                    </button>
                                  )}
                                </div>

                                <div className="file-main-info">
                                  <FileText size={14} className="file-icon" />
                                  <button
                                    type="button"
                                    className="btn-link"
                                    onClick={() =>
                                      void invoke("reveal_indexed_file", {
                                        scopeId,
                                        fileId: id,
                                      }).catch((e) => setError(String(e)))
                                    }
                                  >
                                    {file ? file.display : `File #${id}`}
                                  </button>
                                </div>

                                <div className="file-extra-side">
                                  <span className="file-size-tag">
                                    {file ? formatBytes(file.size) : ""}
                                  </span>
                                  {!isKept && (
                                    <button
                                      type="button"
                                      className="btn-link"
                                      onClick={() =>
                                        setKeptDuplicate(group.groupIdx, id)
                                      }
                                      title="Keep this file copy instead and mark the other copies for removal"
                                    >
                                      Keep this copy instead
                                    </button>
                                  )}
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}

            {/* Category 2: Old Installers */}
            {(categoryFilter === "all" || categoryFilter === "old_installer") &&
              categorizedItems.old_installer.length > 0 && (
                <div className="category-section">
                  <div className="category-header">
                    <button
                      type="button"
                      className="checkbox-btn"
                      onClick={() => toggleCategory("old_installer")}
                      title="Toggle all old installers"
                    >
                      {installerState.state === "checked" ? (
                        <CheckSquare size={18} className="check-icon checked" />
                      ) : installerState.state === "indeterminate" ? (
                        <MinusSquare
                          size={18}
                          className="check-icon indeterminate"
                        />
                      ) : (
                        <Square size={18} className="check-icon" />
                      )}
                    </button>
                    <div className="category-header-title">
                      <span className="finding-kind-badge old_installer">
                        OLD INSTALLERS
                      </span>
                      <strong>
                        Outdated Installation Packages (&gt;30 days old)
                      </strong>
                      <span className="category-stats-chip">
                        {installerState.selected} of {installerState.total}{" "}
                        selected
                      </span>
                    </div>
                  </div>

                  <div className="category-items-list">
                    {categorizedItems.old_installer
                      .slice(0, visibleLimit)
                      .map((item) => {
                        const isSelected = selectedIds.has(item.id);
                        return (
                          <div
                            key={item.id}
                            className={`item-row ${isSelected ? "selected-row" : ""}`}
                            onClick={() => toggleItem(item.id)}
                          >
                            <button
                              type="button"
                              className="checkbox-btn"
                              onClick={(e) => {
                                e.stopPropagation();
                                toggleItem(item.id);
                              }}
                            >
                              {isSelected ? (
                                <CheckSquare
                                  size={17}
                                  className="check-icon checked"
                                />
                              ) : (
                                <Square size={17} className="check-icon" />
                              )}
                            </button>
                            <div className="item-info">
                              <button
                                type="button"
                                className="btn-link"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void invoke("reveal_indexed_file", {
                                    scopeId,
                                    fileId: item.id,
                                  }).catch((e) => setError(String(e)));
                                }}
                              >
                                {item.file
                                  ? item.file.display
                                  : `File #${item.id}`}
                              </button>
                              <span className="item-sub">{item.evidence}</span>
                            </div>
                            <span className="item-size">
                              {item.file ? formatBytes(item.file.size) : ""}
                            </span>
                          </div>
                        );
                      })}
                  </div>
                </div>
              )}

            {/* Category 3: Development Build Artifacts */}
            {(categoryFilter === "all" ||
              categoryFilter === "development_artifact") &&
              categorizedItems.development_artifact.length > 0 && (
                <div className="category-section">
                  <div className="category-header">
                    <button
                      type="button"
                      className="checkbox-btn"
                      onClick={() => toggleCategory("development_artifact")}
                      title="Toggle all development artifacts"
                    >
                      {devState.state === "checked" ? (
                        <CheckSquare size={18} className="check-icon checked" />
                      ) : devState.state === "indeterminate" ? (
                        <MinusSquare
                          size={18}
                          className="check-icon indeterminate"
                        />
                      ) : (
                        <Square size={18} className="check-icon" />
                      )}
                    </button>
                    <div className="category-header-title">
                      <span className="finding-kind-badge development_artifact">
                        DEV ARTIFACTS
                      </span>
                      <strong>
                        Temporary Build Artifacts (node_modules, target,
                        .pyc...)
                      </strong>
                      <span className="category-stats-chip">
                        {devState.selected} of {devState.total} selected
                      </span>
                    </div>
                  </div>

                  <div className="category-items-list">
                    {categorizedItems.development_artifact
                      .slice(0, visibleLimit)
                      .map((item) => {
                        const isSelected = selectedIds.has(item.id);
                        return (
                          <div
                            key={item.id}
                            className={`item-row ${isSelected ? "selected-row" : ""}`}
                            onClick={() => toggleItem(item.id)}
                          >
                            <button
                              type="button"
                              className="checkbox-btn"
                              onClick={(e) => {
                                e.stopPropagation();
                                toggleItem(item.id);
                              }}
                            >
                              {isSelected ? (
                                <CheckSquare
                                  size={17}
                                  className="check-icon checked"
                                />
                              ) : (
                                <Square size={17} className="check-icon" />
                              )}
                            </button>
                            <div className="item-info">
                              <button
                                type="button"
                                className="btn-link"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void invoke("reveal_indexed_file", {
                                    scopeId,
                                    fileId: item.id,
                                  }).catch((e) => setError(String(e)));
                                }}
                              >
                                {item.file
                                  ? item.file.display
                                  : `File #${item.id}`}
                              </button>
                              <span className="item-sub">{item.evidence}</span>
                            </div>
                            <span className="item-size">
                              {item.file ? formatBytes(item.file.size) : ""}
                            </span>
                          </div>
                        );
                      })}
                  </div>
                </div>
              )}

            {/* Category 4: Large Files */}
            {(categoryFilter === "all" || categoryFilter === "large_file") &&
              categorizedItems.large_file.length > 0 && (
                <div className="category-section">
                  <div className="category-header">
                    <button
                      type="button"
                      className="checkbox-btn"
                      onClick={() => toggleCategory("large_file")}
                      title="Toggle all large files"
                    >
                      {largeState.state === "checked" ? (
                        <CheckSquare size={18} className="check-icon checked" />
                      ) : largeState.state === "indeterminate" ? (
                        <MinusSquare
                          size={18}
                          className="check-icon indeterminate"
                        />
                      ) : (
                        <Square size={18} className="check-icon" />
                      )}
                    </button>
                    <div className="category-header-title">
                      <span className="finding-kind-badge large_file">
                        LARGE FILES (&gt;50MB)
                      </span>
                      <strong>Files taking substantial disk space</strong>
                      <span className="category-stats-chip">
                        {largeState.selected} of {largeState.total} selected
                      </span>
                    </div>
                  </div>

                  <div className="category-items-list">
                    {categorizedItems.large_file
                      .slice(0, visibleLimit)
                      .map((item) => {
                        const isSelected = selectedIds.has(item.id);
                        return (
                          <div
                            key={item.id}
                            className={`item-row ${isSelected ? "selected-row" : ""}`}
                            onClick={() => toggleItem(item.id)}
                          >
                            <button
                              type="button"
                              className="checkbox-btn"
                              onClick={(e) => {
                                e.stopPropagation();
                                toggleItem(item.id);
                              }}
                            >
                              {isSelected ? (
                                <CheckSquare
                                  size={17}
                                  className="check-icon checked"
                                />
                              ) : (
                                <Square size={17} className="check-icon" />
                              )}
                            </button>
                            <div className="item-info">
                              <button
                                type="button"
                                className="btn-link"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void invoke("reveal_indexed_file", {
                                    scopeId,
                                    fileId: item.id,
                                  }).catch((e) => setError(String(e)));
                                }}
                              >
                                {item.file
                                  ? item.file.display
                                  : `File #${item.id}`}
                              </button>
                              <span className="item-sub">{item.evidence}</span>
                            </div>
                            <span className="item-size">
                              {item.file ? formatBytes(item.file.size) : ""}
                            </span>
                          </div>
                        );
                      })}
                  </div>
                </div>
              )}

            {result.findings.length === 0 && (
              <div className="no-actions-notice">
                <CheckCircle2 size={20} />
                <span>
                  No large files, duplicate copies, or old installers detected.
                  Folder is tidy!
                </span>
              </div>
            )}
          </div>

          {/* Cleanup Proposal Preview if generated */}
          {cleanupProposal && (
            <div
              className="proposal-result-panel"
              style={{ marginTop: "24px" }}
            >
              <div className="rationale-card">
                <div className="rationale-header">
                  <ShieldCheck size={22} className="shield-icon" />
                  <div>
                    <h4>Storage Cleanup Proposal</h4>
                    <p>{cleanupProposal.rationale}</p>
                  </div>
                </div>
                <div className="rationale-actions">
                  <button
                    type="button"
                    className="btn-accept-proposal"
                    onClick={() => void requestCleanupApproval()}
                    disabled={
                      requestingApproval || cleanupProposal.actions.length === 0
                    }
                  >
                    {requestingApproval ? (
                      <>
                        <LoaderCircle size={15} className="spinning" />
                        <span>Verifying Paths…</span>
                      </>
                    ) : (
                      <>
                        <Trash2 size={15} />
                        <span>
                          Confirm & Move {cleanupProposal.actions.length} Files
                          to Trash
                        </span>
                      </>
                    )}
                  </button>
                </div>
              </div>

              <div className="proposal-actions-table-wrap">
                <table className="proposal-table">
                  <thead>
                    <tr>
                      <th style={{ width: "80px" }}>Action</th>
                      <th>Candidate File</th>
                      <th style={{ width: "30px" }}></th>
                      <th>Destination</th>
                      <th style={{ width: "100px", textAlign: "right" }}>
                        Size
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {paginatedCleanupActions.map((act, index) => {
                      const file = filesMap.get(act.source);
                      const sourceName = file
                        ? file.display
                        : `File #${act.source}`;
                      const sizeStr = file ? formatBytes(file.size) : "";

                      return (
                        <tr key={index}>
                          <td>
                            <span className="action-badge trash">TRASH</span>
                          </td>
                          <td className="source-path">{sourceName}</td>
                          <td className="arrow-cell">
                            <ArrowRight size={14} />
                          </td>
                          <td className="dest-path">
                            <span className="trash-dest">
                              Native Trash (Reversible)
                            </span>
                          </td>
                          <td className="size-cell">{sizeStr}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>

                {cleanupProposal.actions.length > CLEANUP_PAGE_SIZE && (
                  <div className="table-pagination-bar">
                    <span className="pagination-info">
                      Showing {cleanupPage * CLEANUP_PAGE_SIZE + 1}–
                      {Math.min(
                        (cleanupPage + 1) * CLEANUP_PAGE_SIZE,
                        cleanupProposal.actions.length,
                      )}{" "}
                      of {cleanupProposal.actions.length} items
                    </span>
                    <div className="pagination-buttons">
                      <button
                        type="button"
                        className="btn-page-nav"
                        disabled={cleanupPage === 0}
                        onClick={() =>
                          setCleanupPage((p) => Math.max(0, p - 1))
                        }
                      >
                        <ChevronLeft size={16} />
                        <span>Previous</span>
                      </button>
                      <span className="page-indicator">
                        Page {cleanupPage + 1} of {totalCleanupPages}
                      </span>
                      <button
                        type="button"
                        className="btn-page-nav"
                        disabled={cleanupPage >= totalCleanupPages - 1}
                        onClick={() =>
                          setCleanupPage((p) =>
                            Math.min(totalCleanupPages - 1, p + 1),
                          )
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
                      Ready for safe cleanup. Moving{" "}
                      {cleanupProposal.actions.length} file(s) to OS Trash
                      requires explicit one-use approval.
                    </span>
                  </div>
                  <button
                    type="button"
                    className="btn-request-approval"
                    onClick={() => void requestCleanupApproval()}
                    disabled={
                      requestingApproval || cleanupProposal.actions.length === 0
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
                          Approve & Move to Trash (
                          {cleanupProposal.actions.length})
                        </span>
                      </>
                    )}
                  </button>
                </div>
              </div>

              <div className="safety-disclaimer">
                <ShieldCheck size={14} />
                <span>
                  <strong>Safety policy:</strong> TIDY uses native OS Trash
                  only; files are never permanently deleted (`rm -rf` is
                  prohibited). Every Trash action records its recovery location.
                  Restore through Finder Trash; automatic Trash undo is not
                  available.
                </span>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
