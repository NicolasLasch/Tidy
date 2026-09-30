//! Desktop IPC integration for the Phase 5 Safety Engine.
//! Provides approval generation, one-use token validation, durable execution, and undo workflows.

use super::{Shared, blocking, display_error, lock};
use std::{collections::HashMap, path::PathBuf};
use tauri::State;
use tidy_file_indexer::AuthorizedRoot;
use tidy_organization::{FileId, ProposedAction};
use tidy_safety::{ApprovalView, ExecutionReport, TransactionDetail, TransactionSummary};

#[tauri::command]
pub async fn request_plan_approval(
    scope_id: i64,
    rationale: String,
    actions: Vec<ProposedAction>,
    state: State<'_, Shared>,
) -> Result<ApprovalView, String> {
    let state = state.inner().clone();
    blocking(move || {
        let _activity = state
            .activity
            .try_lock()
            .map_err(|_| "File operations are busy")?;
        if lock(&state.job)?.running {
            return Err("Wait for the current scan to finish".into());
        }
        let (root_path, files) = {
            let db = lock(&state.db)?;
            let root_path = db.root(scope_id).map_err(display_error)?;
            let files = db.current_files(scope_id).map_err(display_error)?;
            (root_path, files)
        };

        let root = AuthorizedRoot::authorize(root_path).map_err(display_error)?;
        if !lock(&state.db)?
            .matches_root(scope_id, &root)
            .map_err(display_error)?
        {
            return Err("Selected root changed; select it again".into());
        }

        let mut files_map: HashMap<FileId, PathBuf> = HashMap::new();
        for f in files {
            files_map.insert(FileId(f.id as u64), f.path);
        }

        let cache = lock(&state.db)?.cache(scope_id).map_err(display_error)?;
        for action in &actions {
            let id = match action {
                // Folder actions are validated by the safety engine against the live filesystem.
                ProposedAction::CreateFolder { .. } | ProposedAction::MoveFolder { .. } => continue,
                ProposedAction::Move { source, .. }
                | ProposedAction::Rename { source, .. }
                | ProposedAction::Copy{source,..}
                | ProposedAction::Permissions{source,..}
                | ProposedAction::Trash { source } => source,
            };
            let path = files_map.get(id).ok_or("File no longer indexed")?;
            if std::fs::symlink_metadata(root.path().join(path))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
                lock(&state.db)?.reconcile_verified(scope_id, &[(path.clone(), None)]).map_err(display_error)?;
                return Err(format!("{} is no longer at its indexed location. Its stale index entry was removed; refresh results and review a new plan. No approval was created.",path.display()));
            }
            let expected = cache
                .get(&tidy_file_indexer::index_scan::path_bytes(path))
                .ok_or("File no longer indexed")?;
            if tidy_safety::validator::fingerprint(&root, path).map_err(display_error)?
                != expected.fingerprint
            {
                return Err("File changed since indexing. Rescan and generate a new plan.".into());
            }
        }
        state
            .safety
            .request_plan_approval(&root, scope_id, &rationale, &actions, &files_map)
            .map_err(display_error)
    })
    .await
}

/// Previews whole indexed folders for the native Trash. Nothing moves until the returned
/// one-use approval is executed.
#[tauri::command]
pub async fn request_folder_trash_approval(
    scope_id: i64,
    folders: Vec<String>,
    state: State<'_, Shared>,
) -> Result<ApprovalView, String> {
    let relatives: Vec<PathBuf> = folders.iter().map(PathBuf::from).collect();
    if relatives.iter().any(|relative| {
        relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
    }) {
        return Err("Choose folders inside the selected root, not the root itself".into());
    }
    let state = state.inner().clone();
    blocking(move || {
        let _activity = state
            .activity
            .try_lock()
            .map_err(|_| "File operations are busy")?;
        let root = checked_root(&state, scope_id)?;
        // A folder that is already gone (for example already in the Trash) is dropped from the
        // index so the next list is right, and the message says what happened.
        for relative in &relatives {
            if std::fs::symlink_metadata(root.path().join(relative))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                lock(&state.db)?
                    .remove_tree(scope_id, relative)
                    .map_err(display_error)?;
                return Err(format!(
                    "“{}” is no longer there — it may already be in the Trash. Tidy refreshed its list; ask again to get an up-to-date plan.",
                    relative.display()
                ));
            }
        }
        let rationale = if folders.len() == 1 {
            format!(
                "Move the folder \"{}\" and everything inside it to the Trash",
                folders[0]
            )
        } else {
            format!(
                "Move {} folders and everything inside them to the Trash",
                folders.len()
            )
        };
        state
            .safety
            .request_folder_trash_approval(&root, scope_id, &rationale, &relatives)
            .map_err(display_error)
    })
    .await
}

