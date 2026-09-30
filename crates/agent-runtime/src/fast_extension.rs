//! Exact extension renames are naming operations; file bytes are never converted.
use crate::investigation::{Investigation, Source, Trace};
use std::sync::atomic::{AtomicBool, Ordering};
use tidy_organization::{FileCandidate, Proposal, ProposedAction};
pub fn target(request: &str) -> Option<String> {
    let lower = request.trim().to_lowercase();
    let lower = lower.strip_prefix("please ").unwrap_or(&lower);
    let tail = lower
        .strip_prefix("change ")
        .or_else(|| lower.strip_prefix("rename "))?;
    let tail = tail
        .strip_prefix("the extensions of ")
        .or_else(|| tail.strip_prefix("extensions of "))
        .unwrap_or(tail);
    let tail = tail
        .strip_prefix("all ")
        .or_else(|| tail.strip_prefix("the "))
        .unwrap_or(tail);
    let tail = tail
        .strip_prefix("text files to ")
        .or_else(|| tail.strip_prefix(".txt files to "))?;
    let ext = tail.trim().trim_start_matches('.');
    if ext.is_empty() || ext.len() > 16 || !ext.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(ext.into())
}
pub fn preview(
    files: &[FileCandidate],
    extension: &str,
    cancel: &AtomicBool,
) -> Result<Investigation, String> {
    let mut eligible: Vec<_> = files
        .iter()
        .filter(|f| {
            crate::fast_trash::is_plain_text(&f.relative_path)
                && !crate::fast_trash::protected(&f.relative_path)
                && f.relative_path.extension() != Some(std::ffi::OsStr::new(extension))
        })
        .collect();
    eligible.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    if cancel.load(Ordering::Relaxed) {
        return Err("Planning cancelled".into());
    }
    let mut targets = std::collections::HashSet::new();
    let occupied: std::collections::HashSet<_> = files
        .iter()
        .map(|f| f.relative_path.to_string_lossy().to_lowercase())
        .collect();
    let mut actions = Vec::new();
    let mut sources = Vec::new();
    for f in eligible.iter().take(500) {
        let dest = f.relative_path.with_extension(extension);
        let key = dest.to_string_lossy().to_lowercase();
        if occupied.contains(&key) || !targets.insert(key) {
            return Err(format!(
                "Extension rename would collide at {}. Resolve the collision before planning.",
                dest.display()
            ));
        }
        actions.push(ProposedAction::Move {
            source: f.id,
            destination_relative: dest,
        });
        sources.push(Source {
            id: f.id.0,
            path: f.relative_path.to_string_lossy().into(),
            size: f.size,
        });
    }
    let remaining = eligible.len().saturating_sub(500);
    Ok(Investigation {
        engine: "local_filter".into(),
        workflow: crate::workflows::get("change_extensions").cloned(),
        proposal: Proposal {
            actions,
            rationale: format!(
                "Rename .txt/.text extensions to .{extension}, recursively in the indexed folder. Keep parent folders and filename stems. Contents remain unchanged: this does not convert text into {extension} data. {} eligible files; {remaining} beyond this reviewed batch. Apply then regenerate if needed. No files changed; approval required.",
                eligible.len()
            ),
        },
        sources,
        folders: vec![],
        trace: vec![Trace {
            label: "Exact extension rename".into(),
            detail: format!(
                "Checked {} indexed paths without loading a model",
                files.len()
            ),
        }],
        clarification: None,
        indexed: files.len(),
        examined: eligible.len(),
        remaining_matches: remaining,
        complete: remaining == 0,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn bounded_patterns() {
        assert_eq!(
            target("Change all text files to .json"),
            Some("json".into())
        );
        for q in [
            "Don't change text files to .json",
            "Convert text files to json",
            "change text files to .json except keep.txt",
            "change text files to ../json",
        ] {
            assert_eq!(target(q), None);
        }
    }
    #[test]
    fn preserves_parent_and_bytes() {
        let f = FileCandidate {
            id: tidy_organization::FileId(1),
            relative_path: Path::new("project/notes.txt").into(),
            size: 48,
            modified: 0,
            excerpt: None,
        };
        let r = preview(&[f], "json", &AtomicBool::new(false)).unwrap();
        assert!(
            matches!(&r.proposal.actions[0],ProposedAction::Move{destination_relative,..} if destination_relative==Path::new("project/notes.json"))
        );
    }
}
