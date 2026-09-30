//! Unix read-only, no-follow directory handles. Never opens an arbitrary descendant path.
#![allow(clippy::unnecessary_cast)] // Stat integer widths differ across Unix targets.
use crate::AuthorizedRoot;
use rustix::fs::{self, AtFlags, Dir, Mode, OFlags, Stat};
use std::{
    ffi::OsStr,
    fs::File,
    io,
    os::fd::OwnedFd,
    path::{Component, Path},
};

pub struct Directory {
    fd: OwnedFd,
    device: u64,
}
fn error(e: rustix::io::Errno) -> io::Error {
    e.into()
}
fn refusal(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}
fn single(name: &OsStr) -> io::Result<()> {
    let mut parts = Path::new(name).components();
    if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
        return Err(refusal("expected a single filename"));
    }
    Ok(())
}
impl Directory {
    pub fn root(root: &AuthorizedRoot) -> io::Result<Self> {
        root.validate(root.path())?;
        let mut fd = fs::open("/", directory_flags(), Mode::empty()).map_err(error)?;
        for component in root.path().components() {
            if let Component::Normal(name) = component {
                reject_git(&fd)?;
                fd = fs::openat(&fd, name, directory_flags(), Mode::empty()).map_err(error)?;
            }
        }
        reject_git(&fd)?;
        let stat = fs::fstat(&fd).map_err(error)?;
        if (stat.st_dev as u64, stat.st_ino as u64) != root.identity() {
            return Err(refusal("selected directory changed since authorization"));
        }
        Ok(Self {
            device: stat.st_dev as u64,
            fd,
        })
    }
    pub fn entries(&self) -> io::Result<Dir> {
        Dir::read_from(&self.fd).map_err(error)
    }
    pub fn stat(&self, name: &OsStr) -> io::Result<Stat> {
        single(name)?;
        fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)
    }
    pub fn child(&self, name: &OsStr, expected: &Stat) -> io::Result<Self> {
        single(name)?;
        reject_git(&self.fd)?;
        let fd = fs::openat(&self.fd, name, directory_flags(), Mode::empty()).map_err(error)?;
        let actual = fs::fstat(&fd).map_err(error)?;
        self.check_identity(expected, &actual)?;
        reject_git(&fd)?;
        Ok(Self {
            fd,
            device: self.device,
        })
    }
    pub fn file(&self, name: &OsStr, expected: &Stat) -> io::Result<File> {
        single(name)?;
        reject_git(&self.fd)?;
        if !regular(expected) || placeholder(expected) {
            return Err(refusal("special file or offline placeholder"));
        }
        let fd = fs::openat(
            &self.fd,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(error)?;
        let actual = fs::fstat(&fd).map_err(error)?;
        self.check_identity(expected, &actual)?;
        if !regular(&actual) || placeholder(&actual) || !unchanged(expected, &actual) {
            return Err(refusal("file changed before read"));
        }
        Ok(File::from(fd))
    }
    fn check_identity(&self, expected: &Stat, actual: &Stat) -> io::Result<()> {
        if actual.st_dev as u64 != self.device
            || actual.st_dev != expected.st_dev
            || actual.st_ino != expected.st_ino
        {
            return Err(refusal("mount boundary or replaced entry"));
        }
        Ok(())
    }
    pub fn same_device(&self, stat: &Stat) -> bool {
        stat.st_dev as u64 == self.device
    }
}
fn reject_git(fd: &OwnedFd) -> io::Result<()> {
    match fs::statat(fd, ".git", AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Err(refusal("Git repository boundary")),
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(e) => Err(error(e)),
    }
}
pub fn regular(stat: &Stat) -> bool {
    fs::FileType::from_raw_mode(stat.st_mode) == fs::FileType::RegularFile
}
pub fn directory(stat: &Stat) -> bool {
    fs::FileType::from_raw_mode(stat.st_mode) == fs::FileType::Directory
}
pub fn symlink(stat: &Stat) -> bool {
    fs::FileType::from_raw_mode(stat.st_mode) == fs::FileType::Symlink
}
pub fn placeholder(stat: &Stat) -> bool {
    #[cfg(target_os = "macos")]
    {
        stat.st_flags & 0x4000_0000 != 0
    } // SF_DATALESS: avoid opening / hydrating cloud placeholders.
    #[cfg(not(target_os = "macos"))]
    {
        let _ = stat;
        false
    }
}
pub fn unchanged(a: &Stat, b: &Stat) -> bool {
    a.st_dev == b.st_dev
        && a.st_ino == b.st_ino
        && a.st_size == b.st_size
        && a.st_mtime == b.st_mtime
        && a.st_mtime_nsec == b.st_mtime_nsec
        && a.st_ctime == b.st_ctime
        && a.st_ctime_nsec == b.st_ctime_nsec
}
pub fn stat_file(file: &File) -> io::Result<Stat> {
    fs::fstat(file).map_err(error)
}