#[tauri::command]
pub async fn execute_approved_plan(
    scope_id: i64,
    token: String,
    state: State<'_, Shared>,
) -> Result<ExecutionReport, String> {
    let state = state.inner().clone();
    blocking(move || {
        let _activity = state
            .activity
            .try_lock()
            .map_err(|_| "File operations are busy")?;
        let root = checked_root(&state, scope_id)?;
        let approval = state
            .safety
            .approval_mgr
            .inspect(&token)
            .map_err(display_error)?;
        if approval.scope_id != scope_id {
            return Err("Approval belongs to another scope".into());
        }
        let result = state
            .safety
            .execute_approved_plan(&root, &token)
            .map_err(display_error);
        // Reconcile verified steps even if a later step failed.
        reconcile(&state, scope_id, approval.tx_id)?;
        result.map_err(|e| explain_failure(&state, approval.tx_id, e))
    })
    .await
}

/// Says what already happened before a step failed, so a partly-done plan is never a surprise.
fn explain_failure(state: &Shared, tx: i64, error: String) -> String {
    let mut message = error.clone();
    if let Ok(Some(detail)) = state.safety.get_detail(tx) {
        let done: Vec<&str> = detail
            .steps
            .iter()
            .filter(|s| s.state == "verified")
            .map(|s| s.source_relative.as_str())
            .collect();
        if !done.is_empty() {
            message.push_str(&format!(
                " Already done before it stopped: {}. Nothing after the failed step was attempted.",
                done.join(", ")
            ));
        }
    }
    if error.to_lowercase().contains("permission") {
        message.push_str(" Tidy fixes read-only folders itself; this one is owned by another user or protected by macOS, so it needs to be changed in Finder (Get Info → Sharing & Permissions).");
    }
    message
}

#[tauri::command]
pub async fn request_undo_approval(
    scope_id: i64,
    tx_id: i64,
    state: State<'_, Shared>,
) -> Result<ApprovalView, String> {
    let state = state.inner().clone();
    blocking(move || {
        let _activity = state
            .activity
            .try_lock()
            .map_err(|_| "File operations are busy")?;
        let root = checked_root(&state, scope_id)?;

        state
            .safety
            .request_undo_approval(&root, tx_id)
            .map_err(display_error)
    })
    .await
}

#[tauri::command]
pub async fn execute_approved_undo(
    scope_id: i64,
    original_tx_id: i64,
    undo_token: String,
    state: State<'_, Shared>,
) -> Result<ExecutionReport, String> {
    let state = state.inner().clone();
    blocking(move || {
        let _activity = state
            .activity
            .try_lock()
            .map_err(|_| "File operations are busy")?;
        let root = checked_root(&state, scope_id)?;
        let approval = state
            .safety
            .approval_mgr
            .inspect(&undo_token)
            .map_err(display_error)?;
        if approval.scope_id != scope_id {
            return Err("Approval belongs to another scope".into());
        }
        let result = state
            .safety
            .execute_approved_undo(&root, original_tx_id, &undo_token)
            .map_err(display_error);
        reconcile(&state, scope_id, approval.tx_id)?;
        let report = result?;

        Ok(report)
    })
    .await
}

#[tauri::command]
pub async fn list_journal_history(
    scope_id: Option<i64>,
    state: State<'_, Shared>,
) -> Result<Vec<TransactionSummary>, String> {
    let state = state.inner().clone();
    blocking(move || state.safety.list_history(scope_id).map_err(display_error)).await
}

#[tauri::command]
pub async fn get_transaction_detail(
    tx_id: i64,
    state: State<'_, Shared>,
) -> Result<Option<TransactionDetail>, String> {
    let state = state.inner().clone();
    blocking(move || state.safety.get_detail(tx_id).map_err(display_error)).await
}

