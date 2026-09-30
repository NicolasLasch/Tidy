//! Bounded, metadata-only scanning. No file content, hashes or persistent index yet.
use std::{
    fs, io,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime},
};
pub use tidy_platform::AuthorizedRoot;

#[derive(Debug, Clone)]
pub struct ScanLimits {
    pub max_entries: usize,
    pub max_depth: usize,
    pub max_duration: Duration,
}
impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_depth: 32,
            max_duration: Duration::from_secs(30),
        }
    }
}
#[derive(Debug, Clone)]
pub struct FileRecord {
    pub relative_path: PathBuf,
    pub logical_bytes: u64,
    pub modified: Option<SystemTime>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Complete,
    Cancelled,
    EntryLimit,
    TimeLimit,
}
#[derive(Debug)]
pub struct ScanIssue {
    pub relative_path: PathBuf,
    pub message: String,
}
#[derive(Debug)]
pub struct ScanReport {
    pub files: Vec<FileRecord>,
    pub issues: Vec<ScanIssue>,
    pub visited_entries: usize,
    pub stop_reason: StopReason,
}
impl ScanReport {
    /// Logical size only: hard links, sparse files and APFS clones are not deduplicated.
    pub fn logical_bytes(&self) -> u64 {
        self.files
            .iter()
            .fold(0u64, |sum, f| sum.saturating_add(f.logical_bytes))
    }
}
/// Depth-first streaming traversal; memory is O(entry limit + depth), not tree width.
/// Cancellation and time limits are checked between filesystem calls; an OS call
/// itself may block. Path checks do not defeat concurrent adversarial path swaps.
pub fn scan(
    root: &AuthorizedRoot,
    limits: &ScanLimits,
    cancelled: &AtomicBool,
) -> io::Result<ScanReport> {
    root.validate(root.path())?;
    let started = Instant::now();
    let mut report = ScanReport {
        files: vec![],
        issues: vec![],
        visited_entries: 0,
        stop_reason: StopReason::Complete,
    };
    let mut stack = vec![(
        fs::read_dir(root.path())
            .map_err(|e| tidy_platform::io_context("read directory", root.path(), e))?,
        0usize,
    )];
    while let Some((entries, depth)) = stack.last_mut() {
        if cancelled.load(Ordering::Relaxed) {
            report.stop_reason = StopReason::Cancelled;
            break;
        }
        if started.elapsed() >= limits.max_duration {
            report.stop_reason = StopReason::TimeLimit;
            break;
        }
        if report.visited_entries >= limits.max_entries {
            report.stop_reason = StopReason::EntryLimit;
            break;
        }
        let depth = *depth;
        let Some(entry) = entries.next() else {
            stack.pop();
            continue;
        };
        report.visited_entries += 1;
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                report.issues.push(ScanIssue {
                    relative_path: PathBuf::new(),
                    message: e.to_string(),
                });
                continue;
            }
        };
        let path = entry.path();
        let relative_path = path
            .strip_prefix(root.path())
            .expect("entry from authorized tree")
            .to_path_buf();
        let result = (|| -> io::Result<()> {
            root.validate(&path)?;
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_file() {
                report.files.push(FileRecord {
                    relative_path: relative_path.clone(),
                    logical_bytes: metadata.len(),
                    modified: metadata.modified().ok(),
                });
            } else if metadata.is_dir() {
                if depth >= limits.max_depth {
                    return Err(io::Error::other("depth limit: subtree omitted"));
                }
                stack.push((fs::read_dir(&path)?, depth + 1));
            } else {
                return Err(io::Error::other("special file omitted"));
            }
            Ok(())
        })();
        if let Err(e) = result {
            report.issues.push(ScanIssue {
                relative_path,
                message: e.to_string(),
            });
        }
    }
    report
        .files
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(report)
}

pub mod database;
pub mod index_scan;
