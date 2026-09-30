//! Streaming, scope-bound retrieval across every row in the current index generation.
use super::{FileRow, Index};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Serialize)]
pub struct FileTypeTotal {
    pub extension: String,
    pub files: u64,
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct IndexOverview {
    pub indexed_files: u64,
    pub logical_bytes: u64,
    pub files_with_saved_text: u64,
    pub scan_status: String,
    pub scanned_at: Option<i64>,
    pub excluded_entries: u64,
    pub largest_types: Vec<FileTypeTotal>,
    pub other_type_files: u64,
    pub other_type_bytes: u64,
    pub retrieval_terms: Vec<String>,
    pub matching_files: u64,
}
pub struct FolderContext {
    pub overview: IndexOverview,
    pub examples: Vec<FileRow>,
}
fn terms(question: &str) -> Vec<String> {
    const STOP: &str = "a an and are as at be by can check contains directory do does each file files find folder folders for from give how i in indexed is it kinds list look me metadata my of on or please sample should show size summarize summary tell that the their them there these this to types using want what which with you first about all have has information details";
    let mut out = Vec::new();
    for term in question.split(|c: char| !c.is_alphanumeric()) {
        let term = term.to_lowercase();
        if term.chars().count() >= 2
            && term.len() <= 80
            && !STOP.split_whitespace().any(|s| s == term)
            && !out.contains(&term)
        {
            out.push(term);
        }
        if out.len() == 16 {
            break;
        }
    }
    out
}
fn keep_best(items: &mut Vec<(u32, FileRow)>, score: u32, file: FileRow, cap: usize) {
    items.push((score, file));
    items.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.size.cmp(&a.1.size))
            .then(a.1.id.cmp(&b.1.id))
    });
    items.truncate(cap);
}
impl Index {
    pub fn folder_context(
        &self,
        scope: i64,
        question: &str,
        cancel: &AtomicBool,
    ) -> rusqlite::Result<FolderContext> {
        if question.len() > 1000 {
            return Err(rusqlite::Error::InvalidParameterName(
                "Question exceeds 1,000 bytes".into(),
            ));
        }
        let tx = self.connection.unchecked_transaction()?;
        let (status, scanned_at, excluded_entries) = tx.query_row(
            "SELECT status,scanned_at,omitted FROM scopes WHERE id=?1",
            [scope],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, u64>(2)?,
                ))
            },
        )?;
        let mut overview = IndexOverview {
            indexed_files: 0,
            logical_bytes: 0,
            files_with_saved_text: 0,
            scan_status: status,
            scanned_at,
            excluded_entries,
            largest_types: Vec::new(),
            other_type_files: 0,
            other_type_bytes: 0,
            retrieval_terms: terms(question),
            matching_files: 0,
        };
        let mut types: BTreeMap<String, FileTypeTotal> = BTreeMap::new();
        let mut relevant = Vec::new();
        let mut largest = Vec::new();
        let mut statement = tx.prepare("SELECT f.id,f.display,f.size,f.modified,f.excerpt,f.hash IS NOT NULL FROM files f JOIN scopes s ON s.id=f.scope_id WHERE s.id=?1 AND f.generation=s.generation ORDER BY f.id")?;
        let mut rows = statement.query([scope])?;
        while let Some(row) = rows.next()? {
            if cancel.load(Ordering::Relaxed) {
                return Err(rusqlite::Error::InvalidParameterName(
                    "Index analysis cancelled".into(),
                ));
            }
            let path: String = row.get(1)?;
            let bytes = row.get::<_, i64>(2)?.max(0) as u64;
            let excerpt: Option<String> = row.get(4)?;
            overview.indexed_files += 1;
            overview.logical_bytes = overview.logical_bytes.saturating_add(bytes);
            overview.files_with_saved_text += u64::from(excerpt.is_some());
            let extension = Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("(none)")
                .to_lowercase();
            let extension = if extension.len() > 48 {
                "(long extension)".to_string()
            } else {
                extension
            };
            let total = types.entry(extension.clone()).or_insert(FileTypeTotal {
                extension,
                files: 0,
                bytes: 0,
            });
            total.files += 1;
            total.bytes = total.bytes.saturating_add(bytes);
            let lower_path = path.to_lowercase();
            let lower_text = excerpt.as_deref().unwrap_or("").to_lowercase();
            let score = overview
                .retrieval_terms
                .iter()
                .map(|term| {
                    if lower_path.contains(term) {
                        3
                    } else if lower_text.contains(term) {
                        1
                    } else {
                        0
                    }
                })
                .sum::<u32>();
            let file = FileRow {
                id: row.get(0)?,
                path,
                size: bytes as i64,
                modified: row.get(3)?,
                excerpt: excerpt.map(|s| s.chars().take(400).collect()),
                hashed: row.get(5)?,
            };
            if score > 0 {
                overview.matching_files += 1;
                keep_best(&mut relevant, score, file, 20);
            } else {
                keep_best(&mut largest, 0, file, 20);
            }
        }
        overview.largest_types = types.into_values().collect();
        overview
            .largest_types
            .sort_by(|a, b| b.files.cmp(&a.files).then(a.extension.cmp(&b.extension)));
        for item in overview.largest_types.iter().skip(12) {
            overview.other_type_files += item.files;
            overview.other_type_bytes = overview.other_type_bytes.saturating_add(item.bytes);
        }
        overview.largest_types.truncate(12);
        let mut examples: Vec<_> = relevant.into_iter().map(|(_, f)| f).collect();
        examples.extend(
            largest
                .into_iter()
                .take(20 - examples.len())
                .map(|(_, f)| f),
        );
        Ok(FolderContext { overview, examples })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn index() -> Index {
        let index = Index::open(Path::new(":memory:")).unwrap();
        index.connection.execute("INSERT INTO scopes(id,path,display,root_identity,generation,status,omitted) VALUES(1,x'01','fixture','test',2,'partial',7)", []).unwrap();
        for i in 0..250 {
            let name = if i == 249 {
                "tiny/Apollo.txt".to_string()
            } else {
                format!("big/{i}.zip")
            };
            index.connection.execute("INSERT INTO files(scope_id,path,display,identity,fingerprint,size,modified,generation) VALUES(1,?1,?2,'','',?3,0,2)", rusqlite::params![name.as_bytes(),name,250-i]).unwrap();
        }
        index.connection.execute("INSERT INTO files(scope_id,path,display,identity,fingerprint,size,modified,generation) VALUES(1,x'02','old-hidden.txt','','',999999,0,1)", []).unwrap();
        index
    }
    #[test]
    fn covers_all_rows_and_retrieves_small_file_beyond_first_page() {
        let index = index();
        let context = index
            .folder_context(1, "Find Apollo files", &AtomicBool::new(false))
            .unwrap();
        assert_eq!(context.overview.indexed_files, 250);
        assert_eq!(context.overview.logical_bytes, 31375);
        assert_eq!(context.overview.matching_files, 1);
        assert_eq!(context.examples[0].path, "tiny/Apollo.txt");
        assert_eq!(context.overview.scan_status, "partial");
        assert_eq!(context.overview.excluded_entries, 7);
        assert_eq!(
            context
                .overview
                .largest_types
                .iter()
                .map(|t| t.files)
                .sum::<u64>(),
            250
        );
        assert!(context.examples.len() <= 20);
    }
    #[test]
    fn covers_large_index_and_accounts_for_all_type_groups() {
        let index = index();
        index.connection.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<61342) INSERT INTO files(scope_id,path,display,identity,fingerprint,size,modified,generation) SELECT 1,CAST('extra/'||x AS BLOB),'extra/'||x||'.type'||(x%20),'','',1,0,2 FROM n;").unwrap();
        let context = index
            .folder_context(1, "Summarize", &AtomicBool::new(false))
            .unwrap();
        assert_eq!(context.overview.indexed_files, 61592);
        assert_eq!(context.overview.logical_bytes, 31375 + 61342);
        assert_eq!(context.overview.largest_types.len(), 12);
        assert_eq!(
            context
                .overview
                .largest_types
                .iter()
                .map(|t| t.files)
                .sum::<u64>()
                + context.overview.other_type_files,
            61592
        );
        assert_eq!(
            context
                .overview
                .largest_types
                .iter()
                .map(|t| t.bytes)
                .sum::<u64>()
                + context.overview.other_type_bytes,
            92717
        );
    }
    #[test]
    fn searches_saved_text_beyond_excerpt_prefix_and_does_not_cross_scope() {
        let index = index();
        let text = format!("{} nebula", "plain ".repeat(100));
        index
            .connection
            .execute(
                "UPDATE files SET excerpt=?1 WHERE display='tiny/Apollo.txt'",
                [text],
            )
            .unwrap();
        index.connection.execute_batch("INSERT INTO scopes(id,path,display,root_identity,generation) VALUES(2,x'03','private','test',2); INSERT INTO files(scope_id,path,display,identity,fingerprint,size,modified,generation) VALUES(2,x'04','nebula-secret','','',999999,0,2);").unwrap();
        let context = index
            .folder_context(1, "Find nebula", &AtomicBool::new(false))
            .unwrap();
        assert_eq!(context.overview.matching_files, 1);
        assert_eq!(context.overview.files_with_saved_text, 1);
        assert_eq!(context.examples[0].path, "tiny/Apollo.txt");
        assert!(context.examples.iter().all(|f| f.path != "nebula-secret"));
    }
    #[test]
    fn cancelled_or_unknown_scope_never_returns_partial_context() {
        let index = index();
        assert!(
            index
                .folder_context(1, "summary", &AtomicBool::new(true))
                .is_err()
        );
        assert!(
            index
                .folder_context(2, "summary", &AtomicBool::new(false))
                .is_err()
        );
    }
    #[test]
    fn overview_question_does_not_become_literal_phrase_search() {
        assert!(terms("What kinds of files are in this folder?").is_empty());
        assert_eq!(terms("Find Apollo Apollo"), vec!["apollo"]);
    }
}
