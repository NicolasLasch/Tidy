use super::model::{Rejection, ValidatedAction};
use std::{
    fs,
    path::{Component, Path},
};
use tidy_platform::AuthorizedRoot;

fn check_no_traversal(rel: &Path) -> Result<(), Rejection> {
    if rel.as_os_str().is_empty() {
        return Err(Rejection::OutsideScope);
    }
    for c in rel.components() {
        match c {
            Component::Normal(s) => {
                let name = s.to_string_lossy();
                if name == ".git" {
                    return Err(Rejection::ProtectedPath);
                }
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Rejection::OutsideScope);
            }
            Component::CurDir => return Err(Rejection::OutsideScope),
        }
    }
    Ok(())
}

fn file_modified_secs(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Validates a single proposed action against the authorized root and physical filesystem state.
pub fn validate_move(
    scope: &AuthorizedRoot,
    relative_source: &Path,
    relative_dest: &Path,
) -> Result<ValidatedAction, Rejection> {
    check_no_traversal(relative_source)?;
    check_no_traversal(relative_dest)?;

    let source = scope.path().join(relative_source);
    let destination = scope.path().join(relative_dest);

    // Check source exists and is a regular file (not directory, not symlink)
    let meta = fs::symlink_metadata(&source).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Rejection::ChangedSource(source.display().to_string()),
        std::io::ErrorKind::PermissionDenied => {
            Rejection::PermissionDenied(source.display().to_string())
        }
        _ => Rejection::IoError(e.to_string()),
    })?;

    if meta.file_type().is_symlink() {
        return Err(Rejection::Symlink);
    }
    if !meta.is_file() {
        return Err(Rejection::InvalidAction(format!(
            "Source is not a regular file: {}",
            source.display()
        )));
    }

    // Validate scope boundaries
    scope
        .validate(&source)
        .map_err(|_| Rejection::OutsideScope)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.dev() != scope.identity().0 {
            return Err(Rejection::CrossVolume);
        }
    }

    // Validate destination path. A rename that only changes letter case names the same file on a
    // case-insensitive volume; that is allowed, anything else existing is a collision.
    if let Ok(existing) = fs::symlink_metadata(&destination) {
        #[cfg(unix)]
        let same_file = {
            use std::os::unix::fs::MetadataExt;
            existing.dev() == meta.dev()
                && existing.ino() == meta.ino()
                && relative_source.to_string_lossy().to_lowercase()
                    == relative_dest.to_string_lossy().to_lowercase()
        };
        #[cfg(not(unix))]
        let same_file = false;
        if !same_file {
            return Err(Rejection::Collision(destination.display().to_string()));
        }
    }

    // Verify destination stays within scope
    if !destination.starts_with(scope.path()) {
        return Err(Rejection::OutsideScope);
    }

    // Validate the nearest existing ancestor, including when immediate parents are missing.
    if tidy_platform::protected(&destination) {
        return Err(Rejection::ProtectedPath);
    }
    for parent in destination.ancestors().skip(1) {
        if fs::symlink_metadata(parent).is_ok() {
            scope
                .validate(parent)
                .map_err(|_| Rejection::OutsideScope)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if fs::metadata(parent)
                    .map_err(|e| Rejection::IoError(e.to_string()))?
                    .dev()
                    != scope.identity().0
                {
                    return Err(Rejection::CrossVolume);
                }
            }
            break;
        }
    }
    // Destination parent must be valid within scope if it exists
    if let Some(parent) = destination.parent()
        && parent.exists()
    {
        scope
            .validate(parent)
            .map_err(|_| Rejection::OutsideScope)?;
    }

    Ok(ValidatedAction::Move {
        source,
        destination,
        relative_source: relative_source.to_path_buf(),
        relative_dest: relative_dest.to_path_buf(),
        original_size: meta.len(),
        original_modified: file_modified_secs(&meta),
    })
}

pub fn validate_rename(
    scope: &AuthorizedRoot,
    relative_source: &Path,
    new_name: &str,
) -> Result<ValidatedAction, Rejection> {
    if new_name.is_empty() || new_name.contains(['/', '\\']) || new_name == "." || new_name == ".."
    {
        return Err(Rejection::InvalidAction("Invalid filename".into()));
    }
    let target = relative_source
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(new_name);
    validate_move(scope, relative_source, &target)
}

