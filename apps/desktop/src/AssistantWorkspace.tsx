import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUp,
  ArrowRight,
  Check,
  Folder,
  LoaderCircle,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Square,
  X,
  Undo2,
} from "lucide-react";
type Action =
  | { action: "move" | "copy"; source: number; destination_relative: string }
  | { action: "trash"; source: number }
  | { action: "permissions"; source: number; mode: number };
function destination(a: Action) {
  return a.action === "trash"
    ? "Trash"
    : a.action === "permissions"
      ? `Permissions ${a.mode.toString(8)}`
      : a.destination_relative;
}
function actionGroup(a: Action) {
  return a.action === "trash"
    ? "Trash"
    : a.action === "permissions"
      ? "Permissions"
      : a.destination_relative.split("/").slice(0, -1).join("/") ||
        "(folder root)";
}
type ConversationTurn = {
  role: "user" | "assistant";
  text: string;
  trace?: Trace[];
};
type Workflow = {
  id: string;
  title: string;
  mode: string;
  context: string;
  steps: string;
  handoff: "storage" | "history" | null;
};
type Trace = { label: string; detail: string };
type Plan = {
  engine: "model" | "local_filter";
  workflow: Workflow | null;
  proposal: { actions: Action[]; rationale: string };
  sources: { id: number; path: string; size: number }[];
  trace: Trace[];
  clarification: string | null;
  indexed: number;
  examined: number;
  remaining_matches: number;
  complete: boolean;
};
type Approval = {
  token: string;
  tx_id: number;
  actions_count: number;
  actions: {
    type: "move" | "rename" | "trash" | "copy" | "permissions";
    old_mode?: number;
    new_mode?: number;
    relative_source: string;
    relative_dest?: string;
  }[];
};
type Report = { transaction_id: number; actions_applied: number };
const prompts = [
  {
    title: "Organize photos",
    text: "Put all photos into a Photos folder with subfolders by format. Leave other files alone.",
  },
  {
    title: "Find a project",
    text: "Find files related to my project and propose a useful folder structure. Ask me which project first.",
  },
  {
    title: "Make sense of this folder",
    text: "Investigate this folder and propose an organization that fits its contents. Preserve existing project folders.",
  },
];
function size(n: number) {
  return n < 1024
    ? n + " B"
    : n < 1048576
      ? (n / 1024).toFixed(0) + " KB"
      : (n / 1048576).toFixed(1) + " MB";
}
export default function AssistantWorkspace({
  scopeId,
  scopeName,
  indexed,
  scanning,
  snapshot,
  onScan,
  onModels,
  onRefresh,
  onHistory,
  onStorage,
}: {
  scopeId: number;
  scopeName: string;
  indexed: number;
  scanning: boolean;
  snapshot: number | null;
  onScan: () => void;
  onModels: () => void;
  onRefresh: () => void;
  onHistory: () => void;
  onStorage: () => void;
}) {
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const [workflowId, setWorkflowId] = useState("");
  const [request, setRequest] = useState("");
  const [submitted, setSubmitted] = useState("");
  const [conversation, setConversation] = useState<ConversationTurn[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [trace, setTrace] = useState<Trace[]>([]);
  const [error, setError] = useState("");
  const [selected, setSelected] = useState(new Set<number>());
  const [approval, setApproval] = useState<Approval | null>(null);
  const [undoOriginal, setUndoOriginal] = useState<number | null>(null);
  const [working, setWorking] = useState(false);
  const [reportHasTrash, setReportHasTrash] = useState(false);
  const [report, setReport] = useState<Report | null>(null);
  const [limit, setLimit] = useState(30);
  const [filter, setFilter] = useState("");
  const chatEnd = useRef<HTMLDivElement>(null);
  useEffect(() => {
    chatEnd.current?.scrollIntoView({ block: "end", behavior: "smooth" });
  }, [conversation.length, busy]);
  const mounted = useRef(true);
  const snapshotRef = useRef(snapshot);
  snapshotRef.current = snapshot;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    setPlan(null);
    setApproval(null);
    setSelected(new Set());
  }, [snapshot]);
  useEffect(() => {
    if (!busy) return;
    let disposed = false;
    const poll = async () => {
      try {
        const status = await invoke<{
          job: {
            scope_id: number | null;
            operation: string;
            message: string;
            planning_trace: Trace[];
          };
        }>("ai_status");
        if (
          !disposed &&
          status.job.scope_id === scopeId &&
          status.job.operation === "planning"
        ) {
          setMessage(status.job.message);
          setTrace(status.job.planning_trace);
        }
      } catch {
        /* IPC failure is reported by the planning request. */
      }
    };
    void poll();
    const timer = setInterval(() => void poll(), 900);
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, [busy, scopeId]);
  useEffect(() => {
    if (!approval) return;
    const previousFocus = document.activeElement as HTMLElement | null;
    const dialog = document.querySelector<HTMLElement>(".assistant-approval");
    const items = () =>
      Array.from(
        dialog?.querySelectorAll<HTMLElement>(
          "button:not(:disabled),[tabindex='0']",
        ) ?? [],
      );
    items()[0]?.focus();
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !working) {
        setApproval(null);
      }
      if (event.key === "Tab") {
        const list = items();
        const first = list[0],
          last = list.at(-1);
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
      previousFocus?.focus();
    };
  }, [approval, working]);
  useEffect(() => {
    invoke<Workflow[]>("workflow_catalog")
      .then(setWorkflows)
      .catch((e) => setError(String(e)));
  }, []);
  const sources = useMemo(
    () => new Map(plan?.sources.map((f) => [f.id, f]) ?? []),
    [plan],
  );
  const folders = useMemo(() => {
    const map = new Map<string, number>();
    for (const a of plan?.proposal.actions ?? []) {
      const folder = actionGroup(a);
      map.set(folder, (map.get(folder) ?? 0) + 1);
    }
    return [...map].sort(([a], [b]) => a.localeCompare(b));
  }, [plan]);
  const rows = (plan?.proposal.actions ?? []).filter(
    (a) => !filter || actionGroup(a) === filter,
  );
  function newRequest() {
    setConversation([]);
    setReport(null);
    setWorkflowId("");
    setRequest("");
    setSubmitted("");
    setPlan(null);
    setApproval(null);
    setError("");
    setTrace([]);
    document.getElementById("assistant-message")?.focus();
  }
  async function investigate(continuePlan = false) {
    const text = request.trim();
    const refining =
      /^(actually|instead|only|yes|no[,. ]|keep|leave|exclude|except|use|put (them|those)|all (of )?(them|those)|well|in |into |by |to )/i.test(
        text,
      );
    const followsCurrentRequest =
      conversation.length > 0 && !report && (!!plan?.clarification || refining);
    const goal = continuePlan
      ? submitted
      : followsCurrentRequest
        ? `${submitted}\nUser follow-up: ${text}`
        : text;
    const turns: ConversationTurn[] = continuePlan
      ? conversation
      : [...conversation, { role: "user", text }];
    if (!goal || (!continuePlan && !text) || scanning) return;
    const startingSnapshot = snapshotRef.current;
    const previousIds = new Set(
      plan?.proposal.actions.map((a) => a.source) ?? [],
    );
    setConversation(turns);
    if (!continuePlan) setRequest("");
    setBusy(true);
    setError("");
    setApproval(null);
    setReport(null);
    setMessage("Checking request and indexed files…");
    setTrace([]);
    if (!continuePlan) {
      setPlan(null);
      setSubmitted(goal);
      setFilter("");
      setLimit(30);
    }
    try {
      const result = await invoke<Plan>("plan_with_agent", {
        scopeId,
        request: goal,
        conversation: turns.slice(-4).map(({ role, text }) => ({ role, text })),
        workflowId: continuePlan
          ? (plan?.workflow?.id ?? workflowId) || null
          : workflowId || null,
        previous: continuePlan ? (plan?.proposal.actions ?? []) : [],
      });
      if (!mounted.current) return;
      if (startingSnapshot !== snapshotRef.current) {
        setError(
          "The index refreshed during investigation. Ask again using the updated snapshot.",
        );
        return;
      }
      setPlan(result);

      setConversation([
        ...turns,
        {
          role: "assistant",
          trace: result.trace,
          text:
            result.clarification ||
            result.proposal.rationale ||
            "Preview ready for review.",
        },
      ]);
      setTrace(result.trace);
      setSelected(
        (prev) =>
          new Set(
            result.proposal.actions
              .filter(
                (a) =>
                  !continuePlan ||
                  !previousIds.has(a.source) ||
                  prev.has(a.source),
              )
              .map((a) => a.source),
          ),
      );
    } catch (e) {
      if (mounted.current) setError(String(e));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  async function approve(undoId: number | null = null) {
    setWorking(true);
    setError("");
    try {
      const view =
        undoId !== null
          ? await invoke<Approval>("request_undo_approval", {
              scopeId,
              txId: undoId,
            })
          : await invoke<Approval>("request_plan_approval", {
              scopeId,
              rationale: plan?.proposal.rationale ?? submitted,
              actions:
                plan?.proposal.actions.filter((a) => selected.has(a.source)) ??
                [],
            });
      setUndoOriginal(undoId);
      setApproval(view);
    } catch (e) {
      setError(String(e));
    } finally {
      setWorking(false);
    }
  }
  async function apply() {
    if (!approval) return;
    setWorking(true);
    setError("");
    try {
      const result =
        undoOriginal !== null
          ? await invoke<Report>("execute_approved_undo", {
              scopeId,
              originalTxId: undoOriginal,
              undoToken: approval.token,
            })
          : await invoke<Report>("execute_approved_plan", {
              scopeId,
              token: approval.token,
            });
      setReportHasTrash(approval.actions.some((a) => a.type === "trash"));
      setReport(result);
      setConversation((prev) => [
        ...prev,
        {
          role: "assistant",
          text: `${result.actions_applied} approved changes completed and verified. Details and recovery are available in History.`,
        },
      ]);
      setPlan(null);
      setSelected(new Set());
      setApproval(null);
      onRefresh();
      setMessage(
        undoOriginal !== null
          ? "Verified moves restored."
          : "Approved changes completed and verified.",
      );
      setUndoOriginal(null);
    } catch (e) {
      setApproval(null);
      setError(String(e));
      onRefresh();
    } finally {
      setWorking(false);
    }
  }
  function toggle(id: number) {
    setSelected((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  }
  async function reveal(id: number) {
    try {
      await invoke("reveal_indexed_file", { scopeId, fileId: id });
    } catch (e) {
      setError(String(e));
    }
  }
  const planReview = (
    <>
      {plan?.workflow && (
        <div className="workflow-selected">
          <Sparkles size={17} />
          <div>
            <strong>{plan.workflow.title}</strong>
            <p>{plan.workflow.steps}</p>
          </div>
          {plan.workflow.handoff && (
            <button
              className="secondary"
              onClick={
                plan.workflow.handoff === "storage" ? onStorage : onHistory
              }
            >
              Open {plan.workflow.handoff === "storage" ? "Storage" : "History"}
            </button>
          )}
        </div>
      )}
      {report && (
        <div className="assistant-success">
          <Check size={20} />
          <div>
            <strong>{message}</strong>
            <p>
              {report.actions_applied} verified actions. You can review the
              journal any time.
            </p>
          </div>
          <button
            onClick={() =>
              reportHasTrash ? onHistory() : void approve(report.transaction_id)
            }
            disabled={working}
          >
            <Undo2 size={15} />
            {reportHasTrash ? "Trash recovery in History" : "Review undo"}
          </button>
        </div>
      )}
      {plan && !plan.clarification && (
        <div className="assistant-plan">
          <div className="assistant-plan-heading">
            <div>
              <span className="assistant-kicker">
                {plan.engine === "local_filter"
                  ? "EXACT LOCAL FILTER · NO FILES CHANGED"
                  : "AI PROPOSAL · NO FILES CHANGED"}
              </span>
              <h3>
                {plan.workflow &&
                ["observe", "storage", "history"].includes(plan.workflow.mode)
                  ? "Findings and next steps"
                  : "Here’s the plan"}
              </h3>
            </div>
            <span
              className={
                plan.complete ? "coverage-pill" : "coverage-pill partial"
              }
            >
              {plan.complete ? "Investigation finished" : "Partial preview"}
            </span>
          </div>
          <div className="assistant-evidence">
            <span>{plan.indexed.toLocaleString()} files searchable</span>
            <span>{plan.examined.toLocaleString()} examined</span>
            <span>{plan.proposal.actions.length} proposed changes</span>
            {plan.remaining_matches > 0 && (
              <span>{plan.remaining_matches} matches outside this batch</span>
            )}
          </div>
          {!plan.complete && plan.engine !== "local_filter" && (
            <button
              className="assistant-continue"
              disabled={busy || working || plan.proposal.actions.length >= 500}
              onClick={() => void investigate(true)}
            >
              <Sparkles size={15} />
              Continue investigation
            </button>
          )}
          {folders.length > 0 && (
            <div className="assistant-review-layout">
              <aside className="assistant-folder-tree">
                <h4>Proposed structure</h4>
                <button
                  className={!filter ? "chosen" : ""}
                  onClick={() => {
                    setFilter("");
                    setLimit(30);
                  }}
                >
                  <Folder size={16} />
                  All destinations<span>{plan.proposal.actions.length}</span>
                </button>
                {folders.map(([folder, count]) => (
                  <button
                    key={folder}
                    title={folder}
                    className={filter === folder ? "chosen" : ""}
                    onClick={() => {
                      setFilter(folder);
                      setLimit(30);
                    }}
                    style={{
                      paddingLeft:
                        12 + Math.min(folder.split("/").length - 1, 4) * 10,
                    }}
                  >
                    <Folder size={15} />
                    <span>{folder}</span>
                    <small>{count}</small>
                  </button>
                ))}
              </aside>
              <div className="assistant-file-review">
                <div className="assistant-file-toolbar">
                  <strong>{filter || "Review proposed changes"}</strong>
                  <label>
                    <input
                      type="checkbox"
                      checked={selected.size === plan.proposal.actions.length}
                      onChange={(e) =>
                        setSelected(
                          e.target.checked
                            ? new Set(
                                plan.proposal.actions.map((a) => a.source),
                              )
                            : new Set(),
                        )
                      }
                    />
                    Select all
                  </label>
                </div>
                {rows.slice(0, limit).map((a) => (
                  <div className="assistant-move" key={a.source}>
                    <input
                      aria-label={
                        "Select " + (sources.get(a.source)?.path ?? a.source)
                      }
                      type="checkbox"
                      checked={selected.has(a.source)}
                      onChange={() => toggle(a.source)}
                    />
                    <div>
                      <button
                        className="assistant-source"
                        onClick={() => void reveal(a.source)}
                      >
                        {sources.get(a.source)?.path ?? a.source}
                      </button>
                      <span
                        className={
                          a.action === "trash"
                            ? "assistant-destination assistant-trash"
                            : "assistant-destination"
                        }
                      >
                        <ArrowRight size={12} />
                        {a.action === "trash"
                          ? "Move to Trash · restore from Finder"
                          : `${a.action === "copy" ? "Copy to " : ""}${destination(a)}`}
                      </span>
                    </div>
                    <small>{size(sources.get(a.source)?.size ?? 0)}</small>
                  </div>
                ))}
                {rows.length > limit && (
                  <button
                    className="assistant-more"
                    onClick={() => setLimit((n) => n + 30)}
                  >
                    Show 30 more ({rows.length - limit} remaining)
                  </button>
                )}
              </div>
            </div>
          )}
          {plan.proposal.actions.length > 0 && (
            <div className="assistant-review-footer">
              <span>
                <ShieldCheck size={16} />
                {selected.size} changes selected · Moves support undo; Trash
                restores from Finder
              </span>
              <button
                className="assistant-refine"
                disabled={busy || working}
                onClick={() => {
                  document.getElementById("assistant-message")?.focus();
                }}
              >
                Refine request
              </button>
              <button
                className="primary"
                disabled={busy || working || scanning || !selected.size}
                onClick={() => void approve()}
              >
                {working ? (
                  <LoaderCircle size={16} className="spinning" />
                ) : (
                  <ShieldCheck size={16} />
                )}
                Review approval
              </button>
            </div>
          )}
        </div>
      )}
    </>
  );
  return (
    <section className="assistant-workspace">
      <div className="assistant-intro">
        <div className="assistant-avatar">
          <img src="/tidy-mascot.svg" alt="Tidy assistant" />
        </div>
        <div>
          <span className="assistant-kicker">YOUR LOCAL FILE ASSISTANT</span>
          <h2>What would you like to change?</h2>
          <p>
            Describe the outcome. I’ll investigate <strong>{scopeName}</strong>,
            build a plan, and bring it back for your review.
          </p>
        </div>
      </div>
      <p className="assistant-notice">
        Move · rename · change extensions · copy · file permissions · Trash.
        Every change is reviewed. Extension renames keep file contents
        unchanged.
      </p>
      <div className="assistant-scope">
        <Folder size={16} />
        <strong>{scopeName}</strong>
        <span>{indexed.toLocaleString()} indexed files</span>
        <span className="assistant-private">
          <ShieldCheck size={13} />
          On your device
        </span>
        <button onClick={onScan} disabled={busy || working || scanning}>
          <RefreshCw size={13} />
          {scanning ? "Scanning…" : indexed ? "Refresh index" : "Scan folder"}
        </button>
      </div>
      {!indexed && (
        <p className="assistant-notice">
          Scan this folder to give the assistant an index to investigate.
          Optional text indexing makes project and content searches more useful.
        </p>
      )}
      {!submitted && !busy && (
        <div className="assistant-suggestions">
          {prompts.map((p) => (
            <button key={p.title} onClick={() => setRequest(p.text)}>
              <Sparkles size={16} />
              <span>{p.title}</span>
              <ArrowRight size={14} />
            </button>
          ))}
        </div>
      )}
      <div className="assistant-chat-messages" aria-label="Chat with Tidy">
        {conversation.map((turn, i) => (
          <article key={i} className={`conversation-turn ${turn.role}`}>
            <small>{turn.role === "user" ? "You" : "Tidy"}</small>
            <p>{turn.text}</p>
            {turn.role === "assistant" && !!turn.trace?.length && (
              <details className="chat-reflection">
                <summary>Investigation · {turn.trace.length} steps</summary>
                <ol>
                  {turn.trace.map((t, n) => (
                    <li key={n}>
                      <strong>{t.label}</strong>
                      <p>{t.detail}</p>
                    </li>
                  ))}
                </ol>
              </details>
            )}
            {turn.role === "assistant" &&
              i === conversation.length - 1 &&
              planReview}
          </article>
        ))}
      </div>
      {error && (
        <div className="assistant-error" role="alert">
          <small>Tidy</small>
          <strong>Investigation paused</strong>
          <p>{error}</p>
          <div>
            <button
              disabled={busy || working || scanning}
              onClick={() => void investigate(true)}
            >
              Retry
            </button>
            <button onClick={onModels}>Model settings</button>
            <button onClick={onHistory}>Check history</button>
          </div>
        </div>
      )}
      {busy && (
        <article
          className="conversation-turn assistant chat-working"
          role="status"
        >
          <small>Tidy</small>
          <div className="chat-progress">
            <LoaderCircle size={19} className="spinning" />
            <strong>{message}</strong>
            <button onClick={() => void invoke("cancel_ai")}>
              <Square size={13} />
              Stop
            </button>
          </div>
          {trace.length > 0 && (
            <details className="chat-reflection" open>
              <summary>Investigation · {trace.length} steps</summary>
              <ol>
                {trace.map((t, i) => (
                  <li key={i}>
                    <strong>{t.label}</strong>
                    <p>{t.detail}</p>
                  </li>
                ))}
              </ol>
            </details>
          )}
        </article>
      )}
      <form
        className="assistant-composer"
        onSubmit={(e) => {
          e.preventDefault();
          void investigate();
        }}
      >
        <details className="chat-options">
          <summary>
            Options ·{" "}
            {workflowId
              ? workflows.find((w) => w.id === workflowId)?.title
              : "Automatic workflow"}
          </summary>
          <div className="workflow-picker">
            <label htmlFor="assistant-workflow">Workflow</label>
            <select
              id="assistant-workflow"
              value={workflowId}
              disabled={busy || working}
              onChange={(e) => setWorkflowId(e.target.value)}
            >
              <option value="">
                Automatic · AI selects from {workflows.length || 42} skills
              </option>
              {workflows.map((w) => (
                <option key={w.id} value={w.id}>
                  {w.title}
                </option>
              ))}
            </select>
            {workflowId && (
              <small>
                {workflows.find((w) => w.id === workflowId)?.context}
              </small>
            )}
          </div>
          <button
            type="button"
            className="secondary"
            disabled={busy || working}
            onClick={newRequest}
          >
            Clear conversation
          </button>
        </details>
        <label className="sr-only" htmlFor="assistant-message">
          Message Tidy
        </label>
        <textarea
          id="assistant-message"
          value={request}
          onChange={(e) => setRequest(e.target.value)}
          placeholder={plan?.clarification ? "Reply to Tidy…" : "Message Tidy…"}
          rows={2}
          maxLength={1500}
          disabled={busy || working || scanning}
          onKeyDown={(e) => {
            if (
              e.key === "Enter" &&
              !e.shiftKey &&
              !e.nativeEvent.isComposing
            ) {
              e.preventDefault();
              if (!busy && !working && !scanning && request.trim() && indexed)
                e.currentTarget.form?.requestSubmit();
            }
          }}
        />
        <div className="composer-bottom">
          <span>
            <Sparkles size={14} />
            Local · Changes need your approval
          </span>
          <button
            className="assistant-send"
            type="submit"
            disabled={
              busy || working || scanning || !request.trim() || !indexed
            }
            aria-label="Send message"
          >
            <ArrowUp size={20} />
          </button>
        </div>
      </form>
      <div ref={chatEnd} />
      {approval && (
        <div className="approval-modal-backdrop">
          <section
            className="assistant-approval"
            role="dialog"
            aria-modal="true"
            aria-labelledby="assistant-approve-title"
          >
            <button
              className="assistant-close"
              aria-label="Cancel approval"
              disabled={working}
              onClick={() => setApproval(null)}
            >
              <X size={18} />
            </button>
            <ShieldCheck size={30} />
            <h2 id="assistant-approve-title">
              {undoOriginal !== null
                ? "Restore these moves?"
                : "Apply your selected changes?"}
            </h2>
            <p>
              {approval.actions_count} file changes were checked against the
              current index. Tidy rechecks each source and destination before
              execution.
            </p>
            <strong>{scopeName}</strong>
            <div className="assistant-approved-moves">
              {approval.actions.map((a, i) => (
                <div key={i}>
                  <span>{a.relative_source}</span>
                  <ArrowRight size={13} />
                  <strong
                    className={a.type === "trash" ? "assistant-trash" : ""}
                  >
                    {a.type === "trash"
                      ? "Move to Trash"
                      : a.type === "permissions"
                        ? `Permissions ${a.old_mode?.toString(8)} → ${a.new_mode?.toString(8)}`
                        : `${a.type === "copy" ? "Copy to " : ""}${a.relative_dest}`}
                  </strong>
                </div>
              ))}
            </div>
            <p className="assistant-notice">
              This batch runs step by step. If a later step fails, earlier
              verified changes appear in History. Moves and permissions support
              undo; undoing copies sends the copies to Trash. Trashed files must
              be restored from Finder using the recovery location in History.
              Nothing is permanently deleted.
            </p>
            <div>
              <button
                className="secondary"
                disabled={working}
                onClick={() => setApproval(null)}
              >
                Cancel
              </button>
              <button
                className="primary"
                disabled={working || scanning}
                onClick={() => void apply()}
              >
                {working ? (
                  <LoaderCircle size={16} className="spinning" />
                ) : (
                  <Check size={16} />
                )}
                Approve {approval.actions_count} changes
              </button>
            </div>
          </section>
        </div>
      )}
    </section>
  );
}
