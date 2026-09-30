pub mod rules;
// Deterministic organization planning engine.
// Pure planning: produces proposals with FileId references and relative destinations.
// Never modifies the filesystem directly.
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationMode {
    Project,
    Category,
    Date,
    Custom,
}

/// IDs are opaque references into an authorized index, never model-supplied paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProposedAction {
    Move {
        source: FileId,
        destination_relative: PathBuf,
    },
    Rename {
        source: FileId,
        new_name: String,
    },
    Trash {
        source: FileId,
    },
    Copy {
        source: FileId,
        destination_relative: PathBuf,
    },
    Permissions {
        source: FileId,
        mode: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub actions: Vec<ProposedAction>,
    pub rationale: String,
}

#[derive(Debug, Clone)]
pub struct FileCandidate {
    pub id: FileId,
    pub relative_path: PathBuf,
    pub size: u64,
    pub modified: i64,
    pub excerpt: Option<String>,
}

fn extension_category(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "pdf" | "doc" | "docx" | "txt" | "rtf" | "odt" | "pages" | "epub" | "md" => {
            Some("Documents")
        }
        "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp" | "heic" | "tiff" | "bmp" | "raw"
        | "ico" => Some("Images"),
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "aiff" => Some("Audio"),
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "flv" | "wmv" | "m4v" => Some("Video"),
        "zip" | "tar" | "gz" | "7z" | "rar" | "bz2" | "xz" | "tgz" | "zst" => Some("Archives"),
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "cpp" | "h" | "hpp" | "go" | "java"
        | "html" | "css" | "json" | "toml" | "yaml" | "yml" | "sh" | "sql" | "swift" | "kt" => {
            Some("Code")
        }
        "xls" | "xlsx" | "csv" | "tsv" | "numbers" | "ods" => Some("Spreadsheets"),
        "ppt" | "pptx" | "key" | "odp" => Some("Presentations"),
        "figma" | "sketch" | "ai" | "psd" | "xd" => Some("Design"),
        _ => None,
    }
}

pub fn civil_from_unix_seconds(seconds: i64) -> Option<(i32, u32, u32)> {
    if seconds <= 0 {
        return None;
    }
    let days = seconds / 86400;
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1024 + doe / 1461 - doe / 142408) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    Some((y as i32, m, d))
}

fn path_contains_git(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == ".git")
}

/// Generates an organization proposal for a set of candidate files.
/// Ambiguous files remain unassigned. Existing file locations and destination
/// collisions are respected to prevent accidental overwrite or confusion.
pub fn propose(mode: OrganizationMode, param: Option<&str>, files: &[FileCandidate]) -> Proposal {
    match mode {
        OrganizationMode::Category => propose_category(files),
        OrganizationMode::Date => propose_date(files),
        OrganizationMode::Project => propose_project(param.unwrap_or(""), files),
        OrganizationMode::Custom => propose_custom(param.unwrap_or(""), files),
    }
}

fn propose_category(files: &[FileCandidate]) -> Proposal {
    let mut actions = Vec::new();
    let occupied_destinations: HashSet<PathBuf> =
        files.iter().map(|f| f.relative_path.clone()).collect();
    let mut proposed_targets: HashSet<PathBuf> = HashSet::new();

    let mut categorized_count = 0usize;
    let mut unassigned_count = 0usize;
    let mut already_in_place = 0usize;

    for file in files {
        if file.relative_path.components().count() != 1 {
            continue;
        }
        let Some(file_name) = file.relative_path.file_name() else {
            unassigned_count += 1;
            continue;
        };
        let Some(ext) = file.relative_path.extension().and_then(|e| e.to_str()) else {
            unassigned_count += 1;
            continue;
        };
        let Some(category) = extension_category(ext) else {
            // Ambiguous/unknown extension remains unassigned
            unassigned_count += 1;
            continue;
        };

        let target = PathBuf::from(category).join(file_name);

        // Check if file is already at the target destination
        if file.relative_path == target {
            already_in_place += 1;
            continue;
        }

        // Collision check: if target destination already exists in the folder
        // or has already been targeted by another proposed move, do not collide!
        if occupied_destinations.contains(&target) || proposed_targets.contains(&target) {
            unassigned_count += 1;
            continue;
        }

        proposed_targets.insert(target.clone());
        actions.push(ProposedAction::Move {
            source: file.id,
            destination_relative: target,
        });
        categorized_count += 1;
    }

    let rationale = format!(
        "Categorized {} file{} into standard folders (Documents, Images, Audio, Video, Archives, Code, Spreadsheets, Presentations, Design). {} file{} already organized; {} ambiguous or colliding file{} left unassigned.",
        categorized_count,
        if categorized_count == 1 { "" } else { "s" },
        already_in_place,
        if already_in_place == 1 { "" } else { "s" },
        unassigned_count,
        if unassigned_count == 1 { "" } else { "s" }
    );

    Proposal { actions, rationale }
}