pub fn validate_trash(
    scope: &AuthorizedRoot,
    relative_source: &Path,
) -> Result<ValidatedAction, Rejection> {
    check_no_traversal(relative_source)?;

    let source = scope.path().join(relative_source);

    let meta = fs::symlink_metadata(&source).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Rejection::ChangedSource(source.display().to_string()),
        std::io::ErrorKind::PermissionDenied => {
            Rejection::PermissionDenied(source.display().to_string())
        }
        _ => Rejection::IoError(e.to_string()),
    })?;

    if meta.file_type().is_symlink() {
        return Err(Rejection::Symlink);
    }

    if !meta.is_file() {
        return Err(Rejection::InvalidAction(
            "Only individually reviewed regular files can be trashed".into(),
        ));
    }
    scope
        .validate(&source)
        .map_err(|_| Rejection::OutsideScope)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.dev() != scope.identity().0 {
            return Err(Rejection::CrossVolume);
        }
    }

    Ok(ValidatedAction::Trash {
        source,
        relative_source: relative_source.to_path_buf(),
        original_size: meta.len(),
        original_modified: file_modified_secs(&meta),
    })
}

/// Validates a whole folder for Trash: a real directory inside the scope, on the same volume,
/// never the scope root, with no Git repository, symlinked entry swap or mount point below it.
pub fn validate_trash_dir(
    scope: &AuthorizedRoot,
    relative_source: &Path,
) -> Result<ValidatedAction, Rejection> {
    check_no_traversal(relative_source)?;
    let source = scope.path().join(relative_source);
    let meta = fs::symlink_metadata(&source).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Rejection::ChangedSource(source.display().to_string()),
        std::io::ErrorKind::PermissionDenied => {
            Rejection::PermissionDenied(source.display().to_string())
        }
        _ => Rejection::IoError(e.to_string()),
    })?;
    if meta.file_type().is_symlink() {
        return Err(Rejection::Symlink);
    }
    if !meta.is_dir() {
        return Err(Rejection::InvalidAction(
            "This action needs a folder".into(),
        ));
    }
    scope
        .validate(&source)
        .map_err(|_| Rejection::OutsideScope)?;
    if tidy_platform::protected(&source) {
        return Err(Rejection::ProtectedPath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.dev() != scope.identity().0 {
            return Err(Rejection::CrossVolume);
        }
        let (_, tree) = tidy_platform::handles::inspect_tree(scope, relative_source)
            .map_err(|e| Rejection::InvalidAction(format!("Folder cannot be trashed: {e}")))?;
        Ok(ValidatedAction::TrashDir {
            source,
            relative_source: relative_source.to_path_buf(),
            original_size: tree.bytes,
            original_modified: file_modified_secs(&meta),
            files: tree.files,
            dirs: tree.dirs,
            read_only: {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o200 == 0
            },
        })
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Err(Rejection::InvalidAction(
            "Folder Trash is unavailable on this platform".into(),
        ))
    }
}

/// A folder that does not exist yet, inside the scope, on the same volume as its nearest parent.
pub fn validate_create_dir(
    scope: &AuthorizedRoot,
    relative: &Path,
) -> Result<ValidatedAction, Rejection> {
    check_no_traversal(relative)?;
    let source = scope.path().join(relative);
    if fs::symlink_metadata(&source).is_ok() {
        return Err(Rejection::Collision(source.display().to_string()));
    }
    if tidy_platform::protected(&source) || !source.starts_with(scope.path()) {
        return Err(Rejection::ProtectedPath);
    }
    for parent in source.ancestors().skip(1) {
        if fs::symlink_metadata(parent).is_ok() {
            scope
                .validate(parent)
                .map_err(|_| Rejection::OutsideScope)?;
            break;
        }
    }
    Ok(ValidatedAction::CreateDir {
        source,
        relative_source: relative.to_path_buf(),
    })
}
/// Whole-folder move or rename: existing real directory, absent destination, never into itself.
pub fn validate_move_dir(
    scope: &AuthorizedRoot,
    relative_source: &Path,
    relative_dest: &Path,
) -> Result<ValidatedAction, Rejection> {
    check_no_traversal(relative_dest)?;
    if relative_dest.starts_with(relative_source) {
        return Err(Rejection::InvalidAction(
            "A folder cannot move into itself".into(),
        ));
    }
    let ValidatedAction::TrashDir {
        source,
        relative_source,
        original_size,
        files,
        dirs,
        read_only,
        ..
    } = validate_trash_dir(scope, relative_source)?
    else {
        unreachable!()
    };
    let destination = scope.path().join(relative_dest);
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(Rejection::Collision(destination.display().to_string()));
    }
    if tidy_platform::protected(&destination) || !destination.starts_with(scope.path()) {
        return Err(Rejection::ProtectedPath);
    }
    for parent in destination.ancestors().skip(1) {
        if fs::symlink_metadata(parent).is_ok() {
            scope
                .validate(parent)
                .map_err(|_| Rejection::OutsideScope)?;
            break;
        }
    }
    Ok(ValidatedAction::MoveDir {
        source,
        destination,
        relative_source,
        relative_dest: relative_dest.to_path_buf(),
        files,
        dirs,
        original_size,
        read_only,
    })
}

