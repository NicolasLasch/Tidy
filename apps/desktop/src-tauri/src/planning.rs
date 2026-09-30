//! Desktop planning IPC integration.
//! Generates storage analysis findings and organization proposals (deterministic and validated AI rules).
//! Neither this module nor agent-runtime holds execution authority.
use super::{Shared, blocking, display_error, lock};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tauri::State;
use tidy_agent_runtime::worker::Backend;
use tidy_organization::{
    FileCandidate, FileId, OrganizationMode, Proposal, propose,
    propose_storage_cleanup as org_propose_storage_cleanup,
};
use tidy_storage::{
    AnalyzableFile, FindingKind, StorageAnalysisConfig, StorageAnalysisResult, analyze_storage,
};

#[tauri::command]
pub async fn analyze_storage_scope(
    scope_id: i64,
    state: State<'_, Shared>,
) -> Result<StorageAnalysisResult, String> {
    let state = state.inner().clone();
    blocking(move || {
        let db = lock(&state.db)?;
        let files = db.current_files(scope_id).map_err(display_error)?;
        let analyzable: Vec<AnalyzableFile> = files
            .into_iter()
            .map(|f| AnalyzableFile {
                id: f.id as u64,
                path: f.path,
                size: f.size,
                modified: f.modified,
                hash: f.hash,
                identity: f.identity,
            })
            .collect();
        let config = StorageAnalysisConfig::default();
        Ok(analyze_storage(&analyzable, &config))
    })
    .await
}

