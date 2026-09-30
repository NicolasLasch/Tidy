use super::model::{Rejection, ValidatedAction};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub token: String,
    pub tx_id: i64,
    pub scope_id: i64,
    pub created_at: u64,
    pub expires_at: u64,
    pub actions: Vec<ValidatedAction>,
    pub used: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalView {
    pub approval_id: String,
    pub token: String,
    pub tx_id: i64,
    pub scope_id: i64,
    pub expires_at: u64,
    pub actions_count: usize,
    pub actions: Vec<ValidatedAction>,
}

#[derive(Default)]
pub struct ApprovalManager {
    approvals: Mutex<HashMap<String, Approval>>,
}

impl ApprovalManager {
    pub fn new() -> Self {
        Self {
            approvals: Mutex::new(HashMap::new()),
        }
    }

    /// Creates an explicit one-use approval token.
    /// Expiration defaults to 300 seconds (5 minutes) if 0 is passed.
    pub fn create_approval(
        &self,
        tx_id: i64,
        scope_id: i64,
        actions: Vec<ValidatedAction>,
        ttl_seconds: u64,
    ) -> (Approval, ApprovalView) {
        let ttl = if ttl_seconds == 0 { 300 } else { ttl_seconds };
        let now = now_secs();
        let expires_at = now + ttl;

        let approval_id = Uuid::new_v4().to_string();
        let token = format!("{}-{}", Uuid::new_v4(), Uuid::new_v4());

        let approval = Approval {
            id: approval_id.clone(),
            token: token.clone(),
            tx_id,
            scope_id,
            created_at: now,
            expires_at,
            actions: actions.clone(),
            used: false,
        };

        let view = ApprovalView {
            approval_id,
            token: token.clone(),
            tx_id,
            scope_id,
            expires_at,
            actions_count: actions.len(),
            actions,
        };

        let mut lock = self.approvals.lock().unwrap();
        // Clean up expired approvals while inserting
        lock.retain(|_, app| !app.used && app.expires_at > now);
        lock.insert(token, approval.clone());

        (approval, view)
    }

    pub fn inspect(&self, token: &str) -> Result<Approval, Rejection> {
        let approvals = self
            .approvals
            .lock()
            .map_err(|_| Rejection::StaleApproval)?;
        let approval = approvals.get(token).ok_or(Rejection::StaleApproval)?;
        if approval.used {
            return Err(Rejection::StaleApproval);
        }
        if now_secs() >= approval.expires_at {
            return Err(Rejection::ExpiredApproval);
        }
        Ok(approval.clone())
    }
    /// Consumes a one-use approval token.
    /// Returns the validated Approval if valid, unconsumed, and not expired.
    /// Immediately marks the token as used, preventing replay attacks.
    pub fn consume_approval(&self, token: &str) -> Result<Approval, Rejection> {
        let mut lock = self.approvals.lock().unwrap();
        let approval = lock.get_mut(token).ok_or(Rejection::StaleApproval)?;

        if approval.used {
            return Err(Rejection::StaleApproval);
        }

        let now = now_secs();
        if now >= approval.expires_at {
            approval.used = true;
            return Err(Rejection::ExpiredApproval);
        }

        // Mark consumed immediately
        approval.used = true;
        Ok(approval.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn one_use_approval_cannot_be_consumed_twice() {
        let mgr = ApprovalManager::new();
        let action = ValidatedAction::Trash {
            source: PathBuf::from("/tmp/test"),
            relative_source: PathBuf::from("test"),
            original_size: 100,
            original_modified: 1000,
        };
        let (app, _) = mgr.create_approval(1, 10, vec![action], 60);

        let consumed = mgr.consume_approval(&app.token).unwrap();
        assert_eq!(consumed.tx_id, 1);

        // Replay attempt fails!
        let second = mgr.consume_approval(&app.token).unwrap_err();
        assert_eq!(second, Rejection::StaleApproval);
    }

    #[test]
    fn expired_approval_is_rejected() {
        let mgr = ApprovalManager::new();
        // Create an approval that expired 10 seconds ago
        let approval = Approval {
            id: "test-id".into(),
            token: "test-token".into(),
            tx_id: 1,
            scope_id: 10,
            created_at: 100,
            expires_at: 150, // in the past
            actions: vec![],
            used: false,
        };
        mgr.approvals
            .lock()
            .unwrap()
            .insert(approval.token.clone(), approval);

        let err = mgr.consume_approval("test-token").unwrap_err();
        assert_eq!(err, Rejection::ExpiredApproval);
    }
}
