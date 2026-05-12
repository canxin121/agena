use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use thiserror::Error;
use uuid::Uuid;

use super::request::{
    PendingPermission, PermissionAction, PermissionReply, PermissionReplyKind, PermissionRequest,
};
use super::store::{PermissionRuleStore, PermissionStoreError};
use super::{PermissionDecision, PermissionMode};
use crate::plugin::{
    PermissionAdvice as PluginPermissionAdvice,
    PermissionAskInput as PluginPermissionAskInput,
    PermissionAskOutcome as PluginPermissionAskOutcome,
    PermissionDecision as PluginPermissionDecision, PluginHost,
};

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
        self.decide_or_request_with_plugins(session_id, action, base, None)
    }

    /// Same as [`decide_or_request`] but consults the plugin host's
    /// `permission.ask` hook before falling back to the default flow. A
    /// plugin returning `Decide(Allow|Deny)` short-circuits the runtime; a
    /// plugin returning `Defer` lets the next plugin or the default base
    /// decision win.
    #[tracing::instrument(skip_all, fields(session_id))]
    pub fn decide_or_request_with_plugins(
        &mut self,
        session_id: Option<i64>,
        action: PermissionAction,
        base: PermissionDecision,
        plugins: Option<&Arc<PluginHost>>,
    ) -> Result<PermissionRuntimeDecision, PermissionRuntimeError> {
        if let Some(saved) = self.store.lookup(&action)? {
            return Ok(PermissionRuntimeDecision::Immediate(saved.into()));
        }

        // Plugin override (best-effort: errors are logged and ignored).
        if let Some(host) = plugins
            && !host.is_empty()
        {
            let default_decision = match &base {
                PermissionDecision::Allow => PluginPermissionDecision::Allow,
                PermissionDecision::Deny { .. } => PluginPermissionDecision::Deny,
                PermissionDecision::Ask { .. } => PluginPermissionDecision::Prompt,
            };
            let req = PluginPermissionAskInput {
                session_id: session_id.unwrap_or(-1),
                action: format!("{:?}", action),
                subject: permission_subject(&action),
                default_decision,
            };
            match host.dispatch_permission_ask_blocking(req) {
                Ok(Some(PluginPermissionAskOutcome::Decision {
                    decision: PluginPermissionDecision::Allow,
                    ..
                })) => {
                    return Ok(PermissionRuntimeDecision::Immediate(
                        PermissionDecision::Allow,
                    ));
                }
                Ok(Some(PluginPermissionAskOutcome::Decision {
                    plugin_id,
                    decision: PluginPermissionDecision::Deny,
                    ..
                })) => {
                    return Ok(PermissionRuntimeDecision::Immediate(
                        PermissionDecision::Deny {
                            reason: format!("denied by plugin {plugin_id}"),
                        },
                    ));
                }
                Ok(Some(PluginPermissionAskOutcome::Decision {
                    decision: PluginPermissionDecision::Prompt,
                    ..
                }))
                | Ok(None) => {}
                Ok(Some(PluginPermissionAskOutcome::Advice { advice, .. })) => {
                    return Ok(apply_plugin_advice(session_id, action, base, advice));
                }
                Err(err) => {
                    tracing::warn!(
                        target: "agena_plugin_host::permission",
                        "permission plugin failed: {err}"
                    );
                }
            }
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
                    explanation: "matched static permission policy".to_string(),
                    source: Some("static_policy".to_string()),
                    scope: None,
                    operator: None,
                    risk: super::PermissionRiskLevel::Medium,
                    trace: vec![super::DecisionTraceStep {
                        source_kind: super::PolicySourceKind::StaticPolicy,
                        summary: "matched static permission policy".to_string(),
                        source: Some("static_policy".to_string()),
                        scope: None,
                        operator: None,
                    }],
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

fn apply_plugin_advice<S>(
    session_id: Option<i64>,
    action: PermissionAction,
    base: PermissionDecision,
    advice: PluginPermissionAdvice,
) -> PermissionRuntimeDecision {
    let explanation = if advice.reason.trim().is_empty() {
        "permission advised by plugin".to_string()
    } else {
        advice.reason.clone()
    };
    let trace_step = super::DecisionTraceStep {
        source_kind: super::PolicySourceKind::PluginAdvice,
        summary: explanation.clone(),
        source: Some("plugin_permission_advice".to_string()),
        scope: None,
        operator: None,
    };
    match advice.decision {
        PluginPermissionDecision::Allow => PermissionRuntimeDecision::Immediate(PermissionDecision::Allow),
        PluginPermissionDecision::Deny => PermissionRuntimeDecision::Immediate(PermissionDecision::Deny {
            reason: if advice.reason.trim().is_empty() {
                "denied by plugin advice".to_string()
            } else {
                advice.reason
            },
        }),
        PluginPermissionDecision::Prompt => PermissionRuntimeDecision::Pending(PendingPermission {
            request: PermissionRequest {
                request_id: Uuid::new_v4().to_string(),
                session_id,
                action,
                reason: match base {
                    PermissionDecision::Ask { reason } => reason,
                    PermissionDecision::Deny { reason } => reason,
                    PermissionDecision::Allow => "permission requires confirmation".to_string(),
                },
                explanation,
                source: Some("plugin_permission_advice".to_string()),
                scope: None,
                operator: None,
                risk: match advice.risk {
                    crate::plugin::sdk::PermissionRiskLevel::Low => super::PermissionRiskLevel::Low,
                    crate::plugin::sdk::PermissionRiskLevel::Medium => super::PermissionRiskLevel::Medium,
                    crate::plugin::sdk::PermissionRiskLevel::High => super::PermissionRiskLevel::High,
                    crate::plugin::sdk::PermissionRiskLevel::Critical => super::PermissionRiskLevel::Critical,
                },
                trace: vec![trace_step],
                created_at: Utc::now(),
            },
        }),
    }
}

fn permission_subject(action: &PermissionAction) -> serde_json::Value {
    match action {
        PermissionAction::BuiltinTool { tool_name, .. } => {
            serde_json::json!({
                "kind": "tool",
                "tool_name": tool_name,
            })
        }
        PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => serde_json::json!({
            "kind": "path_access",
            "access_kind": access_kind,
            "workspace_root": workspace_root,
            "target_path": target_path,
        }),
        PermissionAction::NetworkAccess { target, host, port } => serde_json::json!({
            "kind": "network_access",
            "target": target,
            "host": host,
            "port": port,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use super::*;
    use crate::permission::store::{PermissionRuleStore, PermissionStoreError};

    #[derive(Default)]
    struct TestPermissionStore {
        inner: Arc<RwLock<Vec<(PermissionAction, PermissionMode)>>>,
    }

    impl PermissionRuleStore for TestPermissionStore {
        fn lookup(
            &self,
            action: &PermissionAction,
        ) -> Result<Option<PermissionMode>, PermissionStoreError> {
            let guard = self
                .inner
                .read()
                .map_err(|_| PermissionStoreError::LockPoisoned)?;
            Ok(guard
                .iter()
                .rev()
                .find_map(|(a, m)| (a == action).then_some(*m)))
        }

        fn save(
            &self,
            action: PermissionAction,
            mode: PermissionMode,
        ) -> Result<(), PermissionStoreError> {
            self.inner
                .write()
                .map_err(|_| PermissionStoreError::LockPoisoned)?
                .push((action, mode));
            Ok(())
        }
    }

    #[test]
    fn permission_subject_includes_tool_context() {
        let subject = super::permission_subject(&PermissionAction::BuiltinTool {
            tool_name: "bash".to_string(),
            qualifier: None,
        });
        assert_eq!(subject["kind"], "tool");
        assert_eq!(subject["tool_name"], "bash");
    }

    #[test]
    fn permission_subject_includes_path_context() {
        let subject = super::permission_subject(&PermissionAction::PathAccess {
            access_kind: "write".to_string(),
            workspace_root: "/workspace".to_string(),
            target_path: "/workspace/file.txt".to_string(),
        });
        assert_eq!(subject["kind"], "path_access");
        assert_eq!(subject["access_kind"], "write");
        assert_eq!(subject["workspace_root"], "/workspace");
        assert_eq!(subject["target_path"], "/workspace/file.txt");
    }

    #[test]
    fn ask_decision_creates_pending_request_and_allow_always_persists() {
        let mut runtime = PermissionRuntime::new(TestPermissionStore::default());
        let action = PermissionAction::BuiltinTool {
            tool_name: "bash".to_string(),
            qualifier: None,
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