fn propose_date(files: &[FileCandidate]) -> Proposal {
    let mut actions = Vec::new();
    let occupied_destinations: HashSet<PathBuf> =
        files.iter().map(|f| f.relative_path.clone()).collect();
    let mut proposed_targets: HashSet<PathBuf> = HashSet::new();

    let mut organized_count = 0usize;
    let mut unassigned_count = 0usize;
    let mut already_in_place = 0usize;

    for file in files {
        if file.relative_path.components().count() != 1 {
            continue;
        }
        let Some(file_name) = file.relative_path.file_name() else {
            unassigned_count += 1;
            continue;
        };
        let Some((year, month, _day)) = civil_from_unix_seconds(file.modified) else {
            // Missing/invalid timestamp remains unassigned
            unassigned_count += 1;
            continue;
        };

        let folder = format!("{year:04}/{month:02}");
        let target = PathBuf::from(folder).join(file_name);

        if file.relative_path == target {
            already_in_place += 1;
            continue;
        }

        if occupied_destinations.contains(&target) || proposed_targets.contains(&target) {
            unassigned_count += 1;
            continue;
        }

        proposed_targets.insert(target.clone());
        actions.push(ProposedAction::Move {
            source: file.id,
            destination_relative: target,
        });
        organized_count += 1;
    }

    let rationale = format!(
        "Organized {} file{} by modification date (YYYY/MM). {} file{} already organized; {} file{} left unassigned due to missing timestamp or destination collisions.",
        organized_count,
        if organized_count == 1 { "" } else { "s" },
        already_in_place,
        if already_in_place == 1 { "" } else { "s" },
        unassigned_count,
        if unassigned_count == 1 { "" } else { "s" }
    );

    Proposal { actions, rationale }
}

fn propose_project(project_name: &str, files: &[FileCandidate]) -> Proposal {
    let project = project_name.trim();
    if project.is_empty() {
        return Proposal {
            actions: Vec::new(),
            rationale: "Project name is required for project-based organization.".into(),
        };
    }

    let mut actions = Vec::new();
    let occupied_destinations: HashSet<PathBuf> =
        files.iter().map(|f| f.relative_path.clone()).collect();
    let mut proposed_targets: HashSet<PathBuf> = HashSet::new();

    let project_lower = project.to_lowercase();
    let mut matched_count = 0usize;
    let mut excluded_git_count = 0usize;
    let mut already_in_project = 0usize;

    for file in files {
        // Protect Git repositories: files inside .git are never moved
        if path_contains_git(&file.relative_path) {
            excluded_git_count += 1;
            continue;
        }
        if file.relative_path.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some("node_modules" | "target" | "dist" | ".venv")
            )
        }) {
            continue;
        }

        let Some(file_name) = file.relative_path.file_name() else {
            continue;
        };
        let file_name_str = file_name.to_string_lossy().to_lowercase();
        let path_str = file.relative_path.to_string_lossy().to_lowercase();
        let excerpt_match = file
            .excerpt
            .as_ref()
            .is_some_and(|e| e.to_lowercase().contains(&project_lower));

        let is_match = file_name_str.contains(&project_lower)
            || path_str.contains(&project_lower)
            || excerpt_match;

        if !is_match {
            continue;
        }

        let target = PathBuf::from(project).join(file_name);

        if file.relative_path == target {
            already_in_project += 1;
            continue;
        }

        if occupied_destinations.contains(&target) || proposed_targets.contains(&target) {
            continue;
        }

        proposed_targets.insert(target.clone());
        actions.push(ProposedAction::Move {
            source: file.id,
            destination_relative: target,
        });
        matched_count += 1;
    }

    let rationale = format!(
        "Grouped {} candidate file{} matching project '{}' into folder '{}'. {} file{} already inside project folder; {} Git repository file{} protected.",
        matched_count,
        if matched_count == 1 { "" } else { "s" },
        project,
        project,
        already_in_project,
        if already_in_project == 1 { "" } else { "s" },
        excluded_git_count,
        if excluded_git_count == 1 { "" } else { "s" }
    );

    Proposal { actions, rationale }
}

