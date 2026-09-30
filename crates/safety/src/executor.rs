use super::{
    journal::Journal,
    model::{ExecutionReport, JournalState, Rejection, ValidatedAction},
    validator,
};
use std::{path::Path, time::Instant};
use tidy_platform::AuthorizedRoot;
pub fn execute_transaction(
    scope: &AuthorizedRoot,
    tx_id: i64,
    tx_uuid: &str,
    rationale: &str,
    actions: &[ValidatedAction],
    journal: &Journal,
) -> Result<ExecutionReport, Rejection> {
    let started = Instant::now();
    let detail = journal
        .get_transaction_detail(tx_id)
        .map_err(Rejection::IoError)?
        .ok_or(Rejection::StaleApproval)?;
    journal
        .transition_state(tx_id, JournalState::Applying)
        .map_err(Rejection::IoError)?;
    let result = (|| {
        for (idx, action) in actions.iter().enumerate() {
            let step = detail.steps.get(idx).ok_or(Rejection::StaleApproval)?;
            let (before, _, _) = journal.evidence(step.id).map_err(Rejection::IoError)?;
            let current = validator::action_fingerprint(scope, action)?;
            if current != before {
                return Err(Rejection::ChangedSource(
                    action.source().display().to_string(),
                ));
            }
            journal
                .record_step_result(step.id, "applying", None)
                .map_err(Rejection::IoError)?;
            let operation: Result<(), Rejection> = (|| {
                match action {
                    ValidatedAction::Move {
                        relative_source,
                        relative_dest,
                        ..
                    }
                    | ValidatedAction::Rename {
                        relative_source,
                        relative_dest,
                        ..
                    } => {
                        #[cfg(any(target_os = "macos", target_os = "linux"))]
                        let after = tidy_platform::handles::move_no_replace(
                            scope,
                            relative_source,
                            relative_dest,
                            &before,
                        )
                        .map_err(|e| Rejection::IoError(e.to_string()))?;
                        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                        let after: String = return Err(Rejection::InvalidAction(
                            "Safe moves unsupported on this platform".into(),
                        ));
                        journal
                            .record_evidence(step.id, &before, Some(&after), None)
                            .map_err(Rejection::IoError)?;
                    }
                    ValidatedAction::Copy {
                        relative_source,
                        relative_dest,
                        ..
                    } => {
                        #[cfg(unix)]
                        let after = tidy_platform::handles::copy_no_replace(
                            scope,
                            relative_source,
                            relative_dest,
                            &before,
                        )
                        .map_err(|e| Rejection::IoError(e.to_string()))?;
                        #[cfg(not(unix))]
                        let after: String = return Err(Rejection::InvalidAction(
                            "Safe copies unavailable on this platform".into(),
                        ));
                        journal
                            .record_evidence(step.id, &before, Some(&after), None)
                            .map_err(Rejection::IoError)?;
                    }
                    ValidatedAction::Permissions {
                        relative_source,
                        old_mode,
                        new_mode,
                        ..
                    } => {
                        #[cfg(unix)]
                        let after = tidy_platform::handles::set_permissions(
                            scope,
                            relative_source,
                            &before,
                            *old_mode,
                            *new_mode,
                        )
                        .map_err(|e| Rejection::IoError(e.to_string()))?;
                        #[cfg(not(unix))]
                        let after: String = return Err(Rejection::InvalidAction(
                            "Unix permissions unavailable on this platform".into(),
                        ));
                        journal
                            .record_evidence(step.id, &before, Some(&after), None)
                            .map_err(Rejection::IoError)?;
                    }
                    ValidatedAction::TrashDir { source, .. } => {
                        scope
                            .validate(source)
                            .map_err(|_| Rejection::OutsideScope)?;
                        let location =
                            super::native_trash::trash(source).map_err(Rejection::IoError)?;
                        let receipt = location.to_str().ok_or_else(|| {
                            Rejection::IoError(
                                "Trash receipt is not UTF-8; manual recovery required".into(),
                            )
                        })?;
                        journal
                            .record_evidence(step.id, &before, None, Some(receipt))
                            .map_err(Rejection::IoError)?;
                        let receipt_metadata = std::fs::symlink_metadata(&location)
                            .map_err(|e| Rejection::IoError(e.to_string()))?;
                        #[cfg(unix)]
                        let receipt_matches = {
                            use std::os::unix::fs::MetadataExt;
                            let stamp =
                                format!("{}:{}", receipt_metadata.dev(), receipt_metadata.ino());
                            let parts: Vec<_> = before.split(':').collect();
                            parts.len() > 2 && stamp == format!("{}:{}", parts[1], parts[2])
                        };
                        #[cfg(not(unix))]
                        let receipt_matches = false;
                        if std::fs::symlink_metadata(source).is_ok()
                            || !receipt_metadata.file_type().is_dir()
                            || !receipt_matches
                        {
                            return Err(Rejection::IoError(
                                "Trash result needs manual recovery".into(),
                            ));
                        }
                    }
                    ValidatedAction::Trash { source, .. } => {
                        scope
                            .validate(source)
                            .map_err(|_| Rejection::OutsideScope)?;
                        let location =
                            super::native_trash::trash(source).map_err(Rejection::IoError)?;
                        let receipt = location.to_str().ok_or_else(|| {
                            Rejection::IoError(
                                "Trash receipt is not UTF-8; manual recovery required".into(),
                            )
                        })?;
                        journal
                            .record_evidence(step.id, &before, None, Some(receipt))
                            .map_err(Rejection::IoError)?;
                        let receipt_metadata = std::fs::symlink_metadata(&location)
                            .map_err(|e| Rejection::IoError(e.to_string()))?;
                        #[cfg(unix)]
                        let receipt_matches = {
                            use std::os::unix::fs::MetadataExt;
                            let stamp = format!(
                                "{}:{}:{}:{}:{}",
                                receipt_metadata.dev(),
                                receipt_metadata.ino(),
                                receipt_metadata.size(),
                                receipt_metadata.mtime(),
                                receipt_metadata.mtime_nsec()
                            );
                            stamp == before.split(':').take(5).collect::<Vec<_>>().join(":")
                        };
                        #[cfg(not(unix))]
                        let receipt_matches = false;
                        if std::fs::symlink_metadata(source).is_ok()
                            || !receipt_metadata.file_type().is_file()
                            || !receipt_matches
                        {
                            return Err(Rejection::IoError(
                                "Trash result needs manual recovery".into(),
                            ));
                        }
                    }
                }
                Ok(())
            })();
            if let Err(e) = operation {
                journal
                    .record_step_result(step.id, "needs_recovery", Some(&e.to_string()))
                    .map_err(Rejection::IoError)?;
                return Err(e);
            }
            journal
                .record_step_result(step.id, "verified", None)
                .map_err(Rejection::IoError)?;
        }
        journal
            .transition_state(tx_id, JournalState::Applied)
            .map_err(Rejection::IoError)?;
        journal
            .transition_state(tx_id, JournalState::Verified)
            .map_err(Rejection::IoError)?;
        Ok(ExecutionReport {
            transaction_id: tx_id,
            tx_uuid: tx_uuid.into(),
            actions_applied: actions.len(),
            verified: true,
            duration_ms: started.elapsed().as_millis() as u64,
            rationale: rationale.into(),
        })
    })();
    if result.is_err() {
        journal
            .transition_state(tx_id, JournalState::NeedsRecovery)
            .map_err(Rejection::IoError)?;
    }
    result
}
pub fn prepare_undo(
    scope: &AuthorizedRoot,
    journal: &Journal,
    tx_id: i64,
) -> Result<(String, Vec<ValidatedAction>), Rejection> {
    let detail = journal
        .get_transaction_detail(tx_id)
        .map_err(Rejection::IoError)?
        .ok_or(Rejection::StaleApproval)?;
    if !matches!(
        detail.summary.state,
        JournalState::Verified | JournalState::Applied | JournalState::NeedsRecovery
    ) {
        return Err(Rejection::InvalidAction(
            "Transaction is not undoable".into(),
        ));
    }
    if detail
        .steps
        .iter()
        .any(|s| s.action_type == "trash" || s.action_type == "trash_dir")
    {
        return Err(Rejection::InvalidAction("Restore trashed files using Finder Trash. Recorded Trash locations appear in history; automatic Trash undo is not available.".into()));
    }
    let mut actions = Vec::new();
    for step in detail.steps.iter().rev() {
        if step.state != "verified" {
            continue;
        }
        if step.action_type == "permissions" {
            let (_, after, _) = journal.evidence(step.id).map_err(Rejection::IoError)?;
            if after.as_deref()
                != Some(&validator::fingerprint(
                    scope,
                    Path::new(&step.source_relative),
                )?)
            {
                return Err(Rejection::ChangedSource(step.source_relative.clone()));
            }
            let (old, _) = journal
                .permission_modes(step.id)
                .map_err(Rejection::IoError)?;
            actions.push(validator::validate_permissions(
                scope,
                Path::new(&step.source_relative),
                old,
            )?);
            continue;
        }
        let dest = step
            .destination_relative
            .as_ref()
            .ok_or(Rejection::StaleApproval)?;
        let (_, after, _) = journal.evidence(step.id).map_err(Rejection::IoError)?;
        if after.as_deref() != Some(&validator::fingerprint(scope, Path::new(dest))?) {
            return Err(Rejection::ChangedSource(dest.clone()));
        }
        if step.action_type == "copy" {
            actions.push(validator::validate_trash(scope, Path::new(dest))?);
            continue;
        }
        actions.push(validator::validate_move(
            scope,
            Path::new(dest),
            Path::new(&step.source_relative),
        )?);
    }
    Ok((
        format!(
            "Undo {} verified changes (copies go to native Trash) from transaction #{tx_id}. Empty created folders remain. Inspect any unverified steps manually.",
            actions.len()
        ),
        actions,
    ))
}
