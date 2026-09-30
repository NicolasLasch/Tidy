//! Safety Engine, Approval Workflow, Durable Journal, and Undo.
//!
//! Enforces Phase 5 safety guarantees:
//! 1. No autonomous filesystem mutations — execution strictly requires explicit one-use approval tokens.
//! 2. No-replace moves/renames — collisions immediately reject without overwrite.
//! 3. Native OS Trash only — zero permanent deletion (`rm -rf` strictly forbidden).
//! 4. Durable SQLite transaction journal with verified state transitions and startup recovery.
//! 5. Explicitly approved, reversible undo workflows.

pub mod approval;
pub mod executor;
pub mod journal;
pub mod model;
mod native_trash;
pub mod validator;

pub use approval::{Approval, ApprovalManager, ApprovalView};
pub use executor::{execute_transaction, prepare_undo};
pub use journal::{Journal, JournalStepRecord, TransactionDetail, TransactionSummary};
pub use model::{ExecutionReport, JournalState, Rejection, ValidatedAction};

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tidy_organization::{FileId, ProposedAction};
use tidy_platform::AuthorizedRoot;
use uuid::Uuid;

pub struct SafetyEngine {
    operation: Mutex<()>,
    _instance_lock: Option<std::fs::File>,
    pub journal: Arc<Journal>,
    pub approval_mgr: Arc<ApprovalManager>,
}

impl SafetyEngine {
    pub fn new(journal_path: &Path) -> Result<Self, rusqlite::Error> {
        #[cfg(unix)]
        let instance = Some(
            tidy_platform::handles::exclusive_lock(&journal_path.with_extension("lock")).map_err(
                |e| {
                    rusqlite::Error::InvalidParameterName(format!(
                        "Another Tidy instance may be running: {e}"
                    ))
                },
            )?,
        );
        #[cfg(not(unix))]
        let instance = None;
        let journal = Journal::open(journal_path)?;
        Ok(Self {
            operation: Mutex::new(()),
            _instance_lock: instance,
            journal: Arc::new(journal),
            approval_mgr: Arc::new(ApprovalManager::new()),
        })
    }

    pub fn new_in_memory() -> Result<Self, rusqlite::Error> {
        let journal = Journal::open_in_memory()?;
        Ok(Self {
            operation: Mutex::new(()),
            _instance_lock: None,
            journal: Arc::new(journal),
            approval_mgr: Arc::new(ApprovalManager::new()),
        })
    }