fn propose_custom(instruction: &str, files: &[FileCandidate]) -> Proposal {
    let raw = instruction.trim();
    if raw.is_empty() {
        return Proposal {
            actions: Vec::new(),
            rationale: "Custom grouping rule is empty. Please specify a rule or prompt (e.g. 'Photos: png, jpg | Docs: pdf', 'by extension', 'by year', or comma-separated tags like 'Invoices, Taxes').".into(),
        };
    }

    let mut actions = Vec::new();
    let occupied_destinations: HashSet<PathBuf> =
        files.iter().map(|f| f.relative_path.clone()).collect();
    let mut proposed_targets: HashSet<PathBuf> = HashSet::new();

    let mut organized_count = 0usize;
    let mut unassigned_count = 0usize;
    let mut already_in_place = 0usize;
    let mut protected_git = 0usize;

    // 1. Check for Mapping Rule syntax: "Folder1: ext1, ext2 | Folder2: ext3, ext4"
    if raw.contains(':') {
        let mut rules: Vec<(String, Vec<String>)> = Vec::new();
        let sections = raw.split(['|', ';', '\n']);
        for sec in sections {
            let parts: Vec<&str> = sec.splitn(2, ':').collect();
            if parts.len() == 2 {
                let folder = parts[0].trim().replace(['/', '\\'], "");
                if !folder.is_empty() {
                    let tags: Vec<String> = parts[1]
                        .split(',')
                        .map(|t| t.trim().trim_start_matches('.').to_lowercase())
                        .filter(|t| !t.is_empty())
                        .collect();
                    if !tags.is_empty() {
                        rules.push((folder, tags));
                    }
                }
            }
        }

        if !rules.is_empty() {
            for file in files {
                if file.relative_path.components().count() != 1 {
                    continue;
                }
                if path_contains_git(&file.relative_path) {
                    protected_git += 1;
                    continue;
                }
                let Some(file_name) = file.relative_path.file_name() else {
                    unassigned_count += 1;
                    continue;
                };
                let file_name_str = file_name.to_string_lossy().to_lowercase();
                let ext = file
                    .relative_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let excerpt = file.excerpt.as_deref().unwrap_or("").to_lowercase();

                let mut matched_folder: Option<&str> = None;
                for (folder, tags) in &rules {
                    for tag in tags {
                        if ext == *tag || file_name_str.contains(tag) || excerpt.contains(tag) {
                            matched_folder = Some(folder.as_str());
                            break;
                        }
                    }
                    if matched_folder.is_some() {
                        break;
                    }
                }

                if let Some(folder) = matched_folder {
                    let target = PathBuf::from(folder).join(file_name);
                    if file.relative_path == target {
                        already_in_place += 1;
                        continue;
                    }
                    if occupied_destinations.contains(&target) || proposed_targets.contains(&target)
                    {
                        unassigned_count += 1;
                        continue;
                    }
                    proposed_targets.insert(target.clone());
                    actions.push(ProposedAction::Move {
                        source: file.id,
                        destination_relative: target,
                    });
                    organized_count += 1;
                } else {
                    unassigned_count += 1;
                }
            }

            let rationale = format!(
                "Applied custom rules ({} folder categories). Moved {} file{}; {} already organized; {} unassigned.{git_note}",
                rules.len(),
                organized_count,
                if organized_count == 1 { "" } else { "s" },
                already_in_place,
                unassigned_count,
                git_note = if protected_git > 0 {
                    format!(" {} Git file(s) protected.", protected_git)
                } else {
                    String::new()
                }
            );
            return Proposal { actions, rationale };
        }
    }

    let raw_lower = raw.to_lowercase();

    // 2. Extension grouping keyword: "by extension", "extension", "format"
    if raw_lower.contains("extension") || raw_lower == "by format" || raw_lower == "by type" {
        for file in files {
            if file.relative_path.components().count() != 1 {
                continue;
            }
            if path_contains_git(&file.relative_path) {
                protected_git += 1;
                continue;
            }
            let Some(file_name) = file.relative_path.file_name() else {
                unassigned_count += 1;
                continue;
            };
            let Some(ext) = file.relative_path.extension().and_then(|e| e.to_str()) else {
                unassigned_count += 1;
                continue;
            };
            let ext_clean = ext.trim().to_ascii_uppercase();
            if ext_clean.is_empty() {
                unassigned_count += 1;
                continue;
            }

            let target = PathBuf::from(&ext_clean).join(file_name);
            if file.relative_path == target {
                already_in_place += 1;
                continue;
            }
            if occupied_destinations.contains(&target) || proposed_targets.contains(&target) {
                unassigned_count += 1;
                continue;
            }
            proposed_targets.insert(target.clone());
            actions.push(ProposedAction::Move {
                source: file.id,
                destination_relative: target,
            });
            organized_count += 1;
        }

        let rationale = format!(
            "Grouped {} file{} into folders by file extension. {} already organized; {} unassigned.{git_note}",
            organized_count,
            if organized_count == 1 { "" } else { "s" },
            already_in_place,
            unassigned_count,
            git_note = if protected_git > 0 {
                format!(" {} Git file(s) protected.", protected_git)
            } else {
                String::new()
            }
        );
        return Proposal { actions, rationale };
    }

    // 3. Year grouping keyword: "by year", "year"
    if raw_lower.contains("year") || raw_lower == "annual" {
        for file in files {
            if file.relative_path.components().count() != 1 {
                continue;
            }
            if path_contains_git(&file.relative_path) {
                protected_git += 1;
                continue;
            }
            let Some(file_name) = file.relative_path.file_name() else {
                unassigned_count += 1;
                continue;
            };
            let Some((year, _m, _d)) = civil_from_unix_seconds(file.modified) else {
                unassigned_count += 1;
                continue;
            };
            let target = PathBuf::from(format!("{year:04}")).join(file_name);
            if file.relative_path == target {
                already_in_place += 1;
                continue;
            }
            if occupied_destinations.contains(&target) || proposed_targets.contains(&target) {
                unassigned_count += 1;
                continue;
            }
            proposed_targets.insert(target.clone());
            actions.push(ProposedAction::Move {
                source: file.id,
                destination_relative: target,
            });
            organized_count += 1;
        }

        let rationale = format!(
            "Grouped {} file{} into folders by year (YYYY). {} already organized; {} unassigned.{git_note}",
            organized_count,
            if organized_count == 1 { "" } else { "s" },
            already_in_place,
            unassigned_count,
            git_note = if protected_git > 0 {
                format!(" {} Git file(s) protected.", protected_git)
            } else {
                String::new()
            }
        );
        return Proposal { actions, rationale };
    }

    // 4. Comma-separated list of folders / tags: "Clients, Taxes, Receipts, Invoices"
    let tags: Vec<&str> = raw
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();
    if tags.len() > 1 {
        for file in files {
            if file.relative_path.components().count() != 1 {
                continue;
            }
            if path_contains_git(&file.relative_path) {
                protected_git += 1;
                continue;
            }
            let Some(file_name) = file.relative_path.file_name() else {
                unassigned_count += 1;
                continue;
            };
            let file_name_str = file_name.to_string_lossy().to_lowercase();
            let excerpt = file.excerpt.as_deref().unwrap_or("").to_lowercase();

            let mut matched_tag: Option<&str> = None;
            for tag in &tags {
                let tag_lower = tag.to_lowercase();
                if file_name_str.contains(&tag_lower) || excerpt.contains(&tag_lower) {
                    matched_tag = Some(*tag);
                    break;
                }
            }

            if let Some(tag) = matched_tag {
                let target = PathBuf::from(tag).join(file_name);
                if file.relative_path == target {
                    already_in_place += 1;
                    continue;
                }
                if occupied_destinations.contains(&target) || proposed_targets.contains(&target) {
                    unassigned_count += 1;
                    continue;
                }
                proposed_targets.insert(target.clone());
                actions.push(ProposedAction::Move {
                    source: file.id,
                    destination_relative: target,
                });
                organized_count += 1;
            } else {
                unassigned_count += 1;
            }
        }

        let rationale = format!(
            "Grouped {} file{} matching tags [{}] into dedicated folders. {} already organized; {} unassigned.{git_note}",
            organized_count,
            if organized_count == 1 { "" } else { "s" },
            tags.join(", "),
            already_in_place,
            unassigned_count,
            git_note = if protected_git > 0 {
                format!(" {} Git file(s) protected.", protected_git)
            } else {
                String::new()
            }
        );
        return Proposal { actions, rationale };
    }

    // 5. Fallback single folder / keyword: e.g. "Archive", "ClientPortal", etc.
    let single_folder = raw.trim().replace(['/', '\\'], "");
    let single_folder_lower = single_folder.to_lowercase();

    for file in files {
        if file.relative_path.components().count() != 1 {
            continue;
        }
        if path_contains_git(&file.relative_path) {
            protected_git += 1;
            continue;
        }
        let Some(file_name) = file.relative_path.file_name() else {
            unassigned_count += 1;
            continue;
        };
        let file_name_str = file_name.to_string_lossy().to_lowercase();
        let excerpt = file.excerpt.as_deref().unwrap_or("").to_lowercase();

        if file_name_str.contains(&single_folder_lower) || excerpt.contains(&single_folder_lower) {
            let target = PathBuf::from(&single_folder).join(file_name);
            if file.relative_path == target {
                already_in_place += 1;
                continue;
            }
            if occupied_destinations.contains(&target) || proposed_targets.contains(&target) {
                unassigned_count += 1;
                continue;
            }
            proposed_targets.insert(target.clone());
            actions.push(ProposedAction::Move {
                source: file.id,
                destination_relative: target,
            });
            organized_count += 1;
        } else {
            unassigned_count += 1;
        }
    }

    let rationale = format!(
        "Custom rule matched {} file{} with keyword '{}' into folder '{}'. {} already in place; {} unassigned.{git_note}",
        organized_count,
        if organized_count == 1 { "" } else { "s" },
        single_folder,
        single_folder,
        already_in_place,
        unassigned_count,
        git_note = if protected_git > 0 {
            format!(" {} Git file(s) protected.", protected_git)
        } else {
            String::new()
        }
    );

    Proposal { actions, rationale }
}

