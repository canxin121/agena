use crate::{DecisionTraceStep, PermissionMode, PermissionScope};

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
/// How a permission request was resolved.
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
    pub trace: Vec<DecisionTraceStep>,
}
