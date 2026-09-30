//! Whole-disk usage explorer. Read-only: sizes come from allocated blocks (what Finder reports),
//! never cross into other volumes, never follow symlinks, and report what macOS will not let us read.
//! Results are cached in memory and on disk, so revisiting a folder shows the last known sizes
//! instantly while a background pass refreshes and saves them.
use super::{Shared, display_error, lock};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::State;

/// Folders this many levels below a scan root also get their listing cached, so drilling in is instant.
const RECORD_DEPTH: u8 = 3;
/// A cached listing younger than this is shown without starting another pass.
const FRESH_SECS: u64 = 90;

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    name: String,
    path: String,
    bytes: u64,
    files: u64,
    /// Last modification time in Unix seconds.
    #[serde(default)]
    modified: i64,
    is_dir: bool,
    /// macOS refused access to some or all of this folder (Full Disk Access is needed).
    denied: bool,
    /// Another volume is mounted here; not counted.
    mount: bool,
    done: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    at: u64,
    entries: Vec<Entry>,
}
#[derive(Default)]
struct Job {
    running: bool,
    /// Cached numbers are on screen while a fresh pass replaces them folder by folder.
    refreshing: bool,
    updated_at: Option<u64>,
    entries: Vec<Entry>,
}
pub struct DiskState {
    next: AtomicU64,
    /// The folder currently on screen; scans of other folders keep running and saving.
    current: Mutex<String>,
    scans: Mutex<HashMap<String, Job>>,
    /// Newest scan id per folder; an older scan stops as soon as it is superseded.
    flags: Mutex<HashMap<String, Arc<AtomicU64>>>,
    cache: Mutex<HashMap<String, Cached>>,
    file: PathBuf,
}
#[derive(Serialize)]
pub struct Volume {
    total_bytes: u64,
    used_bytes: u64,
    available_bytes: u64,
}
#[derive(Serialize)]
pub struct DiskView {
    path: String,
    running: bool,
    refreshing: bool,
    updated_at: Option<u64>,
    entries: Vec<Entry>,
    measured_bytes: u64,
    volume: Option<Volume>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
impl DiskState {
    pub fn load(dir: &Path) -> Self {
        let file = dir.join("disk_cache.json");
        let cache = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            next: AtomicU64::new(0),
            current: Mutex::new(String::new()),
            scans: Mutex::new(HashMap::new()),
            flags: Mutex::new(HashMap::new()),
            cache: Mutex::new(cache),
            file,
        }
    }
    fn save(&self, cache: &HashMap<String, Cached>) {
        let tmp = self.file.with_extension("json.tmp");
        if serde_json::to_vec(cache)
            .ok()
            .is_some_and(|bytes| std::fs::write(&tmp, bytes).is_ok())
        {
            let _ = std::fs::rename(tmp, &self.file);
        }
    }
}

