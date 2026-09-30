use super::model::{JournalState, ValidatedAction};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionSummary {
    pub id: i64,
    pub tx_uuid: String,
    pub scope_id: i64,
    pub state: JournalState,
    pub rationale: String,
    pub actions_count: usize,
    pub created_at: i64,
    pub applied_at: Option<i64>,
    pub verified_at: Option<i64>,
    pub undone_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalStepRecord {
    pub id: i64,
    pub transaction_id: i64,
    pub step_order: i64,
    pub action_type: String,
    pub source_relative: String,
    pub destination_relative: Option<String>,
    pub original_size: u64,
    pub original_modified: i64,
    pub state: String,
    pub error: Option<String>,
    pub trash_location: Option<String>,
    pub old_mode: Option<u32>,
    pub new_mode: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionDetail {
    pub summary: TransactionSummary,
    pub steps: Vec<JournalStepRecord>,
}

pub struct Journal {
    conn: Mutex<Connection>,
}

impl Journal {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        let journal = Self {
            conn: Mutex::new(conn),
        };
        journal.migrate()?;
        Ok(journal)
    }

    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        let journal = Self {
            conn: Mutex::new(conn),
        };
        journal.migrate()?;
        Ok(journal)
    }

    fn migrate(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS permission_changes(step_id INTEGER PRIMARY KEY, old_mode INTEGER NOT NULL, new_mode INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS step_evidence(step_id INTEGER PRIMARY KEY, before_stamp TEXT NOT NULL, after_stamp TEXT, trash_path TEXT);
             CREATE TABLE IF NOT EXISTS transaction_bindings(tx_id INTEGER PRIMARY KEY, root TEXT NOT NULL, root_identity TEXT NOT NULL, undo_of INTEGER);

             CREATE TABLE IF NOT EXISTS transactions (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 tx_uuid TEXT NOT NULL UNIQUE,
                 scope_id INTEGER NOT NULL,
                 state TEXT NOT NULL,
                 rationale TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 applied_at INTEGER,
                 verified_at INTEGER,
                 undone_at INTEGER
             );

             CREATE TABLE IF NOT EXISTS journal_steps (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
                 step_order INTEGER NOT NULL,
                 action_type TEXT NOT NULL,
                 source_relative TEXT NOT NULL,
                 destination_relative TEXT,
                 original_size INTEGER NOT NULL,
                 original_modified INTEGER NOT NULL,
                 state TEXT NOT NULL,
                 error TEXT
             );

             CREATE INDEX IF NOT EXISTS idx_transactions_scope ON transactions(scope_id);
             CREATE INDEX IF NOT EXISTS idx_journal_steps_tx ON journal_steps(transaction_id);",
        )?;
        Ok(())
    }

    pub fn bind(
        &self,
        tx: i64,
        root: &tidy_platform::AuthorizedRoot,
        undo_of: Option<i64>,
    ) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO transaction_bindings VALUES(?1,?2,?3,?4)",
                params![tx, root.path().to_string_lossy(), root_key(root), undo_of],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn check_binding(
        &self,
        tx: i64,
        root: &tidy_platform::AuthorizedRoot,
        undo_of: Option<i64>,
    ) -> Result<(), String> {
        let value: (String, String, Option<i64>) = self
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT root,root_identity,undo_of FROM transaction_bindings WHERE tx_id=?1",
                [tx],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| {
                "Legacy or unbound transaction; automatic execution/undo refused".to_string()
            })?;
        if value
            != (
                root.path().to_string_lossy().into(),
                root_key(root),
                undo_of,
            )
        {
            return Err("Approval belongs to a different root or undo transaction".into());
        }
        Ok(())
    }
    pub fn evidence(&self, step: i64) -> Result<(String, Option<String>, Option<String>), String> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT before_stamp,after_stamp,trash_path FROM step_evidence WHERE step_id=?1",
                [step],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| e.to_string())
    }
    pub fn record_evidence(
        &self,
        step: i64,
        before: &str,
        after: Option<&str>,
        trash: Option<&str>,
    ) -> Result<(), String> {
        self.conn.lock().unwrap().execute("INSERT INTO step_evidence VALUES(?1,?2,?3,?4) ON CONFLICT(step_id) DO UPDATE SET after_stamp=excluded.after_stamp,trash_path=excluded.trash_path",params![step,before,after,trash]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn record_prepared(
        &self,
        tx_uuid: &str,
        scope_id: i64,
        rationale: &str,
        actions: &[ValidatedAction],
    ) -> Result<i64, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        let created_at = now_secs();
        tx.execute(
            "INSERT INTO transactions (tx_uuid, scope_id, state, rationale, created_at)
             VALUES (?1, ?2, 'prepared', ?3, ?4)",
            params![tx_uuid, scope_id, rationale, created_at],
        )
        .map_err(|e| e.to_string())?;

        let tx_id = tx.last_insert_rowid();

        for (idx, action) in actions.iter().enumerate() {
            let action_type = action.action_type_str();
            let source_rel = action.relative_source().to_string_lossy().to_string();
            let dest_rel = action
                .relative_dest()
                .map(|p| p.to_string_lossy().to_string());
            let size = action.original_size() as i64;
            let modified = match action {
                ValidatedAction::CreateDir { .. }
                | ValidatedAction::MoveDir { .. }
                | ValidatedAction::Restore { .. } => 0,
                ValidatedAction::Move {
                    original_modified, ..
                }
                | ValidatedAction::Rename {
                    original_modified, ..
                }
                | ValidatedAction::Trash {
                    original_modified, ..
                }
                | ValidatedAction::TrashDir {
                    original_modified, ..
                }
                | ValidatedAction::Copy {
                    original_modified, ..
                }
                | ValidatedAction::Permissions {
                    original_modified, ..
                } => *original_modified,
            };

            tx.execute(
                "INSERT INTO journal_steps (
                     transaction_id, step_order, action_type, source_relative,
                     destination_relative, original_size, original_modified, state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending')",
                params![
                    tx_id,
                    idx as i64,
                    action_type,
                    source_rel,
                    dest_rel,
                    size,
                    modified
                ],
            )
            .map_err(|e| e.to_string())?;
            if let ValidatedAction::Permissions {
                old_mode, new_mode, ..
            } = action
            {
                tx.execute(
                    "INSERT INTO permission_changes VALUES(?1,?2,?3)",
                    params![tx.last_insert_rowid(), old_mode, new_mode],
                )
                .map_err(|e| e.to_string())?;
            }
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(tx_id)
    }

    pub fn permission_modes(&self, step: i64) -> Result<(u32, u32), String> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT old_mode,new_mode FROM permission_changes WHERE step_id=?1",
                [step],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())
    }
    pub fn transition_state(&self, tx_id: i64, new_state: JournalState) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let now = now_secs();
        let state_str = new_state.to_string();

        match new_state {
            JournalState::Applying => conn.execute(
                "UPDATE transactions SET state = ?1, applied_at = ?2 WHERE id = ?3",
                params![state_str, now, tx_id],
            ),
            JournalState::Verified => conn.execute(
                "UPDATE transactions SET state = ?1, verified_at = ?2 WHERE id = ?3",
                params![state_str, now, tx_id],
            ),
            JournalState::Undone => conn.execute(
                "UPDATE transactions SET state = ?1, undone_at = ?2 WHERE id = ?3",
                params![state_str, now, tx_id],
            ),
            _ => conn.execute(
                "UPDATE transactions SET state = ?1 WHERE id = ?2",
                params![state_str, tx_id],
            ),
        }
        .map_err(|e| e.to_string())?;

        Ok(())
    }

    pub fn record_step_result(
        &self,
        step_id: i64,
        state: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE journal_steps SET state = ?1, error = ?2 WHERE id = ?3",
            params![state, error, step_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_transactions(
        &self,
        scope_id: Option<i64>,
    ) -> Result<Vec<TransactionSummary>, String> {
        let conn = self.conn.lock().unwrap();
        let sql = match scope_id {
            Some(_) => {
                "SELECT t.id, t.tx_uuid, t.scope_id, t.state, t.rationale,
                        (SELECT COUNT(*) FROM journal_steps s WHERE s.transaction_id = t.id),
                        t.created_at, t.applied_at, t.verified_at, t.undone_at
                 FROM transactions t
                 WHERE t.scope_id = ?1
                 ORDER BY t.created_at DESC, t.id DESC"
            }
            None => {
                "SELECT t.id, t.tx_uuid, t.scope_id, t.state, t.rationale,
                        (SELECT COUNT(*) FROM journal_steps s WHERE s.transaction_id = t.id),
                        t.created_at, t.applied_at, t.verified_at, t.undone_at
                 FROM transactions t
                 ORDER BY t.created_at DESC, t.id DESC"
            }
        };

        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;

        let rows = if let Some(sid) = scope_id {
            stmt.query_map(params![sid], Self::map_summary_row)
        } else {
            stmt.query_map([], Self::map_summary_row)
        }
        .map_err(|e| e.to_string())?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r.map_err(|e| e.to_string())?);
        }
        Ok(result)
    }

    fn map_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransactionSummary> {
        let state_raw: String = row.get(3)?;
        let state = state_raw.parse().unwrap_or(JournalState::Prepared);

        Ok(TransactionSummary {
            id: row.get(0)?,
            tx_uuid: row.get(1)?,
            scope_id: row.get(2)?,
            state,
            rationale: row.get(4)?,
            actions_count: row.get::<_, i64>(5)? as usize,
            created_at: row.get(6)?,
            applied_at: row.get(7)?,
            verified_at: row.get(8)?,
            undone_at: row.get(9)?,
        })
    }

    pub fn get_transaction_detail(&self, tx_id: i64) -> Result<Option<TransactionDetail>, String> {
        let conn = self.conn.lock().unwrap();
        let summary: Option<TransactionSummary> = conn
            .query_row(
                "SELECT t.id, t.tx_uuid, t.scope_id, t.state, t.rationale,
                        (SELECT COUNT(*) FROM journal_steps s WHERE s.transaction_id = t.id),
                        t.created_at, t.applied_at, t.verified_at, t.undone_at
                 FROM transactions t
                 WHERE t.id = ?1",
                params![tx_id],
                Self::map_summary_row,
            )
            .optional()
            .map_err(|e| e.to_string())?;

        let Some(summary) = summary else {
            return Ok(None);
        };

        let mut stmt = conn
            .prepare(
                "SELECT id, transaction_id, step_order, action_type, source_relative,
                        destination_relative, original_size, original_modified, state, error,
                        (SELECT trash_path FROM step_evidence e WHERE e.step_id=journal_steps.id),
                        (SELECT old_mode FROM permission_changes p WHERE p.step_id=journal_steps.id),
                        (SELECT new_mode FROM permission_changes p WHERE p.step_id=journal_steps.id)
                 FROM journal_steps
                 WHERE transaction_id = ?1
                 ORDER BY step_order ASC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![tx_id], |r| {
                Ok(JournalStepRecord {
                    id: r.get(0)?,
                    transaction_id: r.get(1)?,
                    step_order: r.get(2)?,
                    action_type: r.get(3)?,
                    source_relative: r.get(4)?,
                    destination_relative: r.get(5)?,
                    original_size: r.get::<_, i64>(6)? as u64,
                    original_modified: r.get(7)?,
                    state: r.get(8)?,
                    error: r.get(9)?,
                    trash_location: r.get(10)?,
                    old_mode: r.get(11)?,
                    new_mode: r.get(12)?,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut steps = Vec::new();
        for step in rows {
            steps.push(step.map_err(|e| e.to_string())?);
        }

        Ok(Some(TransactionDetail { summary, steps }))
    }

    /// Startup recovery check: any transactions left in 'applying' state are transitioned to 'needs_recovery'
    pub fn check_startup_recovery(&self) -> Result<Vec<i64>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM transactions WHERE state = 'applying'")
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;

        let mut recovered_ids = Vec::new();
        for r in rows {
            recovered_ids.push(r.map_err(|e| e.to_string())?);
        }

        if !recovered_ids.is_empty() {
            conn.execute(
                "UPDATE transactions SET state = 'needs_recovery' WHERE state = 'applying'",
                [],
            )
            .map_err(|e| e.to_string())?;
        }

        Ok(recovered_ids)
    }
}

fn root_key(root: &tidy_platform::AuthorizedRoot) -> String {
    #[cfg(unix)]
    {
        let (dev, ino) = root.identity();
        format!("{dev}:{ino}")
    }
    #[cfg(not(unix))]
    {
        root.path().to_string_lossy().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn records_transaction_and_detects_startup_recovery() {
        let journal = Journal::open_in_memory().unwrap();
        let action = ValidatedAction::Move {
            source: PathBuf::from("/tmp/src"),
            destination: PathBuf::from("/tmp/dst"),
            relative_source: PathBuf::from("src"),
            relative_dest: PathBuf::from("dst"),
            original_size: 100,
            original_modified: 200,
        };

        let tx_id = journal
            .record_prepared("tx-1", 1, "test move", &[action])
            .unwrap();
        assert_eq!(tx_id, 1);

        // Transition to applying
        journal
            .transition_state(tx_id, JournalState::Applying)
            .unwrap();

        // Simulate crash / startup check
        let recovered = journal.check_startup_recovery().unwrap();
        assert_eq!(recovered, vec![tx_id]);

        let detail = journal.get_transaction_detail(tx_id).unwrap().unwrap();
        assert_eq!(detail.summary.state, JournalState::NeedsRecovery);
        assert_eq!(detail.steps.len(), 1);
        assert_eq!(detail.steps[0].action_type, "move");
    }
}
