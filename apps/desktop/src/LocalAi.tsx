import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  Sparkles,
  Download,
  ShieldCheck,
  Square,
  ChevronDown,
} from "lucide-react";
type Model = {
  spec: {
    id: string;
    name: string;
    bytes: number;
    license: string;
    source: string;
  };
  installed: boolean;
};
type Answer = {
  text: string;
  tokens: number;
  elapsed_ms: number;
  backend: string;
  truncated: boolean;
};
type Status = {
  models: Model[];
  worker_available: boolean;
  metal_available: boolean;
  job: {
    running: boolean;
    operation: string;
    model_id: string;
    scope_id: number | null;
    message: string;
    downloaded: number;
    answer: Answer | null;
    sampled_files: number;
    total_indexed: number;
    overview: null | {
      indexed_files: number;
      logical_bytes: number;
      files_with_saved_text: number;
      scan_status: string;
      scanned_at: number | null;
      excluded_entries: number;
      matching_files: number;
      retrieval_terms: string[];
      largest_types: { extension: string; files: number; bytes: number }[];
      other_type_files: number;
    };
  };
};
export default function LocalAi({ scopeId }: { scopeId: number | null }) {
  const [status, setStatus] = useState<Status | null>(null),
    [model, setModel] = useState("qwen3-1.7b-q4"),
    [backend, setBackend] = useState("cpu"),
    [expanded, setExpanded] = useState(false),
    [question, setQuestion] = useState(
      "What kinds of files are in this folder, and what should I look at first?",
    ),
    [error, setError] = useState(""),
    [sending, setSending] = useState(false);
  useEffect(() => {
    if (!isTauri()) return;
    let active = true,
      initialized = false;
    const refresh = () => {
      void invoke<Status>("ai_status")
        .then((s) => {
          if (!active) return;
          setStatus(s);
          if (!initialized) {
            setBackend(s.metal_available ? "metal" : "cpu");
            initialized = true;
          }
        })
        .catch((e) => {
          if (active) setError(String(e));
        });
    };
    refresh();
    const timer = setInterval(refresh, 1000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, []);
  const selected = status?.models.find((m) => m.spec.id === model),
    job = status?.job;
  const busy = sending || !!job?.running;
  async function action(command: string, args?: Record<string, unknown>) {
    setError("");
    setSending(true);
    try {
      await invoke(command, args);
      setStatus(await invoke<Status>("ai_status"));
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
    }
  }
  const currentAnswer = job?.scope_id === scopeId ? job.answer : null;
  const downloadModel = status?.models.find((m) => m.spec.id === job?.model_id);
  const progress =
    job?.operation === "install" && downloadModel
      ? Math.min(
          100,
          Math.floor((job.downloaded / downloadModel.spec.bytes) * 100),
        )
      : 0;
  return (
    <section className="ai-panel" aria-label="Local AI assistant">
      <button
        className="ai-heading"
        aria-expanded={expanded}
        onClick={() => setExpanded(!expanded)}
      >
        <span className="ai-symbol">
          <Sparkles size={17} />
        </span>
        <span>
          <strong>A second look, entirely local.</strong>
          <small>
            {selected?.installed
              ? "Your local model is installed."
              : "Optional AI · scanning and search already work without it."}
          </small>
        </span>
        <span className="ai-toggle">
          {expanded
            ? "Hide"
            : selected?.installed
              ? "Ask locally"
              : "Set up local AI"}
          <ChevronDown size={14} />
        </span>
      </button>
      {expanded && (
        <div className="ai-body">
          {!status?.worker_available && (
            <p className="ai-warning">
              The inference worker is unavailable. Install a complete Tidy
              build, or run the worker build script during development. Your
              file index is still available.
            </p>
          )}
          <div className="ai-controls">
            <label>
              Local model
              <select
                value={model}
                disabled={busy}
                onChange={(e) => setModel(e.target.value)}
              >
                {(status?.models ?? []).map((m) => (
                  <option key={m.spec.id} value={m.spec.id}>
                    {m.spec.name}
                    {m.installed ? " · Installed" : ""}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Compute
              <select
                value={backend}
                disabled={busy}
                onChange={(e) => setBackend(e.target.value)}
              >
                <option value="cpu">CPU · compatible</option>
                {status?.metal_available && (
                  <option value="metal">Metal · Apple GPU</option>
                )}
              </select>
            </label>
          </div>
          {selected && !selected.installed && (
            <div className="model-install">
              <div>
                <strong>One download. Then offline.</strong>
                <p>
                  {(selected.spec.bytes / 1e9).toFixed(2)} GB ·{" "}
                  {selected.spec.license} · ggml-org / Hugging Face.
                  <br />
                  Verified against a pinned SHA-256. No files or prompts are
                  uploaded.
                </p>
              </div>
              <button
                className="secondary"
                disabled={busy || !status?.worker_available}
                onClick={() => void action("install_model", { modelId: model })}
              >
                <Download size={14} />
                Download model
              </button>
            </div>
          )}
          {selected?.installed && (
            <>
              <button
                className="secondary"
                disabled={busy}
                onClick={() => void action("install_model", { modelId: model })}
              >
                Verify / download repair
              </button>
              <p className="ai-context">
                <ShieldCheck size={13} />
                Computes statistics across every indexed file. Searches all
                indexed paths and saved text, then gives AI a bounded set of
                relevant examples. It cannot act on files.
              </p>
              {scopeId === null ? (
                <p className="ai-warning">
                  Choose and scan a folder to ask about it.
                </p>
              ) : (
                <>
                  <label className="ai-question">
                    Ask about the indexed folder
                    <textarea
                      value={question}
                      onChange={(e) => setQuestion(e.target.value)}
                      maxLength={500}
                      rows={2}
                      disabled={busy}
                    />
                  </label>
                  <div className="ai-submit">
                    <span>Model memory is released after each answer.</span>
                    <button
                      className="primary"
                      disabled={
                        busy || !question.trim() || !status?.worker_available
                      }
                      onClick={() =>
                        void action("ask_local", {
                          scopeId,
                          modelId: model,
                          question,
                          backend,
                        })
                      }
                    >
                      <Sparkles size={14} />
                      Ask locally
                    </button>
                  </div>
                </>
              )}
            </>
          )}
          {job?.running && (
            <div className="ai-progress" role="status">
              <div>
                <span>
                  {job.operation === "install"
                    ? `Downloading · ${progress}%`
                    : "Verifying and thinking locally…"}
                </span>
                <button
                  className="secondary"
                  onClick={() => void action("cancel_ai")}
                >
                  <Square size={12} />
                  Cancel
                </button>
              </div>
              {job.operation === "install" && (
                <progress max={100} value={progress} />
              )}
              <small>
                {job.operation === "install"
                  ? "Cancellation may take up to 15 seconds during a stalled network read."
                  : "One bounded request · maximum 90 seconds · scanning stays available."}
              </small>
            </div>
          )}
          {error && (
            <p className="ai-warning" role="alert">
              {error}
            </p>
          )}
          {!job?.running && job?.message && (
            <p className="ai-message" role="status">
              {job.message}
            </p>
          )}
          {currentAnswer && (
            <div className="ai-answer">
              <span className="eyebrow">
                LOCAL ANSWER · CHECK AGAINST YOUR FILES
              </span>
              {job?.overview && (
                <div className="ai-coverage">
                  <strong>
                    Index coverage:{" "}
                    {job.overview.indexed_files.toLocaleString()} /{" "}
                    {job.overview.indexed_files.toLocaleString()} files
                  </strong>
                  <p>
                    {job.overview.logical_bytes.toLocaleString()} logical bytes
                    · {job.overview.files_with_saved_text.toLocaleString()}{" "}
                    files with saved text ·{" "}
                    {job.overview.excluded_entries.toLocaleString()} scan
                    exclusions · Scan: {job.overview.scan_status}
                  </p>
                  <p>
                    {job.overview.largest_types
                      .map((t) => `${t.extension}: ${t.files.toLocaleString()}`)
                      .join(" · ")}
                    {job.overview.other_type_files
                      ? ` · Other types: ${job.overview.other_type_files.toLocaleString()}`
                      : ""}
                  </p>
                  {job.overview.retrieval_terms.length > 0 && (
                    <p>
                      Search terms: {job.overview.retrieval_terms.join(", ")} ·{" "}
                      {job.overview.matching_files.toLocaleString()} matching
                      files across the index.
                    </p>
                  )}
                  <small>
                    Exact index statistics, not AI estimates. Contents were not
                    read for every file.{" "}
                    {job.overview.scanned_at
                      ? `Snapshot: ${new Date(job.overview.scanned_at * 1000).toLocaleString()}.`
                      : ""}
                  </small>
                </div>
              )}
              <div>{currentAnswer.text}</div>
              <footer>
                Full-index statistics + {job?.sampled_files} retrieved examples
                · {currentAnswer.backend} ·{" "}
                {(currentAnswer.elapsed_ms / 1000).toFixed(1)}s
                {currentAnswer.truncated ? " · Reached response limit" : ""}
              </footer>
            </div>
          )}
          <p className="ai-footnote">
            AI answers can be wrong. This phase explains evidence only; it
            cannot organize, rename, trash, or run commands.
          </p>
        </div>
      )}
    </section>
  );
}
