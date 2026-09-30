use super::{Shared, blocking, display_error, lock};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Mutex,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};
use tauri::State;
use tidy_agent_runtime::{
    catalog::{self, ModelSpec},
    prompt::{self, Evidence},
    store::ModelStore,
    worker::{Answer, Backend, Worker},
};
#[derive(Deserialize)]
struct Manifest {
    bytes: u64,
    sha256: String,
}
#[derive(Default, Clone, Serialize)]
pub struct AiJob {
    pub running: bool,
    pub operation: String,
    pub model_id: String,
    pub scope_id: Option<i64>,
    pub message: String,
    pub downloaded: u64,
    pub answer: Option<Answer>,
    pub sampled_files: usize,
    pub total_indexed: i64,
    pub planning_trace: Vec<tidy_agent_runtime::investigation::Trace>,
    pub overview: Option<tidy_file_indexer::database::IndexOverview>,
}
pub struct AiState {
    pub store: ModelStore,
    pub worker: Worker,
    pub job: Mutex<AiJob>,
    pub cancel: AtomicBool,
    pub downloaded: AtomicU64,
}
impl AiState {
    pub fn new(data: PathBuf, resource: PathBuf) -> Result<Self, String> {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../resources/inference/worker-manifest.json"))
                .map_err(display_error)?;
        let name = if cfg!(windows) {
            "tidy-inference-worker.exe"
        } else {
            "tidy-inference-worker"
        };
        let mut executable = resource.join("inference").join(name);
        #[cfg(debug_assertions)]
        if !executable.exists() {
            executable = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources/inference")
                .join(name);
        }
        Ok(Self {
            store: ModelStore::new(data.join("models")),
            worker: Worker {
                executable,
                bytes: manifest.bytes,
                sha256: manifest.sha256,
            },
            job: Mutex::new(AiJob::default()),
            cancel: AtomicBool::new(false),
            downloaded: AtomicU64::new(0),
        })
    }
    pub(super) fn reserve(
        &self,
        operation: &str,
        model_id: &str,
        scope_id: Option<i64>,
    ) -> Result<(), String> {
        let mut job = lock(&self.job)?;
        if job.running {
            return Err("A model operation is already running".into());
        }
        self.cancel.store(false, Ordering::Relaxed);
        self.downloaded.store(0, Ordering::Relaxed);
        *job = AiJob {
            running: true,
            operation: operation.into(),
            model_id: model_id.into(),
            scope_id,
            message: if operation == "install" {
                "Downloading and verifying model…"
            } else {
                "Checking request and preparing local processing…"
            }
            .into(),
            ..Default::default()
        };
        Ok(())
    }
}
#[derive(Serialize)]
pub struct ModelView {
    spec: &'static ModelSpec,
    installed: bool,
}
#[derive(Serialize)]
pub struct Status {
    models: Vec<ModelView>,
    worker_available: bool,
    metal_available: bool,
    job: AiJob,
}
#[tauri::command]
pub fn ai_status(state: State<'_, Shared>) -> Result<Status, String> {
    let mut job = lock(&state.ai.job)?.clone();
    job.downloaded = state.ai.downloaded.load(Ordering::Relaxed);
    Ok(Status {
        models: catalog::MODELS
            .iter()
            .map(|spec| ModelView {
                spec,
                installed: state.ai.store.present(spec),
            })
            .collect(),
        worker_available: state.ai.worker.bytes > 0 && state.ai.worker.executable.is_file(),
        metal_available: cfg!(target_os = "macos"),
        job,
    })
}
#[tauri::command]
pub fn cancel_ai(state: State<'_, Shared>) {
    state.ai.cancel.store(true, Ordering::Relaxed);
}
#[tauri::command]
pub fn install_model(model_id: String, state: State<'_, Shared>) -> Result<(), String> {
    let spec = catalog::model(&model_id)?;
    state.ai.reserve("install", spec.id, None)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let worker = state.clone();
        let result = blocking(move || {
            worker
                .ai
                .store
                .install(spec, &worker.ai.cancel, &worker.ai.downloaded)
                .map(|_| ())
        })
        .await;
        if let Ok(mut job) = state.ai.job.lock() {
            job.running = false;
            job.message = match result {
                Ok(()) => "Model installed and SHA-256 verified. Ready for offline use.".into(),
                Err(e) => e,
            };
        }
    });
    Ok(())
}
#[tauri::command]
pub fn ask_local(
    scope_id: i64,
    model_id: String,
    question: String,
    backend: Backend,
    state: State<'_, Shared>,
) -> Result<(), String> {
    let spec = catalog::model(&model_id)?;
    if question.trim().is_empty() || question.len() > 1000 {
        return Err("Question must contain 1–1,000 UTF-8 bytes".into());
    }
    super::selection::require(state.inner(), scope_id)?;
    state.ai.reserve("inference", spec.id, Some(scope_id))?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let worker = state.clone();
        let result = blocking(move || {
            let context = lock(&worker.db)?
                .folder_context(scope_id, &question, &worker.ai.cancel)
                .map_err(display_error)?;
            if context.overview.indexed_files == 0 {
                return Err("Scan this folder first; there are no indexed files to explain".into());
            }
            let evidence: Vec<_> = context
                .examples
                .into_iter()
                .map(|f| Evidence {
                    id: f.id,
                    path: f.path,
                    bytes: f.size,
                    excerpt: f.excerpt,
                })
                .collect();
            let overview_json = serde_json::to_string(&context.overview).map_err(display_error)?;
            let (prompt, count) = prompt::folder_prompt(&question, &overview_json, &evidence)?;
            let model = worker.ai.store.verify(spec, &worker.ai.cancel)?;
            let answer = worker.ai.worker.run(
                model,
                &prompt,
                backend,
                &worker.ai.cancel,
                Duration::from_secs(90),
            )?;
            lock(&worker.db)?
                .root(scope_id)
                .map_err(|_| "Folder was forgotten; response discarded".to_string())?;
            if worker.ai.cancel.load(Ordering::Relaxed) {
                return Err("Inference cancelled".into());
            }
            Ok((answer, count, context.overview))
        })
        .await;
        if let Ok(mut job) = state.ai.job.lock() {
            job.running = false;
            match result {
                Ok((answer, count, overview)) => {
                    if state.ai.cancel.load(Ordering::Relaxed)
                        || lock(&state.db)
                            .and_then(|db| db.root(scope_id).map_err(display_error))
                            .is_err()
                    {
                        job.message =
                            "Folder access revoked or request cancelled; response discarded".into();
                        job.answer = None;
                        return;
                    }
                    job.message = "Local answer ready. Check its claims against the files.".into();
                    job.answer = Some(answer);
                    job.sampled_files = count;
                    job.total_indexed = overview.indexed_files as i64;
                    job.overview = Some(overview);
                }
                Err(e) => job.message = e,
            };
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_jobs_and_resets_cancel_only_on_new_job() {
        let state = AiState {
            store: ModelStore::new(PathBuf::from("unused")),
            worker: Worker {
                executable: PathBuf::from("unused"),
                bytes: 0,
                sha256: String::new(),
            },
            job: Mutex::new(AiJob::default()),
            cancel: AtomicBool::new(true),
            downloaded: AtomicU64::new(9),
        };
        state.reserve("inference", "test", Some(1)).unwrap();
        assert!(!state.cancel.load(Ordering::Relaxed));
        assert_eq!(state.downloaded.load(Ordering::Relaxed), 0);
        state.cancel.store(true, Ordering::Relaxed);
        assert!(state.reserve("install", "test", None).is_err());
        assert!(state.cancel.load(Ordering::Relaxed));
        assert_eq!(state.job.lock().unwrap().scope_id, Some(1));
        state.job.lock().unwrap().running = false;
        state.reserve("install", "test", None).unwrap();
        assert_eq!(state.job.lock().unwrap().scope_id, None);
    }
}
