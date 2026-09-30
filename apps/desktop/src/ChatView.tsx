import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUp,
  File as FileIcon,
  Check,
  ChevronRight,
  Folder,
  LoaderCircle,
  ShieldCheck,
  Square,
  Undo2,
  X,
} from "lucide-react";

type Action =
  | { action: "move" | "copy"; source: number; destination_relative: string }
  | { action: "trash"; source: number }
  | { action: "permissions"; source: number; mode: number }
  | { action: "rename"; source: number; new_name: string }
  | { action: "create_folder"; path: string }
  | { action: "move_folder"; source: string; destination_relative: string };
const srcId = (a: Action) => ("source" in a && typeof a.source === "number" ? a.source : -1);
type Trace = { label: string; detail: string };
type FolderTarget = { path: string; files: number; bytes: number; note?: string | null };
type ListItem = { kind: "folder" | "file"; path: string; bytes: number; files: number; note?: string | null };
type Section = { title: string; items: ListItem[] };
type Plan = {
  engine: "model" | "local_filter" | "instant";
  proposal: { actions: Action[]; rationale: string };
  sources: { id: number; path: string; size: number }[];
  folders: FolderTarget[];
  sections?: Section[];
  pick?: boolean;
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
    type: "move" | "rename" | "trash" | "trash_dir" | "copy" | "permissions" | "create_dir" | "move_dir";
    relative_source: string;
    relative_dest?: string;
    old_mode?: number;
    new_mode?: number;
    files?: number;
    original_size?: number;
    read_only?: boolean;
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
  "List all my projects",
  "Create a folder called Archive",
  "Move all PDFs into Documents/PDFs",
  "Delete the 10 biggest files",
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
  // The last request that produced a listing, so follow-ups (“why no size?”, “measure them”) have context.
  const lastListing = useRef<string | null>(null);
  // A request with several tasks (trash folders + rename a file): the Trash step is approved first and the
  // remaining steps are carried forward so nothing the user asked for is dropped.
  const nextSteps = useRef<{ plan: Plan; actions: Action[] } | null>(null);
  const scope = scopes.find((s) => s.id === scopeId) ?? null;
  const lastPlan = [...messages].reverse().find((m) => m.plan)?.plan;
  const activePlan =
    messages.at(-1)?.role === "assistant" ? messages.at(-1)?.plan : undefined;

  useEffect(() => {
    lastListing.current = null;
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

  const key = (a: Action) =>
    a.action === "create_folder" ? `c${a.path}` : a.action === "move_folder" ? `m${a.source}` : `f${srcId(a)}`;
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
    (pickedActions?.reduce((n, a) => n + (sources.get(srcId(a))?.size ?? 0), 0) ??
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
          (activePlan?.clarification || activePlan?.pick) && lastPlan
            ? `${messages.filter((m) => m.role === "user").at(-1)?.text ?? ""}\nUser follow-up: ${value}`
            : value,
        previous: lastListing.current,
      });
      if (plan.sections?.length || plan.folders.length) lastListing.current = value;
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
      // Candidate lists start unchecked so nothing is chosen for removal by accident.
      setPicked(
        plan.pick
          ? new Set()
          : new Set([...plan.folders.map((f) => `d${f.path}`), ...plan.proposal.actions.map(key)]),
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
      nextSteps.current = null;
      if (undoId !== null) {
        view = await invoke<Approval>("request_undo_approval", {
          scopeId,
          txId: undoId,
        });
      } else if (pickedFolders?.length) {
        // A folder inside another chosen folder moves with it; send only the outer ones.
        const chosen = pickedFolders.map((f) => f.path);
        const outer = chosen.filter((p) => !chosen.some((o) => o !== p && p.startsWith(o + "/")));
        if (activePlan && pickedActions?.length) nextSteps.current = { plan: activePlan, actions: pickedActions };
        view = await invoke<Approval>("request_folder_trash_approval", {
          scopeId,
          folders: outer,
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
      const next = undoTarget === null ? nextSteps.current : null;
      nextSteps.current = null;
      setMessages((m) => [
        ...m,
        {
          role: "assistant",
          text:
            undoTarget !== null
              ? "Restored. Everything is back where it was."
              : next
                ? `Step 1 done — ${approval.actions_count} ${approval.actions_count === 1 ? "item" : "items"} moved to the Trash (recoverable from Finder). Next: ${next.actions.length} more ${next.actions.length === 1 ? "change" : "changes"} from your request. Review to apply.`
                : trashed
                  ? `Done — ${approval.actions_count} ${approval.actions_count === 1 ? "item" : "items"} moved to the Trash. You can put them back from Finder’s Trash.`
                  : `Done — ${report.actions_applied} changes applied and verified. Find them in History to undo.`,
          plan: next
            ? { ...next.plan, folders: [], pick: false, proposal: { ...next.plan.proposal, actions: next.actions } }
            : undefined,
        },
      ]);
      setApproval(null);
      setUndoTarget(null);
      setPicked(next ? new Set(next.actions.map(key)) : new Set());
      onRefresh();
    } catch (e) {
      const txId = approval.tx_id;
      setApproval(null);
      setError(String(e));
      // Some steps may have run before one failed: drop those from the plan so it isn't stale.
      try {
        const detail = await invoke<{ steps: { source_relative: string; state: string }[] } | null>(
          "get_transaction_detail",
          { txId },
        );
        const done = new Set(detail?.steps.filter((s) => s.state === "verified").map((s) => s.source_relative));
        if (done.size) {
          setMessages((prev) =>
            prev.map((m, i) => {
              if (i !== prev.length - 1 || !m.plan) return m;
              const byId = new Map(m.plan.sources.map((s) => [s.id, s.path]));
              return {
                ...m,
                plan: {
                  ...m.plan,
                  folders: m.plan.folders.filter((f) => !done.has(f.path)),
                  proposal: {
                    ...m.plan.proposal,
                    actions: m.plan.proposal.actions.filter((a) => !done.has(byId.get(srcId(a)) ?? "")),
                  },
                },
              };
            }),
          );
          setPicked((p) => new Set([...p].filter((k) => !done.has(k.slice(1)))));
        }
      } catch {
        /* the error above is already shown */
      }
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
        <img className="x-empty-logo" src="/logo.png" alt="" />
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
            <img className="x-empty-logo" src="/logo.png" alt="" />
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
            {m.role === "assistant" && !!m.plan?.sections?.length && (
              <Sections sections={m.plan.sections} onOpen={(p) => void send(`what's inside ${p}`)} />
            )}
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
              {pickedCount} selected{pickedBytes ? ` · ${size(pickedBytes)}` : ""}
            </b>
            <small>{pickedFolders?.length || pickedActions?.some((a) => a.action === "trash") ? "Nothing changes until you approve" : "You’ll review before anything changes"}</small>
          </div>
          <button
            className="x-primary"
            disabled={working || busy || scanning}
            onClick={() => void review()}
          >
            {working ? <LoaderCircle size={15} className="x-spin" /> : <ShieldCheck size={15} />}
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
                : approval.actions.every((a) => a.type === "trash" || a.type === "trash_dir")
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
                  {a.type === "trash_dir" || a.type === "create_dir" || a.type === "move_dir" ? <Folder size={16} /> : <ChevronRight size={16} />}
                  <span>
                    <b>{a.relative_source}</b>
                    {a.read_only && (
                      <small>Read-only folder — Tidy makes it writable just for the move and puts its permissions back.</small>
                    )}
                    <small>
                      {a.type === "trash_dir"
                        ? `Whole folder · ${(a.files ?? 0).toLocaleString()} files · ${size(a.original_size ?? 0)} → Trash`
                        : a.type === "create_dir"
                          ? "New folder"
                          : a.type === "move_dir"
                            ? `Folder → ${a.relative_dest}`
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
                className={undoTarget !== null || !approval.actions.some((a) => a.type === "trash" || a.type === "trash_dir") ? "x-primary" : "x-danger"}
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
  const grouped = plan.proposal.actions.some((a) => a.action !== "trash");
  return (
    <div className="x-card">
      <div className="x-card-head">
        <b>{plan.pick ? "Tick what to move to the Trash" : grouped ? "Proposed changes" : "Ready for the Trash"}</b>
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
          <Folder size={18} className="x-item-icon" />
          <span>
            <b>{f.path.split("/").at(-1)}</b>
            <small>
              {f.path.includes("/") ? f.path.split("/").slice(0, -1).join("/") + " · " : ""}
              {f.files.toLocaleString()} files{f.note ? ` · ${f.note}` : ""}
            </small>
          </span>
          <em>{size(f.bytes)}</em>
        </label>
      ))}
      {plan.proposal.actions.slice(0, limit).map((a) => {
        const s = sources.get(srcId(a));
        const leaf = (p: string) => p.split("/").at(-1) ?? p;
        const dir = (p: string) => (p.includes("/") ? p.split("/").slice(0, -1).join("/") : "top level");
        const folderAction = a.action === "create_folder" || a.action === "move_folder";
        const title =
          a.action === "create_folder"
            ? `New folder “${leaf(a.path)}”`
            : a.action === "move_folder"
              ? leaf(a.source)
              : (s?.path ? leaf(s.path) : String(srcId(a)));
        const detail =
          a.action === "create_folder"
            ? dir(a.path) === "top level" ? "in this folder" : `in ${dir(a.path)}`
            : a.action === "move_folder"
              ? `${a.source} → ${a.destination_relative}`
              : a.action === "trash"
                ? dir(s?.path ?? "")
                : a.action === "permissions"
                  ? `Permissions ${a.mode.toString(8)}`
                  : a.action === "rename"
                    ? `${dir(s?.path ?? "")} → renamed to ${a.new_name}`
                    : `→ ${a.destination_relative}`;
        return (
          <label key={keyOf(a)} className="x-item">
            <input type="checkbox" checked={picked.has(keyOf(a))} onChange={() => toggle(keyOf(a))} />
            {folderAction && <Folder size={18} className="x-item-icon" />}
            <span>
              <b>{title}</b>
              <small>{detail}</small>
            </span>
            <em>{s ? size(s.size) : ""}</em>
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

function Sections({ sections, onOpen }: { sections: Section[]; onOpen: (path: string) => void }) {
  return (
    <>
      {sections.map((sec) => {
        const max = Math.max(1, ...sec.items.map((i) => i.bytes));
        return (
          <div key={sec.title} className="x-card">
            <div className="x-card-head">
              <b>{sec.title}</b>
              <small className="x-count">{sec.items.length}</small>
            </div>
            {sec.items.map((it) => {
              const name = it.path.split("/").at(-1) ?? it.path;
              const parent = it.path.includes("/") ? it.path.split("/").slice(0, -1).join("/") : "";
              const inner = (
                <>
                  {it.kind === "folder" ? <Folder size={18} className="x-item-icon" /> : <FileIcon size={18} className="x-file-icon" />}
                  <span>
                    <b title={it.path}>{name}</b>
                    <small>
                      {[
                        parent,
                        it.kind === "folder" && it.files ? `${it.files.toLocaleString()} files` : it.kind === "file" ? "file" : "",
                        it.note ?? "",
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </small>
                    {it.bytes > 0 && <i><u style={{ width: `${Math.max(1, (100 * it.bytes) / max)}%` }} /></i>}
                  </span>
                  <em>{it.bytes ? size(it.bytes) : ""}</em>
                </>
              );
              return it.kind === "folder" ? (
                <button key={it.path} className="x-list-row" onClick={() => onOpen(it.path)} title="Show what’s inside">
                  {inner}
                </button>
              ) : (
                <div key={it.path} className="x-list-row static">{inner}</div>
              );
            })}
          </div>
        );
      })}
    </>
  );
}
