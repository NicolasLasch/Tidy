use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tidy_file_indexer::{
    AuthorizedRoot,
    database::Index,
    index_scan::{self, Cache, IndexOptions, IndexedFile, Snapshot},
};
static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        #[cfg(target_os = "macos")]
        let base = PathBuf::from("/private/tmp");
        #[cfg(not(target_os = "macos"))]
        let base = std::env::temp_dir();
        let p = base.join(format!(
            "tidy-phase2-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(p.join("selected")).unwrap();
        Self(fs::canonicalize(p).unwrap())
    }
    fn root(&self) -> AuthorizedRoot {
        AuthorizedRoot::authorize(self.0.join("selected")).unwrap()
    }
    fn file(&self, name: &str, bytes: &[u8]) {
        fs::write(self.0.join("selected").join(name), bytes).unwrap();
    }
    fn db(&self) -> Index {
        Index::open(&self.0.join("index.sqlite3")).unwrap()
    }
    #[cfg(unix)]
    fn scan(&self, content: bool) -> Snapshot {
        index_scan::collect(
            &self.root(),
            &IndexOptions {
                content,
                ..Default::default()
            },
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
            &Cache::new(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn record(name: &str, identity: &str) -> IndexedFile {
    IndexedFile {
        path: PathBuf::from(name),
        identity: identity.into(),
        fingerprint: identity.into(),
        size: 100,
        modified: 1,
        excerpt: Some("project sunbeam".into()),
        hash: None,
    }
}
fn snapshot(files: Vec<IndexedFile>, complete: bool) -> Snapshot {
    Snapshot {
        files,
        complete,
        status: if complete { "complete" } else { "partial" }.into(),
        ..Default::default()
    }
}
#[test]
fn persists_across_restart_and_searches_names_and_text() {
    let f = Fixture::new();
    let id;
    {
        let mut db = f.db();
        id = db.add_scope(&f.root()).unwrap();
        db.commit(id, &snapshot(vec![record("notes.txt", "1")], true), true)
            .unwrap();
    }
    let db = f.db();
    assert_eq!(db.scopes().unwrap()[0].files, 1);
    assert_eq!(db.search(id, "sunbeam", false, 0).unwrap().total, 1);
    assert_eq!(db.search(id, "notes", false, 0).unwrap().total, 1);
    assert_eq!(db.search(id, "", false, 0).unwrap().total, 1);
    assert!(db.matches_root(id, &f.root()).unwrap());
}
#[test]
fn fts_input_is_literal_not_query_syntax() {
    let f = Fixture::new();
    let mut db = f.db();
    let id = db.add_scope(&f.root()).unwrap();
    db.commit(id, &snapshot(vec![record("notes.txt", "1")], true), true)
        .unwrap();
    for query in ["\"", "*", "OR NOT", "a:b", "("] {
        assert!(db.search(id, query, false, 0).is_ok(), "{query}");
    }
}
#[test]
fn partial_scan_preserves_unseen_rows_but_hides_stale_results() {
    let f = Fixture::new();
    let mut db = f.db();
    let id = db.add_scope(&f.root()).unwrap();
    db.commit(
        id,
        &snapshot(vec![record("a", "1"), record("b", "2")], true),
        true,
    )
    .unwrap();
    db.commit(id, &snapshot(vec![record("a", "1")], false), true)
        .unwrap();
    assert_eq!(db.search(id, "", false, 0).unwrap().total, 1);
    let connection = rusqlite::Connection::open(f.0.join("index.sqlite3")).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    db.commit(id, &snapshot(vec![record("a", "1")], true), true)
        .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn rename_keeps_identity_and_hardlinks_remain_separate() {
    let f = Fixture::new();
    let mut db = f.db();
    let id = db.add_scope(&f.root()).unwrap();
    db.commit(id, &snapshot(vec![record("old", "1")], true), true)
        .unwrap();
    let original = db.search(id, "", false, 0).unwrap().files[0].id;
    db.commit(id, &snapshot(vec![record("new", "1")], true), true)
        .unwrap();
    assert_eq!(db.search(id, "", false, 0).unwrap().files[0].id, original);
    db.commit(
        id,
        &snapshot(vec![record("new", "1"), record("link", "1")], true),
        true,
    )
    .unwrap();
    assert_eq!(db.search(id, "", false, 0).unwrap().total, 2);
}
#[test]
fn revoked_scope_cannot_commit_or_search() {
    let f = Fixture::new();
    let mut db = f.db();
    let id = db.add_scope(&f.root()).unwrap();
    db.commit(id, &snapshot(vec![record("a", "1")], true), true)
        .unwrap();
    db.forget(id).unwrap();
    assert!(db.search(id, "", false, 0).is_err());
    assert!(
        db.commit(id, &snapshot(vec![record("a", "1")], true), true)
            .is_err()
    );
    assert!(db.scopes().unwrap().is_empty());
}
#[test]
fn migration_is_idempotent_and_future_schema_is_refused() {
    let f = Fixture::new();
    drop(f.db());
    drop(f.db());
    let c = rusqlite::Connection::open(f.0.join("index.sqlite3")).unwrap();
    c.pragma_update(None, "user_version", 99).unwrap();
    assert!(Index::open(&f.0.join("index.sqlite3")).is_err());
}
#[test]
fn paging_is_bounded_and_queries_are_limited() {
    let f = Fixture::new();
    let mut db = f.db();
    let id = db.add_scope(&f.root()).unwrap();
    db.commit(
        id,
        &snapshot(
            (0..205)
                .map(|i| record(&format!("file{i}"), &i.to_string()))
                .collect(),
            true,
        ),
        true,
    )
    .unwrap();
    assert_eq!(db.search(id, "", true, 0).unwrap().files.len(), 100);
    assert_eq!(db.search(id, "", true, 200).unwrap().files.len(), 5);
    assert!(db.search(id, &"x".repeat(257), true, 0).is_err());
}
#[test]
fn malformed_text_is_rejected() {
    assert!(index_scan::decode_text(vec![0xff]).is_none());
    assert!(index_scan::decode_text(vec![b'a', 0]).is_none());
    assert_eq!(
        index_scan::decode_text(b"hello".to_vec()).as_deref(),
        Some("hello")
    );
}
#[cfg(unix)]
#[test]
fn secure_scan_reads_text_only_when_enabled_and_hashes_size_candidates() {
    let f = Fixture::new();
    f.file("a.txt", b"sunbeam");
    f.file("b.txt", b"sunbeam");
    f.file("bad.txt", &[0xff]);
    let meta = f.scan(false);
    assert!(
        meta.files
            .iter()
            .all(|f| f.excerpt.is_none() && f.hash.is_none())
    );
    let content = f.scan(true);
    assert_eq!(content.files[0].excerpt.as_deref(), Some("sunbeam"));
    assert_eq!(content.files[0].hash, content.files[1].hash);
    assert!(content.files[0].hash.is_some());
    assert!(content.files[2].excerpt.is_none());
}
#[cfg(unix)]
#[test]
fn secure_scan_does_not_expand_links_apps_or_repositories() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("selected/test.app")).unwrap();
    f.file("test.app/hidden", b"x");
    fs::create_dir_all(f.0.join("selected/repo/.git")).unwrap();
    f.file("repo/hidden", b"x");
    symlink(&f.0, f.0.join("selected/link")).unwrap();
    let result = f.scan(true);
    // The repository's working file is indexed; its .git, the app bundle and the link are omitted.
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.omission_count, 3);
}
#[cfg(unix)]
#[test]
fn handles_reject_replaced_entries_and_replaced_roots() {
    use std::os::unix::fs::symlink;
    use tidy_platform::handles::Directory;
    let f = Fixture::new();
    f.file("a", b"safe");
    fs::write(f.0.join("outside"), b"secret").unwrap();
    let grant = f.root();
    let directory = Directory::root(&grant).unwrap();
    let stat = directory.stat(std::ffi::OsStr::new("a")).unwrap();
    fs::rename(f.0.join("selected/a"), f.0.join("selected/old")).unwrap();
    symlink(f.0.join("outside"), f.0.join("selected/a")).unwrap();
    assert!(directory.file(std::ffi::OsStr::new("a"), &stat).is_err());
    fs::rename(f.0.join("selected"), f.0.join("previous")).unwrap();
    fs::create_dir(f.0.join("selected")).unwrap();
    assert!(Directory::root(&grant).is_err());
}
#[cfg(unix)]
#[test]
fn handle_api_rejects_parent_traversal() {
    let f = Fixture::new();
    let d = tidy_platform::handles::Directory::root(&f.root()).unwrap();
    assert!(
        d.stat(std::path::Path::new("../outside").as_os_str())
            .is_err()
    );
}
#[test]
fn cancellation_and_limits_remain_explicit() {
    let f = Fixture::new();
    f.file("a", b"x");
    let root = f.root();
    let cancelled = index_scan::collect(
        &root,
        &Default::default(),
        &AtomicBool::new(true),
        &AtomicUsize::new(0),
        &Cache::new(),
    )
    .unwrap();
    assert!(!cancelled.complete);
    let limited = index_scan::collect(
        &root,
        &IndexOptions {
            max_entries: 0,
            ..Default::default()
        },
        &AtomicBool::new(false),
        &AtomicUsize::new(0),
        &Cache::new(),
    )
    .unwrap();
    assert!(!limited.complete);
}
#[cfg(unix)]
#[test]
fn content_is_refreshed_after_change_and_cached_when_unchanged() {
    let f = Fixture::new();
    f.file("a.txt", b"first");
    let mut db = f.db();
    let id = db.add_scope(&f.root()).unwrap();
    let first = f.scan(true);
    db.commit(id, &first, true).unwrap();
    f.file("a.txt", b"second text");
    let current = index_scan::collect(
        &f.root(),
        &IndexOptions {
            content: true,
            ..Default::default()
        },
        &AtomicBool::new(false),
        &AtomicUsize::new(0),
        &db.cache(id).unwrap(),
    )
    .unwrap();
    assert_eq!(current.files[0].excerpt.as_deref(), Some("second text"));
}

#[cfg(unix)]
#[test]
fn directory_replacement_is_rejected_before_descent() {
    use std::{ffi::OsStr, os::unix::fs::symlink};
    let f = Fixture::new();
    fs::create_dir(f.0.join("selected/sub")).unwrap();
    let directory = tidy_platform::handles::Directory::root(&f.root()).unwrap();
    let before = directory.stat(OsStr::new("sub")).unwrap();
    fs::rename(f.0.join("selected/sub"), f.0.join("previous-sub")).unwrap();
    symlink(&f.0, f.0.join("selected/sub")).unwrap();
    assert!(directory.child(OsStr::new("sub"), &before).is_err());
}
#[cfg(unix)]
#[test]
fn text_size_budget_and_saved_root_identity_are_enforced() {
    let f = Fixture::new();
    f.file("large.txt", &vec![b'a'; 65537]);
    assert!(f.scan(true).files[0].excerpt.is_none());
    let db = f.db();
    let id = db.add_scope(&f.root()).unwrap();
    fs::rename(f.0.join("selected"), f.0.join("original")).unwrap();
    fs::create_dir(f.0.join("selected")).unwrap();
    assert!(!db.matches_root(id, &f.root()).unwrap());
}
