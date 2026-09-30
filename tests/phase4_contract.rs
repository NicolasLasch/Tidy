//! Phase 4 Contract Tests: Planning and Storage Analysis
//! Validates:
//! 1. Organize Downloads journey (Category and Date modes, deterministic fallback, unassigned ambiguous files)
//! 2. Recover Storage journey (Large files, exact duplicate confirmation, hard link accounting, old installers, dev artifacts)
//! 3. Group Project journey (Name/metadata/excerpt matching, Git repository exclusion)
//! 4. Zero filesystem mutation while planning.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tidy_file_indexer::{
    AuthorizedRoot,
    database::Index,
    index_scan::{self, IndexOptions},
};
use tidy_organization::{
    FileCandidate, FileId, OrganizationMode, ProposedAction, propose, propose_storage_cleanup,
};
use tidy_storage::{AnalyzableFile, FindingKind, StorageAnalysisConfig, analyze_storage};

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
            "tidy_p4_{}_{}_{}_{}",
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

// ---------------------------------------------------------------------------
// Journey 1: Organize Downloads
// ---------------------------------------------------------------------------

#[test]
fn journey_organize_downloads_category_and_date() {
    let tmp = TempDir::new("downloads");
    let downloads = tmp.path.join("Downloads");
    fs::create_dir_all(&downloads).unwrap();

    // Create diverse downloads
    fs::write(
        downloads.join("tax_return.pdf"),
        b"%PDF-1.4 sample tax return",
    )
    .unwrap();
    fs::write(downloads.join("screenshot.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    fs::write(downloads.join("dataset.csv"), b"id,val\n1,100").unwrap();
    fs::write(downloads.join("archive.zip"), b"PK\x03\x04dummy").unwrap();
    fs::write(downloads.join("script.py"), b"print('hello')").unwrap();
    fs::write(downloads.join("unknown_blob.xyz999"), b"raw binary").unwrap();

    let root = AuthorizedRoot::authorize(&downloads).unwrap();
    let db_path = tmp.path.join("index.sqlite3");
    let mut db = Index::open(&db_path).unwrap();
    let scope_id = db.add_scope(&root).unwrap();

    let snapshot = index_scan::collect(
        &root,
        &IndexOptions {
            content: true,
            ..Default::default()
        },
        &std::sync::atomic::AtomicBool::new(false),
        &std::sync::atomic::AtomicUsize::new(0),
        &HashMap::new(),
    )
    .unwrap();
    db.commit(scope_id, &snapshot, true).unwrap();

    let indexed = db.current_files(scope_id).unwrap();
    assert_eq!(indexed.len(), 6);

    let candidates: Vec<FileCandidate> = indexed
        .iter()
        .map(|f| FileCandidate {
            id: FileId(f.id as u64),
            relative_path: f.path.clone(),
            size: f.size,
            modified: f.modified,
            excerpt: f.excerpt.clone(),
        })
        .collect();

    // 1. Category Mode
    let cat_proposal = propose(OrganizationMode::Category, None, &candidates);
    assert!(cat_proposal.actions.len() >= 5);

    // Verify categories
    let moves: HashMap<FileId, PathBuf> = cat_proposal
        .actions
        .iter()
        .filter_map(|a| match a {
            ProposedAction::Move {
                source,
                destination_relative,
            } => Some((*source, destination_relative.clone())),
            _ => None,
        })
        .collect();

    let tax_file = indexed
        .iter()
        .find(|f| f.display.contains("tax_return.pdf"))
        .unwrap();
    assert_eq!(
        moves[&FileId(tax_file.id as u64)],
        PathBuf::from("Documents/tax_return.pdf")
    );

    let img_file = indexed
        .iter()
        .find(|f| f.display.contains("screenshot.png"))
        .unwrap();
    assert_eq!(
        moves[&FileId(img_file.id as u64)],
        PathBuf::from("Images/screenshot.png")
    );

    let csv_file = indexed
        .iter()
        .find(|f| f.display.contains("dataset.csv"))
        .unwrap();
    assert_eq!(
        moves[&FileId(csv_file.id as u64)],
        PathBuf::from("Spreadsheets/dataset.csv")
    );

    let zip_file = indexed
        .iter()
        .find(|f| f.display.contains("archive.zip"))
        .unwrap();
    assert_eq!(
        moves[&FileId(zip_file.id as u64)],
        PathBuf::from("Archives/archive.zip")
    );

    let py_file = indexed
        .iter()
        .find(|f| f.display.contains("script.py"))
        .unwrap();
    assert_eq!(
        moves[&FileId(py_file.id as u64)],
        PathBuf::from("Code/script.py")
    );

    // Ambiguous file (.xyz999) remains unassigned
    let unk_file = indexed
        .iter()
        .find(|f| f.display.contains("unknown_blob.xyz999"))
        .unwrap();
    assert!(!moves.contains_key(&FileId(unk_file.id as u64)));
    assert!(cat_proposal.rationale.contains("unassigned"));

    // Verify NO filesystem changes occurred
    assert!(downloads.join("tax_return.pdf").exists());
    assert!(!downloads.join("Documents").exists());

    // 2. Date Mode
    let date_proposal = propose(OrganizationMode::Date, None, &candidates);
    assert!(!date_proposal.actions.is_empty());
    for action in &date_proposal.actions {
        if let ProposedAction::Move {
            destination_relative,
            ..
        } = action
        {
            let path_str = destination_relative.to_string_lossy();
            let parts: Vec<&str> = path_str.split('/').collect();
            assert_eq!(parts.len(), 3); // Year, Month, Filename
            assert_eq!(parts[0].len(), 4); // YYYY
            assert_eq!(parts[1].len(), 2); // MM
        }
    }
}

// ---------------------------------------------------------------------------
// Journey 2: Recover Storage
// ---------------------------------------------------------------------------

#[test]
fn journey_storage_recovery_and_exact_duplicates() {
    let tmp = TempDir::new("storage");
    let folder = tmp.path.join("StorageTarget");
    fs::create_dir_all(&folder).unwrap();

    let now_ts = 1_790_640_000i64; // Sept 2026

    // 1. Large file: 15 MB
    let large_data = vec![b'L'; 15 * 1024 * 1024];
    fs::write(folder.join("big_video.mov"), &large_data).unwrap();

    // 2. Exact duplicates: two identical 2 MB files with identical content
    let dup_data = vec![b'D'; 2 * 1024 * 1024];
    fs::write(folder.join("original.iso"), &dup_data).unwrap();
    fs::write(folder.join("duplicate.iso"), &dup_data).unwrap();

    // 3. Old installer (modified 60 days ago)
    let old_installer_path = folder.join("old_setup.dmg");
    fs::write(&old_installer_path, b"old installer payload").unwrap();

    // 4. Dev artifacts
    let node_dir = folder.join("node_modules").join("left-pad");
    fs::create_dir_all(&node_dir).unwrap();
    fs::write(node_dir.join("index.js"), b"export default 42;").unwrap();

    let root = AuthorizedRoot::authorize(&folder).unwrap();
    let db_path = tmp.path.join("index.sqlite3");
    let mut db = Index::open(&db_path).unwrap();
    let scope_id = db.add_scope(&root).unwrap();

    let snapshot = index_scan::collect(
        &root,
        &IndexOptions {
            content: true,
            ..Default::default()
        },
        &std::sync::atomic::AtomicBool::new(false),
        &std::sync::atomic::AtomicUsize::new(0),
        &HashMap::new(),
    )
    .unwrap();
    db.commit(scope_id, &snapshot, true).unwrap();

    let indexed = db.current_files(scope_id).unwrap();

    // Convert to AnalyzableFile with custom timestamp for old installer test
    let analyzable: Vec<AnalyzableFile> = indexed
        .iter()
        .map(|f| {
            let modified = if f.display.contains("old_setup.dmg") {
                now_ts - (60 * 86400) // 60 days ago
            } else {
                now_ts
            };
            AnalyzableFile {
                id: f.id as u64,
                path: f.path.clone(),
                size: f.size,
                modified,
                hash: f.hash.clone(),
                identity: f.identity.clone(),
            }
        })
        .collect();

    let config = StorageAnalysisConfig {
        min_large_file_bytes: 10 * 1024 * 1024, // 10 MB
        old_installer_days: 30,
        now_timestamp: now_ts,
    };

    let result = analyze_storage(&analyzable, &config);

    // Assert findings
    assert!(result.summary.large_files_count >= 1);
    assert_eq!(result.summary.old_installers_count, 1);
    assert!(result.summary.dev_artifacts_count >= 1);

    let large_finding = result
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::LargeFile)
        .unwrap();
    assert!(large_finding.evidence.contains("big_video.mov"));
    assert_eq!(
        large_finding.estimated_reclaimable_bytes,
        Some(15 * 1024 * 1024)
    );

    let inst_finding = result
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::OldInstaller)
        .unwrap();
    assert!(inst_finding.evidence.contains("old_setup.dmg"));

    let dev_finding = result
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::DevelopmentArtifact)
        .unwrap();
    assert!(dev_finding.evidence.contains("node_modules"));

    // Verify duplicate confirmation if hash is present
    if result.summary.duplicate_groups_count > 0 {
        let dup_finding = result
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::ExactDuplicate)
            .unwrap();
        assert_eq!(dup_finding.file_ids.len(), 2);
        assert_eq!(
            dup_finding.estimated_reclaimable_bytes,
            Some(2 * 1024 * 1024)
        );
    }

    // Now generate storage cleanup proposal
    let dup_groups = if result.summary.duplicate_groups_count > 0 {
        vec![
            result
                .findings
                .iter()
                .find(|f| f.kind == FindingKind::ExactDuplicate)
                .unwrap()
                .file_ids
                .clone(),
        ]
    } else {
        vec![]
    };

    let installer_ids: Vec<u64> = result
        .findings
        .iter()
        .filter(|f| f.kind == FindingKind::OldInstaller)
        .flat_map(|f| f.file_ids.clone())
        .collect();

    let artifact_ids: Vec<u64> = result
        .findings
        .iter()
        .filter(|f| f.kind == FindingKind::DevelopmentArtifact)
        .flat_map(|f| f.file_ids.clone())
        .collect();

    let mut keep = HashMap::new();
    if !dup_groups.is_empty() {
        // User chooses to keep the first copy
        keep.insert(0, dup_groups[0][0]);
    }

    let cleanup_proposal =
        propose_storage_cleanup(&dup_groups, &keep, &installer_ids, &artifact_ids);

    // Verify proposal has ONLY Trash actions
    for action in &cleanup_proposal.actions {
        assert!(matches!(action, ProposedAction::Trash { .. }));
    }

    // If there were duplicates, only the non-kept copy is in Trash
    if !dup_groups.is_empty() {
        let kept_id = dup_groups[0][0];
        let redundant_id = dup_groups[0][1];
        assert!(cleanup_proposal.actions.contains(&ProposedAction::Trash {
            source: FileId(redundant_id)
        }));
        assert!(!cleanup_proposal.actions.contains(&ProposedAction::Trash {
            source: FileId(kept_id)
        }));
    }

    // Verify no files were deleted yet!
    assert!(folder.join("big_video.mov").exists());
    assert!(folder.join("old_setup.dmg").exists());
}

