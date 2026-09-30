//! Native Trash with the actual recovery location returned by the OS.
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
pub fn trash(path: &Path) -> Result<PathBuf, String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let path = path.to_str().ok_or("Non-UTF-8 Trash path is unsupported")?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let mut result = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut result))
        .map_err(|e| e.to_string())?;
    result
        .and_then(|url| url.path())
        .map(|p| PathBuf::from(p.to_string()))
        .ok_or("OS did not return a Trash recovery location".into())
}
#[cfg(not(target_os = "macos"))]
pub fn trash(_path: &Path) -> Result<PathBuf, String> {
    Err("Trash execution is enabled only on macOS until recovery receipts are implemented for this platform".into())
}
