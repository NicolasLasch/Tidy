#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod ai;
mod planning;
mod safety_ipc;
mod selection;
mod storage_overview;
use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tidy_file_indexer::{
    AuthorizedRoot,
    database::{Index, Page, Scope},
    index_scan::{self, IndexOptions, Omission},
};

#[derive(Default, Clone, Serialize)]
struct JobView {
    running: bool,
    scope_id: Option<i64>,
    visited: usize,
    message: String,
    omissions: Vec<Omission>,
}
struct AppState {
    ai: ai::AiState,
    db: Mutex<Index>,
    folder_cache: Mutex<Option<storage_overview::CachedFolders>>,
    job: Mutex<JobView>,
    safety: Arc<tidy_safety::SafetyEngine>,
    selected: Mutex<std::collections::HashSet<i64>>,
    data_dir: std::path::PathBuf,
    activity: Mutex<()>,
    cancel: AtomicBool,
    progress: AtomicUsize,
}
type Shared = Arc<AppState>;
fn lock<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
    mutex
        .lock()
        .map_err(|_| "Internal state unavailable; restart Tidy".into())
}
fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
async fn blocking<T: Send + 'static>(
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(display_error)?
}
#[tauri::command]
async fn choose_folder(
    app: tauri::AppHandle,
    state: State<'_, Shared>,
) -> Result<Option<i64>, String> {
    let state = state.inner().clone();
    blocking(move || {
        let Some(selected) = app
            .dialog()
            .file()
            .set_title("Choose a folder for Tidy to read")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let path = selected.into_path().map_err(display_error)?;
        let root = AuthorizedRoot::authorize(path).map_err(display_error)?;
        let id = lock(&state.db)?.add_scope(&root).map_err(display_error)?;
        Ok(Some(id))
    })
    .await
}
#[tauri::command]
async fn list_scopes(state: State<'_, Shared>) -> Result<Vec<Scope>, String> {
    let state = state.inner().clone();
    blocking(move || lock(&state.db)?.scopes().map_err(display_error)).await
}
#[tauri::command]
async fn search_files(
    scope_id: i64,
    query: String,
    size_sort: bool,
    offset: u32,
    state: State<'_, Shared>,
) -> Result<Page, String> {
    let state = state.inner().clone();
    blocking(move || {
        lock(&state.db)?
            .search(scope_id, &query, size_sort, offset)
            .map_err(display_error)
    })
    .await
}
#[tauri::command]
async fn list_scope_files(
    scope_id: i64,
    state: State<'_, Shared>,
) -> Result<Vec<serde_json::Value>, String> {
    let state = state.inner().clone();
    blocking(move || {
        Ok(lock(&state.db)?.current_files(scope_id).map_err(display_error)?.into_iter().map(|f| serde_json::json!({"id":f.id,"display":f.display,"path":f.path,"size":f.size,"modified":f.modified})).collect())
    })
    .await
}
#[tauri::command]
fn scan_status(state: State<'_, Shared>) -> Result<JobView, String> {
    let mut view = lock(&state.job)?.clone();
    view.visited = state.progress.load(Ordering::Relaxed);
    Ok(view)
}
#[tauri::command]
fn cancel_scan(state: State<'_, Shared>) {
    state.cancel.store(true, Ordering::Relaxed);
}
#[tauri::command]
async fn forget_folder(scope_id: i64, state: State<'_, Shared>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let _activity = state
            .activity
            .try_lock()
            .map_err(|_| "Cannot forget a folder while file operations run")?;
        // Same lock ordering as worker completion: job, then database. Revocation and commit serialize.
        let job = lock(&state.job)?;
        if job.running && job.scope_id == Some(scope_id) {
            state.cancel.store(true, Ordering::Relaxed);
        }
        {
            let mut ai = lock(&state.ai.job)?;
            if ai.scope_id == Some(scope_id) {
                state.ai.cancel.store(true, Ordering::Relaxed);
                ai.answer = None;
            }
        }
        selection::deselect(&state, scope_id)?;
        lock(&state.db)?.forget(scope_id).map_err(display_error)
    })
    .await
}
#[tauri::command]
fn start_scan(scope_id: i64, content: bool, state: State<'_, Shared>) -> Result<(), String> {
    let state = state.inner().clone();
    let activity = state
        .activity
        .try_lock()
        .map_err(|_| "File operations are busy")?;
    {
        let mut job = lock(&state.job)?;
        if job.running {
            return Err("A scan is already running".into());
        }
        state.cancel.store(false, Ordering::Relaxed);
        state.progress.store(0, Ordering::Relaxed);
        *job = JobView {
            running: true,
            scope_id: Some(scope_id),
            message: "Reading folder…".into(),
            ..Default::default()
        };
    }
    drop(activity);
    tauri::async_runtime::spawn(async move {
        let worker = state.clone();
        let result = blocking(move || {
            let started = Instant::now();
            let (path, cache) = {
                let db = lock(&worker.db)?;
                (
                    db.root(scope_id).map_err(display_error)?,
                    db.cache(scope_id).map_err(display_error)?,
                )
            };
            let root = AuthorizedRoot::authorize(path).map_err(display_error)?;
            if !lock(&worker.db)?
                .matches_root(scope_id, &root)
                .map_err(display_error)?
            {
                return Err(
                    "The selected folder was replaced. Forget it and select it again.".into(),
                );
            }
            let snapshot = index_scan::collect(
                &root,
                &IndexOptions {
                    content,
                    ..Default::default()
                },
                &worker.cancel,
                &worker.progress,
                &cache,
            )
            .map_err(display_error)?;
            let _job = lock(&worker.job)?;
            // A forgotten scope cannot be recreated by a late worker result.
            lock(&worker.db)?
                .commit(scope_id, &snapshot, content)
                .map_err(display_error)?;
            Ok((
                format!(
                    "{} · {} files · {} excluded · {:.1}s",
                    snapshot.status,
                    snapshot.files.len(),
                    snapshot.omission_count,
                    started.elapsed().as_secs_f64()
                ),
                snapshot.omissions,
            ))
        })
        .await;
        if let Ok(mut job) = state.job.lock() {
            job.running = false;
            match result {
                Ok((message, omissions)) => {
                    job.message = message;
                    job.omissions = omissions;
                }
                Err(error) => job.message = format!("Scan stopped: {error}"),
            }
        }
    });
    Ok(())
}
/// Switches between the full window and a small always-on-top assistant in the top-right corner.
#[tauri::command]
fn set_compact_mode(app: tauri::AppHandle, compact: bool) -> Result<(), String> {
    use tauri::{LogicalSize, PhysicalPosition};
    let window = app.get_webview_window("main").ok_or("Window unavailable")?;
    if compact {
        window.set_min_size(Some(LogicalSize::new(340.0, 460.0))).map_err(display_error)?;
        window.set_size(LogicalSize::new(400.0, 680.0)).map_err(display_error)?;
        if let Some(monitor) = window.current_monitor().map_err(display_error)? {
            let scale = monitor.scale_factor();
            let (size, origin) = (monitor.size(), monitor.position());
            let x = origin.x as f64 + size.width as f64 - (400.0 + 16.0) * scale;
            let y = origin.y as f64 + 44.0 * scale;
            window.set_position(PhysicalPosition::new(x, y)).map_err(display_error)?;
        }
        window.set_always_on_top(true).map_err(display_error)?;
    } else {
        window.set_always_on_top(false).map_err(display_error)?;
        window.set_min_size(Some(LogicalSize::new(760.0, 560.0))).map_err(display_error)?;
        window.set_size(LogicalSize::new(1120.0, 760.0)).map_err(display_error)?;
        window.center().map_err(display_error)?;
    }
    Ok(())
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = app.path().app_data_dir()?;
            std::fs::create_dir_all(&path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
            }
            let mut db = Index::open(&path.join("index.sqlite3"))?;
            let safety = tidy_safety::SafetyEngine::new(&path.join("safety_journal.sqlite3"))
                .map_err(std::io::Error::other)?;
            let _ = safety.startup_recovery();
            for scope in db.scopes()? {
                let root = db.root(scope.id)?;
                let mut missing = Vec::new();
                for tx in safety
                    .list_history(Some(scope.id))
                    .map_err(std::io::Error::other)?
                {
                    if let Some(detail) = safety.get_detail(tx.id).map_err(std::io::Error::other)? {
                        for step in detail.steps {
                            if step.action_type == "trash_dir" && step.state == "verified" {
                                let relative = std::path::PathBuf::from(&step.source_relative);
                                if relative
                                    .components()
                                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                                    && std::fs::symlink_metadata(root.join(&relative))
                                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                                {
                                    db.remove_tree(scope.id, &relative)?;
                                }
                            } else if step.action_type == "trash" && step.state == "verified" {
                                let relative = std::path::PathBuf::from(step.source_relative);
                                if relative
                                    .components()
                                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                                    && std::fs::symlink_metadata(root.join(&relative))
                                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                                {
                                    missing.push((relative, None));
                                }
                            }
                        }
                    }
                }
                db.reconcile_verified(scope.id, &missing)?;
            }

            let ai = ai::AiState::new(path.clone(), app.path().resource_dir()?)
                .map_err(std::io::Error::other)?;
            app.manage(Arc::new(AppState {
                ai,
                activity: Mutex::new(()),
                db: Mutex::new(db),
                folder_cache: Mutex::new(None),
                job: Mutex::new(JobView::default()),
                safety: Arc::new(safety),
                selected: Mutex::new(selection::load(&path)),
                data_dir: path.clone(),
                cancel: AtomicBool::new(false),
                progress: AtomicUsize::new(0),
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            choose_folder,
            list_scopes,
            list_scope_files,
            safety_ipc::reveal_indexed_file,
            safety_ipc::reveal_trash_file,
            search_files,
            scan_status,
            start_scan,
            cancel_scan,
            forget_folder,
            ai::ai_status,
            ai::install_model,
            ai::ask_local,
            ai::cancel_ai,
            set_compact_mode,
            selection::ai_selection,
            selection::set_ai_selected,
            storage_overview::storage_all_folders,
            storage_overview::storage_overview,
            storage_overview::storage_folder_page,
            storage_overview::use_working_folder,
            planning::workflow_catalog,
            planning::analyze_storage_scope,
            planning::propose_organization,
            planning::plan_with_agent,
            planning::propose_storage_cleanup,
            safety_ipc::request_plan_approval,
            safety_ipc::request_folder_trash_approval,
            safety_ipc::execute_approved_plan,
            safety_ipc::request_undo_approval,
            safety_ipc::execute_approved_undo,
            safety_ipc::list_journal_history,
            safety_ipc::get_transaction_detail
        ])
        .build(tauri::generate_context!())
        .expect("Tidy could not start")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                let state = app.state::<Shared>();
                state.cancel.store(true, Ordering::Relaxed);
                state.ai.cancel.store(true, Ordering::Relaxed);
            }
        });
}