pub fn action_fingerprint(
    scope: &AuthorizedRoot,
    action: &ValidatedAction,
) -> Result<String, Rejection> {
    if let ValidatedAction::CreateDir { source, .. } = action {
        return if fs::symlink_metadata(source).is_ok() {
            Err(Rejection::Collision(source.display().to_string()))
        } else {
            Ok("create".into())
        };
    }
    if let ValidatedAction::TrashDir {
        relative_source, ..
    }
    | ValidatedAction::MoveDir {
        relative_source, ..
    } = action
    {
        #[cfg(unix)]
        {
            return tidy_platform::handles::dir_fingerprint_relative(scope, relative_source)
                .map_err(|e| {
                    Rejection::ChangedSource(format!("{}: {e}", relative_source.display()))
                });
        }
        #[cfg(not(unix))]
        return Err(Rejection::InvalidAction(
            "Folder Trash is unavailable on this platform".into(),
        ));
    }
    fingerprint(scope, action.relative_source())
}

pub fn fingerprint(scope: &AuthorizedRoot, relative: &Path) -> Result<String, Rejection> {
    #[cfg(unix)]
    {
        tidy_platform::handles::fingerprint_relative(scope, relative).map_err(|e| {
            Rejection::ChangedSource(format!(
                "{}: {}. Refresh the folder before retrying.",
                relative.display(),
                e
            ))
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (scope, relative);
        Err(Rejection::InvalidAction("Execution is unavailable on this platform until handle-based validation is implemented".into()))
    }
}

pub fn validate_copy(
    scope: &AuthorizedRoot,
    source: &Path,
    dest: &Path,
) -> Result<ValidatedAction, Rejection> {
    let ValidatedAction::Move {
        source,
        destination,
        relative_source,
        relative_dest,
        original_size,
        original_modified,
    } = validate_move(scope, source, dest)?
    else {
        unreachable!()
    };
    Ok(ValidatedAction::Copy {
        source,
        destination,
        relative_source,
        relative_dest,
        original_size,
        original_modified,
    })
}
pub fn validate_permissions(
    scope: &AuthorizedRoot,
    relative: &Path,
    mode: u32,
) -> Result<ValidatedAction, Rejection> {
    // Keep owner read access so source rechecks and undo remain possible. No special bits, ownership or ACL mutation.
    if mode > 0o777 || mode & 0o400 == 0 {
        return Err(Rejection::InvalidAction(
            "Use ordinary Unix permission bits with owner read access (e.g. 600 or 644)".into(),
        ));
    }
    let ValidatedAction::Trash {
        source,
        relative_source,
        original_size,
        original_modified,
    } = validate_trash(scope, relative)?
    else {
        unreachable!()
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;
        if std::fs::symlink_metadata(&source)
            .map_err(|e| Rejection::IoError(e.to_string()))?
            .nlink()
            != 1
        {
            return Err(Rejection::InvalidAction(
                "Permission changes on hard-linked files could affect other paths".into(),
            ));
        }
        let old_mode = std::fs::symlink_metadata(&source)
            .map_err(|e| Rejection::IoError(e.to_string()))?
            .permissions()
            .mode()
            & 0o7777;
        if old_mode & 0o7000 != 0 || old_mode & 0o400 == 0 {
            return Err(Rejection::InvalidAction(
                "Special permission bits require manual management".into(),
            ));
        }
        Ok(ValidatedAction::Permissions {
            source,
            relative_source,
            original_size,
            original_modified,
            old_mode,
            new_mode: mode,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (source, relative_source, original_size, original_modified);
        Err(Rejection::InvalidAction(
            "Unix mode changes are unavailable on this platform".into(),
        ))
    }
}

pub fn fingerprint_dir(scope: &AuthorizedRoot, relative: &Path) -> Result<String, Rejection> {
    #[cfg(unix)]
    {
        tidy_platform::handles::dir_fingerprint_relative(scope, relative)
            .map_err(|e| Rejection::ChangedSource(format!("{}: {e}", relative.display())))
    }
    #[cfg(not(unix))]
    {
        let _ = (scope, relative);
        Err(Rejection::InvalidAction(
            "Unavailable on this platform".into(),
        ))
    }
}
