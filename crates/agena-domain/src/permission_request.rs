use serde::{Deserialize, Serialize};

use crate::PermissionScope;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionAction {
    Tool {
        tool_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        qualifier: Option<String>,
    },
    PathAccess {
        access_kind: String,
        workspace_root: String,
        target_path: String,
    },
    NetworkAccess {
        target: String,
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRiskLevel {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySourceKind {
    StaticPolicy,
    PersistedRule,
    PluginAdvice,
    ManagedPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionTraceStep {
    pub source_kind: PolicySourceKind,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PermissionScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DecisionTrace {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<DecisionTraceStep>,
}

#[cfg(test)]
mod tests {
    use super::{PermissionAction, PermissionRiskLevel};

    #[test]
    fn permission_request_values_have_stable_wire_shapes() {
        assert_eq!(PermissionRiskLevel::default(), PermissionRiskLevel::Medium);
        assert_eq!(
            serde_json::to_string(&PermissionAction::NetworkAccess {
                target: "https://example.com".to_owned(),
                host: "example.com".to_owned(),
                port: None,
            })
            .unwrap(),
            "{\"kind\":\"network_access\",\"target\":\"https://example.com\",\"host\":\"example.com\"}"
        );
    }
}
