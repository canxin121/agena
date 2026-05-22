use super::{
    DecisionTraceStep, PermissionDecision, PermissionMode, PermissionRiskLevel, PermissionScope,
    PersistedPermissionRule, PolicySourceKind, decide_from_mode,
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
    pub risk: PermissionRiskLevel,
    pub trace: Vec<DecisionTraceStep>,
}

pub fn resolve_permission_with_persisted_rule(
    base: PermissionDecision,
    persisted_rule: Option<&PersistedPermissionRule>,
) -> PermissionResolution {
    if let Some(rule) = persisted_rule {
        let decision = decide_from_mode(rule.mode, persisted_rule_reason(rule, &base));
        let risk = risk_for_decision(&decision);
        let explanation = persisted_rule_explanation(rule);
        return PermissionResolution {
            explanation: explanation.clone(),
            source: PermissionResolutionSource::PersistedRule {
                scope: rule.scope,
                source: rule.source.clone(),
                reason: rule.reason.clone(),
                operator: rule.operator.clone(),
            },
            decision,
            risk,
            trace: vec![DecisionTraceStep {
                source_kind: PolicySourceKind::PersistedRule,
                summary: explanation,
                source: Some(rule.source.clone()),
                scope: Some(rule.scope),
                operator: rule.operator.clone(),
            }],
        };
    }

    let explanation = static_policy_explanation(&base);
    PermissionResolution {
        explanation: explanation.clone(),
        source: PermissionResolutionSource::StaticPolicy,
        risk: risk_for_decision(&base),
        decision: base,
        trace: vec![DecisionTraceStep {
            source_kind: PolicySourceKind::StaticPolicy,
            summary: explanation,
            source: Some("static_policy".to_string()),
            scope: None,
            operator: None,
        }],
    }
}

fn risk_for_decision(decision: &PermissionDecision) -> PermissionRiskLevel {
    match decision {
        PermissionDecision::Allow => PermissionRiskLevel::Low,
        PermissionDecision::Ask { .. } => PermissionRiskLevel::Medium,
        PermissionDecision::Deny { .. } => PermissionRiskLevel::High,
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