/// Reopen each parent without following symlinks. Directory creation is only used
/// for destinations explicitly approved by the user.
fn parent_for(
    root: &AuthorizedRoot,
    relative: &Path,
    create: bool,
) -> io::Result<(Directory, std::ffi::OsString)> {
    let parts: Vec<_> = relative.components().collect();
    if parts.is_empty() || parts.iter().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(refusal("invalid relative path"));
    }
    let mut parent = Directory::root(root)?;
    for part in &parts[..parts.len() - 1] {
        let name = part.as_os_str();
        if create {
            match parent.stat(name) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    reject_git(&parent.fd)?;
                    fs::mkdirat(&parent.fd, name, Mode::from_raw_mode(0o700)).map_err(error)?;
                }
                Err(e) => return Err(e),
                _ => {}
            }
        }
        let stat = parent.stat(name)?;
        parent = parent.child(name, &stat)?;
    }
    Ok((parent, parts.last().unwrap().as_os_str().to_owned()))
}
fn stamp(stat: &Stat) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        stat.st_dev,
        stat.st_ino,
        stat.st_size,
        stat.st_mtime,
        stat.st_mtime_nsec,
        stat.st_ctime,
        stat.st_ctime_nsec
    )
}
pub fn fingerprint_relative(root: &AuthorizedRoot, relative: &Path) -> io::Result<String> {
    if crate::protected(&root.path().join(relative)) {
        return Err(refusal("protected path"));
    }
    let (parent, name) = parent_for(root, relative, false)?;
    let stat = parent.stat(&name)?;
    let file = parent.file(&name, &stat)?;
    Ok(stamp(&stat_file(&file)?))
}
/// Totals for a directory tree inspected without following symlinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeSummary {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
}
const TREE_ENTRY_LIMIT: u64 = 5_000_000;
/// Walks a directory with handle-relative, no-follow opens. Any Git repository, mount point
/// or replaced entry inside the tree is refused, so a whole-folder Trash never crosses them.
pub fn inspect_tree(root: &AuthorizedRoot, relative: &Path) -> io::Result<(Stat, TreeSummary)> {
    use std::os::unix::ffi::OsStrExt;
    if crate::protected(&root.path().join(relative)) {
        return Err(refusal("protected path"));
    }
    let (parent, name) = parent_for(root, relative, false)?;
    let top = parent.stat(&name)?;
    if !directory(&top) || !parent.same_device(&top) {
        return Err(refusal("not a directory on the same volume"));
    }
    let mut summary = TreeSummary {
        files: 0,
        dirs: 1,
        bytes: 0,
    };
    let first = parent.child(&name, &top)?;
    let entries = first.entries()?;
    let mut stack = vec![(first, entries)];
    while let Some((dir, entries)) = stack.last_mut() {
        let Some(entry) = entries.next() else {
            stack.pop();
            continue;
        };
        let entry = entry.map_err(error)?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        if summary.files + summary.dirs > TREE_ENTRY_LIMIT {
            return Err(refusal("folder has too many entries for one Trash action"));
        }
        let child_name = OsStr::from_bytes(raw);
        let stat = dir.stat(child_name)?;
        if directory(&stat) {
            summary.dirs += 1;
            let child = dir.child(child_name, &stat)?;
            let child_entries = child.entries()?;
            stack.push((child, child_entries));
        } else {
            summary.files += 1;
            if regular(&stat) {
                summary.bytes += stat.st_size as u64;
            }
        }
    }
    Ok((top, summary))
}
/// Fingerprint of a directory and everything below it: identity plus entry and byte totals.
pub fn dir_fingerprint_relative(root: &AuthorizedRoot, relative: &Path) -> io::Result<String> {
    let (top, tree) = inspect_tree(root, relative)?;
    Ok(format!(
        "dir:{}:{}:{}:{}:{}",
        top.st_dev, top.st_ino, tree.files, tree.dirs, tree.bytes
    ))
}
/// Atomic no-replace rename; never falls back to overwrite or copy-and-delete.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn move_no_replace(
    root: &AuthorizedRoot,
    source: &Path,
    destination: &Path,
    expected: &str,
) -> io::Result<String> {
    if crate::protected(&root.path().join(destination)) {
        return Err(refusal("protected destination"));
    }
    let (from, name) = parent_for(root, source, false)?;
    let before = from.stat(&name)?;
    let _file = from.file(&name, &before)?;
    if stamp(&before) != expected {
        return Err(refusal("source changed since approval"));
    }
    let (to, target) = parent_for(root, destination, true)?;
    reject_git(&from.fd)?;
    reject_git(&to.fd)?;
    if stamp(&from.stat(&name)?) != expected {
        return Err(refusal("source changed before move"));
    }
    fs::renameat_with(&from.fd, &name, &to.fd, &target, fs::RenameFlags::NOREPLACE)
        .map_err(error)?;
    let after = to.stat(&target)?;
    if before.st_dev != after.st_dev
        || before.st_ino != after.st_ino
        || before.st_size != after.st_size
        || before.st_mtime != after.st_mtime
        || before.st_mtime_nsec != after.st_mtime_nsec
    {
        return Err(refusal("moved file changed; recovery required"));
    }
    fs::fsync(&from.fd).map_err(error)?;
    fs::fsync(&to.fd).map_err(error)?;
    Ok(stamp(&after))
}

