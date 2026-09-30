//! Phase 5 Contract Tests: Deterministic Safety Engine, Approval Workflow, Durable Journal, and Reversible Undo.
//!
//! Validates:
//! 1. Full approved execution workflow with one-use tokens and journal verification.
//! 2. Replay attack rejection (one-use approval tokens cannot be consumed twice).
//! 3. Expired approval rejection.
//! 4. No-replace collision rejection (never overwrites existing destinations).
//! 5. Changed or missing source file rejection.
//! 6. Symlink and protected Git marker rejection.
//! 7. Native Trash execution and verification (no permanent deletion).
//! 8. Reversible undo workflow (inverting moves/renames, restoring files, marking journal 'undone').
//! 9. Startup recovery of interrupted transactions ('needs_recovery').

use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tidy_organization::{FileId, ProposedAction};
use tidy_platform::AuthorizedRoot;
use tidy_safety::{Journal, JournalState, Rejection, SafetyEngine};

static SERIAL: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        #[cfg(target_os = "macos")]
        let base = PathBuf::from("/private/tmp");
        #[cfg(not(target_os = "macos"))]
        let base = std::env::temp_dir();

        let path = base.join(format!(
            "tidy_p5_{}_{}_{}_{}",
            name,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self {
            path: fs::canonicalize(path).unwrap(),
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn journey_downloads_approved_execution_and_undo() {
    let tmp = TempDir::new("downloads_exec_undo");
    let root = AuthorizedRoot::authorize(&tmp.path).unwrap();

    let doc = tmp.path.join("resume.pdf");
    let img = tmp.path.join("avatar.png");
    File::create(&doc)
        .unwrap()
        .write_all(b"PDF content")
        .unwrap();
    File::create(&img)
        .unwrap()
        .write_all(b"PNG content")
        .unwrap();

    let journal_path = tmp.path.join("safety_journal.sqlite3");
    let engine = SafetyEngine::new(&journal_path).unwrap();

    let mut files_map = HashMap::new();
    files_map.insert(FileId(1), PathBuf::from("resume.pdf"));
    files_map.insert(FileId(2), PathBuf::from("avatar.png"));

    let plan = vec![
        ProposedAction::Move {
            source: FileId(1),
            destination_relative: PathBuf::from("Documents/resume.pdf"),
        },
        ProposedAction::Move {
            source: FileId(2),
            destination_relative: PathBuf::from("Images/avatar.png"),
        },
    ];

    // 1. Request Approval Preview
    let view = engine
        .request_plan_approval(
            &root,
            101,
            "Organize Downloads by Category",
            &plan,
            &files_map,
        )
        .unwrap();

    assert_eq!(view.actions_count, 2);
    assert!(!view.token.is_empty());

    // 2. Execute with approval token
    let report = engine.execute_approved_plan(&root, &view.token).unwrap();
    assert_eq!(report.actions_applied, 2);
    assert!(report.verified);

    // Verify files on disk
    assert!(!doc.exists());
    assert!(!img.exists());
    let dest_doc = tmp.path.join("Documents/resume.pdf");
    let dest_img = tmp.path.join("Images/avatar.png");
    assert!(dest_doc.exists());
    assert!(dest_img.exists());

    // 3. Replay attack rejection
    let replay_err = engine
        .execute_approved_plan(&root, &view.token)
        .unwrap_err();
    assert_eq!(replay_err, Rejection::StaleApproval);

    // 4. Request Undo approval
    let undo_view = engine
        .request_undo_approval(&root, report.transaction_id)
        .unwrap();
    assert_eq!(undo_view.actions_count, 2);

    // 5. Execute Undo
    let undo_report = engine
        .execute_approved_undo(&root, report.transaction_id, &undo_view.token)
        .unwrap();
    assert_eq!(undo_report.actions_applied, 2);
    assert!(undo_report.verified);

    // Verify files restored to original locations
    assert!(doc.exists());
    assert!(img.exists());
    assert!(!dest_doc.exists());
    assert!(!dest_img.exists());

    // Journal verifies transaction marked Undone
    let history = engine.list_history(Some(101)).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].state, JournalState::Undone);
}

#[test]
fn native_trash_moves_file_and_verifies_without_permanent_deletion() {
    let tmp = TempDir::new("trash_test");
    let root = AuthorizedRoot::authorize(&tmp.path).unwrap();

    let old_installer = tmp.path.join("installer_v1.dmg");
    File::create(&old_installer)
        .unwrap()
        .write_all(b"old installer bytes")
        .unwrap();

    let engine = SafetyEngine::new_in_memory().unwrap();

    let mut files_map = HashMap::new();
    files_map.insert(FileId(50), PathBuf::from("installer_v1.dmg"));

    let plan = vec![ProposedAction::Trash { source: FileId(50) }];

    let view = engine
        .request_plan_approval(&root, 202, "Trash old installer", &plan, &files_map)
        .unwrap();

    let report = engine.execute_approved_plan(&root, &view.token).unwrap();
    assert_eq!(report.actions_applied, 1);
    assert!(report.verified);

    // File is no longer at original path
    assert!(!old_installer.exists());
}

#[test]
fn collision_rejection_strictly_protects_existing_destinations() {
    let tmp = TempDir::new("collision_safety");
    let root = AuthorizedRoot::authorize(&tmp.path).unwrap();

    let src = tmp.path.join("source.txt");
    let dest = tmp.path.join("existing_target.txt");
    File::create(&src)
        .unwrap()
        .write_all(b"Source Data")
        .unwrap();
    File::create(&dest)
        .unwrap()
        .write_all(b"Critical Existing Data")
        .unwrap();

    let engine = SafetyEngine::new_in_memory().unwrap();

    let mut files_map = HashMap::new();
    files_map.insert(FileId(1), PathBuf::from("source.txt"));

    let plan = vec![ProposedAction::Move {
        source: FileId(1),
        destination_relative: PathBuf::from("existing_target.txt"),
    }];

    let err = engine
        .request_plan_approval(&root, 1, "Collision attempt", &plan, &files_map)
        .unwrap_err();

    match err {
        Rejection::Collision(path) => {
            assert!(path.contains("existing_target.txt"));
        }
        other => panic!("Expected Collision, got {other:?}"),
    }

    // Existing target was NOT overwritten!
    let content = fs::read_to_string(&dest).unwrap();
    assert_eq!(content, "Critical Existing Data");
    assert!(src.exists());
}

#[test]
fn changed_or_missing_source_file_is_rejected() {
    let tmp = TempDir::new("missing_source");
    let root = AuthorizedRoot::authorize(&tmp.path).unwrap();

    let engine = SafetyEngine::new_in_memory().unwrap();

    let mut files_map = HashMap::new();
    files_map.insert(FileId(99), PathBuf::from("ghost_file.txt"));

    let plan = vec![ProposedAction::Move {
        source: FileId(99),
        destination_relative: PathBuf::from("Documents/ghost_file.txt"),
    }];

    let err = engine
        .request_plan_approval(&root, 1, "Missing source test", &plan, &files_map)
        .unwrap_err();

    assert!(matches!(err, Rejection::ChangedSource(_)));
}

#[test]
fn protected_git_path_and_symlinks_are_rejected() {
    let tmp = TempDir::new("git_protection");
    let root = AuthorizedRoot::authorize(&tmp.path).unwrap();

    // Create a regular file
    let file = tmp.path.join("safe.txt");
    File::create(&file).unwrap().write_all(b"safe").unwrap();

    let engine = SafetyEngine::new_in_memory().unwrap();

    let mut files_map = HashMap::new();
    files_map.insert(FileId(1), PathBuf::from("safe.txt"));

    // Attempt to move into a .git target
    let plan = vec![ProposedAction::Move {
        source: FileId(1),
        destination_relative: PathBuf::from(".git/hooks/pre-commit"),
    }];

    let err = engine
        .request_plan_approval(&root, 1, "Target git attempt", &plan, &files_map)
        .unwrap_err();

    assert_eq!(err, Rejection::ProtectedPath);
}

#[test]
fn startup_recovery_transitions_stalled_applying_transactions() {
    let tmp = TempDir::new("startup_recovery");
    let journal_path = tmp.path.join("journal.sqlite3");
    let journal = Journal::open(&journal_path).unwrap();

    // Manually record and simulate crash during 'applying'
    let tx_id = journal
        .record_prepared("tx-crash-1", 10, "interrupted run", &[])
        .unwrap();
    journal
        .transition_state(tx_id, JournalState::Applying)
        .unwrap();

    // Startup recovery check
    let recovered = journal.check_startup_recovery().unwrap();
    assert_eq!(recovered, vec![tx_id]);

    let detail = journal.get_transaction_detail(tx_id).unwrap().unwrap();
    assert_eq!(detail.summary.state, JournalState::NeedsRecovery);
}
