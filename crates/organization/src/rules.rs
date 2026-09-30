//! A model may translate intent into these bounded selectors, never file operations.
use crate::{FileCandidate, Proposal, ProposedAction};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub destination: String,
    pub extensions: Vec<String>,
    pub name_contains: Vec<String>,
}
/// Preserve an explicitly requested extension hierarchy even if the small model
/// emits a flat destination. Other requests retain their original rule semantics.
pub fn apply_rules_for_request(
    raw: &str,
    request: &str,
    files: &[FileCandidate],
) -> Result<Proposal, String> {
    let text = request
        .to_lowercase()
        .replace("sub folders", "subfolders")
        .replace("sub folder", "subfolder")
        .replace("sub-folders", "subfolders");
    let hierarchy = text.contains("subfolder")
        && ![
            "no subfolder",
            "without subfolder",
            "don't create subfolder",
            "do not create subfolder",
        ]
        .iter()
        .any(|s| text.contains(s))
        && ["png", "jpg", "jpeg", "extension", "file type", "format"]
            .iter()
            .any(|s| text.contains(s));
    if !hierarchy {
        return apply_rules(raw, files);
    }
    let clean = raw
        .trim()
        .strip_prefix("```json")
        .or_else(|| raw.trim().strip_prefix("```"))
        .unwrap_or(raw.trim())
        .trim()
        .trim_end_matches("```")
        .trim();
    let mut set: RuleSet =
        serde_json::from_str(clean).map_err(|e| format!("Invalid grouping rules: {e}"))?;
    let photos_only = text.contains("photo") || text.contains("image");
    const PHOTOS: &[&str] = &[
        "png", "jpg", "jpeg", "heic", "heif", "webp", "gif", "bmp", "tif", "tiff", "avif", "raw",
        "dng", "cr2", "cr3", "nef", "arw",
    ];
    for rule in &mut set.rules {
        if photos_only {
            if rule.extensions.is_empty() {
                return Err("The model did not restrict the request to photo formats. No plan created; try again.".into());
            }
            rule.extensions
                .retain(|e| PHOTOS.contains(&e.trim_start_matches('.').to_lowercase().as_str()));
            if rule.extensions.is_empty() {
                return Err("A proposed rule targeted non-photo files. No plan created.".into());
            }
        }
        let dest = Path::new(&rule.destination);
        if dest.file_name().is_some_and(|n| n == "{EXT}") {
            continue;
        }
        let leaf = dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        let base = if rule
            .extensions
            .iter()
            .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&leaf))
        {
            dest.parent().unwrap_or(Path::new(""))
        } else {
            dest
        };
        if base.as_os_str().is_empty() {
            return Err("A parent folder is required for extension subfolders.".into());
        }
        rule.destination = base.join("{EXT}").to_string_lossy().into_owned();
    }
    apply_rules(
        &serde_json::to_string(&set).map_err(|e| e.to_string())?,
        files,
    )
}
pub fn apply_rules(raw: &str, files: &[FileCandidate]) -> Result<Proposal, String> {
    let raw = raw
        .trim()
        .strip_prefix("```json")
        .or_else(|| raw.trim().strip_prefix("```"))
        .unwrap_or(raw.trim())
        .trim();
    let raw = raw.strip_suffix("```").unwrap_or(raw).trim();
    let set: RuleSet = serde_json::from_str(raw).map_err(|e| format!("Local model returned invalid rules: {e}. Try a shorter request or explicit mapping such as Photos: jpg, png."))?;
    if set.rules.is_empty() || set.rules.len() > 8 {
        return Err(
            "Request needs 1–8 explicit grouping rules. No generic sorting was substituted.".into(),
        );
    }
    let mut descriptions = Vec::new();
    for r in &set.rules {
        let p = Path::new(&r.destination);
        if p.as_os_str().is_empty()
            || r.destination.len() > 160
            || p.components().any(|c| !matches!(c, Component::Normal(_)))
            || (r.destination.contains('{') || r.destination.contains('}'))
                && (p.file_name().is_none_or(|s| s != "{EXT}")
                    || p.parent().is_none_or(|s| {
                        s.as_os_str().is_empty() || s.to_string_lossy().contains(['{', '}'])
                    }))
            || r.destination.contains(['\\', ':', '\0'])
            || p.components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
            || (r.extensions.is_empty() && r.name_contains.is_empty())
            || r.extensions.len() > 30
            || r.name_contains.len() > 12
            || r.extensions
                .iter()
                .chain(&r.name_contains)
                .any(|s| s.trim().is_empty() || s.len() > 80 || s.contains(['*', '/', '\\', '\0']))
        {
            return Err("Request produced an unsafe or overly broad rule. Specify file types or words in filenames and a destination folder.".into());
        }
        descriptions.push(format!(
            "{} ← extensions [{}], filename contains [{}]",
            r.destination,
            r.extensions.join(", "),
            r.name_contains.join(", ")
        ));
    }
    let mut occupied: HashSet<String> = files
        .iter()
        .map(|f| f.relative_path.to_string_lossy().to_lowercase())
        .collect();
    let mut actions = Vec::new();
    let mut matched = 0;
    let mut collisions = 0;
    for f in files {
        let Some(name) = f.relative_path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let lower = name.to_lowercase();
        let ext = f
            .relative_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        for r in &set.rules {
            let template = Path::new(&r.destination);
            let by_extension = template.file_name().is_some_and(|s| s == "{EXT}");
            // Also repair files directly inside the requested parent from an old flat plan.
            let in_flat_parent = by_extension && f.relative_path.parent() == template.parent();
            if f.relative_path.components().count() != 1 && !in_flat_parent {
                continue;
            }
            if (!r.extensions.is_empty()
                && !r
                    .extensions
                    .iter()
                    .any(|s| s.trim_start_matches('.').eq_ignore_ascii_case(&ext)))
                || (!r.name_contains.is_empty()
                    && !r
                        .name_contains
                        .iter()
                        .any(|s| lower.contains(&s.to_lowercase())))
            {
                continue;
            }
            matched += 1;
            let folder = if by_extension {
                if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                    continue;
                }
                template.parent().unwrap().join(ext.to_ascii_uppercase())
            } else {
                PathBuf::from(&r.destination)
            };
            let dest = folder.join(name);
            if occupied.insert(dest.to_string_lossy().to_lowercase()) {
                if actions.len() < 500 {
                    actions.push(ProposedAction::Move {
                        source: f.id,
                        destination_relative: dest,
                    });
                }
            } else {
                collisions += 1;
            }
            break;
        }
    }
    Ok(Proposal {
        rationale: format!(
            "Request translated to rules: {}. Checked {} indexed files; {} eligible matches, {} destination collisions skipped. Showing {} actions (maximum 500 per approval). Unrelated nested folders remain intact. {{EXT}} means an uppercase file-extension subfolder.",
            descriptions.join("; "),
            files.len(),
            matched,
            collisions,
            actions.len()
        ),
        actions,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileId;
    #[test]
    fn rules_use_entire_index_and_filter_names() {
        let files: Vec<_> = (0..40)
            .map(|i| FileCandidate {
                id: FileId(i),
                relative_path: format!("invoice-{i}.pdf").into(),
                size: 1,
                modified: 0,
                excerpt: None,
            })
            .collect();
        let p=apply_rules(r#"{"rules":[{"destination":"Invoices","extensions":["pdf"],"name_contains":["invoice"]}]}"#,&files).unwrap();
        assert_eq!(p.actions.len(), 40);
        assert!(
            apply_rules(
                r#"{"rules":[{"destination":"../bad","extensions":["pdf"],"name_contains":[]}]}"#,
                &files
            )
            .is_err()
        );
        assert!(
            apply_rules(
                r#"{"rules":[{"destination":"All","extensions":[],"name_contains":[]}]}"#,
                &files
            )
            .is_err()
        );
    }
    #[test]
    fn photo_subfolders_preserve_extension_and_exclude_other_files() {
        let files: Vec<_> = [
            "a.PNG",
            "b.jpg",
            "c.jpeg",
            "readme.txt",
            "Photos/d.png",
            "Project/e.png",
        ]
        .iter()
        .enumerate()
        .map(|(i, p)| FileCandidate {
            id: FileId(i as u64),
            relative_path: p.into(),
            size: 1,
            modified: 0,
            excerpt: None,
        })
        .collect();
        let raw = r#"{"rules":[{"destination":"Photos","extensions":["png","jpg","jpeg","txt"],"name_contains":[]}]}"#;
        let p=apply_rules_for_request(raw,"Put all Photos into one folder with sub folder for PNG, JPG etc. Don't touch other files",&files).unwrap();
        let destinations: Vec<_> = p
            .actions
            .iter()
            .map(|a| match a {
                ProposedAction::Move {
                    destination_relative,
                    ..
                } => destination_relative.to_string_lossy().into_owned(),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            destinations,
            [
                "Photos/PNG/a.PNG",
                "Photos/JPG/b.jpg",
                "Photos/JPEG/c.jpeg",
                "Photos/PNG/d.png"
            ]
        );
    }
    #[test]
    fn no_subfolders_keeps_flat_destination() {
        let f = FileCandidate {
            id: FileId(1),
            relative_path: "a.png".into(),
            size: 1,
            modified: 0,
            excerpt: None,
        };
        let raw = r#"{"rules":[{"destination":"Photos","extensions":["png"],"name_contains":[]}]}"#;
        let p = apply_rules_for_request(raw, "Photos without subfolders for PNG", &[f]).unwrap();
        assert!(
            matches!(&p.actions[0],ProposedAction::Move{destination_relative,..} if destination_relative==Path::new("Photos/a.png"))
        );
    }
}