/// Prevent two current Tidy processes from executing/recovering the same journal.
pub fn exclusive_lock(path: &Path) -> io::Result<File> {
    let fd = fs::open(
        path,
        OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(error)?;
    fs::flock(&fd, fs::FlockOperation::NonBlockingLockExclusive).map_err(error)?;
    Ok(File::from(fd))
}

/// Create only an approved, absent destination. On interruption leave it for recovery; never unlink.
pub fn copy_no_replace(
    root: &AuthorizedRoot,
    source: &Path,
    destination: &Path,
    expected: &str,
) -> io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    if crate::protected(&root.path().join(destination)) {
        return Err(refusal("protected destination"));
    }
    let (from, name) = parent_for(root, source, false)?;
    let before = from.stat(&name)?;
    let mut input = from.file(&name, &before)?;
    if stamp(&before) != expected {
        return Err(refusal("source changed since approval"));
    }
    let (to, target) = parent_for(root, destination, true)?;
    reject_git(&from.fd)?;
    reject_git(&to.fd)?;
    let fd = fs::openat(
        &to.fd,
        &target,
        OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(error)?;
    let mut output = File::from(fd);
    let count = std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    if count != before.st_size as u64
        || stamp(&stat_file(&input)?) != expected
        || stamp(&from.stat(&name)?) != expected
    {
        return Err(refusal(
            "source changed during copy; partial copy needs recovery",
        ));
    }
    input.seek(SeekFrom::Start(0))?;
    output.seek(SeekFrom::Start(0))?;
    let mut left = [0u8; 65536];
    let mut right = [0u8; 65536];
    loop {
        let n = input.read(&mut left)?;
        if n == 0 {
            break;
        }
        output.read_exact(&mut right[..n])?;
        if left[..n] != right[..n] {
            return Err(refusal("copy verification failed"));
        }
    }
    if stamp(&stat_file(&input)?) != expected {
        return Err(refusal("source changed during verification"));
    }
    let after = stat_file(&output)?;
    let named = to.stat(&target)?;
    if stamp(&after) != stamp(&named) || after.st_size != before.st_size {
        return Err(refusal("copy destination changed"));
    }
    fs::fsync(&to.fd).map_err(error)?;
    Ok(stamp(&after))
}
pub fn set_permissions(
    root: &AuthorizedRoot,
    relative: &Path,
    expected: &str,
    old_mode: u32,
    new_mode: u32,
) -> io::Result<String> {
    if new_mode > 0o777 || new_mode & 0o400 == 0 {
        return Err(refusal("unsafe permission mode"));
    }
    let (parent, name) = parent_for(root, relative, false)?;
    let before = parent.stat(&name)?;
    let file = parent.file(&name, &before)?;
    if before.st_nlink != 1
        || stamp(&before) != expected
        || before.st_mode as u32 & 0o7777 != old_mode
    {
        return Err(refusal("source or permissions changed since approval"));
    }
    reject_git(&parent.fd)?;
    fs::fchmod(&file, Mode::from_raw_mode(new_mode as _)).map_err(error)?;
    file.sync_all()?;
    let after = stat_file(&file)?;
    if after.st_nlink != 1
        || after.st_mode as u32 & 0o7777 != new_mode
        || after.st_ino != before.st_ino
        || after.st_size != before.st_size
        || after.st_mtime != before.st_mtime
        || after.st_mtime_nsec != before.st_mtime_nsec
        || stamp(&after) != stamp(&parent.stat(&name)?)
    {
        return Err(refusal("permission result changed; recovery required"));
    }
    Ok(stamp(&after))
}
