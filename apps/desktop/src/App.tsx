import ModelView, { type AiStatus } from "./ModelView";
import { version } from "../package.json";
import HistoryView from "./HistoryView";
import ChatView from "./ChatView";
import StorageView from "./StorageView";
import { useEffect, useMemo, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  Cpu,
  Folder,
  History,
  LoaderCircle,
  MessageCircle,
  Minimize2,
  Maximize2,
  X,
  HardDrive,
} from "lucide-react";

type Scope = {
  id: number;
  path: string;
  status: string;
  scanned_at: number | null;
  files: number;
  bytes: number;
  omitted: number;
  content: boolean;
};
type Job = {
  running: boolean;
  scope_id: number | null;
  visited: number;
  message: string;
};
type Tab = "chat" | "storage" | "history" | "ai";
const emptyJob: Job = { running: false, scope_id: null, visited: 0, message: "" };
const basename = (path: string) =>
  path.split(/[\\/]/).filter(Boolean).at(-1) || path;

export default function App() {
  const [scopes, setScopes] = useState<Scope[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [tab, setTab] = useState<Tab>("chat");
  const [compact, setCompact] = useState(false);
  const [chatId, setChatId] = useState<number | null>(null);
  const [job, setJob] = useState<Job>(emptyJob);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const [prompt, setPrompt] = useState<string | null>(null);
  const [ai, setAi] = useState<AiStatus | null>(null);
  const lastRunning = useRef(false);

  async function refresh() {
    const [list, ids] = await Promise.all([
      invoke<Scope[]>("list_scopes"),
      invoke<number[]>("ai_selection"),
    ]);
    setScopes(list);
    setSelected(new Set(ids));
  }
  useEffect(() => {
    if (!isTauri()) {
      setError("Open the Tidy desktop app to choose folders.");
      return;
    }
    void refresh().catch((e) => setError(String(e)));
    const timer = setInterval(() => {
      void invoke<Job>("scan_status")
        .then((next) => {
          setJob(next);
          if (lastRunning.current && !next.running) {
            void refresh().catch((e) => setError(String(e)));
            setRevision((n) => n + 1);
          }
          lastRunning.current = next.running;
        })
        .catch((e) => setError(String(e)));
    }, 650);
    return () => clearInterval(timer);
  }, []);

  const loadModels = () => void invoke<AiStatus>("ai_status").then(setAi).catch(() => {});
  useEffect(() => {
    if (isTauri()) loadModels();
  }, [tab]);
  const chatScopes = useMemo(
    () =>
      scopes
        .filter((s) => selected.has(s.id))
        .map((s) => ({ id: s.id, name: basename(s.path), files: s.files })),
    [scopes, selected],
  );
  useEffect(() => {
    setChatId((c) =>
      c !== null && chatScopes.some((s) => s.id === c)
        ? c
        : (chatScopes[0]?.id ?? null),
    );
  }, [chatScopes]);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape" && compact && !document.querySelector('[aria-modal="true"]')) void setCompactMode(false);
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  });
  async function setCompactMode(next: boolean) {
    try {
      await invoke("set_compact_mode", { compact: next });
      setCompact(next);
      if (next) setTab("chat");
    } catch (e) {
      setError(String(e));
    }
  }
  async function scan(id: number, content = false) {
    setError("");
    await invoke("start_scan", { scopeId: id, content });
    lastRunning.current = true;
    setJob({ ...emptyJob, running: true, scope_id: id, message: "Reading folder…" });
  }
  async function toggleBase(id: number, on: boolean) {
    try {
      setSelected(new Set(await invoke<number[]>("set_ai_selected", { scopeId: id, selected: on })));
    } catch (e) {
      setError(String(e));
    }
  }
  async function pickFolder() {
    const id = await invoke<number | null>("choose_folder");
    if (id === null) return;
    await invoke<number[]>("set_ai_selected", { scopeId: id, selected: true });
    await refresh();
    setChatId(id);
    await scan(id);
  }
  async function authorize(path: string) {
    if (job.running) throw new Error("Wait for the current scan to finish, then try again.");
    const id = await invoke<number>("authorize_path", { path });
    await invoke<number[]>("set_ai_selected", { scopeId: id, selected: true });
    await refresh();
    setChatId(id);
    await scan(id);
  }

  const tabs: { id: Tab; label: string; icon: React.ReactNode }[] = [
    { id: "chat", label: "Chat", icon: <MessageCircle size={22} /> },
    { id: "storage", label: "Storage", icon: <HardDrive size={22} /> },
    { id: "history", label: "History", icon: <History size={22} /> },
    { id: "ai", label: "AI", icon: <Cpu size={22} /> },
  ];
  const chatScope = scopes.find((s) => s.id === chatId);
  const scanningNow = job.running;
  const pickers = (
    <>
        {tab === "chat" && chatScopes.length > 0 && (
          <label className="x-scope-pick">
            <Folder size={14} />
            <select
              value={chatId ?? ""}
              onChange={(e) => setChatId(Number(e.target.value))}
              aria-label="Folder Tidy is working in"
            >
              {chatScopes.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
        )}
        {tab === "chat" && ai && (
          <label className="x-scope-pick x-model-pick" title="The AI model used when a request needs one">
            <Cpu size={14} />
            <select
              value={ai.selected ?? ""}
              aria-label="AI model"
              onChange={(e) => {
                if (e.target.value === "__manage") {
                  if (compact) void setCompactMode(false);
                  setTab("ai");
                } else {
                  void invoke("ai_select_model", { modelId: e.target.value }).then(loadModels).catch((x) => setError(String(x)));
                }
              }}
            >
              {!ai.selected && <option value="">No model · instant only</option>}
              {ai.models.filter((m) => m.installed).map((m) => (
                <option key={m.spec.id} value={m.spec.id}>{m.spec.name.replace(" (recommended)", "")}</option>
              ))}
              <option value="__manage">Add or download models…</option>
            </select>
          </label>
        )}
    </>
  );

  return (
    <div className={`x-app ${compact ? "compact" : ""}`}>
      <header className="x-top" data-tauri-drag-region>
        <div className="x-top-left" data-tauri-drag-region>
          <img className="x-logo" src="/logo.png" alt="" />
          <strong data-tauri-drag-region>Tidy</strong>
        </div>
        <span className="x-pickers-top">{pickers}</span>
        <div className="x-top-right">
          {scanningNow && (
            <span className="x-scanning">
              <LoaderCircle size={13} className="x-spin" />
              {job.visited.toLocaleString()}
              <button className="x-link" onClick={() => void invoke("cancel_scan")}>Stop</button>
            </span>
          )}
          <button
            className="x-round"
            onClick={() => void setCompactMode(!compact)}
            aria-label={compact ? "Expand to full window" : "Shrink to a small assistant"}
            title={compact ? "Full window" : "Small assistant"}
          >
            {compact ? <Maximize2 size={15} /> : <Minimize2 size={15} />}
          </button>
        </div>
      </header>
      {error && (
        <div className="x-error banner" role="alert">
          <span>{error}</span>
          <button aria-label="Dismiss" onClick={() => setError("")}>
            <X size={15} />
          </button>
        </div>
      )}
      {tab === "chat" && <div className="x-subbar">{pickers}</div>}
      <main className="x-main">
        <div hidden={tab !== "chat"} className="x-fill">
          <ChatView
            scopes={chatScopes}
            scopeId={chatId}
            scanning={scanningNow}
            prompt={prompt}
            onPromptSent={() => setPrompt(null)}
            compact={compact}
            onRefresh={() => setRevision((r) => r + 1)}
            onOpenFolders={() => {
              if (compact) void setCompactMode(false);
              setTab("storage");
            }}
            onScan={() => chatId !== null && void scan(chatId).catch((e) => setError(String(e)))}
          />
        </div>
        {tab === "storage" && (
          <StorageView
            scopes={scopes.map((s) => ({
              id: s.id,
              path: s.path,
              name: basename(s.path),
              files: s.files,
              bytes: s.bytes,
            }))}
            selected={selected}
            onToggleScope={async (id, on) => void (await toggleBase(id, on))}
            onAuthorize={authorize}
            onPick={pickFolder}
            onAsk={(text, id) => {
              if (!selected.has(id)) {
                setError("Switch this folder on first so Tidy is allowed to work in it.");
                return;
              }
              setChatId(id);
              setPrompt(text);
              setTab("chat");
            }}
            scanning={scanningNow}
            refreshKey={revision}
            onChanged={() => {
              setRevision((r) => r + 1);
              void refresh();
            }}
          />
        )}
        {tab === "history" && (
          <HistoryView
            scopes={scopes.map((s) => ({ id: s.id, path: s.path, name: basename(s.path) }))}
            onRescan={(id) => void scan(id).catch((e) => setError(String(e)))}
            refreshKey={revision}
            onChanged={() => {
              setRevision((r) => r + 1);
              void refresh();
            }}
          />
        )}
        {tab === "ai" && (
          <ModelView
            onChanged={loadModels}
            folders={scopes.map((s) => ({ id: s.id, path: s.path, files: s.files }))}
            selected={selected}
            onForget={async (id) => {
              await invoke("forget_folder", { scopeId: id });
              await refresh();
              setRevision((r) => r + 1);
            }}
          />
        )}
      </main>
      {(
        <nav className="x-tabbar" aria-label="Sections">
          {tabs.map((t) => (
            <button
              key={t.id}
              className={tab === t.id ? "on" : ""}
              onClick={() => setTab(t.id)}
              aria-current={tab === t.id}
            >
              {t.icon}
              <span>{t.label}</span>
            </button>
          ))}
          <span className="x-version">v{version}{chatScope ? ` · ${basename(chatScope.path)}` : ""}</span>
        </nav>
      )}
    </div>
  );
}
