//! Local workflow catalog. Routing never grants filesystem execution authority.
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub title: String,
    pub mode: String,
    pub context: String,
    pub steps: String,
    pub extensions: Vec<String>,
    pub handoff: Option<String>,
    pub evidence: String,
    pub stop: String,
}
pub fn catalog() -> &'static [Workflow] {
    static CATALOG: OnceLock<Vec<Workflow>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../../skills/catalog.json"))
            .expect("Bundled workflow catalog must be valid")
    })
}
pub fn get(id: &str) -> Option<&'static Workflow> {
    catalog().iter().find(|w| w.id == id)
}
pub fn routing_prompt(request: &str) -> String {
    let choices: Vec<_> = catalog()
        .iter()
        .map(|w| serde_json::json!({"id":w.id,"use":w.context}))
        .collect();
    format!(
        "Select the most specific Tidy workflow for USER_REQUEST. /no_think\nReturn one JSON object: kind=workflow, id=one exact catalog ID, reason=brief explanation. This selects guidance only, never executes changes. Distinguish remove words FROM filenames (rename) from remove FILES (Trash). Find-only means observe; group/move means organize; copy means copy_files; requested extension rename means change_extensions; permission/access changes mean file_permissions. Do not ask Are you sure for explicit operations; the user approves an exact preview separately. Duplicate cleanup and storage inspection use Storage workflows; restoring uses History. Ambiguous requests use clarify_request. Do not select a different action than requested. Supported changes operate on regular indexed files within an authorized root. Content conversion, ownership/ACLs, elevation, cross-root operations and directory metadata mutations are unavailable; explain limits without inventing tools.\nUSER_REQUEST: {}\nCATALOG: {}",
        serde_json::to_string(request).unwrap(),
        serde_json::to_string(&choices).unwrap()
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_complete_unique_and_bounded() {
        let all = catalog();
        assert_eq!(all.len(), 42);
        let ids: std::collections::HashSet<_> = all.iter().map(|w| &w.id).collect();
        assert_eq!(ids.len(), all.len());
        assert!(
            all.iter()
                .all(|w| !w.steps.is_empty() && !w.evidence.is_empty() && !w.stop.is_empty())
        );
        assert!(routing_prompt("Organize my Downloads").len() < 9000);
    }
}
