use super::{Shared, blocking, display_error, lock};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Mutex,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tauri::State;
use tidy_agent_runtime::{
    catalog::{self, ModelSpec},
    store::ModelStore,
    worker::Worker,
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
    pub planning_trace: Vec<tidy_agent_runtime::investigation::Trace>,
}
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    custom: Vec<ModelSpec>,
    #[serde(default)]
    selected: Option<String>,
}
pub struct AiState {
    saved: Mutex<Saved>,
    dir: PathBuf,
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
        let saved = std::fs::read_to_string(data.join("ai_models.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Ok(Self {
            saved: Mutex::new(saved),
            dir: data.clone(),
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
    fn persist(&self, saved: &Saved) -> Result<(), String> {
        let tmp = self.dir.join("ai_models.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(saved).map_err(display_error)?)
            .map_err(display_error)?;
        std::fs::rename(tmp, self.dir.join("ai_models.json")).map_err(display_error)
    }
    /// Built-in models plus any the user added from Hugging Face.
    pub fn models(&self) -> Vec<ModelSpec> {
        let mut all = catalog::builtin();
        if let Ok(saved) = self.saved.lock() {
            all.extend(saved.custom.iter().cloned());
        }
        all
    }
    pub fn find(&self, id: &str) -> Result<ModelSpec, String> {
        self.models()
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| "Unknown model".into())
    }
    /// The model Tidy will use: the one picked in the chat if it is installed, otherwise the largest
    /// installed model.
    pub fn chosen(&self) -> Option<ModelSpec> {
        let picked = self.saved.lock().ok().and_then(|s| s.selected.clone());
        let installed: Vec<ModelSpec> = self
            .models()
            .into_iter()
            .filter(|m| self.store.present(m))
            .collect();
        picked
            .and_then(|id| installed.iter().find(|m| m.id == id).cloned())
            .or_else(|| installed.into_iter().max_by_key(|m| m.bytes))
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
    spec: ModelSpec,
    installed: bool,
}
#[derive(Serialize)]
pub struct Status {
    models: Vec<ModelView>,
    /// The model that will actually be used right now.
    selected: Option<String>,
    worker_available: bool,
    metal_available: bool,
    job: AiJob,
}
#[tauri::command]
pub fn ai_status(state: State<'_, Shared>) -> Result<Status, String> {
    let mut job = lock(&state.ai.job)?.clone();
    job.downloaded = state.ai.downloaded.load(Ordering::Relaxed);
    Ok(Status {
        selected: state.ai.chosen().map(|m| m.id),
        models: state
            .ai
            .models()
            .into_iter()
            .map(|spec| ModelView {
                installed: state.ai.store.present(&spec),
                spec,
            })
            .collect(),
        worker_available: state.ai.worker.bytes > 0 && state.ai.worker.executable.is_file(),
        metal_available: cfg!(target_os = "macos"),
        job,
    })
}
#[tauri::command]
pub fn ai_select_model(model_id: String, state: State<'_, Shared>) -> Result<(), String> {
    let spec = state.ai.find(&model_id)?;
    if !state.ai.store.present(&spec) {
        return Err("Download this model first".into());
    }
    let mut saved = lock(&state.ai.saved)?;
    saved.selected = Some(spec.id);
    state.ai.persist(&saved)
}
/// Looks up a Hugging Face .gguf link (metadata only) and adds it to the list, ready to download.
#[tauri::command]
pub async fn add_custom_model(link: String, state: State<'_, Shared>) -> Result<ModelSpec, String> {
    let state = state.inner().clone();
    blocking(move || {
        let spec = catalog::fetch_hugging_face(&link)?;
        if state.ai.models().iter().any(|m| m.id == spec.id) {
            return Err("That model is already in the list".into());
        }
        let mut saved = lock(&state.ai.saved)?;
        saved.custom.push(spec.clone());
        state.ai.persist(&saved)?;
        Ok(spec)
    })
    .await
}
#[tauri::command]
pub fn remove_custom_model(model_id: String, state: State<'_, Shared>) -> Result<(), String> {
    let mut saved = lock(&state.ai.saved)?;
    let Some(at) = saved.custom.iter().position(|m| m.id == model_id) else {
        return Err("Only models you added can be removed".into());
    };
    let spec = saved.custom.remove(at);
    if saved.selected.as_deref() == Some(model_id.as_str()) {
        saved.selected = None;
    }
    let _ = std::fs::remove_file(state.ai.store.path(&spec));
    state.ai.persist(&saved)
}
#[tauri::command]
pub fn cancel_ai(state: State<'_, Shared>) {
    state.ai.cancel.store(true, Ordering::Relaxed);
}
#[tauri::command]
pub fn install_model(model_id: String, state: State<'_, Shared>) -> Result<(), String> {
    let spec = state.ai.find(&model_id)?;
    state.ai.reserve("install", &spec.id, None)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let worker = state.clone();
        let result = blocking(move || {
            worker
                .ai
                .store
                .install(&spec, &worker.ai.cancel, &worker.ai.downloaded)
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_jobs_and_resets_cancel_only_on_new_job() {
        let state = AiState {
            saved: Mutex::new(Saved::default()),
            dir: PathBuf::from("unused"),
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
