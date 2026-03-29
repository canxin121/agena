use std::collections::HashMap;

use chrono::Utc;
use thiserror::Error;
use uuid::Uuid;

use super::request::{
    PendingPermission, PermissionAction, PermissionReply, PermissionReplyKind, PermissionRequest,
};
use super::store::{PermissionRuleStore, PermissionStoreError};
use super::{PermissionDecision, PermissionMode};

#[derive(Debug, Error)]
pub enum PermissionRuntimeError {
    #[error("unknown permission request: {0}")]
    UnknownRequest(String),
    #[error("permission rule store failure: {0}")]
    Store(#[from] PermissionStoreError),
}

#[derive(Debug)]
pub enum PermissionRuntimeDecision {
    Immediate(PermissionDecision),
    Pending(PendingPermission),
}

#[derive(Debug)]
pub struct PermissionRuntime<S>
where
    S: PermissionRuleStore,
{
    store: S,
    pending: HashMap<String, PermissionRequest>,
}

impl<S> PermissionRuntime<S>
where
    S: PermissionRuleStore,
{
    pub fn new(store: S) -> Self {
        Self {
            store,
            pending: HashMap::new(),
        }
    }

    pub fn decide_or_request(
        &mut self,
        session_id: Option<i64>,
        action: PermissionAction,
        base: PermissionDecision,
    ) -> Result<PermissionRuntimeDecision, PermissionRuntimeError> {
        if let Some(saved) = self.store.lookup(&action)? {
            return Ok(PermissionRuntimeDecision::Immediate(saved.into()));
        }

        match base {
            PermissionDecision::Allow | PermissionDecision::Deny { .. } => {
                Ok(PermissionRuntimeDecision::Immediate(base))
            }
            PermissionDecision::Ask { reason } => {
                let request = PermissionRequest {
                    request_id: Uuid::new_v4().to_string(),
                    session_id,
                    action,
                    reason,
                    created_at: Utc::now(),
                };
                self.pending
                    .insert(request.request_id.clone(), request.clone());
                Ok(PermissionRuntimeDecision::Pending(PendingPermission {
                    request,
                }))
            }
        }
    }

    pub fn resolve_reply(
        &mut self,
        reply: PermissionReply,
    ) -> Result<PermissionDecision, PermissionRuntimeError> {
        let request = self
            .pending
            .remove(reply.request_id.as_str())
            .ok_or_else(|| PermissionRuntimeError::UnknownRequest(reply.request_id.clone()))?;

        let (decision, persist_mode) = match reply.kind {
            PermissionReplyKind::AllowOnce => (PermissionDecision::Allow, None),
            PermissionReplyKind::AllowAlways => {
                (PermissionDecision::Allow, Some(PermissionMode::Allow))
            }
            PermissionReplyKind::DenyOnce => {
                let reason = reply
                    .reason
                    .unwrap_or_else(|| "denied by operator".to_string());
                (PermissionDecision::Deny { reason }, None)
            }
            PermissionReplyKind::DenyAlways => {
                let reason = reply
                    .reason
                    .unwrap_or_else(|| "denied by operator".to_string());
                (
                    PermissionDecision::Deny {
                        reason: reason.clone(),
                    },
                    Some(PermissionMode::Deny),
                )
            }
        };

        if let Some(mode) = persist_mode {
            self.store.save(request.action, mode)?;
        }

        Ok(decision)
    }
}

impl From<PermissionMode> for PermissionDecision {
    fn from(value: PermissionMode) -> Self {
        match value {
            PermissionMode::Allow => Self::Allow,
            PermissionMode::Ask => Self::Ask {
                reason: "permission requires confirmation".to_string(),
            },
            PermissionMode::Deny => Self::Deny {
                reason: "permission denied by persisted rule".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::InMemoryPermissionRuleStore;

    #[test]
    fn ask_decision_creates_pending_request_and_allow_always_persists() {
        let mut runtime = PermissionRuntime::new(InMemoryPermissionRuleStore::new());
        let action = PermissionAction::BuiltinTool {
            tool_name: "bash".to_string(),
        };

        let pending = runtime
            .decide_or_request(
                Some(42),
                action.clone(),
                PermissionDecision::Ask {
                    reason: "need approval".to_string(),
                },
            )
            .expect("decision should succeed");

        let PermissionRuntimeDecision::Pending(pending) = pending else {
            panic!("expected pending request")
        };

        let decision = runtime
            .resolve_reply(PermissionReply {
                request_id: pending.request.request_id,
                kind: PermissionReplyKind::AllowAlways,
                reason: None,
                scope: None,
            })
            .expect("reply should succeed");
        assert_eq!(decision, PermissionDecision::Allow);

        let immediate = runtime
            .decide_or_request(
                Some(42),
                action,
                PermissionDecision::Ask {
                    reason: "need approval".to_string(),
                },
            )
            .expect("decision should succeed");
        let PermissionRuntimeDecision::Immediate(decision) = immediate else {
            panic!("expected immediate decision")
        };
        assert_eq!(decision, PermissionDecision::Allow);
    }
}
