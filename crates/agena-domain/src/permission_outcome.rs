use serde::{Deserialize, Serialize};

use crate::{DecisionTraceStep, PermissionAction, PermissionRiskLevel, PermissionScope};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAuthorityKind {
    StaticPolicy,
    PersistedRule,
    PluginPolicy,
    AutoApprovalModel,
}

/// A normal non-execution outcome produced when an effective permission rule
/// explicitly denies one or more actions required by a tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyDeniedResult {
    pub action: PermissionAction,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_actions: Vec<PermissionAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied_actions: Vec<PermissionAction>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PermissionScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    pub authority: PermissionAuthorityKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_revision_ms: Option<i64>,
    #[serde(default)]
    pub risk: PermissionRiskLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<DecisionTraceStep>,
}

/// A normal non-execution outcome produced when a user declines a concrete
/// interactive permission request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserDeclinedResult {
    pub request_id: String,
    pub action: PermissionAction,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_actions: Vec<PermissionAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Scope of the deny rule created by a deny-always reply. `None` means the
    /// user declined only this request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persisted_scope: Option<PermissionScope>,
}
