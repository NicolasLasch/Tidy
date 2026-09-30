//! Authorized index overview; no implicit whole-disk crawl or file mutation.
use super::{Shared, blocking, display_error, lock};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
pub(super) struct CachedFolders {
    scope_id: i64,
    revision: u64,
    folders: Arc<Vec<tidy_storage::FolderUsage>>,
}
use tauri::State;
#[derive(Serialize)]
pub struct Volume {
    id: String,
    label: String,
    total_bytes: u64,
    used_bytes: u64,
    available_bytes: u64,
}
#[derive(Serialize)]
pub struct RootUsage {
    id: i64,
    path: String,
    logical_bytes: u64,
    files: usize,
    scanned_at: Option<i64>,
    status: String,
    omitted: i64,
    volume_id: Option<String>,
    error: Option<String>,
}
#[derive(Serialize)]
pub struct Overview {
    roots: Vec<RootUsage>,
    volumes: Vec<Volume>,
    indexed_bytes: u64,
    indexed_files: usize,
}
#[tauri::command]
pub async fn storage_overview(state: State<'_, Shared>) -> Result<Overview, String> {
    let state = state.inner().clone();
    blocking(move || {
        if lock(&state.job)?.running {
            return Err("Wait for the index scan to finish, then refresh storage.".into());
        }
        let db = lock(&state.db)?;
        let mut scopes = db.scopes().map_err(display_error)?;
        scopes.sort_by_key(|s| std::cmp::Reverse(s.scanned_at)); // latest overlapping snapshot wins
        let mut paths = HashMap::new();
        let mut roots = Vec::new();
        let mut volumes = Vec::new();
        let mut volume_ids = HashSet::new();
        for scope in scopes {
            let root = db.root(scope.id).map_err(display_error)?;
            let files = db.storage_files(scope.id).map_err(display_error)?;
            let logical_bytes = files
                .iter()
                .fold(0u64, |total, f| total.saturating_add(f.size));
            for f in &files {
                paths.entry(root.join(&f.path)).or_insert(f.size);
            }
            let (volume_id, error) = match tidy_platform::AuthorizedRoot::authorize(&root)
                .and_then(|r| tidy_platform::volume_space(r.path()))
            {
                Ok(Some(v)) => {
                    let id = v.identity.clone();
                    if volume_ids.insert(id.clone()) {
                        volumes.push(Volume {
                            id: id.clone(),
                            label: root
                                .strip_prefix("/Volumes")
                                .ok()
                                .and_then(|p| p.components().next())
                                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                                .unwrap_or_else(|| "Computer storage".into()),
                            total_bytes: v.total_bytes,
                            used_bytes: v.total_bytes.saturating_sub(v.free_bytes),
                            available_bytes: v.available_bytes,
                        });
                    }
                    (Some(id), None)
                }
                Ok(None) => (
                    None,
                    Some("Volume capacity is unavailable on this platform".into()),
                ),
                Err(e) => (None, Some(format!("Folder/volume unavailable: {e}"))),
            };
            roots.push(RootUsage {
                id: scope.id,
                path: scope.path,
                logical_bytes,
                files: files.len(),
                scanned_at: scope.scanned_at,
                status: scope.status,
                omitted: scope.omitted,
                volume_id,
                error,
            });
        }
        roots.sort_by_key(|r| std::cmp::Reverse(r.logical_bytes));
        Ok(Overview {
            roots,
            volumes,
            indexed_bytes: paths.values().fold(0u64, |a, b| a.saturating_add(*b)),
            indexed_files: paths.len(),
        })
    })
    .await
}
#[derive(Serialize)]
pub struct FolderPage {
    parent: String,
    folders: Vec<tidy_storage::FolderUsage>,
    folder_count: usize,
    direct_files: usize,
    direct_bytes: u64,
    logical_bytes: u64,
    file_count: usize,
    omitted: i64,
    status: String,
    scanned_at: Option<i64>,
}
#[tauri::command]
pub async fn storage_folder_page(
    scope_id: i64,
    parent: String,
    offset: usize,
    state: State<'_, Shared>,
) -> Result<FolderPage, String> {
    let path = std::path::PathBuf::from(&parent);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Select a relative indexed folder".into());
    }
    let state = state.inner().clone();
    blocking(move || {
        if lock(&state.job)?.running {
            return Err("Wait for the scan to finish.".into());
        }
        let db = lock(&state.db)?;
        let scope = db
            .scopes()
            .map_err(display_error)?
            .into_iter()
            .find(|s| s.id == scope_id)
            .ok_or("Folder is no longer authorized")?;
        let revision = db.revision();
        let mut cache = lock(&state.folder_cache)?;
        let totals = if let Some(c) = cache
            .as_ref()
            .filter(|c| c.scope_id == scope_id && c.revision == revision)
        {
            c.folders.clone()
        } else {
            let files = db
                .storage_files(scope_id)
                .map_err(display_error)?
                .into_iter()
                .map(|f| tidy_storage::AnalyzableFile {
                    id: f.id as u64,
                    path: f.path,
                    size: f.size,
                    modified: f.modified,
                    hash: None,
                    identity: f.identity,
                })
                .collect::<Vec<_>>();
            let folders = Arc::new(tidy_storage::folder_usage(&files));
            *cache = Some(CachedFolders {
                scope_id,
                revision,
                folders: folders.clone(),
            });
            folders
        };
        drop(cache);
        let current = totals
            .iter()
            .find(|f| f.path == path)
            .ok_or("Folder is not in the indexed snapshot")?;
        let children = tidy_storage::children(&totals, &path);
        let folder_count = children.len();
        Ok(FolderPage {
            parent,
            folders: children
                .into_iter()
                .skip(offset)
                .take(50)
                .cloned()
                .collect(),
            folder_count,
            direct_files: current.direct_files,
            direct_bytes: current.direct_bytes,
            logical_bytes: current.logical_bytes,
            file_count: current.file_count,
            omitted: scope.omitted,
            status: scope.status,
            scanned_at: scope.scanned_at,
        })
    })
    .await
}

