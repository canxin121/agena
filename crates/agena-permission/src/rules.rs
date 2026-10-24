//! Persisted-rule synthesis. A rule snapshot is a per-session, versioned view
//! of the stored permission rules; `apply_rules` composes it with the static
//! policy decision while preserving the static-policy authority invariants:
//! a static `Allow` is never downgraded by a remembered `ask`/`auto`, and a
//! static `Deny` is never escalated by a remembered allow.

use agena_domain::{
    DecisionTraceStep, PermissionDecision, PermissionMode, PermissionResolution,
    PermissionResolutionSource, PermissionScope, PolicySourceKind, decide_from_mode,
};

/// Host-agnostic view of one stored permission rule.
#[derive(Debug, Clone, PartialEq, Eq)]
/// A permission rule entry.
pub struct RuleEntry {
    pub id: Option<i64>,
    pub revision_ms: Option<i64>,
    pub scope: PermissionScope,
    pub source: String,
    pub reason: Option<String>,
    pub operator: Option<String>,
    pub mode: PermissionMode,
}

/// Compose the static policy decision with a session's rule snapshot.
pub fn apply_rules(base: &PermissionDecision, rules: &[RuleEntry]) -> PermissionResolution {
    if rules.is_empty() {
        let explanation = static_policy_explanation(base);
        return PermissionResolution {
            explanation: explanation.clone(),
            source: PermissionResolutionSource::StaticPolicy,
            decision: base.clone(),
            trace: vec![DecisionTraceStep {
                source_kind: PolicySourceKind::StaticPolicy,
                summary: explanation,
                source: Some("static_policy".to_string()),
                scope: None,
                operator: None,
            }],
        };
    }

    let static_allows = matches!(base, PermissionDecision::Allow);
    let static_denies = matches!(base, PermissionDecision::Deny { .. });
    let mut decision = base.clone();
    let mut trace = Vec::with_capacity(rules.len());
    let mut applied_rules = Vec::with_capacity(rules.len());
    for rule in rules {
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
            explanation: static_policy_explanation(base),
            source: PermissionResolutionSource::StaticPolicy,
            decision,
            trace,
        };
    };
    let explanation = merged_persisted_rule_explanation(applied_rules.as_slice());
    PermissionResolution {
        explanation,
        source: PermissionResolutionSource::PersistedRule {
            rule_id: effective_rule.id,
            revision_ms: effective_rule.revision_ms,
            scope: effective_rule.scope,
            source: effective_rule.source.clone(),
            reason: effective_rule.reason.clone(),
            operator: effective_rule.operator.clone(),
        },
        decision,
        trace,
    }
}

fn static_policy_explanation(decision: &PermissionDecision) -> String {
    match decision {
        PermissionDecision::Allow => "allowed by static policy".to_string(),
        PermissionDecision::Auto { reason } => reason.clone(),
        PermissionDecision::Ask { reason } => reason.clone(),
        PermissionDecision::Deny { reason } => reason.clone(),
    }
}

fn persisted_rule_reason(rule: &RuleEntry, base: &PermissionDecision) -> String {
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

fn persisted_rule_explanation(rule: &RuleEntry) -> String {
    let action = match rule.mode {
        PermissionMode::Allow => "allowed",
        PermissionMode::Auto => "eligible for automatic approval",
        PermissionMode::Ask => "requires confirmation",
        PermissionMode::Deny => "denied",
    };
    match rule.scope {
        PermissionScope::Session => format!("session rule {action} this action"),
        PermissionScope::Workspace => format!("workspace rule {action} this action"),
        PermissionScope::Global => format!("global rule {action} this action"),
    }
}

fn merged_persisted_rule_explanation(rules: &[&RuleEntry]) -> String {
    let mut summary = String::new();
    for (index, rule) in rules.iter().enumerate() {
        if index > 0 {
            summary.push_str(", ");
        }
        summary.push_str(&persisted_rule_explanation(rule));
    }
    summary
}