#[tauri::command]
pub async fn propose_organization(
    scope_id: i64,
    mode: OrganizationMode,
    project_name: Option<String>,
    custom_instruction: Option<String>,
    use_ai: bool,
    state: State<'_, Shared>,
) -> Result<Proposal, String> {
    let state = state.inner().clone();
    super::selection::require(&state, scope_id)?;
    let use_ai = use_ai && mode == OrganizationMode::Custom;
    if use_ai {
        state.ai.reserve("planning", "local", Some(scope_id))?;
    }
    let completion = state.clone();
    let result=blocking(move || {
        let files = {
            let db = lock(&state.db)?;
            db.current_files(scope_id).map_err(display_error)?
        };

        let candidates: Vec<FileCandidate> = files
            .iter()
            .map(|f| FileCandidate {
                id: FileId(f.id as u64),
                relative_path: f.path.clone(),
                size: f.size,
                modified: f.modified,
                excerpt: f.excerpt.clone(),
            })
            .collect();

        let param=match mode {OrganizationMode::Project=>project_name.as_deref(),OrganizationMode::Custom=>custom_instruction.as_deref(),_=>None};
        let mut baseline=propose(mode,param,&candidates);
        if baseline.actions.len()>500 {
            baseline.rationale.push_str(&format!(" Showing the first 500 of {} actions. Apply this reviewed batch, then generate another plan.",baseline.actions.len()));
            baseline.actions.truncate(500);
        }
        if use_ai {
            if state.ai.worker.bytes == 0 || !state.ai.worker.executable.is_file() {
                return Err("Local inference worker is missing. Reinstall the current Tidy build.".into());
            }
        let spec=tidy_agent_runtime::catalog::MODELS.iter().find(|s|state.ai.store.present(s))
                .ok_or("No local model installed. Install one in Local AI, or turn AI off for explicit mapping rules.")?;
            let model_path=state.ai.store.verify(spec,&state.ai.cancel)
                .map_err(|e|format!("Model verification failed: {e}"))?;
            // Translate intent once, then run deterministic selectors over the complete index.
            let goal=match mode {
                OrganizationMode::Category=>"Group files by their type into named folders".to_string(),
                OrganizationMode::Date=>return Ok(baseline),
                OrganizationMode::Project=>format!("Group files whose filenames contain {} into Projects/{}",project_name.as_deref().unwrap_or(""),project_name.as_deref().unwrap_or("")),
                OrganizationMode::Custom=>custom_instruction.clone().unwrap_or_default(),
            };
            if goal.len()>1000 {return Err("Keep the request under 1,000 bytes.".into());}
            let prompt=format!(r#"Translate the user's file organization request into compact JSON rules. /no_think
Output ONLY {{"rules":[{{"destination":"Invoices","extensions":["pdf"],"name_contains":["invoice","receipt"]}}]}}.
At most 8 rules. extensions is a list of literal file extensions without dots (empty means any). name_contains matches ANY listed substring of the filename (empty means any). Both filters, when present, must match. At least one filter is required. destination is a relative folder. When subfolders by extension/type/format are requested, use the literal destination template Photos/{{EXT}} (replace Photos with the requested parent). For photos, include only photo extensions such as png,jpg,jpeg,heic,heif,webp,gif,bmp,tif,tiff,avif,raw,dng. Never include non-photo files in a photos-only request. Do not output individual files, shell, actions, markdown or commentary.
Only top-level files are eligible. Never infer document contents from filenames. If the request requires content analysis, dates, deletion, renaming, recursive moves, or is ambiguous, output {{"rules":[]}}. Do not substitute general sorting for a specific request.
USER_REQUEST_JSON: {}"#,serde_json::to_string(&goal).map_err(display_error)?);
            let backend=if cfg!(target_os="macos"){Backend::Metal}else{Backend::Cpu};
            let answer=state.ai.worker.run_rules(model_path,&prompt,backend,&state.ai.cancel,Duration::from_secs(90))
                .map_err(|e|format!("Local planning failed: {e}. No substitute plan was created."))?;
            if answer.truncated {return Err("Model response exceeded its output limit. Try one grouping rule at a time. No substitute plan was created.".into());}
            return tidy_organization::rules::apply_rules_for_request(&answer.text,&goal,&candidates);
        }
        Ok(baseline)
    })
    .await;
    if use_ai && let Ok(mut job) = completion.ai.job.lock() {
        job.running = false;
        job.message = "Planning complete; inspect the preview".into();
    }
    result
}

#[tauri::command]
pub async fn propose_storage_cleanup(
    scope_id: i64,
    keep_duplicate_ids: Vec<u64>,
    selected_file_ids: Option<Vec<u64>>,
    state: State<'_, Shared>,
) -> Result<Proposal, String> {
    let state = state.inner().clone();
    blocking(move || {
        let db = lock(&state.db)?;
        let files = db.current_files(scope_id).map_err(display_error)?;
        let valid_file_ids: HashSet<u64> = files.iter().map(|f| f.id as u64).collect();

        // If the user selected specific items (delete one by one or specific categories)
        if let Some(selected) = selected_file_ids

        {
            if selected.len()>500 {return Err("Select at most 500 files per reviewed cleanup batch".into());}
            let selected_set:HashSet<_>=selected.iter().copied().collect();
            if keep_duplicate_ids.iter().any(|id| selected_set.contains(id)) {return Err("A preserved duplicate cannot also be selected for Trash".into());}
            let mut hashes:HashMap<&str,Vec<u64>>=HashMap::new();
            for f in &files {if let Some(hash)=f.hash.as_deref() {hashes.entry(hash).or_default().push(f.id as u64);}}
            if hashes.values().any(|ids|ids.len()>1 && ids.iter().all(|id|selected_set.contains(id))) {return Err("Keep at least one file in every indexed duplicate group".into());}
            if selected.iter().any(|id| !valid_file_ids.contains(id)) {return Err("Selection contains files no longer in this scope".into());}
            let mut actions = Vec::new();
            let mut selected_seen = HashSet::new();
            for id in selected {
                if valid_file_ids.contains(&id) && selected_seen.insert(id) {
                    actions.push(tidy_organization::ProposedAction::Trash {
                        source: FileId(id),
                    });
                }
            }
            let count = actions.len();
            let rationale = format!(
                "Selected storage cleanup: {} item{} proposed for Trash. Files move to OS Trash only after approval. Restore them through Finder Trash.",
                count,
                if count == 1 { "" } else { "s" }
            );
            return Ok(Proposal { actions, rationale });
        }

        let analyzable: Vec<AnalyzableFile> = files
            .into_iter()
            .map(|f| AnalyzableFile {
                id: f.id as u64,
                path: f.path,
                size: f.size,
                modified: f.modified,
                hash: f.hash,
                identity: f.identity,
            })
            .collect();
        let config = StorageAnalysisConfig::default();
        let result = analyze_storage(&analyzable, &config);

        let mut dup_groups = Vec::new();
        let mut installer_ids = Vec::new();
        let mut artifact_ids = Vec::new();

        for finding in result.findings {
            match finding.kind {
                FindingKind::ExactDuplicate => {
                    dup_groups.push(finding.file_ids);
                }
                FindingKind::OldInstaller => {
                    installer_ids.extend(finding.file_ids);
                }
                FindingKind::DevelopmentArtifact => {
                    artifact_ids.extend(finding.file_ids);
                }
                FindingKind::LargeFile => {}
            }
        }

        let keep_set: HashSet<u64> = keep_duplicate_ids.into_iter().collect();
        let mut kept_copy_by_group = HashMap::new();
        for (idx, group) in dup_groups.iter().enumerate() {
            if let Some(&kept) = group.iter().find(|id| keep_set.contains(id)) {
                kept_copy_by_group.insert(idx, kept);
            }
        }

        Ok(org_propose_storage_cleanup(
            &dup_groups,
            &kept_copy_by_group,
            &installer_ids,
            &artifact_ids,
        ))
    })
    .await
}

#[derive(serde::Deserialize)]
pub struct ConversationTurn {
    role: String,
    text: String,
}
/// The model investigates and proposes. Execution remains solely in safety_ipc.
#[tauri::command]
pub async fn plan_with_agent(
    scope_id: i64,
    request: String,
    previous: Vec<tidy_organization::ProposedAction>,
    workflow_id: Option<String>,
    conversation: Vec<ConversationTurn>,
    state: State<'_, Shared>,
) -> Result<tidy_agent_runtime::investigation::Investigation, String> {
    if conversation.len() > 10
        || conversation
            .iter()
            .any(|t| !matches!(t.role.as_str(), "user" | "assistant") || t.text.len() > 3000)
    {
        return Err("Conversation is too long; start a new request".into());
    }
    let conversation_json = serde_json::to_string(
        &conversation
            .iter()
            .map(|t| serde_json::json!({"role":t.role,"text":t.text}))
            .collect::<Vec<_>>(),
    )
    .map_err(display_error)?;
    let state = state.inner().clone();
    state.ai.reserve("planning", "local", Some(scope_id))?;
    let completion = state.clone();
    let result = blocking(move || {
        if lock(&state.job)?.running {
            return Err("Wait for the folder scan to finish.".into());
        }
        if workflow_id.is_none() {
            let (indexed, scope_name) = {
                let db = lock(&state.db)?;
                let name = db
                    .root(scope_id)
                    .map_err(display_error)?
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "this folder".into());
                (db.storage_files(scope_id).map_err(display_error)?, name)
            };
            if indexed.is_empty() {
                return Err("Index this folder first: press Scan folder.".into());
            }
            let candidates: Vec<_> = indexed
                .into_iter()
                .map(|f| FileCandidate {
                    id: FileId(f.id as u64),
                    relative_path: f.path,
                    size: f.size,
                    modified: f.modified,
                    excerpt: f.excerpt,
                })
                .collect();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if let Some(response) =
                tidy_agent_runtime::intent::respond(&request, &candidates, &scope_name, now)
            {
                if let Ok(mut job) = state.ai.job.lock() {
                    job.message = "Plan ready; no model needed".into();
                    job.planning_trace = response.trace.clone();
                }
                return Ok(response);
            }
        }
        let extension = if workflow_id
            .as_deref()
            .is_none_or(|id| id == "change_extensions")
        {
            tidy_agent_runtime::fast_extension::target(&request)
        } else {
            None
        };
        let fast =
            tidy_agent_runtime::fast_trash::plain_text_removal(&request, workflow_id.as_deref());
        let files = if fast || extension.is_some() {
            lock(&state.db)?
                .storage_files(scope_id)
                .map_err(display_error)?
        } else {
            lock(&state.db)?
                .current_files(scope_id)
                .map_err(display_error)?
        };
        if files.is_empty() {
            return Err("Index this folder first using Scan folder.".into());
        }
        let candidates: Vec<_> = files
            .into_iter()
            .map(|f| FileCandidate {
                id: FileId(f.id as u64),
                relative_path: f.path,
                size: f.size,
                modified: f.modified,
                excerpt: f.excerpt,
            })
            .collect();
        if let Some(extension) = extension {
            let response = tidy_agent_runtime::fast_extension::preview(
                &candidates,
                &extension,
                &state.ai.cancel,
            )?;
            lock(&state.db)?.root(scope_id).map_err(display_error)?;
            return Ok(response);
        }
        if fast {
            let response =
                tidy_agent_runtime::fast_trash::preview(&candidates, &previous, &state.ai.cancel)?;
            if let Ok(mut job) = state.ai.job.lock() {
                job.message = "Exact text-file preview ready; model loading skipped".into();
                job.planning_trace = response.trace.clone();
            }
            lock(&state.db)?
                .root(scope_id)
                .map_err(|_| "Folder removed; preview discarded".to_string())?;
            return Ok(response);
        }
        let Some(spec) = tidy_agent_runtime::catalog::MODELS
            .iter()
            .find(|s| state.ai.store.present(s))
        else {
            return Ok(tidy_agent_runtime::intent::not_understood(
                candidates.len(),
                "I don't have the AI model installed, so I can only handle direct requests.",
            ));
        };
        let model = state.ai.store.verify(spec, &state.ai.cancel)?;
        let backend = if cfg!(target_os = "macos") {
            Backend::Metal
        } else {
            Backend::Cpu
        };
        let mut session = tidy_agent_runtime::session::Session::start(
            &state.ai.worker,
            model,
            backend,
            &state.ai.cancel,
        )?;
        let start = std::time::Instant::now();
        let response = match tidy_agent_runtime::investigation::investigate_in_conversation(
            &request,
            &candidates,
            &previous,
            workflow_id.as_deref(),
            &conversation_json,
            &state.ai.cancel,
            |prompt| {
                let remaining = Duration::from_secs(240).saturating_sub(start.elapsed());
                if remaining.is_zero() {
                    return Err("Investigation reached its four-minute budget".into());
                }
                session.request(
                    prompt,
                    &state.ai.cancel,
                    remaining.min(Duration::from_secs(90)),
                )
            },
            |entry| {
                if let Ok(mut job) = state.ai.job.lock() {
                    job.message = entry.label.clone();
                    job.planning_trace.push(entry.clone());
                }
            },
        ) {
            Ok(r) => r,
            Err(e) if !state.ai.cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                tidy_agent_runtime::intent::not_understood(
                    candidates.len(),
                    &format!("The AI could not turn that into a plan ({e})."),
                )
            }
            Err(e) => return Err(e),
        };
        lock(&state.db)?
            .root(scope_id)
            .map_err(|_| "Folder removed; preview discarded".to_string())?;
        Ok(response)
    })
    .await;
    if let Ok(mut job) = completion.ai.job.lock() {
        job.running = false;
        job.message = match &result {
            Ok(_) => "Preview ready for your review".into(),
            Err(e) => e.clone(),
        };
    }
    result
}

#[tauri::command]
pub fn workflow_catalog() -> Vec<tidy_agent_runtime::workflows::Workflow> {
    tidy_agent_runtime::workflows::catalog().to_vec()
}
