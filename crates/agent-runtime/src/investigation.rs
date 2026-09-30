//! What the request engine returns to the app: a reviewable plan (actions, folders, listings) plus
//! an explanation. It holds proposals only; nothing here can change a file.
use serde::Serialize;
use tidy_organization::Proposal;

/// One line of “how I got this”.
#[derive(Clone, Serialize)]
pub struct Trace {
    pub label: String,
    pub detail: String,
}

/// A whole indexed folder proposed for the native Trash; approval runs through the safety engine.
#[derive(Clone, Serialize)]
pub struct FolderTarget {
    pub path: String,
    pub files: usize,
    pub bytes: u64,
    /// A short fact shown beside the folder (for example a game version).
    pub note: Option<String>,
}
/// One row of a structured listing, drawn as a folder or file with its size.
#[derive(Clone, Serialize)]
pub struct ListItem {
    pub kind: &'static str,
    pub path: String,
    pub bytes: u64,
    pub files: usize,
    pub note: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct Section {
    pub title: String,
    pub items: Vec<ListItem>,
}
#[derive(Serialize)]
pub struct Source {
    pub id: u64,
    pub path: String,
    pub size: u64,
}
#[derive(Serialize)]
pub struct Investigation {
    pub engine: String,
    /// The kind of work this answered (for example `trash_named_files`).
    pub workflow: Option<String>,
    pub proposal: Proposal,
    pub sources: Vec<Source>,
    pub folders: Vec<FolderTarget>,
    /// The folders are candidates to choose from (unchecked), not a ready-made removal.
    pub pick: bool,
    /// Structured listings (folders and files with sizes) shown as cards instead of text.
    pub sections: Vec<Section>,
    /// The engine found nothing and a smarter fallback should try before giving up.
    pub unresolved: bool,
    pub trace: Vec<Trace>,
    pub clarification: Option<String>,
    pub indexed: usize,
    pub examined: usize,
    pub remaining_matches: usize,
    pub complete: bool,
}
