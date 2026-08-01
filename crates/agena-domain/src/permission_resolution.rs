use crate::{DecisionTraceStep, PermissionMode, PermissionRiskLevel, PermissionScope};

/// The outcome of evaluating a permission policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Auto { reason: String },
    Ask { reason: String },
    Deny { reason: String },
}

/// Materialize a policy mode as an actionable permission decision while
/// preserving the caller's explanation for interactive/denied cases.
pub fn decide_from_mode(mode: PermissionMode, reason: impl Into<String>) -> PermissionDecision {
    match mode {
        PermissionMode::Allow => PermissionDecision::Allow,
        PermissionMode::Auto => PermissionDecision::Auto {
            reason: reason.into(),
        },
        PermissionMode::Ask => PermissionDecision::Ask {
            reason: reason.into(),
        },
        PermissionMode::Deny => PermissionDecision::Deny {
            reason: reason.into(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResolutionSource {
    PersistedRule {
        rule_id: Option<i64>,
        revision_ms: Option<i64>,
        scope: PermissionScope,
        source: String,
        reason: Option<String>,
        operator: Option<String>,
    },
    StaticPolicy,
}

/// The explainable outcome of composing static and persisted permission policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionResolution {
    pub decision: PermissionDecision,
    pub source: PermissionResolutionSource,
    pub explanation: String,
    pub risk: PermissionRiskLevel,
    pub trace: Vec<DecisionTraceStep>,
}

#[cfg(test)]
mod tests {
    use crate::PermissionMode;

    use super::{PermissionDecision, PermissionResolutionSource, decide_from_mode};

    #[test]
    fn decision_and_resolution_source_preserve_reasons() {
        assert_eq!(
            PermissionDecision::Deny {
                reason: "policy".into(),
            },
            PermissionDecision::Deny {
                reason: "policy".into(),
            }
        );
        assert_eq!(
            PermissionResolutionSource::StaticPolicy,
            PermissionResolutionSource::StaticPolicy
        );
    }

    #[test]
    fn policy_modes_materialize_to_the_expected_decision() {
        assert_eq!(
            decide_from_mode(PermissionMode::Allow, "ignored"),
            PermissionDecision::Allow
        );
        assert_eq!(
            decide_from_mode(PermissionMode::Ask, "confirm"),
            PermissionDecision::Ask {
                reason: "confirm".to_owned()
            }
        );
        assert_eq!(
            decide_from_mode(PermissionMode::Auto, "automatic"),
            PermissionDecision::Auto {
                reason: "automatic".to_owned()
            }
        );
        assert_eq!(
            decide_from_mode(PermissionMode::Deny, "blocked"),
            PermissionDecision::Deny {
                reason: "blocked".to_owned()
            }
        );
    }
}
