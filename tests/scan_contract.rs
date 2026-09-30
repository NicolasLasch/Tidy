use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use tidy_file_indexer::{AuthorizedRoot, ScanLimits, StopReason, scan};
static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        #[cfg(target_os = "macos")]
        let base = PathBuf::from("/private/tmp/tidy-scan-tests");
        #[cfg(not(target_os = "macos"))]
        let base = std::env::temp_dir().join("tidy-scan-tests");
        fs::create_dir_all(&base).unwrap();
        let path = base.join(format!(
            "{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn root(&self) -> AuthorizedRoot {
        AuthorizedRoot::authorize(&self.0).unwrap()
    }
    fn file(&self, path: &str, content: &[u8]) {
        fs::write(self.0.join(path), content).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(f: &Fixture) -> tidy_file_indexer::ScanReport {
    scan(&f.root(), &ScanLimits::default(), &AtomicBool::new(false)).unwrap()
}
#[test]
fn records_nested_metadata_without_changing_files() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("nested")).unwrap();
    f.file("z.txt", b"hello");
    f.file("nested/a.txt", b"world!");
    let r = run(&f);
    assert_eq!(r.files.len(), 2);
    assert_eq!(r.logical_bytes(), 11);
    assert_eq!(r.files[0].relative_path, PathBuf::from("nested/a.txt"));
    assert_eq!(r.stop_reason, StopReason::Complete);
    assert!(r.issues.is_empty());
    assert_eq!(fs::read(f.0.join("z.txt")).unwrap(), b"hello");
}
#[test]
fn empty_folder() {
    assert!(run(&Fixture::new()).files.is_empty());
}
#[test]
fn refuses_relative_traversal_files_and_missing_roots() {
    let f = Fixture::new();
    f.file("file", b"x");
    for p in [
        PathBuf::from("."),
        f.0.join(".."),
        f.0.join("file"),
        f.0.join("missing"),
    ] {
        assert!(AuthorizedRoot::authorize(p).is_err());
    }
}
#[test]
fn git_internals_are_omitted_but_repository_files_are_managed() {
    let f = Fixture::new();
    for name in ["repo", "worktree"] {
        fs::create_dir(f.0.join(name)).unwrap();
        f.file(&format!("{name}/secret"), b"hidden");
    }
    fs::create_dir(f.0.join("repo/.git")).unwrap();
    f.file("worktree/.git", b"gitdir: elsewhere");
    let r = run(&f);
    // Working files are indexed; only the .git directory / worktree pointer is left alone.
    assert_eq!(r.files.len(), 2);
    assert_eq!(r.issues.len(), 2);
    assert!(AuthorizedRoot::authorize(f.0.join("repo")).is_ok());
    fs::create_dir(f.0.join("repo/sub")).unwrap();
    assert!(AuthorizedRoot::authorize(f.0.join("repo/sub")).is_ok());
    assert!(AuthorizedRoot::authorize(f.0.join("repo/.git")).is_err());
}
#[test]
fn git_marker_created_after_authorization_is_skipped_not_fatal() {
    let f = Fixture::new();
    let root = f.root();
    f.file(".git", b"gitdir: elsewhere");
    let report = scan(&root, &ScanLimits::default(), &AtomicBool::new(false)).unwrap();
    assert!(report.files.is_empty());
}
#[test]
fn entry_budget_includes_directories() {
    let f = Fixture::new();
    for n in 0..20 {
        fs::create_dir(f.0.join(n.to_string())).unwrap();
    }
    let limits = ScanLimits {
        max_entries: 3,
        ..ScanLimits::default()
    };
    let r = scan(&f.root(), &limits, &AtomicBool::new(false)).unwrap();
    assert_eq!(r.visited_entries, 3);
    assert_eq!(r.stop_reason, StopReason::EntryLimit);
}
#[test]
fn depth_budget_reports_omissions() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("sub")).unwrap();
    f.file("sub/file", b"x");
    let limits = ScanLimits {
        max_depth: 0,
        ..ScanLimits::default()
    };
    let r = scan(&f.root(), &limits, &AtomicBool::new(false)).unwrap();
    assert!(r.files.is_empty());
    assert_eq!(r.issues.len(), 1);
}
#[test]
fn cancellation_and_timeout_are_explicit() {
    let f = Fixture::new();
    f.file("file", b"x");
    let r = scan(&f.root(), &ScanLimits::default(), &AtomicBool::new(true)).unwrap();
    assert_eq!(r.stop_reason, StopReason::Cancelled);
    assert_eq!(r.visited_entries, 0);
    let limits = ScanLimits {
        max_duration: Duration::ZERO,
        ..ScanLimits::default()
    };
    assert_eq!(
        scan(&f.root(), &limits, &AtomicBool::new(false))
            .unwrap()
            .stop_reason,
        StopReason::TimeLimit
    );
}
#[test]
fn rescanning_observes_changed_files() {
    let f = Fixture::new();
    f.file("file", b"a");
    assert_eq!(run(&f).logical_bytes(), 1);
    f.file("file", b"changed");
    assert_eq!(run(&f).logical_bytes(), 7);
}
#[cfg(unix)]
#[test]
fn symlinks_broken_links_and_cycles_are_omitted() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let outside = Fixture::new();
    outside.file("secret", b"secret");
    symlink(&outside.0, f.0.join("outside")).unwrap();
    symlink(&f.0, f.0.join("cycle")).unwrap();
    symlink("missing", f.0.join("broken")).unwrap();
    let r = run(&f);
    assert!(r.files.is_empty());
    assert_eq!(r.issues.len(), 3);
    assert!(AuthorizedRoot::authorize(f.0.join("outside")).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_in_selected_path_is_refused() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("real/sub")).unwrap();
    symlink(f.0.join("real"), f.0.join("alias")).unwrap();
    assert!(AuthorizedRoot::authorize(f.0.join("alias/sub")).is_err());
}
#[cfg(unix)]
#[test]
fn permissions_are_reported_without_aborting_other_files() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::create_dir(f.0.join("locked")).unwrap();
    f.file("visible", b"x");
    let locked = f.0.join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o0)).unwrap();
    let inaccessible = fs::read_dir(&locked).is_err();
    let r = run(&f);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(r.files.len(), 1);
    if inaccessible {
        assert_eq!(r.issues.len(), 1);
    } // elevated accounts bypass POSIX mode bits
}
#[cfg(target_os = "linux")]
#[test]
fn native_non_utf8_names_are_preserved() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let f = Fixture::new();
    let name = OsString::from_vec(vec![0xff]);
    fs::write(f.0.join(&name), b"x").unwrap();
    assert_eq!(run(&f).files[0].relative_path.as_os_str(), &name);
}
#[test]
fn protected_root_is_refused() {
    #[cfg(unix)]
    assert!(AuthorizedRoot::authorize("/").is_err());
    let f = Fixture::new();
    fs::create_dir(f.0.join(".ssh")).unwrap();
    assert!(AuthorizedRoot::authorize(f.0.join(".ssh")).is_err());
}
#[test]
fn scope_validation_refuses_sibling() {
    let f = Fixture::new();
    let other = Fixture::new();
    assert!(f.root().validate(&other.0).is_err());
}
