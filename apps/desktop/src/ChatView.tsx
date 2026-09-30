import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUp,
  Check,
  ChevronRight,
  Folder,
  FolderOpen,
  LoaderCircle,
  ShieldCheck,
  Sparkles,
  Square,
  Trash2,
  Undo2,
  X,
} from "lucide-react";

type Action =
  | { action: "move" | "copy"; source: number; destination_relative: string }
  | { action: "trash"; source: number }
  | { action: "permissions"; source: number; mode: number };
type Trace = { label: string; detail: string };
type FolderTarget = { path: string; files: number; bytes: number };
type Plan = {
  engine: "model" | "local_filter" | "instant";
  proposal: { actions: Action[]; rationale: string };
  sources: { id: number; path: string; size: number }[];
  folders: FolderTarget[];
  trace: Trace[];
  clarification: string | null;
  indexed: number;
  remaining_matches: number;
  complete: boolean;
};
type Approval = {
  token: string;
  tx_id: number;
  actions_count: number;
  actions: {
    type: "move" | "rename" | "trash" | "trash_dir" | "copy" | "permissions";
    relative_source: string;
    relative_dest?: string;
    old_mode?: number;
    new_mode?: number;
    files?: number;
    original_size?: number;
  }[];
};
type Message = {
  role: "user" | "assistant";
  text: string;
  trace?: Trace[];
  plan?: Plan;
};
export type ChatScope = { id: number; name: string; files: number };

export function size(n: number) {
  if (n < 1024) return `${n} B`;
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), 4);
  return `${(n / 1024 ** i).toFixed(i > 1 ? 1 : 0)} ${["B", "KB", "MB", "GB", "TB"][i]}`;
}
const YES =
  /^(yes|yep|yeah|yup|ok|okay|sure|do it|go ahead|go|confirm|approve|proceed|please do|do that|oui|vas-y|yes please)\b/i;
