//! Shared end-to-end environment: builds a mock folder, scans it with the real indexer, and gives
//! tests the real request engine and safety engine. Not a test itself.
#![allow(dead_code)]
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize},
};
use tidy_agent_runtime::{intent::respond_in, investigation::Investigation};
use tidy_file_indexer::{
    AuthorizedRoot,
    database::Index,
    index_scan::{self, IndexOptions},
};
use tidy_organization::{FileCandidate, FileId, ProposedAction};
use tidy_safety::SafetyEngine;

pub struct Env {
    pub dir: PathBuf,
    pub root: AuthorizedRoot,
    pub db: Index,
    pub scope: i64,
    pub engine: SafetyEngine,
    pub files: Vec<FileCandidate>,
}
impl Env {
    /// Builds a folder with `build`, authorizes it, scans it and returns the working environment.
    pub fn with(name: &str, build: impl FnOnce(&Path)) -> Self {
        Self::with_sub(name, build, "")
    }
    /// Like `with`, but the authorized folder is a subfolder of what was built (for example the
    /// messy `Inbox` on its own, as a user would add just that folder).
    pub fn with_sub(name: &str, build: impl FnOnce(&Path), sub: &str) -> Self {
        let dir = PathBuf::from(format!(
            "/private/tmp/tidy_e2e_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dir = fs::canonicalize(dir).unwrap();
        build(&dir.join("Mock"));
        let root = AuthorizedRoot::authorize(dir.join("Mock").join(sub)).unwrap();
        let db = Index::open(&dir.join("index.sqlite3")).unwrap();
        let scope = db.add_scope(&root).unwrap();
        let mut env = Self {
            dir,
            root,
            db,
            scope,
            engine: SafetyEngine::new_in_memory().unwrap(),
            files: vec![],
        };
        env.scan();
        env
    }
    pub fn scan(&mut self) {
        let cache = self.db.cache(self.scope).unwrap();
        let snapshot = index_scan::collect(
            &self.root,
            &IndexOptions::default(),
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
            &cache,
        )
        .unwrap();
        self.db.commit(self.scope, &snapshot, false).unwrap();
        self.files = self
            .db
            .storage_files(self.scope)
            .unwrap()
            .into_iter()
            .map(|f| FileCandidate {
                id: FileId(f.id as u64),
                relative_path: f.path,
                size: f.size,
                modified: f.modified,
                excerpt: f.excerpt,
            })
            .collect();
    }
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }
    pub fn ask(&self, request: &str) -> Investigation {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        respond_in(request, &self.files, "Mock", now, Some(self.root.path()))
            .unwrap_or_else(|| panic!("not understood: {request}"))
    }
    pub fn indexed(&self, rel: &str) -> bool {
        self.files.iter().any(|f| f.relative_path == Path::new(rel))
    }
    /// Approves and executes file actions exactly as the app does after the user reviews them.
    pub fn run_actions(&mut self, actions: &[ProposedAction]) -> i64 {
        let map: HashMap<FileId, PathBuf> = self
            .files
            .iter()
            .map(|f| (f.id, f.relative_path.clone()))
            .collect();
        let view = self
            .engine
            .request_plan_approval(&self.root, self.scope, "e2e", actions, &map)
            .unwrap();
        let report = self
            .engine
            .execute_approved_plan(&self.root, &view.token)
            .unwrap();
        assert!(report.verified);
        self.scan();
        report.transaction_id
    }
    pub fn trash_folders(&mut self, folders: &[String]) -> i64 {
        let rel: Vec<PathBuf> = folders.iter().map(PathBuf::from).collect();
        let view = self
            .engine
            .request_folder_trash_approval(&self.root, self.scope, "e2e", &rel)
            .unwrap();
        let report = self
            .engine
            .execute_approved_plan(&self.root, &view.token)
            .unwrap();
        self.scan();
        report.transaction_id
    }
    /// Puts every trashed step of a transaction back (also proves the recorded Trash location works).
    pub fn restore_all(&mut self, tx: i64) {
        let detail = self.engine.get_detail(tx).unwrap().unwrap();
        for step in detail
            .steps
            .iter()
            .filter(|s| s.state == "verified" && s.action_type.starts_with("trash"))
        {
            self.engine
                .restore_trashed(&self.root, self.scope, tx, step.id)
                .unwrap();
        }
        self.scan();
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
