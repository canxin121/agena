//! Execution capability and permission composition, independent of Agena identity.

mod path_rules;
mod path_validation;
mod platform_validation;
#[cfg(test)]
mod policy_validation_tests;
pub use path_validation::validate_permission_config_for_workspace;
pub(crate) use path_validation::{canonical_resource_path, same_existing_file};

use std::path::Path;

use agena_domain::{
    AccessKind, NetworkTarget, PermissionDecision, PermissionMode, SHELL_COMMAND_RULE_GROUP,
    command_class_keyword,
};
pub use agena_domain::{
    NetworkPermissionConfig, PathAccessModes, PathPermissionConfig, PermissionConfig,
    ToolPermissionConfig, ToolPermissionRules,
};
use agena_plugin_sdk::{PluginKey, ToolTag};
use indexmap::IndexMap;

use crate::command_class::CommandClass;
use crate::permission::protected_paths::ProtectedPaths;
use crate::permission::{
    NetworkPermissionPolicy, PermissionConfigError, PermissionPolicy, ToolPermissionPolicy,
};
pub use crate::permission::{
    SandboxFilesystemPolicy, SandboxFilesystemRestrictions, SandboxPathAccess, SandboxPathPattern,
    SandboxPathPolicy, SandboxPathRule, SandboxPermissionSnapshot, SandboxPersistedRule,
    SandboxPolicyError,
};

pub fn apply_to_permission_policy(
    config: &PermissionConfig,
    base: PermissionPolicy,
) -> Result<PermissionPolicy, PermissionConfigError> {
    match config.path.as_ref() {
        Some(path) => apply_path_permission_config(path, base),
        None => Ok(base),
    }
}

pub fn apply_to_tool_permission_policy(
    config: &PermissionConfig,
    mut base: ToolPermissionPolicy,
) -> Result<ToolPermissionPolicy, PermissionConfigError> {
    for (id, enabled) in &config.plugins {
        let plugin = id
            .parse::<PluginKey>()
            .ok()
            .filter(|key| key.to_string() == *id)
            .ok_or_else(|| PermissionConfigError::InvalidPluginId { plugin: id.clone() })?;
        if *enabled {
            base.disabled_plugins.remove(&plugin);
        } else {
            base.disabled_plugins.insert(plugin);
        }
    }
    match config.tools.as_ref() {
        Some(tools) => apply_tool_permission_config(tools, base),
        None => Ok(base),
    }
}

pub fn apply_to_network_permission_policy(
    config: &PermissionConfig,
    base: NetworkPermissionPolicy,
) -> Result<NetworkPermissionPolicy, PermissionConfigError> {
    match config.network.as_ref() {
        Some(network) => apply_network_permission_config(network, base),
        None => Ok(base),
    }
}

/// Validate a serialized permission configuration without constructing a
/// session/runtime execution principal. Configuration loading uses this narrow adapter so
/// policy validation can move independently of the concrete execution principal.
pub fn validate_permission_config(config: &PermissionConfig) -> Result<(), PermissionConfigError> {
    platform_validation::validate(config)?;
    let _ = apply_to_permission_policy(config, PermissionPolicy::allow_all())?;
    let _ = apply_to_network_permission_policy(config, NetworkPermissionPolicy::allow_all())?;
    let _ = apply_to_tool_permission_policy(config, ToolPermissionPolicy::allow_all())?;
    Ok(())
}

fn apply_path_permission_config(
    value: &PathPermissionConfig,
    base: PermissionPolicy,
) -> Result<PermissionPolicy, PermissionConfigError> {
    path_rules::apply(value, base)
}

fn apply_network_permission_config(
    value: &NetworkPermissionConfig,
    _base: NetworkPermissionPolicy,
) -> Result<NetworkPermissionPolicy, PermissionConfigError> {
    NetworkPermissionPolicy::from_config(value)
}