const NO = /^(no|nope|cancel|never ?mind|stop|don'?t|non)\b/i;
const chips = [
  "What’s taking the most space?",
  "Delete the 10 biggest files",
  "Clean up node_modules and build artifacts",
  "Organize by type",
];

export default function ChatView({
  scopes,
  scopeId,
  scanning,
  prompt,
  onPromptSent,
  compact,
  onRefresh,
  onOpenFolders,
  onScan,
}: {
  scopes: ChatScope[];
  scopeId: number | null;
  scanning: boolean;
  prompt: string | null;
  onPromptSent: () => void;
  compact: boolean;
  onRefresh: () => void;
  onOpenFolders: () => void;
  onScan: () => void;
}) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [limit, setLimit] = useState(12);
  const [approval, setApproval] = useState<Approval | null>(null);
  const [working, setWorking] = useState(false);
  const [lastTx, setLastTx] = useState<{ id: number; reversible: boolean } | null>(
    null,
  );
  const end = useRef<HTMLDivElement>(null);
  const scope = scopes.find((s) => s.id === scopeId) ?? null;
  const lastPlan = [...messages].reverse().find((m) => m.plan)?.plan;
  const activePlan =
    messages.at(-1)?.role === "assistant" ? messages.at(-1)?.plan : undefined;

  useEffect(() => {
    setMessages([]);
    setApproval(null);
    setError("");
    setLastTx(null);
  }, [scopeId]);
  useEffect(() => {
    if (prompt && scopeId !== null && !busy && !scanning) {
      onPromptSent();
      void send(prompt);
    }
  }, [prompt, scopeId]);
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end", behavior: "smooth" });
  }, [messages.length, busy, approval]);
  useEffect(() => {
    if (!busy) return;
    const timer = setInterval(() => {
      invoke<{ job: { message: string } }>("ai_status")
        .then((s) => setStatus(s.job.message))
        .catch(() => {});
    }, 700);
    return () => clearInterval(timer);
  }, [busy]);

  const key = (a: Action) => `f${a.source}`;
  const sources = useMemo(
    () => new Map(activePlan?.sources.map((s) => [s.id, s]) ?? []),
    [activePlan],
  );
  const pickedFolders = activePlan?.folders.filter((f) =>
    picked.has(`d${f.path}`),
  );
  const pickedActions = activePlan?.proposal.actions.filter((a) =>
    picked.has(key(a)),
  );
  const pickedBytes =
    (pickedFolders?.reduce((n, f) => n + f.bytes, 0) ?? 0) +
    (pickedActions?.reduce((n, a) => n + (sources.get(a.source)?.size ?? 0), 0) ??
      0);
  const pickedCount =
    (pickedFolders?.length ?? 0) + (pickedActions?.length ?? 0);

  async function send(raw?: string) {
    const value = (raw ?? text).trim();
    if (!value || busy || working || scanning || scopeId === null) return;
    setText("");
    setError("");
    const next: Message[] = [...messages, { role: "user", text: value }];
    setMessages(next);
    if (activePlan && pickedCount > 0 && YES.test(value)) {
      await review();
      return;
    }
    if (activePlan && NO.test(value)) {
      setMessages([
        ...next,
        { role: "assistant", text: "Okay — nothing was changed." },
      ]);
      return;
    }
    setBusy(true);
    setStatus("Thinking…");
    try {
      const plan = await invoke<Plan>("plan_with_agent", {
        scopeId,
        request:
          activePlan?.clarification && lastPlan
            ? `${messages.filter((m) => m.role === "user").at(-1)?.text ?? ""}\nUser follow-up: ${value}`
            : value,
        conversation: next
          .slice(-4)
          .map(({ role, text }) => ({ role, text: text.slice(0, 2500) })),
        workflowId: null,
        previous: [],
      });
      setMessages([
        ...next,
        {
          role: "assistant",
          text:
            plan.clarification ||
            plan.proposal.rationale ||
            "Here’s what I found.",
          trace: plan.trace,
          plan,
        },
      ]);
      setPicked(
        new Set([
          ...plan.folders.map((f) => `d${f.path}`),
          ...plan.proposal.actions.map(key),
        ]),
      );
      setLimit(12);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function review(undoId: number | null = null) {
    if (scopeId === null) return;
    setWorking(true);
    setError("");
    try {
      let view: Approval;
      if (undoId !== null) {
        view = await invoke<Approval>("request_undo_approval", {
          scopeId,
          txId: undoId,
        });
      } else if (pickedFolders?.length) {
        view = await invoke<Approval>("request_folder_trash_approval", {
          scopeId,
          folders: pickedFolders.map((f) => f.path),
        });
      } else {
        view = await invoke<Approval>("request_plan_approval", {
          scopeId,
          rationale: activePlan?.proposal.rationale ?? "Requested changes",
          actions: pickedActions ?? [],
        });
      }
      setApproval(view);
      setUndoTarget(undoId);
    } catch (e) {
      setError(String(e));
    } finally {
      setWorking(false);
    }
  }
  const [undoTarget, setUndoTarget] = useState<number | null>(null);
  async function apply() {
    if (!approval || scopeId === null) return;
    setWorking(true);
    setError("");
    try {
      const report =
        undoTarget !== null
          ? await invoke<{ transaction_id: number; actions_applied: number }>(
              "execute_approved_undo",
              {
                scopeId,
                originalTxId: undoTarget,
                undoToken: approval.token,
              },
            )
          : await invoke<{ transaction_id: number; actions_applied: number }>(
              "execute_approved_plan",
              { scopeId, token: approval.token },
            );
      const trashed = approval.actions.some(
        (a) => a.type === "trash" || a.type === "trash_dir",
      );
      setLastTx({ id: report.transaction_id, reversible: !trashed });
      setMessages((m) => [
        ...m,
        {
          role: "assistant",
          text:
            undoTarget !== null
              ? "Restored. Everything is back where it was."
              : trashed
                ? `Done — ${approval.actions_count} ${approval.actions_count === 1 ? "item" : "items"} moved to the Trash. You can put them back from Finder’s Trash.`
                : `Done — ${report.actions_applied} changes applied and verified. You can undo them.`,
        },
      ]);
      setApproval(null);
      setUndoTarget(null);
      setPicked(new Set());
      onRefresh();
    } catch (e) {
      setApproval(null);
      setError(String(e));
      onRefresh();
    } finally {
      setWorking(false);
    }
  }
  function toggle(id: string) {
    setPicked((p) => {
      const n = new Set(p);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });
  }

  if (scopeId === null || !scope) {
    return (
      <div className="x-empty">
        <div className="x-empty-icon">
          <FolderOpen size={34} />
        </div>
        <h2>Choose what Tidy can see</h2>
        <p>
          Tidy only works inside folders you switch on. Open Storage, see what
          weighs the most, and turn on the ones you want help with.
        </p>
        <button className="x-primary" onClick={onOpenFolders}>
          Open Storage
        </button>
      </div>
    );
  }
  return (
    <div className={`x-chat ${compact ? "compact" : ""}`}>
      <div className="x-chat-scroll">
        {messages.length === 0 && !busy && (
          <div className="x-hello">
            <div className="x-hello-mark">
              <Sparkles size={26} />
            </div>
            <h2>What should I do in {scope.name}?</h2>
            <p>
              Just tell me. I’ll prepare it, show you exactly what changes, and
              wait for your OK.
            </p>
            {!scope.files && (
              <button className="x-secondary" onClick={onScan}>
                Scan this folder first
              </button>
            )}
            <div className="x-chips">
              {chips.map((c) => (
                <button key={c} onClick={() => void send(c)}>
                  {c}
                </button>
              ))}
            </div>
          </div>
        )}
        {messages.map((m, i) => (
          <div key={i} className={`x-row ${m.role}`}>
            <div className={`x-bubble ${m.role}`}>{m.text}</div>
            {m.role === "assistant" && m.plan && i === messages.length - 1 && (
              <PlanCard
                plan={m.plan}
                picked={picked}
                sources={sources}
                limit={limit}
                onMore={() => setLimit((n) => n + 20)}
                toggle={toggle}
                setPicked={setPicked}
                keyOf={key}
              />
            )}
            {m.role === "assistant" && !!m.trace?.length && (
              <details className="x-trace">
                <summary>How I got this</summary>
                {m.trace.map((t, n) => (
                  <p key={n}>
                    <b>{t.label}</b> {t.detail}
                  </p>
                ))}
              </details>
            )}
          </div>
        ))}
        {busy && (
          <div className="x-row assistant">
            <div className="x-bubble assistant x-typing">
              <LoaderCircle size={15} className="x-spin" />
              <span>{status || "Working…"}</span>
              <button
                aria-label="Stop"
                onClick={() => void invoke("cancel_ai")}
              >
                <Square size={11} />
              </button>
            </div>
          </div>
        )}
        {error && (
          <div className="x-error" role="alert">
            <span>{error}</span>
            <button aria-label="Dismiss" onClick={() => setError("")}>
              <X size={15} />
            </button>
          </div>
        )}
        {lastTx?.reversible && !approval && (
          <button
            className="x-undo"
            disabled={working}
            onClick={() => void review(lastTx.id)}
          >
            <Undo2 size={15} /> Undo last change
          </button>
        )}
        <div ref={end} />
      </div>
      {activePlan && pickedCount > 0 && !approval && (
        <div className="x-actionbar">
          <div>
            <b>
              {pickedCount} selected · {size(pickedBytes)}
            </b>
            <small>Goes to Trash after you approve</small>
          </div>
          <button
            className="x-danger"
            disabled={working || busy || scanning}
            onClick={() => void review()}
          >
            {working ? <LoaderCircle size={15} className="x-spin" /> : <Trash2 size={15} />}
            Review
          </button>
        </div>
      )}
      <form
        className="x-composer"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        <textarea
          value={text}
          rows={1}
          maxLength={1500}
          placeholder={
            scanning
              ? "Scanning…"
              : activePlan && pickedCount
                ? "Say “yes” to review, or refine…"
                : `Ask Tidy about ${scope.name}…`
          }
          disabled={busy || working || scanning}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
        />
        <button
          type="submit"
          aria-label="Send"
          disabled={busy || working || scanning || !text.trim()}
        >
          <ArrowUp size={18} />
        </button>
      </form>
      {approval && (
        <div className="x-sheet-backdrop" onClick={() => !working && setApproval(null)}>
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
            <h3>
              {undoTarget !== null
                ? "Restore these changes?"
                : approval.actions.some((a) => a.type === "trash_dir")
                  ? "Move to Trash?"
                  : "Apply these changes?"}
            </h3>
            <p>
              Tidy re-checks every item right before it runs. Nothing is ever
              deleted permanently.
            </p>
            <div className="x-sheet-list">
              {approval.actions.slice(0, 40).map((a, i) => (
                <div key={i}>
                  {a.type === "trash_dir" ? <Folder size={16} /> : <ChevronRight size={16} />}
                  <span>
                    <b>{a.relative_source}</b>
                    <small>
                      {a.type === "trash_dir"
                        ? `Whole folder · ${(a.files ?? 0).toLocaleString()} files · ${size(a.original_size ?? 0)} → Trash`
                        : a.type === "trash"
                          ? "→ Trash"
                          : a.type === "permissions"
                            ? `Permissions ${a.old_mode?.toString(8)} → ${a.new_mode?.toString(8)}`
                            : `→ ${a.relative_dest}`}
                    </small>
                  </span>
                </div>
              ))}
              {approval.actions.length > 40 && (
                <small className="x-more">
                  and {approval.actions.length - 40} more
                </small>
              )}
            </div>
            <div className="x-sheet-buttons">
              <button
                className="x-secondary"
                disabled={working}
                onClick={() => setApproval(null)}
              >
                Cancel
              </button>
              <button
                className={undoTarget !== null ? "x-primary" : "x-danger"}
                disabled={working || scanning}
                onClick={() => void apply()}
              >
                {working ? <LoaderCircle size={16} className="x-spin" /> : <Check size={16} />}
                {undoTarget !== null ? "Restore" : "Approve"}
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}

function PlanCard({
  plan,
  picked,
  sources,
  limit,
  onMore,
  toggle,
  setPicked,
  keyOf,
}: {
  plan: Plan;
  picked: Set<string>;
  sources: Map<number, { id: number; path: string; size: number }>;
  limit: number;
  onMore: () => void;
  toggle: (id: string) => void;
  setPicked: (s: Set<string>) => void;
  keyOf: (a: Action) => string;
}) {
  const all = [
    ...plan.folders.map((f) => `d${f.path}`),
    ...plan.proposal.actions.map(keyOf),
  ];
  if (!all.length) return null;
  const grouped = plan.proposal.actions.some((a) => a.action === "move");
  return (
    <div className="x-card">
      <div className="x-card-head">
        <b>{grouped ? "Proposed changes" : "Ready for the Trash"}</b>
        <button
          onClick={() =>
            setPicked(picked.size === all.length ? new Set() : new Set(all))
          }
        >
          {picked.size === all.length ? "Deselect all" : "Select all"}
        </button>
      </div>
      {plan.folders.map((f) => (
        <label key={f.path} className="x-item">
          <input
            type="checkbox"
            checked={picked.has(`d${f.path}`)}
            onChange={() => toggle(`d${f.path}`)}
          />
          <Folder size={18} className="x-folder-icon" />
          <span>
            <b>{f.path.split("/").at(-1)}</b>
            <small>
              {f.path.includes("/") ? f.path.split("/").slice(0, -1).join("/") + " · " : ""}
              {f.files.toLocaleString()} files
            </small>
          </span>
          <em>{size(f.bytes)}</em>
        </label>
      ))}
      {plan.proposal.actions.slice(0, limit).map((a) => {
        const s = sources.get(a.source);
        return (
          <label key={a.source} className="x-item">
            <input
              type="checkbox"
              checked={picked.has(keyOf(a))}
              onChange={() => toggle(keyOf(a))}
            />
            <span>
              <b>{s?.path.split("/").at(-1) ?? a.source}</b>
              <small>
                {a.action === "trash"
                  ? (s?.path.includes("/") ? s.path.split("/").slice(0, -1).join("/") : "top level")
                  : a.action === "permissions"
                    ? `Permissions ${a.mode.toString(8)}`
                    : `→ ${a.destination_relative}`}
              </small>
            </span>
            <em>{size(s?.size ?? 0)}</em>
          </label>
        );
      })}
      {plan.proposal.actions.length > limit && (
        <button className="x-more-btn" onClick={onMore}>
          Show {Math.min(20, plan.proposal.actions.length - limit)} more
        </button>
      )}
    </div>
  );
}
