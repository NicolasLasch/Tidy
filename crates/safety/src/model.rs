use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalState {
    Prepared,
    Approved,
    Applying,
    Applied,
    Verified,
    NeedsRecovery,
    Undone,
}

impl std::fmt::Display for JournalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prepared => write!(f, "prepared"),
            Self::Approved => write!(f, "approved"),
            Self::Applying => write!(f, "applying"),
            Self::Applied => write!(f, "applied"),
            Self::Verified => write!(f, "verified"),
            Self::NeedsRecovery => write!(f, "needs_recovery"),
            Self::Undone => write!(f, "undone"),
        }
    }
}

impl std::str::FromStr for JournalState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "prepared" => Ok(Self::Prepared),
            "approved" => Ok(Self::Approved),
            "applying" => Ok(Self::Applying),
            "applied" => Ok(Self::Applied),
            "verified" => Ok(Self::Verified),
            "needs_recovery" => Ok(Self::NeedsRecovery),
            "undone" => Ok(Self::Undone),
            other => Err(format!("Unknown journal state: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    OutsideScope,
    ProtectedPath,
    Symlink,
    Collision(String),
    ChangedSource(String),
    StaleApproval,
    ExpiredApproval,
    PermissionDenied(String),
    CrossVolume,
    InvalidAction(String),
    IoError(String),
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideScope => write!(f, "Path is outside the authorized scope"),
            Self::ProtectedPath => write!(
                f,
                "Path targets or contains a protected file or Git repository"
            ),
            Self::Symlink => write!(
                f,
                "Symlink boundary detected; symbolic links are not permitted"
            ),
            Self::Collision(target) => write!(
                f,
                "Destination already exists; refusing to overwrite: {target}"
            ),
            Self::ChangedSource(src) => {
                write!(f, "Source file was modified, moved, or missing: {src}")
            }
            Self::StaleApproval => write!(
                f,
                "Approval token has already been consumed (one-use protection)"
            ),
            Self::ExpiredApproval => write!(f, "Approval token has expired"),
            Self::PermissionDenied(msg) => write!(f, "Permission denied: {msg}"),
            Self::CrossVolume => write!(
                f,
                "Cross-volume move detected; only same-volume atomic operations are permitted"
            ),
            Self::InvalidAction(msg) => write!(f, "Invalid action: {msg}"),
            Self::IoError(msg) => write!(f, "Filesystem error: {msg}"),
        }
    }
}

impl std::error::Error for Rejection {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValidatedAction {
    Move {
        source: PathBuf,
        destination: PathBuf,
        relative_source: PathBuf,
        relative_dest: PathBuf,
        original_size: u64,
        original_modified: i64,
    },
    Rename {
        source: PathBuf,
        destination: PathBuf,
        relative_source: PathBuf,
        relative_dest: PathBuf,
        original_size: u64,
        original_modified: i64,
    },
    Copy {
        source: PathBuf,
        destination: PathBuf,
        relative_source: PathBuf,
        relative_dest: PathBuf,
        original_size: u64,
        original_modified: i64,
    },
    Permissions {
        source: PathBuf,
        relative_source: PathBuf,
        original_size: u64,
        original_modified: i64,
        old_mode: u32,
        new_mode: u32,
    },
    Trash {
        source: PathBuf,
        relative_source: PathBuf,
        original_size: u64,
        original_modified: i64,
    },
    /// A whole folder moved to the native Trash as one reviewed action.
    TrashDir {
        source: PathBuf,
        relative_source: PathBuf,
        original_size: u64,
        original_modified: i64,
        files: u64,
        dirs: u64,
    },
}

impl ValidatedAction {
    pub fn source(&self) -> &PathBuf {
        match self {
            Self::Move { source, .. } => source,
            Self::Rename { source, .. } => source,
            Self::Trash { source, .. }
            | Self::TrashDir { source, .. }
            | Self::Copy { source, .. }
            | Self::Permissions { source, .. } => source,
        }
    }

    pub fn relative_source(&self) -> &PathBuf {
        match self {
            Self::Move {
                relative_source, ..
            } => relative_source,
            Self::Rename {
                relative_source, ..
            } => relative_source,
            Self::Trash {
                relative_source, ..
            }
            | Self::TrashDir {
                relative_source, ..
            }
            | Self::Copy {
                relative_source, ..
            }
            | Self::Permissions {
                relative_source, ..
            } => relative_source,
        }
    }

    pub fn relative_dest(&self) -> Option<&PathBuf> {
        match self {
            Self::Move { relative_dest, .. } => Some(relative_dest),
            Self::Rename { relative_dest, .. } => Some(relative_dest),
            Self::Copy { relative_dest, .. } => Some(relative_dest),
            Self::Trash { .. } | Self::TrashDir { .. } | Self::Permissions { .. } => None,
        }
    }

    pub fn original_size(&self) -> u64 {
        match self {
            Self::Move { original_size, .. } => *original_size,
            Self::Rename { original_size, .. } => *original_size,
            Self::Trash { original_size, .. }
            | Self::TrashDir { original_size, .. }
            | Self::Copy { original_size, .. }
            | Self::Permissions { original_size, .. } => *original_size,
        }
    }

    pub fn action_type_str(&self) -> &'static str {
        match self {
            Self::Move { .. } => "move",
            Self::Rename { .. } => "rename",
            Self::Trash { .. } => "trash",
            Self::TrashDir { .. } => "trash_dir",
            Self::Copy { .. } => "copy",
            Self::Permissions { .. } => "permissions",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub transaction_id: i64,
    pub tx_uuid: String,
    pub actions_applied: usize,
    pub verified: bool,
    pub duration_ms: u64,
    pub rationale: String,
}