pub fn apply_tool_permission_config(
    value: &ToolPermissionConfig,
    mut base: ToolPermissionPolicy,
) -> Result<ToolPermissionPolicy, PermissionConfigError> {
    validate_tool_permission_names(value)?;
    let value = value.clone().normalized();
    if let Some(mode) = value.default {
        base.default_mode = mode;
    }
    for (tool_name, mode) in value.names.iter().chain(value.plugin.iter()) {
        let name = tool_name.trim();
        if name.is_empty() {
            continue;
        }
        base.tool_modes.insert(name.to_string(), *mode);
    }
    for (tool_name, rules) in &value.rules {
        let name = tool_name.trim();
        if name.is_empty() {
            continue;
        }
        base = apply_tool_permission_rules(base, name, rules)?;
    }
    Ok(base)
}

fn validate_tool_permission_names(
    value: &ToolPermissionConfig,
) -> Result<(), PermissionConfigError> {
    value
        .validate_aliases()
        .map_err(|reason| PermissionConfigError::InvalidToolRule {
            tool: "aliases".into(),
            pattern: "names/rules".into(),
            reason,
        })?;
    for (name, is_rule) in value
        .names
        .keys()
        .map(|name| (name, false))
        .chain(value.plugin.keys().map(|name| (name, false)))
        .chain(value.rules.keys().map(|name| (name, true)))
    {
        if name.is_empty()
            || name.trim() != name
            || name.chars().any(char::is_whitespace)
            || name.chars().any(char::is_control)
            || (name.contains(['*', '?'])
                && !(is_rule && matches!(name.as_str(), "*" | SHELL_COMMAND_RULE_GROUP)))
        {
            return Err(PermissionConfigError::InvalidToolRule {
                tool: name.clone(),
                pattern: name.clone(),
                reason:
                    "use an exact, non-empty tool name; tools.default controls the default decision"
                        .into(),
            });
        }
    }
    Ok(())
}

fn apply_tool_permission_rules(
    mut base: ToolPermissionPolicy,
    tool_name: &str,
    rules: &ToolPermissionRules,
) -> Result<ToolPermissionPolicy, PermissionConfigError> {
    match rules {
        ToolPermissionRules::Mode(mode) => {
            if tool_name == SHELL_COMMAND_RULE_GROUP {
                return Err(PermissionConfigError::InvalidToolRule {
                    tool: tool_name.into(),
                    pattern: "*".into(),
                    reason:
                        "the shared shell rule group must be an ordered map of command patterns"
                            .into(),
                });
            }
            if tool_name == "*" {
                return Err(PermissionConfigError::InvalidToolRule {
                    tool: tool_name.into(),
                    pattern: "*".into(),
                    reason: "a wildcard tool name is not a default rule; use tools.default, or tools.rules.\"*\".\"read-only\" for the supported tool class".into(),
                });
            }
            base.tool_modes.insert(tool_name.to_string(), *mode);
            Ok(base)
        }
        ToolPermissionRules::Ordered(entries) => {
            // Keep accepting historical command tables for the current
            // WebSocket-only monitor.
            if matches!(tool_name, SHELL_COMMAND_RULE_GROUP | "agena.monitor.start") {
                for (pattern, mode) in ordered_rule_entries(entries) {
                    let trimmed = pattern.trim();
                    if trimmed.is_empty()
                        || trimmed != pattern
                        || trimmed.chars().any(char::is_control)
                    {
                        return Err(PermissionConfigError::InvalidToolRule {
                            tool: tool_name.to_owned(),
                            pattern: pattern.to_owned(),
                            reason: "command rules must be non-empty and must not contain surrounding whitespace or control characters".into(),
                        });
                    }
                    // JavaScript orders array-index object keys numerically, so a
                    // web edit cannot preserve their position in this ordered map.
                    if trimmed
                        .parse::<u32>()
                        .is_ok_and(|index| index != u32::MAX && index.to_string() == trimmed)
                    {
                        return Err(PermissionConfigError::InvalidToolRule {
                            tool: tool_name.to_owned(),
                            pattern: pattern.to_owned(),
                            reason: "numeric-only command keys cannot preserve rule order in JSON editors; use a command pattern with nonnumeric text or tools.names for a tool-level decision".into(),
                        });
                    }
                    if tool_name != SHELL_COMMAND_RULE_GROUP {
                        continue;
                    }
                    let class = match command_class_keyword(trimmed) {
                        Some(agena_domain::COMMAND_CLASS_NO_OP) => Some(CommandClass::NoOp),
                        Some(agena_domain::COMMAND_CLASS_ROUTINE) => Some(CommandClass::Routine),
                        Some(agena_domain::COMMAND_CLASS_DANGEROUS) => {
                            Some(CommandClass::Dangerous)
                        }
                        _ => None,
                    };
                    if let Some(class) = class {
                        base.add_bash_class_rule(class, trimmed, mode);
                    } else {
                        // "*" is a command pattern in the shared, ordered table.
                        base.add_bash_overlay_rule(trimmed, mode);
                    }
                }
                Ok(base)
            } else {
                let mut fallback = None;
                for (pattern, mode) in ordered_rule_entries(entries) {
                    let trimmed = pattern.trim();
                    if trimmed.is_empty()
                        || trimmed != pattern
                        || trimmed.chars().any(char::is_control)
                    {
                        return Err(PermissionConfigError::InvalidToolRule {
                            tool: tool_name.to_owned(),
                            pattern: pattern.to_owned(),
                            reason: "tool conditions must be non-empty and must not contain surrounding whitespace or control characters".into(),
                        });
                    }
                    if tool_name == "*" && tool_class_keyword(trimmed).is_some() {
                        base.set_read_only_mode(Some(mode));
                    } else if trimmed == "*" && tool_name != "*" {
                        fallback = Some(mode);
                    } else {
                        return Err(PermissionConfigError::InvalidToolRule {
                            tool: tool_name.to_owned(),
                            pattern: pattern.to_owned(),
                            reason: "conditional patterns are supported only for shell commands; use permission.path for filesystem rules, permission.network for destinations, or tools.names for a tool-level decision".into(),
                        });
                    }
                }
                if let Some(mode) = fallback {
                    base.tool_modes.insert(tool_name.to_string(), mode);
                }
                Ok(base)
            }
        }
    }
}

