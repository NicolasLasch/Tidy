//! Planning IPC: storage findings for the disk explorer and the chat's request pipeline.
//! Neither this module nor agent-runtime holds execution authority; they only propose.
use super::{Shared, blocking, display_error, lock};
use std::{collections::HashMap, time::Duration};
use tauri::State;
use tidy_agent_runtime::worker::Backend;
use tidy_organization::{FileCandidate, FileId};
use tidy_storage::{AnalyzableFile, StorageAnalysisConfig, StorageAnalysisResult, analyze_storage};

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

/// Turns a chat request into a reviewable plan: the deterministic engine first, then (only when it
/// finds nothing) the local model as described in docs/HOW-AI-IS-USED.md. Execution is solely in
/// safety_ipc.
#[tauri::command]
pub async fn plan_with_agent(
    scope_id: i64,
    request: String,
    previous: Option<String>,
    state: State<'_, Shared>,
) -> Result<tidy_agent_runtime::investigation::Investigation, String> {
    super::selection::require(state.inner(), scope_id)?;
    let state = state.inner().clone();
    state.ai.reserve("planning", "local", Some(scope_id))?;
    let completion = state.clone();
    let result = blocking(move || {
        if lock(&state.job)?.running {
            return Err("Wait for the folder scan to finish.".into());
        }
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
        let publish = |response: &tidy_agent_runtime::investigation::Investigation, note: &str| {
            if let Ok(mut job) = state.ai.job.lock() {
                job.message = note.into();
                job.planning_trace = response.trace.clone();
            }
        };
        // Follow-ups about the last listing ("why no size?", "measure them", "only the Rust ones").
        if let Some(previous) = previous.as_deref() {
            let root = lock(&state.db)?.root(scope_id).map_err(display_error)?;
            if let Some(response) = tidy_agent_runtime::intent::follow_up(&request, previous, &candidates, &scope_name, now, Some(&root)) {
                publish(&response, "Plan ready; no model needed");
                return Ok(response);
            }
        }
        if tidy_agent_runtime::intent::wants_projects(&request) {
            let root = lock(&state.db)?.root(scope_id).map_err(display_error)?;
            let response =
                tidy_agent_runtime::intent::list_projects(&root, &scope_name, &candidates);
            publish(&response, "Project list ready");
            return Ok(response);
        }
        let scope_root = lock(&state.db)?.root(scope_id).map_err(display_error)?;
        let mut fallback = None;
        match tidy_agent_runtime::intent::respond_in(&request, &candidates, &scope_name, now, Some(&scope_root)) {
            Some(response) if !response.unresolved => {
                publish(&response, "Plan ready; no model needed");
                return Ok(response);
            }
            Some(response) => fallback = Some(response),
            None => {}
        }
        // Beyond the direct engine, the local AI is used three ways, in order:
        // 1. restate the request as a plain command (fixing names against real folders);
        // 2. for folder building, translate it into matching rules that build a reviewed plan;
        // 3. answer questions from the index (where things are, what is what).
        // The model never produces file actions itself.
        let Some(spec) = state.ai.chosen() else {
            return Ok(fallback.unwrap_or_else(|| {
                tidy_agent_runtime::intent::not_understood(
                    candidates.len(),
                    "I couldn't map that to an action, and no local AI model is installed to help interpret it.",
                )
            }));
        };
        let model = state.ai.store.verify(&spec, &state.ai.cancel)?;
        let backend = if cfg!(target_os = "macos") { Backend::Metal } else { Backend::Cpu };
        let cancelled = || state.ai.cancel.load(std::sync::atomic::Ordering::Relaxed);
        let model_name = spec.name.clone();
        let finish = |mut r: tidy_agent_runtime::investigation::Investigation, note: &str| {
            r.trace.push(tidy_agent_runtime::investigation::Trace {
                label: "AI model".into(),
                detail: model_name.clone(),
            });
            publish(&r, note);
            Ok::<_, String>(r)
        };
        let last = request.rsplit("User follow-up:").next().unwrap_or(&request).trim().to_string();

        // 1. Restate as a command.
        let prompt = tidy_agent_runtime::intent::rewrite_prompt(&request, &candidates);
        match state.ai.worker.run(model.clone(), &prompt, backend, &state.ai.cancel, Duration::from_secs(60)) {
            Ok(answer) => {
                if let Some(command) = tidy_agent_runtime::intent::clean_rewrite(&answer.text) {
                    let mut response = if tidy_agent_runtime::intent::wants_projects(&command) {
                        let root = lock(&state.db)?.root(scope_id).map_err(display_error)?;
                        Some(tidy_agent_runtime::intent::list_projects(&root, &scope_name, &candidates))
                    } else {
                        tidy_agent_runtime::intent::respond_in(&command, &candidates, &scope_name, now, Some(&scope_root))
                    };
                    if let Some(mut r) = response.take()
                        && !r.unresolved
                    {
                        r.trace.insert(
                            0,
                            tidy_agent_runtime::investigation::Trace {
                                label: "Understood as".into(),
                                detail: command.clone(),
                            },
                        );
                        return finish(r, "Plan ready");
                    }
                }
            }
            Err(e) if cancelled() => return Err(e),
            Err(_) => {}
        }

        // 2. Folder building through reviewed matching rules.
        let lower = last.to_lowercase();
        let organizing = ["organize", "organise", "sort", "group", "arrange", "structure", "categorize", "categorise", "put all", "into folders", "subfolders", "folder for", "tidy"]
            .iter()
            .any(|w| lower.contains(w));
        if organizing && last.len() <= 900 {
            let prompt = rules_prompt(&last)?;
            if let Ok(answer) = state.ai.worker.run_rules(model.clone(), &prompt, backend, &state.ai.cancel, Duration::from_secs(90))
                && !answer.truncated
                && let Ok(plan) = tidy_organization::rules::apply_rules_for_request(&answer.text, &last, &candidates)
                && !plan.actions.is_empty()
            {
                let by_id: HashMap<u64, &FileCandidate> = candidates.iter().map(|f| (f.id.0, f)).collect();
                let mut plan = plan;
                plan.actions.truncate(500);
                let sources = plan
                    .actions
                    .iter()
                    .filter_map(|a| match a {
                        tidy_organization::ProposedAction::Move { source, .. } => by_id.get(&source.0).map(|f| tidy_agent_runtime::investigation::Source {
                            id: f.id.0,
                            path: f.relative_path.to_string_lossy().into(),
                            size: f.size,
                        }),
                        _ => None,
                    })
                    .collect();
                let mut r = tidy_agent_runtime::intent::not_understood(candidates.len(), "");
                r.engine = "model".into();
                r.clarification = None;
                r.proposal = tidy_organization::Proposal {
                    rationale: format!("{} The AI turned your request into matching rules; every move is listed for you to check. Nothing changes until you approve.", plan.rationale),
                    actions: plan.actions,
                };
                r.sources = sources;
                r.trace = vec![tidy_agent_runtime::investigation::Trace {
                    label: "AI built a folder plan".into(),
                    detail: "Matching rules were applied to every indexed file".into(),
                }];
                r.complete = true;
                return finish(r, "Folder plan ready");
            }
        }

        // 3. Answer from the index.
        let context = lock(&state.db)?
            .folder_context(scope_id, &last, &state.ai.cancel)
            .map_err(display_error)?;
        let mut evidence: Vec<tidy_agent_runtime::prompt::Evidence> = context
            .examples
            .into_iter()
            .take(10)
            .map(|f| tidy_agent_runtime::prompt::Evidence { id: f.id, path: f.path, bytes: f.size, excerpt: f.excerpt })
            .collect();
        let listing: Vec<tidy_storage::AnalyzableFile> = candidates
            .iter()
            .map(|f| tidy_storage::AnalyzableFile { id: f.id.0, path: f.relative_path.clone(), size: f.size, modified: f.modified, hash: None, identity: String::new() })
            .collect();
        let mut top = tidy_storage::folder_usage(&listing);
        top.retain(|f| f.path.components().count() <= 4 && !f.path.as_os_str().is_empty());
        top.sort_by_key(|f| std::cmp::Reverse(f.logical_bytes));
        evidence.extend(top.into_iter().take(10).map(|f| tidy_agent_runtime::prompt::Evidence {
            id: 0,
            path: format!("{}/", f.path.display()),
            bytes: f.logical_bytes as i64,
            excerpt: None,
        }));
        let overview = serde_json::to_string(&context.overview).map_err(display_error)?;
        if let Ok((prompt, _)) = tidy_agent_runtime::prompt::folder_prompt(&last, &overview, &evidence)
            && let Ok(answer) = state.ai.worker.run(model, &prompt, backend, &state.ai.cancel, Duration::from_secs(90))
            && !answer.text.trim().is_empty()
        {
            let mut r = tidy_agent_runtime::intent::not_understood(candidates.len(), "");
            r.engine = "model".into();
            r.proposal.rationale = answer.text.trim().to_string();
            r.trace = vec![tidy_agent_runtime::investigation::Trace {
                label: "Answered from your index".into(),
                detail: format!("Read statistics for all {} files plus the closest matches", candidates.len()),
            }];
            return finish(r, "Answer ready");
        }
        Ok(fallback.unwrap_or_else(|| {
            tidy_agent_runtime::intent::not_understood(
                candidates.len(),
                "I couldn't work out what you'd like me to do with that.",
            )
        }))
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

fn rules_prompt(goal: &str) -> Result<String, String> {
    Ok(format!(
        r#"Translate the user's file organization request into compact JSON rules. /no_think
Output ONLY {{"rules":[{{"destination":"Invoices","extensions":["pdf"],"name_contains":["invoice","receipt"]}}]}}.
At most 8 rules. extensions is a list of literal file extensions without dots (empty means any). name_contains matches ANY listed substring of the filename (empty means any). Both filters, when present, must match. At least one filter is required. destination is a relative folder. When subfolders by extension/type/format are requested, use the literal destination template Photos/{{EXT}} (replace Photos with the requested parent). For photos, include only photo extensions such as png,jpg,jpeg,heic,heif,webp,gif,bmp,tif,tiff,avif,raw,dng. Never include non-photo files in a photos-only request. Do not output individual files, shell, actions, markdown or commentary.
Only top-level files are eligible. Never infer document contents from filenames. If the request requires content analysis, dates, deletion, renaming, recursive moves, or is ambiguous, output {{"rules":[]}}. Do not substitute general sorting for a specific request.
USER_REQUEST_JSON: {}"#,
        serde_json::to_string(goal).map_err(display_error)?
    ))
}
