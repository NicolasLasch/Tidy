//! Recursive logical contents totals from indexed regular files, never directory st_size.
use crate::AnalyzableFile;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Default, Serialize)]
pub struct FolderUsage {
    pub path: PathBuf,
    pub logical_bytes: u64,
    pub file_count: usize,
    pub direct_bytes: u64,
    pub direct_files: usize,
}
pub fn folder_usage(files: &[AnalyzableFile]) -> Vec<FolderUsage> {
    let mut folders: BTreeMap<PathBuf, FolderUsage> = BTreeMap::new();
    folders.insert(PathBuf::new(), FolderUsage::default());
    for file in files {
        let parent = file.path.parent().unwrap_or(Path::new(""));
        for ancestor in parent.ancestors() {
            let row = folders
                .entry(ancestor.into())
                .or_insert_with(|| FolderUsage {
                    path: ancestor.into(),
                    ..Default::default()
                });
            row.logical_bytes = row.logical_bytes.saturating_add(file.size);
            row.file_count += 1;
            if ancestor == parent {
                row.direct_bytes = row.direct_bytes.saturating_add(file.size);
                row.direct_files += 1;
            }
        }
    }
    folders.into_values().collect()
}
pub fn children<'a>(folders: &'a [FolderUsage], parent: &Path) -> Vec<&'a FolderUsage> {
    let mut rows: Vec<_> = folders
        .iter()
        .filter(|f| !f.path.as_os_str().is_empty() && f.path.parent() == Some(parent))
        .collect();
    rows.sort_by(|a, b| {
        b.logical_bytes
            .cmp(&a.logical_bytes)
            .then_with(|| a.path.cmp(&b.path))
    });
    rows
}
#[cfg(test)]
mod tests {
    use super::*;
    fn file(path: &str, size: u64) -> AnalyzableFile {
        AnalyzableFile {
            id: 1,
            path: path.into(),
            size,
            modified: 0,
            hash: None,
            identity: String::new(),
        }
    }
    #[test]
    fn recursive_totals_include_small_descendants_and_direct_files() {
        let rows = folder_usage(&[
            file("root.txt", 3),
            file("a/one.txt", 10),
            file("a/deep/two.txt", 20),
            file("b/three.txt", 5),
        ]);
        let root = rows.iter().find(|f| f.path.as_os_str().is_empty()).unwrap();
        assert_eq!(
            (root.logical_bytes, root.direct_bytes, root.file_count),
            (38, 3, 4)
        );
        let a = rows.iter().find(|f| f.path == Path::new("a")).unwrap();
        assert_eq!((a.logical_bytes, a.direct_bytes, a.file_count), (30, 10, 2));
        assert_eq!(children(&rows, Path::new(""))[0].path, Path::new("a"));
    }
    #[test]
    fn empty_index_and_hard_links_are_logical_not_physical_sizes() {
        assert_eq!(folder_usage(&[])[0].logical_bytes, 0);
        let rows = folder_usage(&[file("a.txt", 10), file("b.txt", 10)]);
        assert_eq!(rows[0].logical_bytes, 20); // logical path lengths, not allocated space
    }
}
