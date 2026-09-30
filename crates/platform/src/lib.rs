//! Conservative path policy. No mutation or shell APIs.
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

/// An explicit, process-local read grant. Construct only from a user's selection.
/// This is not an OS capability or a race-proof directory handle.
#[derive(Debug, Clone)]
pub struct AuthorizedRoot(PathBuf, #[cfg(unix)] (u64, u64));
impl AuthorizedRoot {
    pub fn authorize(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(denied("select an absolute path without parent traversal"));
        }
        reject_links(path)?;
        let canonical = fs::canonicalize(path).map_err(|e| io_context("resolve path", path, e))?;
        if !canonical.is_dir() || protected(&canonical) {
            return Err(denied("protected path or not a directory"));
        }
        for ancestor in canonical.ancestors() {
            if git_marker(ancestor)? {
                return Err(denied("Git repository boundary"));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = fs::symlink_metadata(&canonical)?;
            Ok(Self(canonical, (metadata.dev(), metadata.ino())))
        }
        #[cfg(not(unix))]
        {
            Ok(Self(canonical))
        }
    }
    #[cfg(unix)]
    pub fn identity(&self) -> (u64, u64) {
        self.1
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn validate(&self, path: &Path) -> io::Result<()> {
        if !path.starts_with(&self.0) || protected(path) {
            return Err(denied("outside authorized scope or protected"));
        }
        reject_links(path)?;
        if !fs::canonicalize(path)
            .map_err(|e| io_context("resolve path", path, e))?
            .starts_with(&self.0)
        {
            return Err(denied("resolved path outside scope"));
        }
        // Recheck ancestors, including repositories created after authorization.
        for ancestor in path.ancestors() {
            if git_marker(ancestor)? {
                return Err(denied("Git repository boundary"));
            }
        }
        Ok(())
    }
}
fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
pub fn git_marker(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path.join(".git")) {
        Ok(_) => Ok(true), // includes worktree .git files and broken links
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(io_context("inspect Git marker", &path.join(".git"), e)),
    }
}
fn reject_links(path: &Path) -> io::Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        #[cfg(windows)]
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(&current).map_err(|e| io_context("inspect path", &current, e))?;
        if metadata.file_type().is_symlink() {
            return Err(denied("symlink boundary"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(denied("reparse point boundary"));
            }
        }
    }
    Ok(())
}
pub fn protected(path: &Path) -> bool {
    if path.parent().is_none() || path == Path::new("/private") {
        return true;
    }
    #[cfg(unix)]
    {
        for root in [
            "/System",
            "/Library",
            "/Applications",
            "/bin",
            "/sbin",
            "/usr",
            "/etc",
            "/private/etc",
            "/private/var",
            "/dev",
            "/proc",
            "/sys",
            "/var",
            "/boot",
            "/root",
            "/run",
        ] {
            if path.starts_with(root) {
                return true;
            }
        }
        // Avoid alternate paths to macOS system data through the APFS mount.
        if path.starts_with("/Volumes") {
            return true;
        }
    }
    #[cfg(windows)]
    {
        let native = path.to_string_lossy().to_lowercase();
        // canonicalize returns verbatim local paths on Windows.
        let lower = native.strip_prefix(r"\\?\").unwrap_or(&native);
        if lower.starts_with(r"unc\") || lower.starts_with(r"\\") {
            return true;
        }
        for variable in [
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
        ] {
            if let Some(value) = std::env::var_os(variable) {
                let root = value.to_string_lossy().to_lowercase();
                if lower == root.as_str() || lower.starts_with(&(root + r"\")) {
                    return true;
                }
            }
        }
    }
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        matches!(
            name.as_ref(),
            ".git" | ".ssh" | ".gnupg" | ".Trash" | ".Trashes"
        ) || cfg!(target_os = "macos") && name == "Library"
    })
}

/// Adds operation/path context while retaining the original OS error as a source.
#[derive(Debug)]
struct PathError {
    operation: &'static str,
    path: PathBuf,
    source: io::Error,
}
impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {:?}: {}", self.operation, self.path, self.source)
    }
}
impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}
pub fn io_context(operation: &'static str, path: &Path, source: io::Error) -> io::Error {
    io::Error::new(
        source.kind(),
        PathError {
            operation,
            path: path.to_path_buf(),
            source,
        },
    )
}
/// Distinguishes OS access failures from TIDY's own policy refusals.
pub fn os_access_denied(error: &io::Error) -> bool {
    if let Some(context) = error.get_ref().and_then(|e| e.downcast_ref::<PathError>()) {
        return os_access_denied(&context.source);
    }
    error.raw_os_error().is_some() && error.kind() == io::ErrorKind::PermissionDenied
}
#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[test]
    fn context_preserves_os_permission_failure() {
        let error = io_context(
            "read directory",
            Path::new("/selected"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        );
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("/selected"));
        assert!(error.to_string().contains("read directory"));
    }
    #[cfg(unix)]
    #[test]
    fn permission_advice_is_only_for_os_errors() {
        let error = io_context(
            "inspect path",
            Path::new("/selected"),
            io::Error::from_raw_os_error(1),
        );
        assert!(os_access_denied(&error));
        assert!(!os_access_denied(&denied("Git repository boundary")));
        assert!(!os_access_denied(&io::Error::from_raw_os_error(2)));
    }
}

#[cfg(unix)]
pub mod handles;

/// Read-only volume capacity, independent of indexed file logical lengths.
#[derive(Debug, Clone)]
pub struct VolumeSpace {
    pub identity: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub available_bytes: u64,
}
pub fn volume_space(path: &Path) -> io::Result<Option<VolumeSpace>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let stat = rustix::fs::statvfs(path)?;
        let unit = if stat.f_frsize == 0 {
            stat.f_bsize
        } else {
            stat.f_frsize
        };
        Ok(Some(VolumeSpace {
            identity: fs::metadata(path)?.dev().to_string(),
            total_bytes: stat.f_blocks.saturating_mul(unit),
            free_bytes: stat.f_bfree.saturating_mul(unit),
            available_bytes: stat.f_bavail.saturating_mul(unit),
        }))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(None)
    }
}