    /// Previews and validates proposed actions against the physical authorized scope,
    /// logs the transaction in state 'prepared', and issues a one-use approval token.
    pub fn request_plan_approval(
        &self,
        scope: &AuthorizedRoot,
        scope_id: i64,
        rationale: &str,
        actions: &[ProposedAction],
        files_map: &HashMap<FileId, PathBuf>,
    ) -> Result<ApprovalView, Rejection> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| Rejection::IoError("Safety state unavailable".into()))?;
        if actions.len() > 500 {
            return Err(Rejection::InvalidAction(
                "Review at most 500 actions per batch".into(),
            ));
        }
        if actions.is_empty() {
            return Err(Rejection::InvalidAction("Plan contains no actions".into()));
        }

        let mut sources = std::collections::HashSet::new();
        let mut destinations = std::collections::HashSet::new();
        let mut validated_actions = Vec::new();

        for action in actions {
            match action {
                ProposedAction::Move {
                    source,
                    destination_relative,
                } => {
                    let relative_src = files_map
                        .get(source)
                        .ok_or_else(|| Rejection::ChangedSource(format!("FileId #{}", source.0)))?;
                    let val = validator::validate_move(scope, relative_src, destination_relative)?;
                    validated_actions.push(val);
                }
                ProposedAction::Rename { source, new_name } => {
                    let relative_src = files_map
                        .get(source)
                        .ok_or_else(|| Rejection::ChangedSource(format!("FileId #{}", source.0)))?;
                    let val = validator::validate_rename(scope, relative_src, new_name)?;
                    validated_actions.push(val);
                }
                ProposedAction::Copy {
                    source,
                    destination_relative,
                } => {
                    let relative = files_map
                        .get(source)
                        .ok_or_else(|| Rejection::ChangedSource(format!("FileId #{}", source.0)))?;
                    validated_actions.push(validator::validate_copy(
                        scope,
                        relative,
                        destination_relative,
                    )?);
                }
                ProposedAction::Permissions { source, mode } => {
                    let relative = files_map
                        .get(source)
                        .ok_or_else(|| Rejection::ChangedSource(format!("FileId #{}", source.0)))?;
                    validated_actions
                        .push(validator::validate_permissions(scope, relative, *mode)?);
                }
                ProposedAction::CreateFolder { path } => {
                    validated_actions.push(validator::validate_create_dir(scope, path)?);
                }
                ProposedAction::MoveFolder {
                    source,
                    destination_relative,
                } => {
                    validated_actions.push(validator::validate_move_dir(
                        scope,
                        source,
                        destination_relative,
                    )?);
                }
                ProposedAction::Trash { source } => {
                    let relative_src = files_map
                        .get(source)
                        .ok_or_else(|| Rejection::ChangedSource(format!("FileId #{}", source.0)))?;
                    let val = validator::validate_trash(scope, relative_src)?;
                    validated_actions.push(val);
                }
            }
        }

        for action in &validated_actions {
            if !sources.insert(action.source().clone()) {
                return Err(Rejection::InvalidAction("Repeated source in plan".into()));
            }
            if let Some(dest) = action.relative_dest()
                && !destinations.insert(dest.to_string_lossy().to_lowercase())
            {
                return Err(Rejection::Collision(dest.display().to_string()));
            }
        }
        let tx_uuid = Uuid::new_v4().to_string();
        let tx_id = self
            .journal
            .record_prepared(&tx_uuid, scope_id, rationale, &validated_actions)
            .map_err(Rejection::IoError)?;

        self.capture_evidence(scope, tx_id, &validated_actions, None)?;
        let (_approval, view) =
            self.approval_mgr
                .create_approval(tx_id, scope_id, validated_actions, 300);

        Ok(view)
    }

    /// Previews whole folders for the native Trash as one transaction and issues a one-use token.
    pub fn request_folder_trash_approval(
        &self,
        scope: &AuthorizedRoot,
        scope_id: i64,
        rationale: &str,
        relative_dirs: &[PathBuf],
    ) -> Result<ApprovalView, Rejection> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| Rejection::IoError("Safety state unavailable".into()))?;
        if relative_dirs.is_empty() || relative_dirs.len() > 50 {
            return Err(Rejection::InvalidAction(
                "Choose between 1 and 50 folders per approval".into(),
            ));
        }
        for (i, a) in relative_dirs.iter().enumerate() {
            for b in &relative_dirs[i + 1..] {
                if a.starts_with(b) || b.starts_with(a) {
                    return Err(Rejection::InvalidAction(format!(
                        "{} and {} overlap; choose the outer folder only",
                        a.display(),
                        b.display()
                    )));
                }
            }
        }
        let actions = relative_dirs
            .iter()
            .map(|dir| validator::validate_trash_dir(scope, dir))
            .collect::<Result<Vec<_>, _>>()?;
        let tx_uuid = Uuid::new_v4().to_string();
        let tx_id = self
            .journal
            .record_prepared(&tx_uuid, scope_id, rationale, &actions)
            .map_err(Rejection::IoError)?;
        self.capture_evidence(scope, tx_id, &actions, None)?;
        let (_approval, view) = self
            .approval_mgr
            .create_approval(tx_id, scope_id, actions, 300);
        Ok(view)
    }

    /// Puts one trashed item back at its original place. The caller has already shown the user a
    /// confirmation for this exact item. Nothing is ever replaced, and the action is journaled.
    pub fn restore_trashed(
        &self,
        scope: &AuthorizedRoot,
        scope_id: i64,
        tx_id: i64,
        step_id: i64,
    ) -> Result<ExecutionReport, Rejection> {
        #[cfg(not(unix))]
        {
            let _ = (scope, scope_id, tx_id, step_id);
            Err(Rejection::InvalidAction(
                "Restore is unavailable on this platform".into(),
            ))
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let started = std::time::Instant::now();
            let _operation = self
                .operation
                .lock()
                .map_err(|_| Rejection::IoError("Safety state unavailable".into()))?;
            self.journal
                .check_binding(tx_id, scope, None)
                .map_err(Rejection::InvalidAction)?;
            let detail = self
                .journal
                .get_transaction_detail(tx_id)
                .map_err(Rejection::IoError)?
                .ok_or(Rejection::StaleApproval)?;
            let step = detail
                .steps
                .iter()
                .find(|s| s.id == step_id)
                .ok_or(Rejection::StaleApproval)?;
            if !matches!(step.action_type.as_str(), "trash" | "trash_dir")
                || step.state != "verified"
            {
                return Err(Rejection::InvalidAction(
                    "Only an item that was moved to the Trash by Tidy can be put back".into(),
                ));
            }
            let (before, _, trash) = self.journal.evidence(step_id).map_err(Rejection::IoError)?;
            let trash = PathBuf::from(trash.ok_or_else(|| {
                Rejection::InvalidAction("No Trash location was recorded for this item".into())
            })?);
            if !trash
                .components()
                .any(|c| matches!(c.as_os_str().to_str(), Some(".Trash" | ".Trashes")))
            {
                return Err(Rejection::InvalidAction(
                    "Recorded location is not in a Trash folder".into(),
                ));
            }
            let meta = std::fs::symlink_metadata(&trash).map_err(|_| {
                Rejection::ChangedSource(format!(
                    "{} is no longer in the Trash (already restored or emptied)",
                    trash.display()
                ))
            })?;
            let parts: Vec<&str> = before.split(':').collect();
            let (dev, ino) = if step.action_type == "trash_dir" {
                (parts.get(1), parts.get(2))
            } else {
                (parts.first(), parts.get(1))
            };
            if dev.map(|d| d.to_string()) != Some(meta.dev().to_string())
                || ino.map(|i| i.to_string()) != Some(meta.ino().to_string())
            {
                return Err(Rejection::ChangedSource(
                    "The item in the Trash is not the one Tidy moved".into(),
                ));
            }
            let relative = PathBuf::from(&step.source_relative);
            let destination = scope.path().join(&relative);
            if relative.as_os_str().is_empty()
                || relative.components().any(|c| {
                    !matches!(c, std::path::Component::Normal(_)) || c.as_os_str() == ".git"
                })
                || !destination.starts_with(scope.path())
            {
                return Err(Rejection::OutsideScope);
            }
            if std::fs::symlink_metadata(&destination).is_ok() {
                return Err(Rejection::Collision(destination.display().to_string()));
            }
            if let Some(parent) = destination.parent() {
                let nearest = parent
                    .ancestors()
                    .find(|p| std::fs::symlink_metadata(p).is_ok())
                    .ok_or(Rejection::OutsideScope)?;
                scope
                    .validate(nearest)
                    .map_err(|_| Rejection::OutsideScope)?;
                std::fs::create_dir_all(parent).map_err(|e| Rejection::IoError(e.to_string()))?;
            }
            let action = ValidatedAction::Restore {
                source: trash.clone(),
                destination: destination.clone(),
                relative_source: relative,
                original_size: step.original_size,
            };
            let uuid = Uuid::new_v4().to_string();
            let rationale = format!("Put “{}” back from the Trash", step.source_relative);
            let new_tx = self
                .journal
                .record_prepared(&uuid, scope_id, &rationale, &[action])
                .map_err(Rejection::IoError)?;
            self.journal
                .bind(new_tx, scope, None)
                .map_err(Rejection::IoError)?;
            let new_step = self
                .journal
                .get_transaction_detail(new_tx)
                .map_err(Rejection::IoError)?
                .and_then(|d| d.steps.first().map(|s| s.id))
                .ok_or(Rejection::StaleApproval)?;
            self.journal
                .record_evidence(new_step, &before, None, None)
                .map_err(Rejection::IoError)?;
            for state in [JournalState::Approved, JournalState::Applying] {
                self.journal
                    .transition_state(new_tx, state)
                    .map_err(Rejection::IoError)?;
            }
            // A read-only folder cannot be renamed out of the Trash to a new parent; make it
            // writable for the move and put its original mode back afterwards.
            let trash_mode = {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode()
            };
            let made_writable = meta.is_dir() && trash_mode & 0o200 == 0;
            if made_writable {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    &trash,
                    std::fs::Permissions::from_mode(trash_mode | 0o200),
                )
                .map_err(|e| {
                    Rejection::PermissionDenied(format!(
                        "Could not make the folder writable to put it back: {e}"
                    ))
                })?;
            }
            let moved = tidy_platform::handles::rename_absolute_no_replace(&trash, &destination);
            if made_writable {
                use std::os::unix::fs::PermissionsExt;
                let target = if moved.is_ok() { &destination } else { &trash };
                let _ =
                    std::fs::set_permissions(target, std::fs::Permissions::from_mode(trash_mode));
            }
            if let Err(e) = moved {
                let _ = self.journal.record_step_result(
                    new_step,
                    "needs_recovery",
                    Some(&e.to_string()),
                );
                let _ = self
                    .journal
                    .transition_state(new_tx, JournalState::NeedsRecovery);
                return Err(Rejection::IoError(format!("Could not put it back: {e}")));
            }
            let after = std::fs::symlink_metadata(&destination)
                .map_err(|e| Rejection::IoError(e.to_string()))?;
            if after.ino() != meta.ino() || after.dev() != meta.dev() {
                return Err(Rejection::IoError(
                    "Restored item changed; inspect it manually".into(),
                ));
            }
            self.journal
                .record_step_result(new_step, "verified", None)
                .map_err(Rejection::IoError)?;
            self.journal
                .record_step_result(step_id, "restored", None)
                .map_err(Rejection::IoError)?;
            for state in [JournalState::Applied, JournalState::Verified] {
                self.journal
                    .transition_state(new_tx, state)
                    .map_err(Rejection::IoError)?;
            }
            Ok(ExecutionReport {
                transaction_id: new_tx,
                tx_uuid: uuid,
                actions_applied: 1,
                verified: true,
                duration_ms: started.elapsed().as_millis() as u64,
                rationale,
            })
        }
    }

    /// Consumes the one-use approval token, marks the transaction 'approved' -> 'applying',
    /// executes each action safely, verifies the outcome, and marks 'applied' -> 'verified'.
    pub fn execute_approved_plan(
        &self,
        scope: &AuthorizedRoot,
        token: &str,
    ) -> Result<ExecutionReport, Rejection> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| Rejection::IoError("Safety state unavailable".into()))?;
        // 1. Consume token (prevents replay)
        let approval = self.approval_mgr.consume_approval(token)?;

        let detail = self
            .journal
            .get_transaction_detail(approval.tx_id)
            .map_err(Rejection::IoError)?
            .ok_or_else(|| {
                Rejection::InvalidAction(format!("Transaction #{} not found", approval.tx_id))
            })?;

        self.journal
            .check_binding(approval.tx_id, scope, None)
            .map_err(Rejection::InvalidAction)?;
        // 2. Mark approved
        self.journal
            .transition_state(approval.tx_id, JournalState::Approved)
            .map_err(Rejection::IoError)?;

        // 3. Execute with durability and verification
        execute_transaction(
            scope,
            approval.tx_id,
            &detail.summary.tx_uuid,
            &detail.summary.rationale,
            &approval.actions,
            &self.journal,
        )
    }

    /// Generates an undo plan for an already completed transaction, issuing a one-use undo approval token.
    pub fn request_undo_approval(
        &self,
        scope: &AuthorizedRoot,
        tx_id: i64,
    ) -> Result<ApprovalView, Rejection> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| Rejection::IoError("Safety state unavailable".into()))?;
        self.journal
            .check_binding(tx_id, scope, None)
            .map_err(Rejection::InvalidAction)?;
        let (rationale, undo_actions) = prepare_undo(scope, &self.journal, tx_id)?;

        if undo_actions.is_empty() {
            return Err(Rejection::InvalidAction(
                "No reversible actions found in this transaction".into(),
            ));
        }

        let detail = self
            .journal
            .get_transaction_detail(tx_id)
            .map_err(Rejection::IoError)?
            .ok_or_else(|| Rejection::InvalidAction(format!("Transaction #{tx_id} not found")))?;

        let undo_uuid = format!("undo-{}", Uuid::new_v4());
        let undo_tx_id = self
            .journal
            .record_prepared(
                &undo_uuid,
                detail.summary.scope_id,
                &rationale,
                &undo_actions,
            )
            .map_err(Rejection::IoError)?;

        self.capture_evidence(scope, undo_tx_id, &undo_actions, Some(tx_id))?;
        let (_approval, view) = self.approval_mgr.create_approval(
            undo_tx_id,
            detail.summary.scope_id,
            undo_actions,
            300,
        );

        Ok(view)
    }

    /// Consumes the undo approval token and executes the inverse actions, marking the original transaction 'undone'.
    pub fn execute_approved_undo(
        &self,
        scope: &AuthorizedRoot,
        original_tx_id: i64,
        undo_token: &str,
    ) -> Result<ExecutionReport, Rejection> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| Rejection::IoError("Safety state unavailable".into()))?;
        let approval = self.approval_mgr.consume_approval(undo_token)?;
        self.journal
            .check_binding(approval.tx_id, scope, Some(original_tx_id))
            .map_err(Rejection::InvalidAction)?;
        self.journal
            .check_binding(original_tx_id, scope, None)
            .map_err(Rejection::InvalidAction)?;

        let detail = self
            .journal
            .get_transaction_detail(approval.tx_id)
            .map_err(Rejection::IoError)?
            .ok_or_else(|| {
                Rejection::InvalidAction(format!("Undo transaction #{} not found", approval.tx_id))
            })?;

        let report = execute_transaction(
            scope,
            approval.tx_id,
            &detail.summary.tx_uuid,
            &detail.summary.rationale,
            &approval.actions,
            &self.journal,
        )?;

        let original = self
            .journal
            .get_transaction_detail(original_tx_id)
            .map_err(Rejection::IoError)?
            .ok_or(Rejection::StaleApproval)?;
        let undoable = original
            .steps
            .iter()
            .filter(|s| s.action_type != "create_dir")
            .count();
        let final_state = if report.actions_applied == undoable {
            JournalState::Undone
        } else {
            JournalState::NeedsRecovery
        };
        // An interrupted batch may still contain unknown steps requiring manual inspection.
        self.journal
            .transition_state(original_tx_id, final_state)
            .map_err(Rejection::IoError)?;

        Ok(report)
    }

    fn capture_evidence(
        &self,
        scope: &AuthorizedRoot,
        tx: i64,
        actions: &[ValidatedAction],
        undo_of: Option<i64>,
    ) -> Result<(), Rejection> {
        self.journal
            .bind(tx, scope, undo_of)
            .map_err(Rejection::IoError)?;
        let detail = self
            .journal
            .get_transaction_detail(tx)
            .map_err(Rejection::IoError)?
            .ok_or(Rejection::StaleApproval)?;
        for (step, action) in detail.steps.iter().zip(actions) {
            let stamp = validator::action_fingerprint(scope, action)?;
            self.journal
                .record_evidence(step.id, &stamp, None, None)
                .map_err(Rejection::IoError)?;
        }
        Ok(())
    }
    pub fn list_history(&self, scope_id: Option<i64>) -> Result<Vec<TransactionSummary>, String> {
        self.journal.list_transactions(scope_id)
    }

    pub fn get_detail(&self, tx_id: i64) -> Result<Option<TransactionDetail>, String> {
        self.journal.get_transaction_detail(tx_id)
    }

    pub fn startup_recovery(&self) -> Result<Vec<i64>, String> {
        self.journal.check_startup_recovery()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::{self, File},
        io::Write,
    };

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            #[cfg(target_os = "macos")]
            let base = PathBuf::from("/private/tmp");
            #[cfg(not(target_os = "macos"))]
            let base = std::env::temp_dir();

            let path = base.join(format!("tidy_safety_test_{name}_{}", Uuid::new_v4()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self {
                path: fs::canonicalize(path).unwrap(),
            }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn copy_retains_source_and_collision_is_refused() {
        let tmp = TestDir::new("copy");
        fs::write(tmp.path.join("a.txt"), b"original").unwrap();
        let root = AuthorizedRoot::authorize(&tmp.path).unwrap();
        let engine = SafetyEngine::new_in_memory().unwrap();
        let files = HashMap::from([(FileId(1), PathBuf::from("a.txt"))]);
        let action = ProposedAction::Copy {
            source: FileId(1),
            destination_relative: PathBuf::from("Backup/a.txt"),
        };
        let approval = engine
            .request_plan_approval(
                &root,
                1,
                "Copy retains original",
                std::slice::from_ref(&action),
                &files,
            )
            .unwrap();
        let report = engine
            .execute_approved_plan(&root, &approval.token)
            .unwrap();
        assert_eq!(fs::read(tmp.path.join("a.txt")).unwrap(), b"original");
        assert_eq!(
            fs::read(tmp.path.join("Backup/a.txt")).unwrap(),
            b"original"
        );
        assert!(
            engine
                .request_plan_approval(&root, 1, "Refuse collision", &[action], &files)
                .is_err()
        );
        let undo = engine
            .request_undo_approval(&root, report.transaction_id)
            .unwrap();
        assert!(
            matches!(&undo.actions[0],ValidatedAction::Trash{relative_source,..} if relative_source==Path::new("Backup/a.txt"))
        );
        // Native Trash execution is a separate user-run check.
    }
    #[cfg(unix)]
    #[test]
    fn folder_trash_reviews_whole_tree_and_refuses_root_files_and_overlaps() {
        let tmp = TestDir::new("folder_trash");
        fs::create_dir_all(tmp.path.join("Game/saves/deep")).unwrap();
        fs::write(tmp.path.join("Game/a.bin"), vec![0u8; 100]).unwrap();
        fs::write(tmp.path.join("Game/saves/deep/b.bin"), vec![0u8; 28]).unwrap();
        fs::create_dir_all(tmp.path.join("Repo/.git")).unwrap();
        fs::write(tmp.path.join("Repo/x.txt"), b"x").unwrap();
        let root = AuthorizedRoot::authorize(&tmp.path).unwrap();
        let engine = SafetyEngine::new_in_memory().unwrap();
        let view = engine
            .request_folder_trash_approval(&root, 1, "test", &[PathBuf::from("Game")])
            .unwrap();
        assert!(matches!(
            &view.actions[0],
            ValidatedAction::TrashDir {
                files: 2,
                dirs: 3,
                original_size: 128,
                ..
            }
        ));
        assert!(tmp.path.join("Game/a.bin").exists());
        // The folder changes after review: the approval must not execute.
        fs::write(tmp.path.join("Game/new.bin"), b"late").unwrap();
        assert!(engine.execute_approved_plan(&root, &view.token).is_err());
        assert!(tmp.path.join("Game/a.bin").exists());
        // A repository folder is trashed whole (recoverable); its .git is never opened.
        assert!(
            engine
                .request_folder_trash_approval(&root, 1, "repo", &[PathBuf::from("Repo")])
                .is_ok()
        );
        assert!(
            engine
                .request_folder_trash_approval(&root, 1, "root", &[PathBuf::new()])
                .is_err()
        );
        assert!(
            engine
                .request_folder_trash_approval(&root, 1, "file", &[PathBuf::from("Game/a.bin")])
                .is_err()
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn trashed_folder_can_be_put_back_once_and_never_over_something() {
        let tmp = TestDir::new("restore");
        fs::create_dir_all(tmp.path.join("Keep/inner")).unwrap();
        fs::write(tmp.path.join("Keep/inner/a.txt"), b"a").unwrap();
        let root = AuthorizedRoot::authorize(&tmp.path).unwrap();
        let engine = SafetyEngine::new_in_memory().unwrap();
        let view = engine
            .request_folder_trash_approval(&root, 1, "trash", &[PathBuf::from("Keep")])
            .unwrap();
        let report = engine.execute_approved_plan(&root, &view.token).unwrap();
        assert!(!tmp.path.join("Keep").exists());
        let detail = engine.get_detail(report.transaction_id).unwrap().unwrap();
        let step = detail.steps[0].id;
        // Something new now occupies the original place: refuse to overwrite.
        fs::create_dir(tmp.path.join("Keep")).unwrap();
        assert!(
            engine
                .restore_trashed(&root, 1, report.transaction_id, step)
                .is_err()
        );
        fs::remove_dir(tmp.path.join("Keep")).unwrap();
        engine
            .restore_trashed(&root, 1, report.transaction_id, step)
            .unwrap();
        assert_eq!(fs::read(tmp.path.join("Keep/inner/a.txt")).unwrap(), b"a");
        // A second restore has nothing to restore.
        assert!(
            engine
                .restore_trashed(&root, 1, report.transaction_id, step)
                .is_err()
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn read_only_folders_are_made_writable_for_the_move_and_restored_after() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TestDir::new("readonly");
        fs::create_dir_all(tmp.path.join("Locked/inner")).unwrap();
        fs::write(tmp.path.join("Locked/inner/a.txt"), b"a").unwrap();
        fs::create_dir_all(tmp.path.join("Other")).unwrap();
        fs::set_permissions(tmp.path.join("Locked"), fs::Permissions::from_mode(0o555)).unwrap();
        let root = AuthorizedRoot::authorize(&tmp.path).unwrap();
        let engine = SafetyEngine::new_in_memory().unwrap();
        // Moving it to another parent works and puts the read-only mode back.
        let plan = [ProposedAction::MoveFolder {
            source: "Locked".into(),
            destination_relative: "Other/Locked".into(),
        }];
        let view = engine
            .request_plan_approval(&root, 1, "move", &plan, &HashMap::new())
            .unwrap();
        assert!(matches!(
            &view.actions[0],
            ValidatedAction::MoveDir {
                read_only: true,
                ..
            }
        ));
        engine.execute_approved_plan(&root, &view.token).unwrap();
        let mode = |p: &str| fs::metadata(tmp.path.join(p)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode("Other/Locked"), 0o555);
        // Trashing it works too, and Put back restores the mode.
        let view = engine
            .request_folder_trash_approval(&root, 1, "trash", &[PathBuf::from("Other/Locked")])
            .unwrap();
        assert!(matches!(
            &view.actions[0],
            ValidatedAction::TrashDir {
                read_only: true,
                ..
            }
        ));
        let report = engine.execute_approved_plan(&root, &view.token).unwrap();
        assert!(!tmp.path.join("Other/Locked").exists());
        let step = engine
            .get_detail(report.transaction_id)
            .unwrap()
            .unwrap()
            .steps[0]
            .id;
        engine
            .restore_trashed(&root, 1, report.transaction_id, step)
            .unwrap();
        assert_eq!(mode("Other/Locked"), 0o555, "original permissions are kept");
        fs::set_permissions(
            tmp.path.join("Other/Locked"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn create_and_rename_folders_then_undo_the_rename() {
        let tmp = TestDir::new("dirs");
        fs::create_dir_all(tmp.path.join("Old/inner")).unwrap();
        fs::write(tmp.path.join("Old/inner/a.txt"), b"a").unwrap();
        let root = AuthorizedRoot::authorize(&tmp.path).unwrap();
        let engine = SafetyEngine::new_in_memory().unwrap();
        let files = HashMap::new();
        let plan = [
            ProposedAction::CreateFolder {
                path: PathBuf::from("Archive/2026"),
            },
            ProposedAction::MoveFolder {
                source: PathBuf::from("Old"),
                destination_relative: PathBuf::from("Archive/2026/New"),
            },
        ];
        let view = engine
            .request_plan_approval(&root, 1, "restructure", &plan, &files)
            .unwrap();
        let report = engine.execute_approved_plan(&root, &view.token).unwrap();
        assert_eq!(report.actions_applied, 2);
        assert!(!tmp.path.join("Old").exists());
        assert_eq!(
            fs::read(tmp.path.join("Archive/2026/New/inner/a.txt")).unwrap(),
            b"a"
        );
        // Colliding and self-nested moves are refused.
        assert!(
            engine
                .request_plan_approval(
                    &root,
                    1,
                    "into itself",
                    &[ProposedAction::MoveFolder {
                        source: PathBuf::from("Archive"),
                        destination_relative: PathBuf::from("Archive/x"),
                    }],
                    &files
                )
                .is_err()
        );
        let undo = engine
            .request_undo_approval(&root, report.transaction_id)
            .unwrap();
        assert_eq!(undo.actions.len(), 1);
        assert!(matches!(&undo.actions[0], ValidatedAction::MoveDir { .. }));
        engine
            .execute_approved_undo(&root, report.transaction_id, &undo.token)
            .unwrap();
        assert_eq!(fs::read(tmp.path.join("Old/inner/a.txt")).unwrap(), b"a");
    }
    #[cfg(unix)]
    #[test]
    fn permissions_journal_and_undo_restore_original_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TestDir::new("permissions");
        let path = tmp.path.join("a.txt");
        fs::write(&path, b"text").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let root = AuthorizedRoot::authorize(&tmp.path).unwrap();
        let engine = SafetyEngine::new_in_memory().unwrap();
        let files = HashMap::from([(FileId(1), PathBuf::from("a.txt"))]);
        let approval = engine
            .request_plan_approval(
                &root,
                1,
                "Make private",
                &[ProposedAction::Permissions {
                    source: FileId(1),
                    mode: 0o600,
                }],
                &files,
            )
            .unwrap();
        let report = engine
            .execute_approved_plan(&root, &approval.token)
            .unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let undo = engine
            .request_undo_approval(&root, report.transaction_id)
            .unwrap();
        engine
            .execute_approved_undo(&root, report.transaction_id, &undo.token)
            .unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(fs::read(&path).unwrap(), b"text");
        assert!(
            engine
                .request_plan_approval(
                    &root,
                    1,
                    "Refuse lockout",
                    &[ProposedAction::Permissions {
                        source: FileId(1),
                        mode: 0
                    }],
                    &files
                )
                .is_err()
        );
    }
    #[test]
    fn full_safety_lifecycle_move_and_undo() {
        let tmp = TestDir::new("lifecycle");
        let root = AuthorizedRoot::authorize(&tmp.path).unwrap();

        // Create a test file
        let file_path = tmp.path.join("report.txt");
        let mut f = File::create(&file_path).unwrap();
        writeln!(f, "Important Report Data").unwrap();
        drop(f);

        let engine = SafetyEngine::new_in_memory().unwrap();

        let mut files_map = HashMap::new();
        files_map.insert(FileId(1), PathBuf::from("report.txt"));

        let plan = vec![ProposedAction::Move {
            source: FileId(1),
            destination_relative: PathBuf::from("Documents/report.txt"),
        }];

        // 1. Request approval -> generates token
        let view = engine
            .request_plan_approval(&root, 1, "Categorize report", &plan, &files_map)
            .unwrap();

        assert_eq!(view.actions_count, 1);
        assert!(!view.token.is_empty());

        // 2. Execute with approval token
        let report = engine.execute_approved_plan(&root, &view.token).unwrap();
        assert_eq!(report.actions_applied, 1);
        assert!(report.verified);

        // Verification: original is gone, destination exists
        assert!(!file_path.exists());
        let dest_path = tmp.path.join("Documents/report.txt");
        assert!(dest_path.exists());

        // 3. Replay attack attempt fails!
        let replay = engine
            .execute_approved_plan(&root, &view.token)
            .unwrap_err();
        assert_eq!(replay, Rejection::StaleApproval);

        // 4. Request Undo approval
        let undo_view = engine
            .request_undo_approval(&root, report.transaction_id)
            .unwrap();
        assert_eq!(undo_view.actions_count, 1);

        // 5. Execute Undo
        let undo_report = engine
            .execute_approved_undo(&root, report.transaction_id, &undo_view.token)
            .unwrap();
        assert_eq!(undo_report.actions_applied, 1);

        // Verification after Undo: restored to original!
        assert!(file_path.exists());
        assert!(!dest_path.exists());

        // Journal history check
        let history = engine.list_history(Some(1)).unwrap();
        assert_eq!(history.len(), 2); // original + undo transaction
        assert_eq!(history[1].state, JournalState::Undone);
    }

    #[test]
    fn collision_refuses_to_overwrite_existing_file() {
        let tmp = TestDir::new("collision");
        let root = AuthorizedRoot::authorize(&tmp.path).unwrap();

        let src = tmp.path.join("src.txt");
        let dst = tmp.path.join("dst.txt");
        File::create(&src).unwrap();
        File::create(&dst).unwrap(); // collision!

        let engine = SafetyEngine::new_in_memory().unwrap();

        let mut files_map = HashMap::new();
        files_map.insert(FileId(1), PathBuf::from("src.txt"));

        let plan = vec![ProposedAction::Move {
            source: FileId(1),
            destination_relative: PathBuf::from("dst.txt"),
        }];

        let err = engine
            .request_plan_approval(&root, 1, "Collision test", &plan, &files_map)
            .unwrap_err();

        match err {
            Rejection::Collision(path) => {
                assert!(path.contains("dst.txt"));
            }
            other => panic!("Expected collision error, got {other:?}"),
        }
    }
    fn approved_file(tmp: &TestDir, engine: &SafetyEngine) -> (AuthorizedRoot, ApprovalView) {
        fs::write(tmp.path.join("source.txt"), b"original").unwrap();
        let root = AuthorizedRoot::authorize(&tmp.path).unwrap();
        let files = HashMap::from([(FileId(1), PathBuf::from("source.txt"))]);
        let approval = engine
            .request_plan_approval(
                &root,
                1,
                "test",
                &[ProposedAction::Move {
                    source: FileId(1),
                    destination_relative: "Sorted/source.txt".into(),
                }],
                &files,
            )
            .unwrap();
        (root, approval)
    }
    #[test]
    fn rejects_changed_file_after_approval_without_moving_it() {
        let tmp = TestDir::new("changed_after_approval");
        let engine = SafetyEngine::new_in_memory().unwrap();
        let (root, approval) = approved_file(&tmp, &engine);
        fs::write(tmp.path.join("source.txt"), b"changed contents").unwrap();
        assert!(
            engine
                .execute_approved_plan(&root, &approval.token)
                .is_err()
        );
        assert_eq!(
            fs::read(tmp.path.join("source.txt")).unwrap(),
            b"changed contents"
        );
        assert!(!tmp.path.join("Sorted/source.txt").exists());
    }
    #[test]
    fn rejects_approval_for_different_root() {
        let tmp = TestDir::new("root_a");
        let other = TestDir::new("root_b");
        let engine = SafetyEngine::new_in_memory().unwrap();
        let (_, approval) = approved_file(&tmp, &engine);
        assert!(
            engine
                .execute_approved_plan(
                    &AuthorizedRoot::authorize(&other.path).unwrap(),
                    &approval.token
                )
                .is_err()
        );
        assert!(tmp.path.join("source.txt").exists());
    }
    #[cfg(unix)]
    #[test]
    fn destination_symlink_inserted_after_approval_never_redirects_move() {
        let tmp = TestDir::new("symlink_swap");
        let outside = TestDir::new("outside");
        let engine = SafetyEngine::new_in_memory().unwrap();
        let (root, approval) = approved_file(&tmp, &engine);
        std::os::unix::fs::symlink(&outside.path, tmp.path.join("Sorted")).unwrap();
        assert!(
            engine
                .execute_approved_plan(&root, &approval.token)
                .is_err()
        );
        assert!(tmp.path.join("source.txt").exists());
        assert!(!outside.path.join("source.txt").exists());
    }
    #[test]
    fn changed_moved_file_cannot_be_undone_silently() {
        let tmp = TestDir::new("changed_undo");
        let engine = SafetyEngine::new_in_memory().unwrap();
        let (root, approval) = approved_file(&tmp, &engine);
        let report = engine
            .execute_approved_plan(&root, &approval.token)
            .unwrap();
        fs::write(tmp.path.join("Sorted/source.txt"), b"new work").unwrap();
        assert!(
            engine
                .request_undo_approval(&root, report.transaction_id)
                .is_err()
        );
        assert_eq!(
            fs::read(tmp.path.join("Sorted/source.txt")).unwrap(),
            b"new work"
        );
    }
}
