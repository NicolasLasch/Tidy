//! Exact request patterns avoid model latency; no fuzzy interpretation or execution.
use crate::investigation::{Investigation, Source, Trace};
use std::{
    collections::HashSet,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use tidy_organization::{FileCandidate, Proposal, ProposedAction};
fn words(request: &str) -> Vec<String> {
    request
        .to_lowercase()
        .replace('’', "'")
        .split_whitespace()
        .map(|s| {
            s.trim_matches(|c: char| matches!(c, '.' | '!' | '?' | ',' | ';'))
                .to_string()
        })
        .collect()
}
fn target_tail(request: &str) -> Option<Vec<String>> {
    let mut w = words(request);
    if w.first().is_some_and(|s| s == "please") {
        w.remove(0);
    }
    if w.starts_with(&["can".into(), "you".into()])
        || w.starts_with(&["could".into(), "you".into()])
    {
        w.drain(..2);
    }
    if w.first().is_some_and(|s| s == "please") {
        w.remove(0);
    }
    if !w
        .first()
        .is_some_and(|s| matches!(s.as_str(), "remove" | "delete" | "trash"))
    {
        return None;
    }
    w.remove(0);
    while w
        .first()
        .is_some_and(|s| matches!(s.as_str(), "all" | "the" | "my" | "these" | "only"))
    {
        w.remove(0);
    }
    if w.first().is_some_and(|s| s == "plain") {
        w.remove(0);
    }
    if w.len() < 2 || !matches!(w[0].as_str(), "text" | "txt" | "text/plain") || w[1] != "files" {
        return None;
    }
    Some(w[2..].to_vec())
}
pub fn requests_text_files(request: &str) -> bool {
    target_tail(request).is_some()
}
pub fn plain_text_removal(request: &str, workflow: Option<&str>) -> bool {
    if workflow.is_some_and(|id| !matches!(id, "trash_named_files" | "trash_filtered_files")) {
        return false;
    }
    let Some(tail) = target_tail(request) else {
        return false;
    };
    let mut tail = tail;
    if tail
        .first()
        .is_some_and(|s| matches!(s.as_str(), "from" | "in"))
    {
        tail.remove(0);
        if tail
            .first()
            .is_some_and(|s| matches!(s.as_str(), "this" | "my" | "the" | "selected"))
        {
            tail.remove(0);
        }
        if tail.first().is_none_or(|s| s != "folder") {
            return false;
        }
        tail.remove(0);
    }
    if tail.first().is_some_and(|s| s == "only") {
        tail.remove(0);
    }
    let remainder = tail.join(" ");
    matches!(
        remainder.as_str(),
        "" | "don't touch other files"
            | "do not touch other files"
            | "leave other files alone"
            | "leave other files untouched"
    )
}
pub fn is_plain_text(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "txt" | "text"))
}
pub(crate) fn protected(path: &Path) -> bool {
    path.components().any(|c| {
        let p = c.as_os_str().to_string_lossy().to_lowercase();
        matches!(p.as_str(), ".git" | "node_modules" | "target" | ".venv")
            || p.ends_with(".app")
            || p.ends_with(".framework")
    })
}
pub fn preview(
    files: &[FileCandidate],
    previous: &[ProposedAction],
    cancel: &AtomicBool,
) -> Result<Investigation, String> {
    let mut matches: Vec<_> = files
        .iter()
        .filter(|f| is_plain_text(&f.relative_path) && !protected(&f.relative_path))
        .collect();
    matches.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let valid: HashSet<_> = matches.iter().map(|f| f.id.0).collect();
    if previous
        .iter()
        .any(|a| !matches!(a,ProposedAction::Trash{source} if valid.contains(&source.0)))
    {
        return Err(
            "Previous preview no longer matches the text-file request. Start a new preview.".into(),
        );
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Planning cancelled. No files changed.".into());
    }
    let examined = matches.len();
    let sources: Vec<_> = matches
        .iter()
        .take(500)
        .map(|f| Source {
            id: f.id.0,
            path: f.relative_path.to_string_lossy().into(),
            size: f.size,
        })
        .collect();
    let actions = matches
        .iter()
        .take(500)
        .map(|f| ProposedAction::Trash { source: f.id })
        .collect();
    let remaining = examined.saturating_sub(500);
    let rationale = format!(
        "Exact local filter: .txt and .text files in the selected indexed folder and its subfolders. {examined} eligible matches; {} proposed for native Trash. Other types, Git paths, application bundles and dependency/build folders are excluded. {} No model was loaded. No files changed; approval is required.",
        sources.len(),
        if remaining > 0 {
            format!(
                "{remaining} matches remain beyond this batch; apply this reviewed batch and generate another preview."
            )
        } else {
            String::new()
        }
    );
    Ok(Investigation {
        engine: "local_filter".into(),
        workflow: crate::workflows::get("trash_filtered_files").cloned(),
        proposal: Proposal { actions, rationale },
        sources,
        folders: vec![],
        trace: vec![Trace {
            label: "Exact text-file filter".into(),
            detail: format!(
                "Checked all {} indexed paths; {examined} eligible .txt/.text files. Model loading skipped.",
                files.len()
            ),
        }],
        clarification: None,
        indexed: files.len(),
        examined,
        remaining_matches: remaining,
        complete: remaining == 0,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use tidy_organization::FileId;
    fn file(id: u64, p: &str) -> FileCandidate {
        FileCandidate {
            id: FileId(id),
            relative_path: p.into(),
            size: 10,
            modified: 0,
            excerpt: None,
        }
    }
    #[test]
    fn exact_patterns_not_negation_or_extra_conditions() {
        for q in [
            "Remove the text files from my folder",
            "remove all .txt files",
            "Please delete plain text files. Don't touch other files",
        ] {
            assert!(plain_text_removal(q, None), "{q}");
        }
        for q in [
            "Don't remove text files",
            "Remove text files except notes.txt",
            "Remove text files older than 2020",
            "Remove copy.txt",
            "Remove text files and photos",
        ] {
            assert!(!plain_text_removal(q, None), "{q}");
        }
    }
    #[test]
    fn all_paths_checked_other_types_and_bundles_untouched() {
        let files = vec![
            file(1, "copy.txt"),
            file(2, "project/notes.TEXT"),
            file(3, "a.png"),
            file(4, "node_modules/license.txt"),
            file(5, "Thing.app/readme.txt"),
        ];
        let r = preview(&files, &[], &AtomicBool::new(false)).unwrap();
        assert_eq!(r.proposal.actions.len(), 2);
        assert!(r.complete);
        assert_eq!(r.engine, "local_filter");
    }
    #[test]
    fn batches_disclose_remaining_matches() {
        let files: Vec<_> = (0..501).map(|id| file(id, &format!("{id}.txt"))).collect();
        let r = preview(&files, &[], &AtomicBool::new(false)).unwrap();
        assert_eq!(r.sources.len(), 500);
        assert_eq!(r.remaining_matches, 1);
        assert!(!r.complete);
    }
}