fn checked_root(state: &Shared, scope: i64) -> Result<AuthorizedRoot, String> {
    if lock(&state.job)?.running {
        return Err("Wait for the current scan to finish".into());
    }
    let db = lock(&state.db)?;
    let root =
        AuthorizedRoot::authorize(db.root(scope).map_err(display_error)?).map_err(display_error)?;
    if !db.matches_root(scope, &root).map_err(display_error)? {
        return Err("Selected root was replaced".into());
    }
    Ok(root)
}
fn reconcile(state: &Shared, scope: i64, tx: i64) -> Result<(), String> {
    let mut changes = Vec::new();
    let mut copies = Vec::new();
    let mut trees = Vec::new();
    let mut renames = Vec::new();
    if let Some(detail) = state.safety.get_detail(tx)? {
        if detail.summary.scope_id != scope {
            return Err("Journal scope mismatch".into());
        }
        for step in detail.steps.into_iter().filter(|s| s.state == "verified") {
            if step.action_type == "copy" {
                let (_, Some(stamp), _) = state.safety.journal.evidence(step.id)? else {
                    continue;
                };
                let dest = step
                    .destination_relative
                    .ok_or("Copy destination missing")?;
                copies.push((
                    PathBuf::from(step.source_relative),
                    PathBuf::from(dest),
                    stamp,
                ));
                continue;
            }
            if step.action_type == "move_dir" {
                if let Some(dest) = step.destination_relative {
                    renames.push((PathBuf::from(step.source_relative), PathBuf::from(dest)));
                }
                continue;
            }
            if step.action_type == "create_dir" {
                continue;
            }
            if step.action_type == "trash_dir" {
                trees.push(PathBuf::from(step.source_relative));
                continue;
            }
            if step.action_type == "permissions" {
                let (_, Some(stamp), _) = state.safety.journal.evidence(step.id)? else {
                    continue;
                };
                let path = PathBuf::from(step.source_relative);
                changes.push((path.clone(), Some((path, stamp))));
                continue;
            }
            let destination = if let Some(dest) = step.destination_relative {
                let (_, Some(stamp), _) = state.safety.journal.evidence(step.id)? else {
                    continue;
                };
                Some((PathBuf::from(dest), stamp))
            } else {
                None
            };
            changes.push((PathBuf::from(step.source_relative), destination));
        }
    }
    for (from, to) in &renames {
        lock(&state.db)?
            .rename_tree(scope, from, to)
            .map_err(display_error)?;
    }
    for tree in &trees {
        lock(&state.db)?
            .remove_tree(scope, tree)
            .map_err(display_error)?;
    }
    lock(&state.db)?
        .reconcile_copies(scope, &copies)
        .map_err(display_error)?;
    lock(&state.db)?
        .reconcile_verified(scope, &changes)
        .map_err(display_error)
}
#[tauri::command]
pub async fn reveal_trash_file(
    scope_id: i64,
    tx_id: i64,
    step_id: i64,
    state: State<'_, Shared>,
) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let detail=state.safety.get_detail(tx_id)?.ok_or("Transaction not found")?;
        if detail.summary.scope_id!=scope_id || !detail.steps.iter().any(|s|s.id==step_id && matches!(s.action_type.as_str(),"trash"|"trash_dir")) {return Err("Trash receipt does not belong to this transaction".into());}
        let (_,_,path)=state.safety.journal.evidence(step_id)?;
        let path=path.ok_or("No automatic recovery location was recorded. Inspect Finder Trash manually.")?;
        if !std::path::Path::new(&path).exists() {return Err("Recorded Trash item is no longer present. It may have been restored or removed in Finder.".into());}
        #[cfg(target_os="macos")] {
            let status=std::process::Command::new("/usr/bin/open").arg("-R").arg(path).status().map_err(display_error)?;
            if !status.success(){return Err("Finder could not reveal the Trash item".into());}Ok(())
        }
        #[cfg(not(target_os="macos"))] {Err("Reveal is currently available on macOS".into())}
    }).await
}

/// Puts one item from the Trash back where it was (never replacing anything) and journals it.
#[tauri::command]
pub async fn restore_from_trash(
    scope_id: i64,
    tx_id: i64,
    step_id: i64,
    state: State<'_, Shared>,
) -> Result<ExecutionReport, String> {
    let state = state.inner().clone();
    blocking(move || {
        let _activity = state
            .activity
            .try_lock()
            .map_err(|_| "File operations are busy")?;
        let root = checked_root(&state, scope_id)?;
        state
            .safety
            .restore_trashed(&root, scope_id, tx_id, step_id)
            .map_err(display_error)
    })
    .await
}
/// Finds the index id of a file by its path relative to a scope, so a file listed on disk can be
/// sent through the normal reviewed Trash flow.
#[tauri::command]
pub async fn file_id_for_path(
    scope_id: i64,
    relative: String,
    state: State<'_, Shared>,
) -> Result<i64, String> {
    let state = state.inner().clone();
    blocking(move || {
        let wanted = std::path::PathBuf::from(relative);
        lock(&state.db)?
            .current_files(scope_id)
            .map_err(display_error)?
            .into_iter()
            .find(|f| f.path == wanted)
            .map(|f| f.id)
            .ok_or_else(|| {
                "This file is not in Tidy's index yet. Rescan the folder, then try again."
                    .to_string()
            })
    })
    .await
}
