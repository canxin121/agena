//! Declarative aggregate permission configuration.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    ApprovalModelSelection, NetworkPermissionConfig, PathAccessModes, PathPermissionConfig,
    PathPermissionMode, PermissionMode, SHELL_COMMAND_RULE_GROUP, ToolPermissionConfig,
    ToolPermissionRules,
};

/// Stable, serializable permission configuration independent of policy
/// compilation and host-specific tag interpretation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathPermissionConfig>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::network_permission::deserialize_network"
    )]
    pub network: Option<NetworkPermissionConfig>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_tools"
    )]
    pub tools: Option<ToolPermissionConfig>,
    /// Exact plugin IDs. Omitted entries follow the enclosing permission policy.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, bool>,
    /// Model used to make an automatic permission decision. The runtime must
    /// fail closed to an interactive `ask` when this reference is absent or
    /// cannot be resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_model: Option<ApprovalModelSelection>,
}

fn deserialize_tools<'de, D>(deserializer: D) -> Result<Option<ToolPermissionConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tools = Option::<ToolPermissionConfig>::deserialize(deserializer)?;
    if let Some(tools) = &tools {
        tools.validate_aliases().map_err(serde::de::Error::custom)?;
    }
    Ok(tools.map(ToolPermissionConfig::normalized))
}

/// The selected instance's managed directory for the current workspace.
/// The permission matcher resolves this alias per request; the instance's
/// credentials and other workspaces are outside this grant.
pub const RUNTIME_STATE_PATH: &str = "<state>";
/// Legacy subtree spelling retained for migration and internal callers.
/// Public concrete directory rules already include all descendants.
pub const RUNTIME_STATE_PATH_GLOB: &str = "<state>/**";

/// Class keywords in the shared shell rule group.
pub const COMMAND_CLASS_NO_OP: &str = "no-op";
pub const COMMAND_CLASS_ROUTINE: &str = "routine";
pub const COMMAND_CLASS_DANGEROUS: &str = "dangerous";
/// The keyword for the read-only tool class. It is written under the wildcard
/// tool name, `tools.rules."*"`, because the class is derived from a tool's
/// declared read_only tag rather than from its name.
pub const TOOL_CLASS_READ_ONLY: &str = "read-only";

fn allow_rw() -> PathAccessModes {
    PathAccessModes::from_mode(PathPermissionMode::Readwrite)
}

/// Which of the approval prompt's conditional path promises are in effect.
///
/// Each flag is true when the sandbox's compiled path policy approves that
/// class of action outright, i.e. the approval model is free to stop worrying
/// about it. The prompt is assembled from these because telling the model that
/// temp-directory writes need no thought would silently defeat a user who
/// narrowed the rule covering them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathClassPromptFlags {
    /// Paths inside the runtime's own per-workspace state directory.
    pub internal_paths_allowed: bool,
    /// Paths inside the system temporary directory.
    pub temp_paths_allowed: bool,
}

impl PathClassPromptFlags {
    /// The flags for a sandbox whose path policy approves both classes
    /// outright.
    pub fn all_allowed() -> Self {
        Self {
            internal_paths_allowed: true,
            temp_paths_allowed: true,
        }
    }

    /// Whether every conditional promise is in effect, i.e. the prompt renders
    /// its full class-default bullet.
    pub fn every_class_allowed(&self) -> bool {
        self.internal_paths_allowed && self.temp_paths_allowed
    }
}

impl PermissionConfig {
    /// A complete named policy owns its defaults and all of its rule tables.
    pub fn standalone(&self) -> Self {
        let mut result = self.clone();
        let paths = result
            .path
            .get_or_insert_with(PathPermissionConfig::default);
        let deny = PathAccessModes::from_mode(PathPermissionMode::Deny);
        paths.workspace.get_or_insert_with(|| deny.clone());
        paths.external.get_or_insert(deny);
        result.network = Some(
            result
                .network
                .as_ref()
                .cloned()
                .unwrap_or_default()
                .resolved(),
        );
        result
            .tools
            .get_or_insert_with(ToolPermissionConfig::default)
            .default
            .get_or_insert(PermissionMode::Deny);
        result
    }