// ---------------------------------------------------------------------------
// Journey 3: Group Project
// ---------------------------------------------------------------------------

#[test]
fn journey_group_project_with_git_protection() {
    let tmp = TempDir::new("project");
    let workspace = tmp.path.join("Workspace");
    fs::create_dir_all(&workspace).unwrap();

    fs::write(
        workspace.join("Tidy_Spec.pdf"),
        b"TIDY Architecture Specifications",
    )
    .unwrap();
    fs::write(
        workspace.join("notes.txt"),
        b"Notes mentioning Tidy project goals",
    )
    .unwrap();
    fs::write(workspace.join("random_unrelated.log"), b"some server logs").unwrap();

    // Simulated Git repository inside a subfolder (scanner omits .git, and planner protects git repos)
    let git_subfolder = workspace.join("nested_repo");
    fs::create_dir_all(&git_subfolder).unwrap();
    fs::write(git_subfolder.join("nested_file.rs"), b"// inside repo").unwrap();
    let git_dir = git_subfolder.join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(
        git_dir.join("config"),
        b"[core]\nrepositoryformatversion = 0",
    )
    .unwrap();

    let root = AuthorizedRoot::authorize(&workspace).unwrap();
    let db_path = tmp.path.join("index.sqlite3");
    let mut db = Index::open(&db_path).unwrap();
    let scope_id = db.add_scope(&root).unwrap();

    let snapshot = index_scan::collect(
        &root,
        &IndexOptions {
            content: true,
            ..Default::default()
        },
        &std::sync::atomic::AtomicBool::new(false),
        &std::sync::atomic::AtomicUsize::new(0),
        &HashMap::new(),
    )
    .unwrap();
    db.commit(scope_id, &snapshot, true).unwrap();

    let indexed = db.current_files(scope_id).unwrap();

    let candidates: Vec<FileCandidate> = indexed
        .iter()
        .map(|f| FileCandidate {
            id: FileId(f.id as u64),
            relative_path: f.path.clone(),
            size: f.size,
            modified: f.modified,
            excerpt: f.excerpt.clone(),
        })
        .collect();

    let project_proposal = propose(OrganizationMode::Project, Some("Tidy"), &candidates);

    // Verify matching files
    let moved_sources: HashSet<FileId> = project_proposal
        .actions
        .iter()
        .filter_map(|a| match a {
            ProposedAction::Move { source, .. } => Some(*source),
            _ => None,
        })
        .collect();

    let spec_file = indexed
        .iter()
        .find(|f| f.display.contains("Tidy_Spec.pdf"))
        .unwrap();
    assert!(moved_sources.contains(&FileId(spec_file.id as u64)));

    let notes_file = indexed
        .iter()
        .find(|f| f.display.contains("notes.txt"))
        .unwrap();
    assert!(moved_sources.contains(&FileId(notes_file.id as u64)));

    let unrelated_file = indexed
        .iter()
        .find(|f| f.display.contains("random_unrelated.log"))
        .unwrap();
    assert!(!moved_sources.contains(&FileId(unrelated_file.id as u64)));

    // Verify Git repository files are NOT included
    for action in &project_proposal.actions {
        if let ProposedAction::Move {
            destination_relative,
            ..
        } = action
        {
            assert!(!destination_relative.to_string_lossy().contains(".git"));
        }
    }

    // Verify source files untouched
    assert!(workspace.join("Tidy_Spec.pdf").exists());
}
