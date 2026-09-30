//! Storage analyzer engine. Findings are evidence-backed candidates for recovery,
//! not autonomous deletion instructions.
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    LargeFile,
    ExactDuplicate,
    OldInstaller,
    DevelopmentArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub file_ids: Vec<u64>,
    pub kind: FindingKind,
    pub evidence: String,
    /// Estimated reclaimable bytes if redundant copies or candidates are removed.
    /// Distinguishes physical allocation from hard link references.
    pub estimated_reclaimable_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSummary {
    pub total_findings: usize,
    pub potential_reclaimable_bytes: u64,
    pub large_files_count: usize,
    pub large_files_bytes: u64,
    pub duplicate_groups_count: usize,
    pub duplicate_redundant_files_count: usize,
    pub duplicate_reclaimable_bytes: u64,
    pub old_installers_count: usize,
    pub old_installers_bytes: u64,
    pub dev_artifacts_count: usize,
    pub dev_artifacts_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageAnalysisResult {
    pub findings: Vec<Finding>,
    pub summary: StorageSummary,
}

#[derive(Debug, Clone)]
pub struct AnalyzableFile {
    pub id: u64,
    pub path: PathBuf,
    pub size: u64,
    pub modified: i64,
    pub hash: Option<String>,
    pub identity: String,
}

#[derive(Debug, Clone)]
pub struct StorageAnalysisConfig {
    /// Threshold in bytes to consider a file "large" (default: 50 MiB).
    pub min_large_file_bytes: u64,
    /// Age threshold in days to consider an installer "old" (default: 30 days).
    pub old_installer_days: u64,
    /// Current Unix timestamp in seconds (for deterministic aging calculation).
    pub now_timestamp: i64,
}

impl Default for StorageAnalysisConfig {
    fn default() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self {
            min_large_file_bytes: 50 * 1024 * 1024,
            old_installer_days: 30,
            now_timestamp: now,
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.2} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn is_installer_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "dmg" | "pkg" | "iso" | "exe" | "msi" | "appxbundle" | "deb" | "rpm" | "apk"
    )
}

fn is_dev_artifact(path: &Path) -> bool {
    for component in path.components() {
        let comp_str = component.as_os_str().to_string_lossy();
        if matches!(
            comp_str.as_ref(),
            "target"
                | "node_modules"
                | "dist"
                | ".gradle"
                | "__pycache__"
                | ".pytest_cache"
                | ".next"
                | ".nuxt"
                | ".turbo"
                | ".cargo-ok"
        ) {
            return true;
        }
    }
    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && matches!(
            ext.to_ascii_lowercase().as_str(),
            "pyc" | "pyo" | "o" | "obj" | "a" | "class" | "pdb"
        )
    {
        return true;
    }
    false
}