/// Recognize the read-only tool class keyword. It is written under the
/// wildcard tool name, `tools.rules."*"`, because the class is a property of
/// the tool's declared tags rather than of its name.
fn tool_class_keyword(value: &str) -> Option<()> {
    match normalized_keyword(value).as_str() {
        "read-only" | "read_only" | "readonly" => Some(()),
        _ => None,
    }
}

fn normalized_keyword(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

fn ordered_rule_entries(entries: &IndexMap<String, PermissionMode>) -> Vec<(&str, PermissionMode)> {
    // Command patterns explicitly use insertion order: the last matching
    // command rule wins. Sorting by string length changes the user's policy.
    entries
        .iter()
        .map(|(pattern, mode)| (pattern.as_str(), *mode))
        .collect()
}

#[derive(Debug, Clone)]
/// Principal of an execution: blocked flag and permission policies.
pub struct ExecutionPrincipal {
    pub blocked: bool,
    pub permission_policy: PermissionPolicy,
    pub network_policy: NetworkPermissionPolicy,
    pub tool_policy: ToolPermissionPolicy,
    permission_ceiling_policy: Vec<PermissionPolicy>,
    network_ceiling_policy: Vec<NetworkPermissionPolicy>,
    tool_ceiling_policy: Vec<ToolPermissionPolicy>,
}

impl ExecutionPrincipal {
    /// Capture every authority layer and resolve host path aliases before a
    /// child process receives its restricted environment.
    pub fn sandbox_permission_snapshot(&self, workspace_root: &Path) -> SandboxPermissionSnapshot {
        SandboxPermissionSnapshot::from_policies(
            self.blocked,
            workspace_root,
            std::iter::once(&self.permission_policy).chain(self.permission_ceiling_policy.iter()),
            std::iter::once(&self.network_policy).chain(self.network_ceiling_policy.iter()),
        )
    }

    pub fn new(permission_policy: PermissionPolicy, tool_policy: ToolPermissionPolicy) -> Self {
        Self {
            blocked: false,
            permission_policy,
            network_policy: NetworkPermissionPolicy::allow_all(),
            tool_policy,
            permission_ceiling_policy: Vec::new(),
            network_ceiling_policy: Vec::new(),
            tool_ceiling_policy: Vec::new(),
        }
    }

    pub fn try_apply_permission_config(
        mut self,
        config: &PermissionConfig,
    ) -> Result<Self, PermissionConfigError> {
        if config.is_empty() {
            return Ok(self);
        }
        self.permission_policy =
            apply_to_permission_policy(config, self.permission_policy.clone())?;
        self.network_policy =
            apply_to_network_permission_policy(config, self.network_policy.clone())?;
        self.tool_policy = apply_to_tool_permission_policy(config, self.tool_policy.clone())?;
        Ok(self)
    }

    pub fn apply_permission_config_or_self(self, config: &PermissionConfig) -> Self {
        match self.clone().try_apply_permission_config(config) {
            Ok(principal) => principal,
            Err(err) => {
                tracing::error!(
                    target: "agena::permission",
                    "refusing invalid execution permission config at runtime: {err}"
                );
                // Effective permissions are a security boundary. Persisted or
                // imported runtime state can still reach this layer even when
                // normal configuration validation was bypassed, so malformed
                // policy must never fall back to the base principal's privileges.
                let mut denied = self;
                denied.blocked = true;
                denied
            }
        }
    }

    pub fn try_apply_permission_ceiling(
        mut self,
        config: &PermissionConfig,
    ) -> Result<Self, PermissionConfigError> {
        if config.is_empty() {
            return Ok(self);
        }
        // Independent authorities only intersect. Replacing the earlier
        // ceiling would allow a later channel/child grant to erase a stricter
        // parent restriction even though the effective base policy stayed the
        // same. Compile all three before appending this authority layer.
        let paths = apply_to_permission_policy(config, PermissionPolicy::allow_all())?;
        let network =
            apply_to_network_permission_policy(config, NetworkPermissionPolicy::allow_all())?;
        let tools = apply_to_tool_permission_policy(config, ToolPermissionPolicy::allow_all())?;
        self.permission_ceiling_policy.push(paths);
        self.network_ceiling_policy.push(network);
        self.tool_ceiling_policy.push(tools);
        Ok(self)
    }

    pub fn apply_permission_ceiling_or_self(self, config: &PermissionConfig) -> Self {
        match self.clone().try_apply_permission_ceiling(config) {
            Ok(principal) => principal,
            Err(err) => {
                tracing::error!(
                    target: "agena::permission",
                    "refusing invalid permission ceiling at runtime: {err}"
                );
                // Invalid boundaries must fail closed rather than silently
                // granting the child the unrestricted base principal.
                let mut denied = self;
                denied.blocked = true;
                denied
            }
        }
    }

    pub fn authorize_tool(
        &self,
        tool_name: &str,
        command: Option<&str>,
        tags: &[ToolTag],
    ) -> PermissionDecision {
        self.authorize_tool_names(&[tool_name], command, tags)
    }

    pub fn authorize_tool_names(
        &self,
        tool_names: &[&str],
        command: Option<&str>,
        tags: &[ToolTag],
    ) -> PermissionDecision {
        if self.blocked {
            return PermissionDecision::Deny {
                reason: "execution principal is blocked".to_owned(),
            };
        }
        let decision = self
            .tool_policy
            .check_tool_with_names(tool_names, command, tags);
        self.tool_ceiling_policy
            .iter()
            .fold(decision, |decision, ceiling| {
                restrictive_decision(
                    decision,
                    ceiling.check_tool_with_names(tool_names, command, tags),
                )
            })
    }

    pub fn authorize_tool_name(&self, tool_name: &str) -> PermissionDecision {
        self.authorize_tool(tool_name, None, &[])
    }

    pub fn plugin_is_enabled(&self, plugin: &PluginKey) -> bool {
        std::iter::once(&self.tool_policy)
            .chain(&self.tool_ceiling_policy)
            .all(|policy| !policy.disabled_plugins.contains(plugin))
    }

    pub fn tool_is_discoverable(&self, names: &[&str], tags: &[ToolTag]) -> bool {
        !self.blocked
            && std::iter::once(&self.tool_policy)
                .chain(&self.tool_ceiling_policy)
                .all(|policy| policy.tool_is_discoverable(names, tags))
    }

    pub fn authorize_network_connect(&self, target: &NetworkTarget) -> PermissionDecision {
        if self.blocked {
            return PermissionDecision::Deny {
                reason: "execution principal is blocked".to_owned(),
            };
        }
        let decision = self.network_policy.check_connect(target);
        self.network_ceiling_policy
            .iter()
            .fold(decision, |decision, ceiling| {
                restrictive_decision(decision, ceiling.check_connect(target))
            })
    }

    pub fn authorize_path_access(
        &self,
        access: AccessKind,
        workspace_root: &Path,
        target_path: &Path,
    ) -> PermissionDecision {
        if self.blocked {
            return PermissionDecision::Deny {
                reason: "execution principal is blocked".to_owned(),
            };
        }
        let target = if target_path.is_absolute() || workspace_root.as_os_str().is_empty() {
            target_path.to_path_buf()
        } else {
            workspace_root.join(target_path)
        };
        let protected = ProtectedPaths::for_workspace(
            workspace_root,
            &agena_storage_layout::StorageLayout::current(),
            Some(&self.permission_policy),
        );
        if let Some(denial) = protected.check(access, &target) {
            return denial;
        }
        let decision = self
            .permission_policy
            .check_access(access, workspace_root, target_path);
        self.permission_ceiling_policy
            .iter()
            .fold(decision, |decision, ceiling| {
                restrictive_decision(
                    decision,
                    ceiling.check_access(access, workspace_root, target_path),
                )
            })
    }
}

fn restrictive_decision(
    primary: PermissionDecision,
    ceiling: PermissionDecision,
) -> PermissionDecision {
    match (&primary, &ceiling) {
        (PermissionDecision::Deny { .. }, _) => primary,
        (_, PermissionDecision::Deny { .. }) => ceiling,
        (PermissionDecision::Ask { .. }, _) => primary,
        (_, PermissionDecision::Ask { .. }) => ceiling,
        (PermissionDecision::Auto { .. }, _) => primary,
        (_, PermissionDecision::Auto { .. }) => ceiling,
        (PermissionDecision::Allow, PermissionDecision::Allow) => PermissionDecision::Allow,
    }
}

#[cfg(test)]
mod intersecting_ceiling_tests {
    use super::*;
    #[test]
    fn later_channel_allow_cannot_erase_parent_tool_deny() {
        let deny: PermissionConfig = serde_json::from_value(
            serde_json::json!({"tools":{"names":{"agena.fs.write":"deny"}}}),
        )
        .unwrap();
        let allow: PermissionConfig =
            serde_json::from_value(serde_json::json!({"tools":{"default":"allow"}})).unwrap();
        let principal = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_ceiling(&deny)
        .unwrap()
        .try_apply_permission_ceiling(&allow)
        .unwrap();
        assert!(matches!(
            principal.authorize_tool_name("agena.fs.write"),
            PermissionDecision::Deny { .. }
        ));
        assert!(matches!(
            principal.authorize_tool_name("agena.fs.read"),
            PermissionDecision::Allow
        ));
    }
    #[test]
    fn later_workspace_allow_cannot_erase_parent_path_deny() {
        let deny: PermissionConfig = serde_json::from_value(
            serde_json::json!({"path":{"workspace":{"read":"allow","write":"deny"}}}),
        )
        .unwrap();
        let allow: PermissionConfig = serde_json::from_value(
            serde_json::json!({"path":{"workspace":{"read":"allow","write":"allow"}}}),
        )
        .unwrap();
        let principal = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_ceiling(&deny)
        .unwrap()
        .try_apply_permission_ceiling(&allow)
        .unwrap();
        assert!(matches!(
            principal.authorize_path_access(
                AccessKind::Write,
                Path::new("/project"),
                Path::new("/project/file")
            ),
            PermissionDecision::Deny { .. }
        ));
        assert!(matches!(
            principal.authorize_path_access(
                AccessKind::Read,
                Path::new("/project"),
                Path::new("/project/file")
            ),
            PermissionDecision::Allow
        ));
    }
}
