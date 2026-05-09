use serde::{Deserialize, Serialize};

use crate::manifest::PathKind;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Prompt,
}

/// One filesystem path that a tool intends to read or write. Returned by
/// [`crate::Plugin::permission_paths`] for paths that cannot be expressed as
/// declarative `InputPathSpec` JSONPath rules (e.g. paths derived from a
/// patch body or shell command parsing).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathRequest {
    pub path: String,
    pub kind: PathKind,
}

impl PathRequest {
    pub fn read(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: PathKind::Read,
        }
    }

    pub fn write(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: PathKind::Write,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionAskInput {
    pub session_id: i64,
    pub action: String,
    #[serde(default)]
    pub subject: serde_json::Value,
    pub default_decision: PermissionDecision,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRiskLevel {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionAdvice {
    pub decision: PermissionDecision,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default)]
    pub risk: PermissionRiskLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum PermissionAskDecision {
    Decide(PermissionDecision),
    Advise(PermissionAdvice),
    Defer,
}