fn allocated(meta: &std::fs::Metadata) -> u64 {
    meta.blocks().saturating_mul(512)
}
struct Walk<'a> {
    device: u64,
    alive: &'a dyn Fn() -> bool,
    seen: &'a Mutex<HashSet<(u64, u64)>>,
    lists: Vec<(String, Vec<Entry>)>,
    bytes: &'a AtomicU64,
    files: &'a AtomicU64,
    report: &'a mut dyn FnMut(u64, u64),
    since_report: u32,
}
/// Sums allocated bytes below `dir`; folders up to `RECORD_DEPTH` keep their own child listing.
fn walk(dir: &Path, depth: u8, w: &mut Walk) -> (u64, u64, bool) {
    let (mut bytes, mut files, mut denied) = (0u64, 0u64, false);
    if !(w.alive)() {
        return (0, 0, false);
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return (0, 0, true);
    };
    let record = depth <= RECORD_DEPTH;
    let mut listing = Vec::new();
    for entry in read.flatten() {
        let Ok(meta) = entry.metadata() else {
            denied = true;
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let (mut b, mut f, mut d, mut mount) = (0u64, 0u64, false, false);
        let is_dir = meta.is_dir() && !meta.file_type().is_symlink();
        if is_dir {
            if meta.dev() == w.device {
                (b, f, d) = walk(&entry.path(), depth + 1, w);
            } else {
                mount = true;
            }
        } else {
            let duplicate =
                meta.nlink() > 1 && !w.seen.lock().unwrap().insert((meta.dev(), meta.ino()));
            if !duplicate {
                b = allocated(&meta);
                f = 1;
            }
        }
        bytes += b;
        files += f;
        denied |= d && !is_dir;
        w.bytes
            .fetch_add(if is_dir { 0 } else { b }, Ordering::Relaxed);
        w.files
            .fetch_add(if is_dir { 0 } else { f }, Ordering::Relaxed);
        w.since_report += 1;
        if w.since_report >= 4000 {
            w.since_report = 0;
            (w.report)(
                w.bytes.load(Ordering::Relaxed),
                w.files.load(Ordering::Relaxed),
            );
        }
        if record {
            listing.push(Entry {
                path: entry.path().to_string_lossy().into_owned(),
                name,
                bytes: b,
                files: f,
                modified: meta.mtime(),
                is_dir,
                denied: d,
                mount,
                done: true,
            });
        }
        if d {
            denied = true;
        }
    }
    if record && !listing.is_empty() {
        w.lists.push((dir.to_string_lossy().into_owned(), listing));
    }
    (bytes, files, denied)
}

/// Starts (or refreshes) the listing for `path`. Cached results are shown immediately.
fn start(disk: &Arc<DiskState>, path: String, force: bool) -> Result<(), String> {
    let root = PathBuf::from(&path);
    if !root.is_absolute() || !root.is_dir() {
        return Err("Choose an existing folder".into());
    }
    *lock(&disk.current)? = path.clone();
    if !force
        && lock(&disk.scans)?
            .get(&path)
            .is_some_and(|scan| scan.running)
    {
        return Ok(());
    }
    let cached = lock(&disk.cache)?.get(&path).cloned();
    let mine = disk.next.fetch_add(1, Ordering::SeqCst) + 1;
    let flag = lock(&disk.flags)?.entry(path.clone()).or_default().clone();
    if let Some(c) = &cached
        && !force
        && now().saturating_sub(c.at) < FRESH_SECS
    {
        lock(&disk.scans)?.insert(
            path,
            Job {
                running: false,
                refreshing: false,
                updated_at: Some(c.at),
                entries: c.entries.clone(),
            },
        );
        return Ok(());
    }
    flag.store(mine, Ordering::SeqCst);
    // Scans of unrelated folders stop; scans of ancestors keep going (they also fill this folder's cache).
    {
        let flags = lock(&disk.flags)?;
        for (other, f) in flags.iter() {
            if *other != path && !root.starts_with(other) {
                f.store(0, Ordering::SeqCst);
            }
        }
    }
    let device = std::fs::metadata(&root).map_err(display_error)?.dev();
    let mut entries = Vec::new();
    let mut pending = Vec::new();
    for child in std::fs::read_dir(&root).map_err(display_error)?.flatten() {
        let Ok(meta) = child.metadata() else { continue };
        let is_dir = meta.is_dir() && !meta.file_type().is_symlink();
        let mount = is_dir && meta.dev() != device;
        let mut entry = Entry {
            name: child.file_name().to_string_lossy().into_owned(),
            path: child.path().to_string_lossy().into_owned(),
            bytes: if is_dir { 0 } else { allocated(&meta) },
            files: u64::from(!is_dir),
            modified: meta.mtime(),
            is_dir,
            denied: false,
            mount,
            done: !is_dir || mount,
        };
        if is_dir && !mount {
            pending.push(entries.len());
        }
        // Keep last known numbers on screen while refreshing.
        if is_dir
            && !mount
            && let Some(old) = cached
                .as_ref()
                .and_then(|c| c.entries.iter().find(|e| e.path == entry.path))
        {
            entry.bytes = old.bytes;
            entry.files = old.files;
            entry.denied = old.denied;
            entry.done = true;
        }
        entries.push(entry);
    }
    let refreshing = cached.is_some();
    lock(&disk.scans)?.insert(
        path.clone(),
        Job {
            running: !pending.is_empty(),
            refreshing,
            updated_at: cached.as_ref().map(|c| c.at),
            entries: entries.clone(),
        },
    );
    if pending.is_empty() {
        let mut cache = lock(&disk.cache)?;
        cache.insert(path, Cached { at: now(), entries });
        disk.save(&cache);
        return Ok(());
    }
    let queue = Arc::new(Mutex::new(pending));
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8);
    let remaining = Arc::new(AtomicUsize::new(workers));
    let seen = Arc::new(Mutex::new(HashSet::new()));
    let lists = Arc::new(Mutex::new(Vec::<(String, Vec<Entry>)>::new()));
    for _ in 0..workers {
        let (disk, queue, remaining, seen, lists, path, flag) = (
            disk.clone(),
            queue.clone(),
            remaining.clone(),
            seen.clone(),
            lists.clone(),
            path.clone(),
            flag.clone(),
        );
        let _ = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || {
                let alive = || flag.load(Ordering::Relaxed) == mine;
                loop {
                    let Some(index) = queue.lock().unwrap().pop() else {
                        break;
                    };
                    if !alive() {
                        break;
                    }
                    let target = disk.scans.lock().unwrap()[&path].entries[index]
                        .path
                        .clone();
                    let (bytes_now, files_now) = (AtomicU64::new(0), AtomicU64::new(0));
                    let mut report = |bytes: u64, files: u64| {
                        if !refreshing
                            && let Ok(mut scans) = disk.scans.lock()
                            && alive()
                            && let Some(job) = scans.get_mut(&path)
                        {
                            job.entries[index].bytes = bytes;
                            job.entries[index].files = files;
                        }
                    };
                    let mut w = Walk {
                        device,
                        alive: &alive,
                        seen: &seen,
                        lists: Vec::new(),
                        bytes: &bytes_now,
                        files: &files_now,
                        report: &mut report,
                        since_report: 0,
                    };
                    let (bytes, files, denied) = walk(Path::new(&target), 1, &mut w);
                    let recorded = std::mem::take(&mut w.lists);
                    if let Ok(mut scans) = disk.scans.lock()
                        && alive()
                        && let Some(job) = scans.get_mut(&path)
                    {
                        let entry = &mut job.entries[index];
                        entry.bytes = bytes;
                        entry.files = files;
                        entry.denied = denied;
                        entry.done = true;
                    }
                    if alive() {
                        lists.lock().unwrap().extend(recorded);
                    }
                }
                if remaining.fetch_sub(1, Ordering::SeqCst) == 1 && alive() {
                    let at = now();
                    let entries = {
                        let Ok(mut scans) = disk.scans.lock() else {
                            return;
                        };
                        let Some(job) = scans.get_mut(&path) else {
                            return;
                        };
                        job.running = false;
                        job.refreshing = false;
                        job.updated_at = Some(at);
                        job.entries.clone()
                    };
                    if let Ok(mut cache) = disk.cache.lock() {
                        cache.insert(path.clone(), Cached { at, entries });
                        for (dir, listing) in lists.lock().unwrap().drain(..) {
                            cache.insert(
                                dir,
                                Cached {
                                    at,
                                    entries: listing,
                                },
                            );
                        }
                        disk.save(&cache);
                    }
                }
            });
    }
    Ok(())
}
/// Refreshes the whole-disk view in the background at launch so it is ready before it is opened.
pub fn warm(disk: &Arc<DiskState>) {
    for path in ["/System/Volumes/Data", "/"] {
        if start(disk, path.into(), false).is_ok() {
            break;
        }
    }
}
#[tauri::command]
pub fn disk_start(path: String, force: bool, state: State<'_, Shared>) -> Result<(), String> {
    start(&state.disk, path, force)
}
#[tauri::command]
pub fn disk_status(state: State<'_, Shared>) -> Result<DiskView, String> {
    let path = lock(&state.disk.current)?.clone();
    let scans = lock(&state.disk.scans)?;
    let empty = Job::default();
    let job = scans.get(&path).unwrap_or(&empty);
    let mut entries = job.entries.clone();
    entries.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
    let volume = tidy_platform::volume_space(Path::new(&path))
        .ok()
        .flatten()
        .map(|v| Volume {
            total_bytes: v.total_bytes,
            used_bytes: v.total_bytes.saturating_sub(v.free_bytes),
            available_bytes: v.available_bytes,
        });
    Ok(DiskView {
        path,
        running: job.running,
        refreshing: job.refreshing,
        updated_at: job.updated_at,
        measured_bytes: entries.iter().map(|e| e.bytes).sum(),
        entries,
        volume,
    })
}
#[tauri::command]
pub fn disk_home() -> Option<String> {
    std::env::var("HOME").ok()
}
/// Shows an item in Finder. Selecting it never modifies anything.
#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() || std::fs::symlink_metadata(&path).is_err() {
        return Err("That item no longer exists".into());
    }
    let status = std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .status()
        .map_err(display_error)?;
    if status.success() {
        Ok(())
    } else {
        Err("Finder could not reveal this item".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measures_allocated_bytes_files_and_records_nested_listings() {
        let dir = std::env::temp_dir().join(format!("tidy_disk_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/one.bin"), vec![1u8; 100_000]).unwrap();
        std::fs::write(dir.join("a/b/two.bin"), vec![1u8; 50_000]).unwrap();
        std::fs::hard_link(dir.join("a/one.bin"), dir.join("a/b/link.bin")).unwrap();
        std::os::unix::fs::symlink("/", dir.join("a/root")).unwrap();
        let device = std::fs::metadata(&dir).unwrap().dev();
        let (b, f) = (AtomicU64::new(0), AtomicU64::new(0));
        let seen = Mutex::new(HashSet::new());
        let mut report = |_: u64, _: u64| {};
        let mut w = Walk {
            device,
            alive: &|| true,
            seen: &seen,
            lists: vec![],
            bytes: &b,
            files: &f,
            report: &mut report,
            since_report: 0,
        };
        let (bytes, files, denied) = walk(&dir.join("a"), 1, &mut w);
        let lists = std::mem::take(&mut w.lists);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!denied);
        assert_eq!(files, 3); // two real files (hard link counted once) + the symlink itself
        assert!((150_000..200_000).contains(&bytes), "{bytes}");
        assert!(
            lists
                .iter()
                .any(|(d, l)| d.ends_with("/a/b") && l.len() == 2)
        );
        assert!(lists.iter().any(|(d, _)| d.ends_with("/a")));
    }
}

/// A click on "Let Tidy manage this folder" authorizes it like the folder picker would.
#[tauri::command]
pub fn authorize_path(path: String, state: State<'_, Shared>) -> Result<i64, String> {
    let requested = PathBuf::from(&path);
    let root = tidy_file_indexer::AuthorizedRoot::authorize(&requested)
        .map_err(|e| tidy_platform::explain_refusal(&requested).replace("{e}", &e.to_string()))?;
    lock(&state.db)?.add_scope(&root).map_err(display_error)
}
#[tauri::command]
pub fn open_full_disk_access() -> Result<(), String> {
    std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
        .status()
        .map_err(display_error)
        .map(|_| ())
}

/// Opens a model page in the default browser. Only Hugging Face pages are allowed.
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("https://huggingface.co/") || url.contains(char::is_whitespace) {
        return Err("Only Hugging Face links can be opened from here".into());
    }
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(display_error)
        .map(|_| ())
}