/// Generates a storage cleanup proposal from exact duplicate findings and old installers/artifacts.
/// For exact duplicates, exactly one copy per group is preserved, while redundant copies are proposed for Trash.
pub fn propose_storage_cleanup(
    duplicate_groups: &[Vec<u64>],
    kept_copy_by_group: &HashMap<usize, u64>,
    installer_ids: &[u64],
    artifact_ids: &[u64],
) -> Proposal {
    let mut actions = Vec::new();
    let mut duplicate_trash_count = 0usize;

    for (group_idx, group) in duplicate_groups.iter().enumerate() {
        if group.len() < 2 {
            continue;
        }
        // Choose kept file: either user selection or first in group
        let kept_id = kept_copy_by_group
            .get(&group_idx)
            .copied()
            .unwrap_or(group[0]);

        for &id in group {
            if id != kept_id {
                actions.push(ProposedAction::Trash { source: FileId(id) });
                duplicate_trash_count += 1;
            }
        }
    }

    for &id in installer_ids {
        actions.push(ProposedAction::Trash { source: FileId(id) });
    }

    for &id in artifact_ids {
        actions.push(ProposedAction::Trash { source: FileId(id) });
    }

    let rationale = format!(
        "Storage cleanup proposal: {} redundant duplicate copy(ies) (preserving 1 copy per group), {} old installer(s), and {} development artifact(s) proposed for Trash. All actions require explicit approval and are reversible.",
        duplicate_trash_count,
        installer_ids.len(),
        artifact_ids.len()
    );

    Proposal { actions, rationale }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_date_calculation_matches_known_dates() {
        // 2026-09-29 is approx 1790678400 (or close)
        // Let's test Jan 1, 1970 + 86400 = Jan 2, 1970
        let (y, m, d) = civil_from_unix_seconds(86400).unwrap();
        assert_eq!((y, m, d), (1970, 1, 2));

        // 2026-09-29 00:00:00 UTC = 1790640000
        let (y, m, d) = civil_from_unix_seconds(1790640000).unwrap();
        assert_eq!((y, m, d), (2026, 9, 29));
    }

    #[test]
    fn category_mode_organizes_by_extension_and_leaves_unknown_unassigned() {
        let files = vec![
            FileCandidate {
                id: FileId(1),
                relative_path: PathBuf::from("report.pdf"),
                size: 1000,
                modified: 1000,
                excerpt: None,
            },
            FileCandidate {
                id: FileId(2),
                relative_path: PathBuf::from("photo.jpg"),
                size: 2000,
                modified: 1000,
                excerpt: None,
            },
            FileCandidate {
                id: FileId(3),
                relative_path: PathBuf::from("mystery.xyz123"),
                size: 500,
                modified: 1000,
                excerpt: None,
            },
            FileCandidate {
                id: FileId(4),
                relative_path: PathBuf::from("Documents/existing.pdf"),
                size: 500,
                modified: 1000,
                excerpt: None,
            },
        ];

        let proposal = propose(OrganizationMode::Category, None, &files);
        assert_eq!(proposal.actions.len(), 2);
        assert!(proposal.actions.contains(&ProposedAction::Move {
            source: FileId(1),
            destination_relative: PathBuf::from("Documents/report.pdf"),
        }));
        assert!(proposal.actions.contains(&ProposedAction::Move {
            source: FileId(2),
            destination_relative: PathBuf::from("Images/photo.jpg"),
        }));
        // File 3 is unknown extension -> unassigned
        // File 4 is already in Documents -> no move proposed
        assert!(proposal.rationale.contains("Categorized 2 files"));
    }

    #[test]
    fn date_mode_organizes_into_year_month_folders() {
        let files = vec![
            FileCandidate {
                id: FileId(1),
                relative_path: PathBuf::from("file_sept_2026.txt"),
                size: 100,
                modified: 1790640000, // 2026-09-29
                excerpt: None,
            },
            FileCandidate {
                id: FileId(2),
                relative_path: PathBuf::from("file_invalid_date.txt"),
                size: 100,
                modified: 0, // invalid/zero timestamp
                excerpt: None,
            },
        ];

        let proposal = propose(OrganizationMode::Date, None, &files);
        assert_eq!(proposal.actions.len(), 1);
        assert_eq!(
            proposal.actions[0],
            ProposedAction::Move {
                source: FileId(1),
                destination_relative: PathBuf::from("2026/09/file_sept_2026.txt"),
            }
        );
    }

    #[test]
    fn project_mode_matches_filename_and_excerpt_and_excludes_git() {
        let files = vec![
            FileCandidate {
                id: FileId(1),
                relative_path: PathBuf::from("Tidy_architecture.md"),
                size: 100,
                modified: 1000,
                excerpt: None,
            },
            FileCandidate {
                id: FileId(2),
                relative_path: PathBuf::from("notes.txt"),
                size: 100,
                modified: 1000,
                excerpt: Some("This file talks about Tidy desktop assistant".into()),
            },
            FileCandidate {
                id: FileId(3),
                relative_path: PathBuf::from("unrelated.txt"),
                size: 100,
                modified: 1000,
                excerpt: Some("Just random notes".into()),
            },
            FileCandidate {
                id: FileId(4),
                relative_path: PathBuf::from(".git/config"),
                size: 100,
                modified: 1000,
                excerpt: Some("Tidy repo config".into()),
            },
        ];

        let proposal = propose(OrganizationMode::Project, Some("Tidy"), &files);
        assert_eq!(proposal.actions.len(), 2);
        assert!(proposal.actions.contains(&ProposedAction::Move {
            source: FileId(1),
            destination_relative: PathBuf::from("Tidy/Tidy_architecture.md"),
        }));
        assert!(proposal.actions.contains(&ProposedAction::Move {
            source: FileId(2),
            destination_relative: PathBuf::from("Tidy/notes.txt"),
        }));
        // File 4 (.git) is excluded from moving!
        assert!(
            proposal
                .rationale
                .contains("1 Git repository file protected")
        );
    }

    #[test]
    fn storage_cleanup_preserves_one_duplicate_and_trashes_redundant() {
        let duplicate_groups = vec![vec![10, 20, 30]];
        let mut keep = HashMap::new();
        keep.insert(0, 20); // user specifies to keep id 20

        let installer_ids = vec![40];
        let artifact_ids = vec![50];

        let proposal =
            propose_storage_cleanup(&duplicate_groups, &keep, &installer_ids, &artifact_ids);

        assert_eq!(proposal.actions.len(), 4);
        assert!(
            proposal
                .actions
                .contains(&ProposedAction::Trash { source: FileId(10) })
        );
        assert!(
            proposal
                .actions
                .contains(&ProposedAction::Trash { source: FileId(30) })
        );
        // id 20 is kept, so NOT in trash actions!
        assert!(
            !proposal
                .actions
                .contains(&ProposedAction::Trash { source: FileId(20) })
        );
        assert!(
            proposal
                .actions
                .contains(&ProposedAction::Trash { source: FileId(40) })
        );
        assert!(
            proposal
                .actions
                .contains(&ProposedAction::Trash { source: FileId(50) })
        );
    }

    #[test]
    fn custom_mode_organizes_with_mapping_rules() {
        let files = vec![
            FileCandidate {
                id: FileId(1),
                relative_path: PathBuf::from("receipt.pdf"),
                size: 100,
                modified: 1000,
                excerpt: None,
            },
            FileCandidate {
                id: FileId(2),
                relative_path: PathBuf::from("vacation.png"),
                size: 200,
                modified: 1000,
                excerpt: None,
            },
            FileCandidate {
                id: FileId(3),
                relative_path: PathBuf::from("mystery.bin"),
                size: 300,
                modified: 1000,
                excerpt: None,
            },
        ];

        let rule = "Receipts: pdf | Photos: png, jpg";
        let proposal = propose(OrganizationMode::Custom, Some(rule), &files);
        assert_eq!(proposal.actions.len(), 2);
        assert!(proposal.actions.contains(&ProposedAction::Move {
            source: FileId(1),
            destination_relative: PathBuf::from("Receipts/receipt.pdf"),
        }));
        assert!(proposal.actions.contains(&ProposedAction::Move {
            source: FileId(2),
            destination_relative: PathBuf::from("Photos/vacation.png"),
        }));
        assert!(proposal.rationale.contains("Applied custom rules"));
    }

    #[test]
    fn custom_mode_organizes_by_extension_keyword() {
        let files = vec![
            FileCandidate {
                id: FileId(1),
                relative_path: PathBuf::from("document.pdf"),
                size: 100,
                modified: 1000,
                excerpt: None,
            },
            FileCandidate {
                id: FileId(2),
                relative_path: PathBuf::from("archive.zip"),
                size: 100,
                modified: 1000,
                excerpt: None,
            },
        ];

        let proposal = propose(OrganizationMode::Custom, Some("group by extension"), &files);
        assert_eq!(proposal.actions.len(), 2);
        assert!(proposal.actions.contains(&ProposedAction::Move {
            source: FileId(1),
            destination_relative: PathBuf::from("PDF/document.pdf"),
        }));
        assert!(proposal.actions.contains(&ProposedAction::Move {
            source: FileId(2),
            destination_relative: PathBuf::from("ZIP/archive.zip"),
        }));
    }
}
