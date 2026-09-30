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
        let final_state = if report.actions_applied == original.steps.len() {
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
    fn folder_trash_reviews_whole_tree_and_refuses_git_and_root() {
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
        assert!(
            engine
                .request_folder_trash_approval(&root, 1, "git", &[PathBuf::from("Repo")])
                .is_err()
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
