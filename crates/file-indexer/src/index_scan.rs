//! Desktop scan snapshots. Content access is opt-in and bounded independently of metadata.
use crate::AuthorizedRoot;
use serde::Serialize;
#[cfg(unix)]
use std::time::Instant;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub path: PathBuf,
    pub identity: String,
    pub fingerprint: String,
    pub size: u64,
    pub modified: i64,
    pub excerpt: Option<String>,
    pub hash: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Omission {
    pub path: String,
    pub reason: String,
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub files: Vec<IndexedFile>,
    pub omissions: Vec<Omission>,
    pub omission_count: usize,
    pub visited: usize,
    pub status: String,
    pub complete: bool,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            files: vec![],
            omissions: vec![],
            omission_count: 0,
            visited: 0,
            status: "complete".into(),
            complete: true,
        }
    }
}
#[cfg(unix)]
impl Snapshot {
    fn omit(&mut self, path: &Path, reason: impl ToString, incomplete: bool) {
        self.omission_count += 1;
        if self.omissions.len() < 100 {
            self.omissions.push(Omission {
                path: path.to_string_lossy().into(),
                reason: reason.to_string(),
            });
        }
        self.complete &= !incomplete;
    }
}
#[derive(Clone)]
pub struct IndexOptions {
    pub content: bool,
    pub max_entries: usize,
    pub max_duration: Duration,
}
impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            content: false,
            max_entries: 100_000,
            max_duration: Duration::from_secs(60),
        }
    }
}
pub type Cache = HashMap<Vec<u8>, IndexedFile>;
pub fn path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect()
    }
}
pub fn path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        OsString::from_vec(bytes).into()
    }
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        OsString::from_wide(
            &bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .into()
    }
}