/// A user click authorizes this descendant for indexing; no filesystem modification.
#[tauri::command]
pub async fn use_working_folder(
    scope_id: i64,
    parent: String,
    state: State<'_, Shared>,
) -> Result<i64, String> {
    let path = std::path::PathBuf::from(parent);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Choose a relative subfolder".into());
    }
    let state = state.inner().clone();
    blocking(move || {
        let _activity = state
            .activity
            .try_lock()
            .map_err(|_| "Wait for file operations to finish")?;
        let job = lock(&state.job)?;
        if job.running {
            return Err("Wait for the scan to finish".into());
        }
        let db = lock(&state.db)?;
        let root =
            tidy_platform::AuthorizedRoot::authorize(db.root(scope_id).map_err(display_error)?)
                .map_err(display_error)?;
        if !db.matches_root(scope_id, &root).map_err(display_error)? {
            return Err("Folder identity changed; authorize and scan it again".into());
        }
        root.validate(&root.path().join(&path))
            .map_err(display_error)?;
        if path.as_os_str().is_empty() {
            return Ok(scope_id);
        }
        let child = tidy_platform::AuthorizedRoot::authorize(root.path().join(path))
            .map_err(display_error)?;
        db.add_scope(&child).map_err(display_error)
    })
    .await
}

#[derive(Serialize)]
pub struct AnyFolder {
    path: String,
    depth: usize,
    logical_bytes: u64,
    file_count: usize,
    /// Scope that already covers exactly this folder, if the user ever indexed it on its own.
    scope_id: Option<i64>,
}
/// Every indexed folder below one base folder, heaviest first, for switching folders on and off.
#[tauri::command]
pub async fn storage_all_folders(
    scope_id: i64,
    limit: usize,
    state: State<'_, Shared>,
) -> Result<Vec<AnyFolder>, String> {
    let state = state.inner().clone();
    blocking(move || {
        if lock(&state.job)?.running {
            return Err("Wait for the scan to finish.".into());
        }
        let db = lock(&state.db)?;
        let base = db.root(scope_id).map_err(display_error)?;
        let files = db
            .storage_files(scope_id)
            .map_err(display_error)?
            .into_iter()
            .map(|f| tidy_storage::AnalyzableFile {
                id: f.id as u64,
                path: f.path,
                size: f.size,
                modified: f.modified,
                hash: None,
                identity: f.identity,
            })
            .collect::<Vec<_>>();
        let mut folders: Vec<_> = tidy_storage::folder_usage(&files)
            .into_iter()
            .filter(|f| !f.path.as_os_str().is_empty())
            .collect();
        folders.sort_by(|a, b| {
            b.logical_bytes
                .cmp(&a.logical_bytes)
                .then(a.path.cmp(&b.path))
        });
        folders.truncate(limit.clamp(1, 1000));
        let mut by_path = HashMap::new();
        for scope in db.scopes().map_err(display_error)? {
            if let Ok(root) = db.root(scope.id) {
                by_path.insert(root, scope.id);
            }
        }
        Ok(folders
            .into_iter()
            .map(|f| AnyFolder {
                depth: f.path.components().count(),
                scope_id: by_path.get(&base.join(&f.path)).copied(),
                path: f.path.to_string_lossy().into_owned(),
                logical_bytes: f.logical_bytes,
                file_count: f.file_count,
            })
            .collect())
    })
    .await
}
