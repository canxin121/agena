use super::{
    PermissionDecision, PermissionMode, PermissionScope, PersistedPermissionRule, decide_from_mode,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResolutionSource {
    PersistedRule {
        scope: PermissionScope,
        source: String,
        reason: Option<String>,
        operator: Option<String>,
    },
    StaticPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionResolution {
    pub decision: PermissionDecision,
    pub source: PermissionResolutionSource,
    pub explanation: String,
}

pub fn resolve_permission_with_persisted_rule(
    base: PermissionDecision,
    persisted_rule: Option<&PersistedPermissionRule>,
) -> PermissionResolution {
    if let Some(rule) = persisted_rule {
        let decision = decide_from_mode(rule.mode, persisted_rule_reason(rule, &base));
        return PermissionResolution {
            explanation: persisted_rule_explanation(rule),
            source: PermissionResolutionSource::PersistedRule {
                scope: rule.scope,
                source: rule.source.clone(),
                reason: rule.reason.clone(),
                operator: rule.operator.clone(),
            },
            decision,
        };
    }

    PermissionResolution {
        explanation: static_policy_explanation(&base),
        source: PermissionResolutionSource::StaticPolicy,
        decision: base,
    }
}

fn persisted_rule_reason(rule: &PersistedPermissionRule, base: &PermissionDecision) -> String {
    if let Some(reason) = rule.reason.as_deref()
        && !reason.trim().is_empty()
    {
        return reason.to_string();
    }

    match rule.mode {
        PermissionMode::Allow => "allowed by persisted permission rule".to_string(),
        PermissionMode::Ask => match base {
            PermissionDecision::Ask { reason } | PermissionDecision::Deny { reason } => {
                reason.clone()
            }
            PermissionDecision::Allow => "permission requires confirmation".to_string(),
        },
        PermissionMode::Deny => "permission denied by persisted rule".to_string(),
    }
}

fn persisted_rule_explanation(rule: &PersistedPermissionRule) -> String {
    let subject = match rule.scope {
        PermissionScope::Session => match rule.session_id {
            Some(session_id) => format!("session-scoped rule for session #{session_id}"),
            None => "session-scoped rule".to_string(),
        },
        PermissionScope::Workspace => match rule.workspace_id {
            Some(workspace_id) => format!("workspace-scoped rule for workspace #{workspace_id}"),
            None => "workspace-scoped rule".to_string(),
        },
        PermissionScope::Global => "global rule".to_string(),
    };
    format!("matched {subject} from {}", rule.source)
}

fn static_policy_explanation(base: &PermissionDecision) -> String {
    match base {
        PermissionDecision::Allow => "matched static permission policy".to_string(),
        PermissionDecision::Ask { reason } => reason.clone(),
        PermissionDecision::Deny { reason } => reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_rule_overrides_static_ask() {
        let rule = PersistedPermissionRule {
            action_key: "tool".to_string(),
            mode: PermissionMode::Allow,
            scope: PermissionScope::Workspace,
            session_id: None,
            workspace_id: Some(7),
            source: "permission_reply".to_string(),
            reason: None,
            operator: None,
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        };
        let resolution = resolve_permission_with_persisted_rule(
            PermissionDecision::Ask {
                reason: "needs approval".to_string(),
            },
            Some(&rule),
        );
        assert_eq!(resolution.decision, PermissionDecision::Allow);
        assert!(resolution.explanation.contains("workspace-scoped rule"));
    }

    #[test]
    fn persisted_deny_overrides_static_allow() {
        let rule = PersistedPermissionRule {
            action_key: "tool".to_string(),
            mode: PermissionMode::Deny,
            scope: PermissionScope::Session,
            session_id: Some(42),
            workspace_id: None,
            source: "permission_reply".to_string(),
            reason: Some("blocked by reviewer".to_string()),
            operator: Some("alice".to_string()),
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        };
        let resolution =
            resolve_permission_with_persisted_rule(PermissionDecision::Allow, Some(&rule));
        assert_eq!(
            resolution.decision,
            PermissionDecision::Deny {
                reason: "blocked by reviewer".to_string(),
            }
        );
        assert!(resolution.explanation.contains("session-scoped rule"));
    }

    #[test]
    fn global_rule_explanation_is_rendered() {
        let rule = PersistedPermissionRule {
            action_key: "tool".to_string(),
            mode: PermissionMode::Allow,
            scope: PermissionScope::Global,
            session_id: None,
            workspace_id: None,
            source: "managed".to_string(),
            reason: None,
            operator: None,
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        };
        let resolution = resolve_permission_with_persisted_rule(
            PermissionDecision::Ask {
                reason: "needs approval".to_string(),
            },
            Some(&rule),
        );
        assert_eq!(resolution.decision, PermissionDecision::Allow);
        assert!(resolution.explanation.contains("global rule"));
    }

    #[test]
    fn static_policy_passes_through_without_persisted_rule() {
        let resolution = resolve_permission_with_persisted_rule(
            PermissionDecision::Ask {
                reason: "static ask".to_string(),
            },
            None,
        );
        assert_eq!(
            resolution.decision,
            PermissionDecision::Ask {
                reason: "static ask".to_string(),
            }
        );
        assert_eq!(resolution.source, PermissionResolutionSource::StaticPolicy);
    }
}