#[cfg(unix)]
pub fn collect(
    root: &AuthorizedRoot,
    options: &IndexOptions,
    cancel: &AtomicBool,
    progress: &AtomicUsize,
    cache: &Cache,
) -> std::io::Result<Snapshot> {
    use std::{ffi::OsStr, io::Read, os::unix::ffi::OsStrExt};
    use tidy_platform::handles::{self, Directory};
    let started = Instant::now();
    let directory = Directory::root(root)?;
    let entries = directory.entries()?;
    let mut stack = vec![(directory, entries, PathBuf::new(), 0usize)];
    let mut output = Snapshot::default();
    let mut text_budget = 8 * 1024 * 1024usize;
    while let Some((parent, entries, relative, depth)) = stack.last_mut() {
        if cancel.load(Ordering::Relaxed) {
            output.status = "cancelled".into();
            output.complete = false;
            break;
        }
        if started.elapsed() >= options.max_duration {
            output.status = "time limit".into();
            output.complete = false;
            break;
        }
        if output.visited >= options.max_entries.min(100_000) {
            output.status = "entry limit".into();
            output.complete = false;
            break;
        }
        let Some(entry) = entries.next() else {
            stack.pop();
            continue;
        };
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                output.omit(relative, e, true);
                stack.pop();
                continue;
            }
        };
        let name = OsStr::from_bytes(entry.file_name().to_bytes());
        if name == "." || name == ".." {
            continue;
        }
        output.visited += 1;
        progress.store(output.visited, Ordering::Relaxed);
        let path = relative.join(name);
        let depth = *depth;
        if tidy_platform::protected(&root.path().join(&path)) {
            output.omit(&path, "protected path", false);
            continue;
        }
        let mut pending_child = None;
        let result = (|| -> std::io::Result<()> {
            let stat = parent.stat(name)?;
            if !parent.same_device(&stat) {
                output.omit(&path, "mount boundary", false);
                return Ok(());
            }
            if handles::symlink(&stat) {
                output.omit(&path, "symbolic link", false);
                return Ok(());
            }
            if handles::placeholder(&stat) {
                output.omit(&path, "offline cloud placeholder", false);
                return Ok(());
            }
            if handles::directory(&stat) {
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("app"))
                {
                    output.omit(&path, "application bundle (not expanded)", false);
                    return Ok(());
                }
                if depth >= 32 {
                    output.omit(&path, "depth limit", true);
                    return Ok(());
                }
                let child = parent.child(name, &stat)?;
                let child_entries = child.entries()?;
                pending_child = Some((child, child_entries, path.clone(), depth + 1));
            } else if handles::regular(&stat) {
                let fingerprint = format!(
                    "{}:{}:{}:{}:{}:{}:{}",
                    stat.st_dev,
                    stat.st_ino,
                    stat.st_size,
                    stat.st_mtime,
                    stat.st_mtime_nsec,
                    stat.st_ctime,
                    stat.st_ctime_nsec
                );
                let mut file = IndexedFile {
                    path: path.clone(),
                    identity: format!("{}:{}", stat.st_dev, stat.st_ino),
                    fingerprint,
                    size: stat.st_size.max(0) as u64,
                    modified: stat.st_mtime,
                    excerpt: None,
                    hash: None,
                };
                if options.content {
                    if let Some(old) = cache
                        .get(&path_bytes(&path))
                        .filter(|old| old.fingerprint == file.fingerprint)
                    {
                        if let Some(text) = &old.excerpt
                            && text.len() <= text_budget
                        {
                            text_budget -= text.len();
                            file.excerpt = Some(text.clone());
                        }
                        file.hash = old.hash.clone();
                    }
                    if file.excerpt.is_none()
                        && text_file(&path)
                        && file.size <= 65536
                        && file.size as usize <= text_budget
                    {
                        let mut handle = parent.file(name, &stat)?;
                        let mut bytes = Vec::new();
                        (&mut handle).take(65537).read_to_end(&mut bytes)?;
                        if !handles::unchanged(&stat, &handles::stat_file(&handle)?)
                            || bytes.len() as u64 != file.size
                        {
                            return Err(std::io::Error::other("file changed during text read"));
                        }
                        text_budget -= bytes.len();
                        file.excerpt = decode_text(bytes);
                    }
                }
                output.files.push(file);
            } else {
                output.omit(&path, "special file", false);
            }
            Ok(())
        })();
        if let Some(child) = pending_child {
            stack.push(child);
        }
        if let Err(error) = result {
            output.omit(&path, error, true);
        }
    }
    // Hash only same-size candidates, with an independent 64 MiB total / 16 MiB per-file bound.
    if options.content && !cancel.load(Ordering::Relaxed) {
        use sha2::{Digest, Sha256};
        let mut sizes = HashMap::new();
        for file in &output.files {
            *sizes.entry(file.size).or_insert(0usize) += 1;
        }
        let mut hash_budget = 64 * 1024 * 1024u64;
        for file in &mut output.files {
            if cancel.load(Ordering::Relaxed) || started.elapsed() >= options.max_duration {
                break;
            }
            if file.hash.is_some()
                || sizes[&file.size] < 2
                || file.size > 16 * 1024 * 1024
                || file.size > hash_budget
            {
                continue;
            }
            hash_budget -= file.size;
            let result = (|| -> std::io::Result<String> {
                let mut parent = Directory::root(root)?;
                let mut parts = file.path.components().peekable();
                while let Some(part) = parts.next() {
                    let name = part.as_os_str();
                    let stat = parent.stat(name)?;
                    if parts.peek().is_some() {
                        parent = parent.child(name, &stat)?;
                        continue;
                    }
                    let fingerprint = format!(
                        "{}:{}:{}:{}:{}:{}:{}",
                        stat.st_dev,
                        stat.st_ino,
                        stat.st_size,
                        stat.st_mtime,
                        stat.st_mtime_nsec,
                        stat.st_ctime,
                        stat.st_ctime_nsec
                    );
                    if fingerprint != file.fingerprint {
                        return Err(std::io::Error::other("file changed before hashing"));
                    }
                    let mut handle = parent.file(name, &stat)?;
                    let mut hash = Sha256::new();
                    let mut buffer = [0u8; 65536];
                    let mut read = 0u64;
                    loop {
                        if cancel.load(Ordering::Relaxed)
                            || started.elapsed() >= options.max_duration
                        {
                            return Err(std::io::Error::other("hash interrupted"));
                        }
                        let count = handle.read(&mut buffer)?;
                        if count == 0 {
                            break;
                        }
                        read += count as u64;
                        if read > file.size {
                            return Err(std::io::Error::other("file grew during hashing"));
                        }
                        hash.update(&buffer[..count]);
                    }
                    if read != file.size
                        || !handles::unchanged(&stat, &handles::stat_file(&handle)?)
                    {
                        return Err(std::io::Error::other("file changed during hashing"));
                    }
                    return Ok(format!("{:x}", hash.finalize()));
                }
                Err(std::io::Error::other("empty file path"))
            })();
            if let Ok(hash) = result {
                file.hash = Some(hash);
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        output.status = "cancelled".into();
        output.complete = false;
    }
    if output.status == "complete" && !output.complete {
        output.status = "partial".into();
    }
    output.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(output)
}
#[cfg(unix)]
fn text_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "txt"
                | "md"
                | "csv"
                | "tsv"
                | "json"
                | "toml"
                | "yaml"
                | "yml"
                | "rs"
                | "ts"
                | "tsx"
                | "js"
                | "css"
                | "html"
        )
    })
}
pub fn decode_text(bytes: Vec<u8>) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}
#[cfg(not(unix))]
pub fn collect(
    root: &AuthorizedRoot,
    options: &IndexOptions,
    cancel: &AtomicBool,
    progress: &AtomicUsize,
    _cache: &Cache,
) -> std::io::Result<Snapshot> {
    let report = crate::scan(
        root,
        &crate::ScanLimits {
            max_entries: options.max_entries.min(100_000),
            max_duration: options.max_duration,
            ..Default::default()
        },
        cancel,
    )?;
    progress.store(report.visited_entries, Ordering::Relaxed);
    Ok(Snapshot {
        files: report
            .files
            .into_iter()
            .map(|f| IndexedFile {
                path: f.relative_path,
                identity: String::new(),
                fingerprint: format!("{}:{:?}", f.logical_bytes, f.modified),
                size: f.logical_bytes,
                modified: f
                    .modified
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |t| t.as_secs() as i64),
                excerpt: None,
                hash: None,
            })
            .collect(),
        omission_count: report.issues.len(),
        omissions: report
            .issues
            .into_iter()
            .take(100)
            .map(|i| Omission {
                path: i.relative_path.to_string_lossy().into(),
                reason: i.message,
            })
            .collect(),
        visited: report.visited_entries,
        status: format!("{:?}", report.stop_reason),
        complete: report.stop_reason == crate::StopReason::Complete,
    })
}