    /// The shipped configuration: what a user gets when their file says nothing
    /// about permissions.
    ///
    /// The class defaults live here as ordinary entries of the sections they
    /// belong to, not as keys of their own — the runtime-state path
    /// is a `path.rules` entry, the interaction tool is a `tools.names` entry,
    /// and the read-only and command classes are `tools.rules` entries. That is
    /// what makes them editable and restorable (a user writes the same key, or
    /// deletes it) without the schema growing a parallel set of keys.
    pub fn global_default() -> Self {
        let allow = PermissionMode::Allow;
        let deny = PermissionMode::Deny;
        Self {
            path: Some(PathPermissionConfig {
                // The project is writable by default; external paths still
                // require an explicit grant. Approval remains a tool decision.
                workspace: Some(PathAccessModes::from_mode(PathPermissionMode::Readwrite)),
                external: Some(PathAccessModes::from_mode(PathPermissionMode::Deny)),
                // Shared OS temporary roots cannot be safely mounted into one
                // sandbox. Each worker receives its own private temporary area.
                rules: IndexMap::from([(RUNTIME_STATE_PATH.to_string(), allow_rw())]),
            }),
            network: Some(NetworkPermissionConfig::allow_all()),
            tools: Some(ToolPermissionConfig {
                // Ordinary execution tools default to Allow: their effects are
                // already constrained by the path, network, and shell-command
                // policies. Ask/Deny are opt-in per tool name or via the shell
                // command pattern table.
                default: Some(allow),
                names: BTreeMap::from([
                    // `agena.interaction.ask` *is* the prompt: a permission
                    // `Ask` here would confirm the question instead of asking
                    // it, so the tool is allowed and raises its own
                    // `UserInputRequired`. Both spellings are listed because
                    // the executor answers to the canonical and the compact
                    // tool name.
                    ("agena.interaction.ask".to_string(), allow),
                    ("interaction.ask".to_string(), allow),
                    // Web fetch/search are allowlisted because network policy
                    // already governs their targets.
                    ("agena.web.search".to_string(), allow),
                    ("agena.web.fetch".to_string(), allow),
                ]),
                rules: BTreeMap::from([
                    (
                        SHELL_COMMAND_RULE_GROUP.to_string(),
                        ToolPermissionRules::Ordered(IndexMap::from([
                            (COMMAND_CLASS_NO_OP.to_string(), allow),
                            (COMMAND_CLASS_ROUTINE.to_string(), allow),
                            (COMMAND_CLASS_DANGEROUS.to_string(), deny),
                        ])),
                    ),
                    // Tools whose contract is read-only, and neither shell nor
                    // interactive. Reading cannot change anything.
                    (
                        "*".to_string(),
                        ToolPermissionRules::Ordered(IndexMap::from([(
                            TOOL_CLASS_READ_ONLY.to_string(),
                            allow,
                        )])),
                    ),
                ]),
                ..Default::default()
            }),
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.path
            .as_ref()
            .is_none_or(PathPermissionConfig::is_empty)
            && self
                .network
                .as_ref()
                .is_none_or(NetworkPermissionConfig::is_empty)
            && self
                .tools
                .as_ref()
                .is_none_or(ToolPermissionConfig::is_empty)
            && self.plugins.is_empty()
            && self.approval_model.is_none()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        merge_path_section(&mut self.path, overlay.path);
        merge_network_section(&mut self.network, overlay.network);
        merge_tool_section(&mut self.tools, overlay.tools);
        self.plugins.extend(overlay.plugins);
        if overlay.approval_model.is_some() {
            self.approval_model = overlay.approval_model;
        }
    }

    pub fn merged_with(&self, overlay: &Self) -> Self {
        let mut merged = self.clone();
        merged.merge_from(overlay.clone());
        merged
    }
}

fn merge_path_section(
    current: &mut Option<PathPermissionConfig>,
    overlay: Option<PathPermissionConfig>,
) {
    match (current.as_mut(), overlay) {
        (Some(current), Some(overlay)) => current.merge_from(overlay),
        (None, Some(overlay)) => *current = Some(overlay),
        (_, None) => {}
    }
}

fn merge_network_section(
    current: &mut Option<NetworkPermissionConfig>,
    overlay: Option<NetworkPermissionConfig>,
) {
    match (current.as_mut(), overlay) {
        (Some(current), Some(overlay)) => current.merge_from(overlay),
        (None, Some(overlay)) => *current = Some(overlay),
        (_, None) => {}
    }
}

fn merge_tool_section(
    current: &mut Option<ToolPermissionConfig>,
    overlay: Option<ToolPermissionConfig>,
) {
    match (current.as_mut(), overlay) {
        (Some(current), Some(overlay)) => current.merge_from(overlay),
        (None, Some(overlay)) => *current = Some(overlay),
        (_, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NetworkAction;
    use serde_json::json;

    #[test]
    fn shipped_defaults_allow_project_writes_and_portable_outbound_network() {
        let config = PermissionConfig::global_default();
        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(value["path"]["workspace"], "readwrite");
        assert_eq!(value["path"]["external"], "deny");
        assert_eq!(value["path"]["rules"][RUNTIME_STATE_PATH], "readwrite");
        assert_eq!(
            value["network"],
            json!({"version":2,"default":"allow","allow":[],"deny":[],"inbound":"deny","hostLoopback":"deny"})
        );
        assert_eq!(value["tools"]["default"], "allow");
        assert_eq!(
            value["tools"]["rules"][SHELL_COMMAND_RULE_GROUP],
            json!({"no-op": "allow", "routine": "allow", "dangerous": "deny"})
        );
        let network = config.network.unwrap();
        for platform in ["linux", "macos", "windows"] {
            network.validate_for_platform(platform).unwrap();
        }
        let ingress = network.resolved().ingress.unwrap();
        assert_eq!(ingress.default, Some(NetworkAction::Deny));
        assert_eq!(ingress.host_loopback, Some(NetworkAction::Deny));
    }

    #[test]
    fn empty_permission_is_an_overlay_and_explicit_restrictions_remain_effective() {
        assert_eq!(
            serde_json::to_value(PermissionConfig::default()).unwrap(),
            json!({})
        );
        for mode in ["readonly", "deny"] {
            for network in [json!({}), json!({"default":"deny"})] {
                let overlay: PermissionConfig = serde_json::from_value(json!({
                    "path":{"workspace":mode},
                    "network":network
                }))
                .unwrap();
                let mut config = PermissionConfig::global_default();
                config.merge_from(overlay);
                let value = serde_json::to_value(&config).unwrap();
                assert_eq!(value["path"]["workspace"], mode);
                assert_eq!(value["path"]["external"], "deny");
                assert_eq!(config.network.unwrap(), NetworkPermissionConfig::offline());
            }
        }
    }
}
