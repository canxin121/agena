use agena_domain::{
    DecisionTraceStep, PermissionDecision, PermissionResolution, PermissionResolutionSource,
    PermissionRiskLevel, PermissionScope, PolicySourceKind,
};

use agena_domain::{PermissionMode, decide_from_mode};
use agena_storage::PersistedPermissionRule;

pub fn resolve_permission_with_persisted_rules(
    base: PermissionDecision,
    persisted_rules: &[PersistedPermissionRule],
) -> PermissionResolution {
    if persisted_rules.is_empty() {
        let explanation = static_policy_explanation(&base);
        return PermissionResolution {
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
        };
    }

    let static_allows = matches!(&base, PermissionDecision::Allow);
    let static_denies = matches!(&base, PermissionDecision::Deny { .. });
    let mut decision = base.clone();
    let mut trace = Vec::with_capacity(persisted_rules.len());
    let mut applied_rules = Vec::with_capacity(persisted_rules.len());
    for rule in persisted_rules {
        // A static explicit allow is already an authoritative grant. Older
        // databases can contain fallback `ask` rules created before the
        // built-in workspace-read allow was introduced; applying those rules
        // here would make the UI say `allow` while execution still prompts.
        // Likewise, a static deny must never be escalated by a remembered
        // allow. Persisted deny remains restrictive in both cases.
        let ignored_as_non_authoritative = (static_allows
            && matches!(rule.mode, PermissionMode::Auto | PermissionMode::Ask))
            || (static_denies
                && matches!(
                    rule.mode,
                    PermissionMode::Allow | PermissionMode::Auto | PermissionMode::Ask
                ));
        let summary = if ignored_as_non_authoritative {
            format!(
                "{}; static policy remains authoritative",
                persisted_rule_explanation(rule)
            )
        } else {
            let reason = persisted_rule_reason(rule, &decision);
            decision = decide_from_mode(rule.mode, reason);
            applied_rules.push(rule);
            persisted_rule_explanation(rule)
        };
        trace.push(DecisionTraceStep {
            source_kind: PolicySourceKind::PersistedRule,
            summary,
            source: Some(rule.source.clone()),
            scope: Some(rule.scope),
            operator: rule.operator.clone(),
        });
    }

    let Some(effective_rule) = applied_rules.last().copied() else {
        return PermissionResolution {
            explanation: static_policy_explanation(&base),
            source: PermissionResolutionSource::StaticPolicy,
            risk: risk_for_decision(&decision),
            decision,
            trace,
        };
    };
    let explanation = merged_persisted_rule_explanation(applied_rules.as_slice());
    PermissionResolution {
        explanation,
        source: PermissionResolutionSource::PersistedRule {
            rule_id: effective_rule.id,
            revision_ms: effective_rule.updated_at_ms,
            scope: effective_rule.scope,
            source: effective_rule.source.clone(),
            reason: effective_rule.reason.clone(),
            operator: effective_rule.operator.clone(),
        },
        risk: risk_for_decision(&decision),
        decision,
        trace,
    }
}

fn risk_for_decision(decision: &PermissionDecision) -> PermissionRiskLevel {
    match decision {
        PermissionDecision::Allow => PermissionRiskLevel::Low,
        PermissionDecision::Auto { .. } => PermissionRiskLevel::Medium,
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
        PermissionMode::Auto => "permission is eligible for automatic approval".to_string(),
        PermissionMode::Ask => match base {
            PermissionDecision::Auto { reason }
            | PermissionDecision::Ask { reason }
            | PermissionDecision::Deny { reason } => reason.clone(),
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

fn merged_persisted_rule_explanation(persisted_rules: &[&PersistedPermissionRule]) -> String {
    let effective_rule = *persisted_rules
        .last()
        .expect("persisted_rules should be non-empty");
    if persisted_rules.len() == 1 {
        return persisted_rule_explanation(effective_rule);
    }

    format!(
        "merged {} persisted permission rules; effective {}",
        persisted_rules.len(),
        persisted_rule_explanation(effective_rule)
    )
}

fn static_policy_explanation(base: &PermissionDecision) -> String {
    match base {
        PermissionDecision::Allow => "matched static permission policy".to_string(),
        PermissionDecision::Auto { reason } => reason.clone(),
        PermissionDecision::Ask { reason } => reason.clone(),
        PermissionDecision::Deny { reason } => reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use agena_domain::{PermissionDecision, PermissionMode, PermissionScope};
    use agena_storage::PersistedPermissionRule;

    use super::resolve_permission_with_persisted_rules;

    fn rule(mode: PermissionMode, scope: PermissionScope) -> PersistedPermissionRule {
        PersistedPermissionRule {
            id: Some(7),
            created_at_ms: Some(10),
            updated_at_ms: Some(11),
            action_key: "path.read".to_owned(),
            mode,
            scope,
            session_id: (scope == PermissionScope::Session).then_some(42),
            workspace_id: (scope == PermissionScope::Workspace).then_some(3),
            source: "permission_reply".to_owned(),
            reason: None,
            operator: None,
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        }
    }

    #[test]
    fn workspace_read_allow_is_actionable_without_persisted_rules() {
        let resolution = resolve_permission_with_persisted_rules(PermissionDecision::Allow, &[]);

        assert_eq!(resolution.decision, PermissionDecision::Allow);
        assert!(matches!(
            resolution.source,
            agena_domain::PermissionResolutionSource::StaticPolicy
        ));
        assert_eq!(resolution.risk, agena_domain::PermissionRiskLevel::Low);
    }

    #[test]
    fn persisted_rules_keep_scope_precedence_and_auto_remains_auto() {
        let resolution = resolve_permission_with_persisted_rules(
            PermissionDecision::Ask {
                reason: "requires confirmation".to_owned(),
            },
            &[
                rule(PermissionMode::Auto, PermissionScope::Workspace),
                rule(PermissionMode::Allow, PermissionScope::Session),
            ],
        );

        assert_eq!(resolution.decision, PermissionDecision::Allow);
        assert_eq!(resolution.trace.len(), 2);
        assert!(matches!(
            resolution.source,
            agena_domain::PermissionResolutionSource::PersistedRule {
                scope: PermissionScope::Session,
                ..
            }
        ));
    }

    #[test]
    fn legacy_ask_cannot_downgrade_an_explicit_static_allow() {
        let resolution = resolve_permission_with_persisted_rules(
            PermissionDecision::Allow,
            &[rule(PermissionMode::Ask, PermissionScope::Workspace)],
        );

        assert_eq!(resolution.decision, PermissionDecision::Allow);
        assert!(matches!(
            resolution.source,
            agena_domain::PermissionResolutionSource::StaticPolicy
        ));
        assert_eq!(resolution.risk, agena_domain::PermissionRiskLevel::Low);
    }

    #[test]
    fn persisted_deny_still_restricts_an_explicit_static_allow() {
        let resolution = resolve_permission_with_persisted_rules(
            PermissionDecision::Allow,
            &[rule(PermissionMode::Deny, PermissionScope::Workspace)],
        );

        assert!(matches!(
            resolution.decision,
            PermissionDecision::Deny { .. }
        ));
        assert_eq!(resolution.risk, agena_domain::PermissionRiskLevel::High);
    }
}