/// Analyzes an indexed snapshot of files in an authorized folder.
/// Returns categorized findings with explicit evidence and reclaim estimates.
pub fn analyze_storage(
    files: &[AnalyzableFile],
    config: &StorageAnalysisConfig,
) -> StorageAnalysisResult {
    let mut findings = Vec::new();
    let mut summary = StorageSummary {
        total_findings: 0,
        potential_reclaimable_bytes: 0,
        large_files_count: 0,
        large_files_bytes: 0,
        duplicate_groups_count: 0,
        duplicate_redundant_files_count: 0,
        duplicate_reclaimable_bytes: 0,
        old_installers_count: 0,
        old_installers_bytes: 0,
        dev_artifacts_count: 0,
        dev_artifacts_bytes: 0,
    };

    // 1. Exact duplicates: group by SHA-256 hash
    let mut hash_groups: HashMap<&str, Vec<&AnalyzableFile>> = HashMap::new();
    for file in files {
        if let Some(hash) = &file.hash
            && !hash.is_empty()
        {
            hash_groups.entry(hash.as_str()).or_default().push(file);
        }
    }

    let mut sorted_hashes: Vec<_> = hash_groups.into_iter().collect();
    sorted_hashes
        .sort_by_key(|(_, list)| std::cmp::Reverse(list.first().map(|f| f.size).unwrap_or(0)));

    for (hash, group) in sorted_hashes {
        if group.len() < 2 {
            continue;
        }
        let size = group[0].size;
        let file_ids: Vec<u64> = group.iter().map(|f| f.id).collect();

        // Check inode identities to account for hard links
        let distinct_identities: HashSet<&str> =
            group.iter().map(|f| f.identity.as_str()).collect();
        let reclaimable = if distinct_identities.len() <= 1 {
            // All copies share the same physical inode (hard links)
            0
        } else {
            // Each additional distinct inode copy can be reclaimed if all but one are removed
            size.saturating_mul((distinct_identities.len() - 1) as u64)
        };

        let hash_prefix = if hash.len() > 12 { &hash[..12] } else { hash };
        let evidence = if distinct_identities.len() <= 1 {
            format!(
                "{} hard-linked references ({}) share the same inode; unlinking references does not free physical space until all links are removed",
                group.len(),
                format_bytes(size)
            )
        } else {
            format!(
                "{} exact duplicate copies (hash {}…) across {} physical files ({}); up to {} reclaimable",
                group.len(),
                hash_prefix,
                distinct_identities.len(),
                format_bytes(size),
                format_bytes(reclaimable)
            )
        };

        findings.push(Finding {
            file_ids,
            kind: FindingKind::ExactDuplicate,
            evidence,
            estimated_reclaimable_bytes: Some(reclaimable),
        });

        summary.duplicate_groups_count += 1;
        summary.duplicate_redundant_files_count += group.len().saturating_sub(1);
        summary.duplicate_reclaimable_bytes = summary
            .duplicate_reclaimable_bytes
            .saturating_add(reclaimable);
    }

    // 2. Old installers
    let installer_age_limit = config
        .now_timestamp
        .saturating_sub((config.old_installer_days as i64).saturating_mul(86400));
    for file in files {
        if let Some(ext) = file.path.extension().and_then(|e| e.to_str())
            && is_installer_extension(ext)
            && file.modified <= installer_age_limit
        {
            let age_days = (config.now_timestamp.saturating_sub(file.modified) / 86400).max(0);
            let evidence = format!(
                "Installer package '{}' modified {} days ago ({})",
                file.path.display(),
                age_days,
                format_bytes(file.size)
            );
            findings.push(Finding {
                file_ids: vec![file.id],
                kind: FindingKind::OldInstaller,
                evidence,
                estimated_reclaimable_bytes: Some(file.size),
            });
            summary.old_installers_count += 1;
            summary.old_installers_bytes = summary.old_installers_bytes.saturating_add(file.size);
        }
    }

    // 3. Development artifacts
    for file in files {
        if is_dev_artifact(&file.path) {
            let evidence = format!(
                "Development build artifact/cache '{}' ({})",
                file.path.display(),
                format_bytes(file.size)
            );
            findings.push(Finding {
                file_ids: vec![file.id],
                kind: FindingKind::DevelopmentArtifact,
                evidence,
                estimated_reclaimable_bytes: Some(file.size),
            });
            summary.dev_artifacts_count += 1;
            summary.dev_artifacts_bytes = summary.dev_artifacts_bytes.saturating_add(file.size);
        }
    }

    // 4. Large files
    let mut large_files: Vec<&AnalyzableFile> = files
        .iter()
        .filter(|f| f.size >= config.min_large_file_bytes)
        .collect();
    large_files.sort_by_key(|f| std::cmp::Reverse(f.size));

    for file in large_files {
        let evidence = format!(
            "Large file '{}' ({})",
            file.path.display(),
            format_bytes(file.size)
        );
        findings.push(Finding {
            file_ids: vec![file.id],
            kind: FindingKind::LargeFile,
            evidence,
            estimated_reclaimable_bytes: Some(file.size),
        });
        summary.large_files_count += 1;
        summary.large_files_bytes = summary.large_files_bytes.saturating_add(file.size);
    }

    summary.total_findings = findings.len();
    summary.potential_reclaimable_bytes = summary
        .duplicate_reclaimable_bytes
        .saturating_add(summary.old_installers_bytes)
        .saturating_add(summary.dev_artifacts_bytes);

    StorageAnalysisResult { findings, summary }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_large_files_above_threshold() {
        let files = vec![
            AnalyzableFile {
                id: 1,
                path: PathBuf::from("big_video.mp4"),
                size: 100 * 1024 * 1024,
                modified: 1000,
                hash: None,
                identity: "1:1".into(),
            },
            AnalyzableFile {
                id: 2,
                path: PathBuf::from("small.txt"),
                size: 1024,
                modified: 1000,
                hash: None,
                identity: "1:2".into(),
            },
        ];
        let config = StorageAnalysisConfig {
            min_large_file_bytes: 50 * 1024 * 1024,
            old_installer_days: 30,
            now_timestamp: 1000,
        };
        let result = analyze_storage(&files, &config);
        assert_eq!(result.summary.large_files_count, 1);
        assert_eq!(result.summary.large_files_bytes, 100 * 1024 * 1024);
        let large_finding = result
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::LargeFile)
            .unwrap();
        assert_eq!(large_finding.file_ids, vec![1]);
        assert_eq!(
            large_finding.estimated_reclaimable_bytes,
            Some(100 * 1024 * 1024)
        );
    }

    #[test]
    fn detects_exact_duplicates_and_calculates_reclaimable() {
        let files = vec![
            AnalyzableFile {
                id: 1,
                path: PathBuf::from("copy1.pdf"),
                size: 5_000_000,
                modified: 1000,
                hash: Some("sha256_abc123".into()),
                identity: "1:10".into(),
            },
            AnalyzableFile {
                id: 2,
                path: PathBuf::from("copy2.pdf"),
                size: 5_000_000,
                modified: 1000,
                hash: Some("sha256_abc123".into()),
                identity: "1:20".into(),
            },
            AnalyzableFile {
                id: 3,
                path: PathBuf::from("copy3.pdf"),
                size: 5_000_000,
                modified: 1000,
                hash: Some("sha256_abc123".into()),
                identity: "1:30".into(),
            },
        ];
        let config = StorageAnalysisConfig::default();
        let result = analyze_storage(&files, &config);
        assert_eq!(result.summary.duplicate_groups_count, 1);
        assert_eq!(result.summary.duplicate_redundant_files_count, 2);
        assert_eq!(result.summary.duplicate_reclaimable_bytes, 10_000_000);
        let dup_finding = result
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::ExactDuplicate)
            .unwrap();
        assert_eq!(dup_finding.file_ids.len(), 3);
        assert_eq!(dup_finding.estimated_reclaimable_bytes, Some(10_000_000));
    }

    #[test]
    fn hard_links_with_identical_inode_report_zero_physical_reclaim() {
        let files = vec![
            AnalyzableFile {
                id: 1,
                path: PathBuf::from("link1.dat"),
                size: 20_000_000,
                modified: 1000,
                hash: Some("hash_same".into()),
                identity: "1:999".into(), // Same inode
            },
            AnalyzableFile {
                id: 2,
                path: PathBuf::from("link2.dat"),
                size: 20_000_000,
                modified: 1000,
                hash: Some("hash_same".into()),
                identity: "1:999".into(), // Same inode
            },
        ];
        let config = StorageAnalysisConfig::default();
        let result = analyze_storage(&files, &config);
        let dup_finding = result
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::ExactDuplicate)
            .unwrap();
        assert_eq!(dup_finding.estimated_reclaimable_bytes, Some(0));
        assert!(dup_finding.evidence.contains("share the same inode"));
    }

    #[test]
    fn detects_old_installers_by_extension_and_age() {
        let now = 100_000_000;
        let old_time = now - (40 * 86400); // 40 days old
        let recent_time = now - (5 * 86400); // 5 days old
        let files = vec![
            AnalyzableFile {
                id: 1,
                path: PathBuf::from("installer.dmg"),
                size: 50_000_000,
                modified: old_time,
                hash: None,
                identity: "1:1".into(),
            },
            AnalyzableFile {
                id: 2,
                path: PathBuf::from("recent_setup.pkg"),
                size: 30_000_000,
                modified: recent_time,
                hash: None,
                identity: "1:2".into(),
            },
        ];
        let config = StorageAnalysisConfig {
            min_large_file_bytes: 100 * 1024 * 1024,
            old_installer_days: 30,
            now_timestamp: now,
        };
        let result = analyze_storage(&files, &config);
        assert_eq!(result.summary.old_installers_count, 1);
        assert_eq!(result.summary.old_installers_bytes, 50_000_000);
        let inst_finding = result
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::OldInstaller)
            .unwrap();
        assert_eq!(inst_finding.file_ids, vec![1]);
    }

    #[test]
    fn detects_development_artifacts_in_target_or_node_modules() {
        let files = vec![
            AnalyzableFile {
                id: 1,
                path: PathBuf::from("node_modules/lodash/index.js"),
                size: 40_000,
                modified: 1000,
                hash: None,
                identity: "1:1".into(),
            },
            AnalyzableFile {
                id: 2,
                path: PathBuf::from("target/debug/libtidy.rlib"),
                size: 2_000_000,
                modified: 1000,
                hash: None,
                identity: "1:2".into(),
            },
            AnalyzableFile {
                id: 3,
                path: PathBuf::from("src/main.rs"),
                size: 500,
                modified: 1000,
                hash: None,
                identity: "1:3".into(),
            },
        ];
        let config = StorageAnalysisConfig::default();
        let result = analyze_storage(&files, &config);
        assert_eq!(result.summary.dev_artifacts_count, 2);
        assert_eq!(result.summary.dev_artifacts_bytes, 2_040_000);
    }
}

mod folders;
pub use folders::{FolderUsage, children, folder_usage};
