import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, Cpu, Download, ExternalLink, LoaderCircle, Plus, ShieldCheck, Square, Trash2 } from "lucide-react";

export type ModelInfo = {
  spec: { id: string; name: string; bytes: number; license: string; source: string; custom: boolean };
  installed: boolean;
};
export type AiStatus = {
  models: ModelInfo[];
  selected: string | null;
  worker_available: boolean;
  job: { running: boolean; operation: string; model_id: string; message: string; downloaded: number };
};
const gb = (n: number) => `${(n / 1e9).toFixed(2)} GB`;

export default function ModelView({
  onChanged,
  folders,
  selected,
  onForget,
}: {
  onChanged: () => void;
  folders: { id: number; path: string; files: number }[];
  selected: Set<number>;
  onForget: (id: number) => Promise<void>;
}) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [error, setError] = useState("");
  const [link, setLink] = useState("");
  const [adding, setAdding] = useState(false);
  const refresh = () => invoke<AiStatus>("ai_status").then(setStatus).catch((e) => setError(String(e)));
  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), 1000);
    return () => clearInterval(t);
  }, []);
  const job = status?.job;
  async function run(fn: () => Promise<unknown>) {
    setError("");
    try {
      await fn();
      await refresh();
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  }
  async function add() {
    setAdding(true);
    await run(async () => {
      await invoke("add_custom_model", { link });
      setLink("");
    });
    setAdding(false);
  }
  return (
    <div className="x-page">
      <div className="x-large-title">
        <div>
          <h1>AI models</h1>
          <p>Everyday requests work instantly with no model. A local model handles unusual requests, folder plans and questions. It runs on this Mac; nothing is uploaded. Pick the one to use, or add your own.</p>
        </div>
      </div>
      {error && <div className="x-error" role="alert"><span>{error}</span></div>}
      {status && !status.worker_available && (
        <div className="x-error"><span>The local inference worker is missing from this build, so models can’t run. Everything else still works.</span></div>
      )}
      <div className="x-group">
        {status?.models.map((m) => {
          const progress = job?.running && job.model_id === m.spec.id && job.operation === "install" ? Math.min(100, Math.floor((job.downloaded / m.spec.bytes) * 100)) : null;
          const inUse = status.selected === m.spec.id;
          return (
            <div key={m.spec.id} className="x-model">
              <Cpu size={20} />
              <span>
                <b>
                  {m.spec.name} {inUse && <span className="x-badge ok">In use</span>}
                </b>
                <small>
                  {gb(m.spec.bytes)} · {m.spec.license} · {m.installed ? "Installed" : "Not installed"}
                  {" · "}
                  <a href={m.spec.source} onClick={(e) => { e.preventDefault(); void invoke("open_external", { url: m.spec.source }); }}>source <ExternalLink size={10} /></a>
                </small>
                {progress !== null && <progress max={100} value={progress} />}
              </span>
              {job?.running && job.model_id === m.spec.id ? (
                <button className="x-secondary" onClick={() => void invoke("cancel_ai")}><Square size={12} /> Cancel {progress}%</button>
              ) : m.installed ? (
                <>
                  {!inUse && (
                    <button className="x-primary" onClick={() => void run(() => invoke("ai_select_model", { modelId: m.spec.id }))}>
                      <Check size={14} /> Use
                    </button>
                  )}
                  <button className="x-secondary" disabled={!!job?.running} onClick={() => void run(() => invoke("install_model", { modelId: m.spec.id }))}>Verify</button>
                </>
              ) : (
                <button className="x-primary" disabled={!!job?.running || !status.worker_available} onClick={() => void run(() => invoke("install_model", { modelId: m.spec.id }))}>
                  {job?.running ? <LoaderCircle size={14} className="x-spin" /> : <Download size={14} />} Download
                </button>
              )}
              {m.spec.custom && !job?.running && (
                <button className="x-icon-btn danger" title="Remove this model and its file" aria-label="Remove model" onClick={() => void run(() => invoke("remove_custom_model", { modelId: m.spec.id }))}>
                  <Trash2 size={16} />
                </button>
              )}
            </div>
          );
        })}
      </div>
      <h4 className="x-section"><Plus size={14} /> Add your own model</h4>
      <div className="x-group x-add-model">
        <p>
          Open a <b>.gguf</b> file on <a href="https://huggingface.co/models?library=gguf&sort=trending" onClick={(e) => { e.preventDefault(); void invoke("open_external", { url: "https://huggingface.co/models?library=gguf&sort=trending" }); }}>Hugging Face</a> and paste its address. Tidy looks up the file’s size and checksum, then downloads it once and verifies it. Larger models (7B–14B) are slower and need more memory but understand more.
        </p>
        <div className="x-add-row">
          <input
            placeholder="https://huggingface.co/…/blob/main/model-Q4_K_M.gguf"
            value={link}
            onChange={(e) => setLink(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && link.trim() && !adding && void add()}
            aria-label="Hugging Face link to a .gguf file"
          />
          <button className="x-primary" disabled={!link.trim() || adding} onClick={() => void add()}>
            {adding ? <LoaderCircle size={14} className="x-spin" /> : <Plus size={14} />} Add
          </button>
        </div>
        <small>Works best with models that use the ChatML format (Qwen and many fine-tunes). Single-file, ungated models only.</small>
      </div>
      <h4 className="x-section"><ShieldCheck size={14} /> Folders Tidy knows about</h4>
      <div className="x-group">
        {folders.map((f) => (
          <div key={f.id} className="x-model">
            <span>
              <b>{f.path}</b>
              <small>{f.files.toLocaleString()} files indexed · {selected.has(f.id) ? "visible to the assistant" : "hidden from the assistant"}</small>
            </span>
            <button className="x-secondary small" title="Deletes Tidy's saved index and search text for this folder. Your files are not touched." onClick={() => void run(() => onForget(f.id))}>
              Forget
            </button>
          </div>
        ))}
        {!folders.length && <p className="x-note">No folders yet. Add one in Storage.</p>}
      </div>
      <p className="x-foot">Forgetting removes only Tidy's saved index and text excerpts. It never changes or deletes your files.</p>
      {job?.message && !job.running && <p className="x-foot">{job.message}</p>}
      <p className="x-foot"><ShieldCheck size={12} style={{ verticalAlign: "-2px" }} /> Downloads are pinned to a checksum. The model only proposes; you approve every change.</p>
    </div>
  );
}
