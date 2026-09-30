//! Which indexed folders the assistant may see. Storage overview can show every folder, but only
//! folders the user switched on are reachable by planning, questions and AI-proposed changes.
use super::{Shared, blocking, display_error, lock};
use std::{collections::HashSet, path::Path};
use tauri::State;

pub(super) fn load(dir: &Path) -> HashSet<i64> {
    std::fs::read_to_string(dir.join("ai_selection.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<i64>>(&text).ok())
        .map(|ids| ids.into_iter().collect())
        .unwrap_or_default()
}
fn save(dir: &Path, ids: &HashSet<i64>) -> Result<(), String> {
    let mut sorted: Vec<_> = ids.iter().copied().collect();
    sorted.sort_unstable();
    let text = serde_json::to_string(&sorted).map_err(display_error)?;
    let tmp = dir.join("ai_selection.json.tmp");
    std::fs::write(&tmp, text).map_err(display_error)?;
    std::fs::rename(tmp, dir.join("ai_selection.json")).map_err(display_error)
}
pub(super) fn require(state: &Shared, scope_id: i64) -> Result<(), String> {
    if lock(&state.selected)?.contains(&scope_id) {
        Ok(())
    } else {
        Err("This folder is not selected for Tidy. Switch it on in Storage first.".into())
    }
}
pub(super) fn deselect(state: &Shared, scope_id: i64) -> Result<(), String> {
    let mut selected = lock(&state.selected)?;
    if selected.remove(&scope_id) {
        save(&state.data_dir, &selected)?;
    }
    Ok(())
}
#[tauri::command]
pub fn ai_selection(state: State<'_, Shared>) -> Result<Vec<i64>, String> {
    let mut ids: Vec<_> = lock(&state.selected)?.iter().copied().collect();
    ids.sort_unstable();
    Ok(ids)
}
#[tauri::command]
pub async fn set_ai_selected(
    scope_id: i64,
    selected: bool,
    state: State<'_, Shared>,
) -> Result<Vec<i64>, String> {
    let state = state.inner().clone();
    blocking(move || {
        if selected {
            lock(&state.db)?.root(scope_id).map_err(display_error)?;
        }
        let mut ids = lock(&state.selected)?;
        if selected {
            ids.insert(scope_id);
        } else {
            ids.remove(&scope_id);
        }
        save(&state.data_dir, &ids)?;
        let mut out: Vec<_> = ids.iter().copied().collect();
        out.sort_unstable();
        Ok(out)
    })
    .await
}
