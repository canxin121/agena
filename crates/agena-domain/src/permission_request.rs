use serde::{Deserialize, Serialize};

use crate::PermissionScope;

/// Structured decision input for the permission pipeline. Unlike
/// [`PermissionAction`] (the wire/UI contract), an `ActionSpec` carries the
/// tool's full [`ToolPermissionContract`] so decisions never depend on
/// tool-name allowlists and never depend on tool tags: tags are metadata for
/// discovery/UI, while the contract is the authority-bearing declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionSpec {
    Tool {
        tool_name: String,
        /// The tool's full permission contract: `read_only`, `shell`,
        /// `interactive`, `task` flags plus declared path/network specs.
        #[serde(default)]
        contract: ToolPermissionContract,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    Path {
        access: String,
        workspace_root: String,
        target: String,
    },
    Network {
        target: String,
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
}

impl ActionSpec {
    pub fn from_action(action: &PermissionAction) -> Self {
        match action {
            PermissionAction::Tool {
                tool_name,
                qualifier,
            } => Self::Tool {
                tool_name: tool_name.clone(),
                contract: ToolPermissionContract::default(),
                command: qualifier.clone(),
            },
            PermissionAction::PathAccess {
                access_kind,
                workspace_root,
                target_path,
            } => Self::Path {
                access: access_kind.clone(),
                workspace_root: workspace_root.clone(),
                target: target_path.clone(),
            },
            PermissionAction::NetworkAccess { target, host, port } => Self::Network {
                target: target.clone(),
                host: host.clone(),
                port: *port,
            },
        }
    }
}

/// The tool's full permission contract: authority-bearing declarations of
/// what the tool can do. The permission engine consumes this instead of tool
/// tags: a tag is metadata (discovery, search, UI), while a contract flag or
/// declared path/network spec is a declaration the permission engine may act
/// on. `read_only` is only trusted when the contract contains no shell,
/// interactive, network, or write access, so the automatic fast path never
/// auto-approves arbitrary execution or remote effects.
pub use crate::tool_permission_contract::ToolPermissionContract;

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
    use super::PermissionAction;

    #[test]
    fn permission_request_values_have_stable_wire_shapes() {
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
