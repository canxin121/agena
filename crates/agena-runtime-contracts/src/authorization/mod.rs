//! Execution capability and permission composition, independent of Agena identity.

use std::path::Path;

use agena_domain::{AccessKind, AccessSelector, NetworkTarget, PermissionDecision, PermissionMode};
pub use agena_domain::{
    NetworkPermissionConfig, PathAccessModes, PathAccessRuleConfig, PathPermissionConfig,
    PermissionConfig, ToolPermissionConfig, ToolPermissionRules,
};
use agena_plugin_host::sdk::ToolPermissionContract;
use indexmap::IndexMap;

use crate::permission::{
    NetworkPermissionPolicy, PermissionConfigError, PermissionPolicy, ToolPermissionPolicy,
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
    base: ToolPermissionPolicy,
) -> Result<ToolPermissionPolicy, PermissionConfigError> {
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
    let _ = apply_to_permission_policy(config, PermissionPolicy::allow_all())?;
    let _ = apply_to_network_permission_policy(config, NetworkPermissionPolicy::allow_all())?;
    let _ = apply_to_tool_permission_policy(config, ToolPermissionPolicy::allow_all())?;
    Ok(())
}

fn apply_path_permission_config(
    value: &PathPermissionConfig,
    mut base: PermissionPolicy,
) -> Result<PermissionPolicy, PermissionConfigError> {
    if let Some(workspace) = value.workspace.as_ref() {
        if let Some(mode) = workspace.read {
            base.workspace_read_default = mode;
        }
        if let Some(mode) = workspace.write {
            base.workspace_write_default = mode;
        }
    }
    if let Some(external) = value.external.as_ref() {
        if let Some(mode) = external.read {
            base.external_read_default = mode;
        }
        if let Some(mode) = external.write {
            base.external_write_default = mode;
        }
    }
    for (pattern, access) in &value.rules {
        let modes = path_access_rule_to_modes(access)?;
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(mode) = modes.read {
            base.add_path_pattern_rule(AccessSelector::Read, mode, trimmed)?;
        }
        if let Some(mode) = modes.write {
            base.add_path_pattern_rule(AccessSelector::Write, mode, trimmed)?;
        }
    }
    Ok(base)
}

fn path_access_rule_to_modes(
    value: &PathAccessRuleConfig,
) -> Result<PathAccessModes, PermissionConfigError> {
    match value {
        PathAccessRuleConfig::Modes(modes) => Ok(modes.clone()),
        PathAccessRuleConfig::Shorthand(value) => path_access_shorthand(value),
    }
}

fn path_access_shorthand(value: &str) -> Result<PathAccessModes, PermissionConfigError> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    let both = |mode| PathAccessModes {
        read: Some(mode),
        write: Some(mode),
    };
    match normalized.as_str() {
        "allow" => Ok(both(PermissionMode::Allow)),
        "auto" => Ok(both(PermissionMode::Auto)),
        "ask" => Ok(both(PermissionMode::Ask)),
        "deny" | "none" => Ok(both(PermissionMode::Deny)),
        "read" | "read_only" | "ro" => Ok(PathAccessModes {
            read: Some(PermissionMode::Allow),
            write: Some(PermissionMode::Deny),
        }),
        "write" | "write_only" | "wo" => Ok(PathAccessModes {
            read: Some(PermissionMode::Deny),
            write: Some(PermissionMode::Allow),
        }),
        "read_write" | "rw" => Ok(both(PermissionMode::Allow)),
        _ => Err(PermissionConfigError::InvalidPathAccessShorthand {
            value: value.to_string(),
        }),
    }
}

fn apply_network_permission_config(
    value: &NetworkPermissionConfig,
    mut base: NetworkPermissionPolicy,
) -> Result<NetworkPermissionPolicy, PermissionConfigError> {
    if let Some(mode) = value.internet {
        base.internet_default = mode;
    }
    if let Some(mode) = value.private {
        base.private_default = mode;
    }
    if let Some(mode) = value.loopback {
        base.loopback_default = mode;
    }
    for (pattern, mode) in &value.rules {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            continue;
        }
        base.add_rule(trimmed, *mode)?;
    }
    Ok(base)
}

pub fn apply_tool_permission_config(
    value: &ToolPermissionConfig,
    mut base: ToolPermissionPolicy,
) -> Result<ToolPermissionPolicy, PermissionConfigError> {
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

fn apply_tool_permission_rules(
    mut base: ToolPermissionPolicy,
    tool_name: &str,
    rules: &ToolPermissionRules,
) -> Result<ToolPermissionPolicy, PermissionConfigError> {
    match rules {
        ToolPermissionRules::Mode(mode) => {
            base.tool_modes.insert(tool_name.to_string(), *mode);
            Ok(base)
        }
        ToolPermissionRules::Ordered(entries) => {
            if matches!(tool_name, "agena.shell.run") {
                for (pattern, mode) in sorted_rule_entries(entries) {
                    let trimmed = pattern.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == "*" {
                        base.tool_modes.insert(tool_name.to_string(), mode);
                    } else {
                        base.add_bash_overlay_rule(trimmed, mode);
                    }
                }
                Ok(base)
            } else {
                let mut fallback = None;
                for (pattern, mode) in sorted_rule_entries(entries) {
                    let trimmed = pattern.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == "*" {
                        fallback = Some(mode);
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

fn sorted_rule_entries(entries: &IndexMap<String, PermissionMode>) -> Vec<(&str, PermissionMode)> {
    let mut out = entries
        .iter()
        .map(|(pattern, mode)| (pattern.as_str(), *mode))
        .collect::<Vec<_>>();
    out.sort_by(|(left_pattern, _), (right_pattern, _)| {
        left_pattern
            .len()
            .cmp(&right_pattern.len())
            .then_with(|| left_pattern.cmp(right_pattern))
    });
    out
}

#[derive(Debug, Clone)]
/// Principal of an execution: blocked flag and permission policies.
pub struct ExecutionPrincipal {
    pub blocked: bool,
    pub permission_policy: PermissionPolicy,
    pub network_policy: NetworkPermissionPolicy,
    pub tool_policy: ToolPermissionPolicy,
    permission_ceiling_policy: Option<PermissionPolicy>,
    network_ceiling_policy: Option<NetworkPermissionPolicy>,
    tool_ceiling_policy: Option<ToolPermissionPolicy>,
}

impl ExecutionPrincipal {
    pub fn new(permission_policy: PermissionPolicy, tool_policy: ToolPermissionPolicy) -> Self {
        Self {
            blocked: false,
            permission_policy,
            network_policy: NetworkPermissionPolicy::allow_all(),
            tool_policy,
            permission_ceiling_policy: None,
            network_ceiling_policy: None,
            tool_ceiling_policy: None,
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
        self.permission_ceiling_policy = Some(apply_to_permission_policy(
            config,
            PermissionPolicy::allow_all(),
        )?);
        self.network_ceiling_policy = Some(apply_to_network_permission_policy(
            config,
            NetworkPermissionPolicy::allow_all(),
        )?);
        self.tool_ceiling_policy = Some(apply_to_tool_permission_policy(
            config,
            ToolPermissionPolicy::allow_all(),
        )?);
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
        contract: &ToolPermissionContract,
    ) -> PermissionDecision {
        if self.blocked {
            return PermissionDecision::Deny {
                reason: "execution principal is blocked".to_owned(),
            };
        }
        let decision = self.tool_policy.check_tool(tool_name, command, contract);
        match self.tool_ceiling_policy.as_ref() {
            Some(ceiling) => {
                restrictive_decision(decision, ceiling.check_tool(tool_name, command, contract))
            }
            None => decision,
        }
    }

    pub fn authorize_tool_names(
        &self,
        tool_names: &[&str],
        command: Option<&str>,
        contract: &ToolPermissionContract,
    ) -> PermissionDecision {
        if self.blocked {
            return PermissionDecision::Deny {
                reason: "execution principal is blocked".to_owned(),
            };
        }
        let decision = self
            .tool_policy
            .check_tool_with_names(tool_names, command, contract);
        match self.tool_ceiling_policy.as_ref() {
            Some(ceiling) => restrictive_decision(
                decision,
                ceiling.check_tool_with_names(tool_names, command, contract),
            ),
            None => decision,
        }
    }

    pub fn authorize_tool_name(&self, tool_name: &str) -> PermissionDecision {
        self.authorize_tool(tool_name, None, &ToolPermissionContract::default())
    }

    pub fn authorize_tool_contract(
        &self,
        tool_name: &str,
        contract: &ToolPermissionContract,
    ) -> PermissionDecision {
        if self.blocked {
            return PermissionDecision::Deny {
                reason: "execution principal is blocked".to_owned(),
            };
        }
        self.authorize_tool(tool_name, None, contract)
    }

    pub fn authorize_network_connect(&self, target: &NetworkTarget) -> PermissionDecision {
        if self.blocked {
            return PermissionDecision::Deny {
                reason: "execution principal is blocked".to_owned(),
            };
        }
        let decision = self.network_policy.check_connect(target);
        match self.network_ceiling_policy.as_ref() {
            Some(ceiling) => restrictive_decision(decision, ceiling.check_connect(target)),
            None => decision,
        }
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
        let decision = self
            .permission_policy
            .check_access(access, workspace_root, target_path);
        match self.permission_ceiling_policy.as_ref() {
            Some(ceiling) => restrictive_decision(
                decision,
                ceiling.check_access(access, workspace_root, target_path),
            ),
            None => decision,
        }
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
mod permission_ceiling_tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn global_web_read_tools_follow_network_policy_without_a_second_tool_prompt() {
        let principal = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&PermissionConfig::global_default())
        .expect("global permission policy");

        for tool in ["agena.web.search", "agena.web.fetch"] {
            assert_eq!(
                principal.authorize_tool_contract(
                    tool,
                    &ToolPermissionContract {
                        read_only: true,
                        ..ToolPermissionContract::default()
                    },
                ),
                PermissionDecision::Allow,
            );
        }
        assert!(matches!(
            principal
                .authorize_network_connect(&"https://example.com".parse().expect("network target")),
            PermissionDecision::Auto { .. }
        ));

        let allow_network = PermissionConfig {
            network: Some(NetworkPermissionConfig {
                internet: Some(PermissionMode::Allow),
                private: Some(PermissionMode::Allow),
                loopback: Some(PermissionMode::Allow),
                ..Default::default()
            }),
            ..Default::default()
        };
        let principal = principal
            .try_apply_permission_config(&allow_network)
            .expect("allowed network zones");
        for target in ["https://example.com", "http://10.0.0.1", "http://localhost"] {
            assert_eq!(
                principal.authorize_network_connect(&target.parse().expect("network target")),
                PermissionDecision::Allow,
            );
        }
    }

    #[test]
    fn global_default_allows_workspace_reads_at_both_permission_boundaries() {
        let principal = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&PermissionConfig::global_default())
        .expect("global permission policy");
        let workspace = std::path::Path::new("/workspace");

        assert_eq!(
            principal.authorize_path_access(
                AccessKind::Read,
                workspace,
                &workspace.join("README.md"),
            ),
            PermissionDecision::Allow,
            "the built-in workspace read default must not open an approval request"
        );
        assert!(
            matches!(
                principal.authorize_tool_contract(
                    "agena.fs.read",
                    &ToolPermissionContract {
                        read_only: true,
                        ..ToolPermissionContract::default()
                    },
                ),
                PermissionDecision::Allow
            ),
            "ordinary execution tools default to allow; their effects are already governed by path/network/shell policies"
        );
    }

    #[test]
    fn parent_tool_rule_is_evaluated_independently_of_child_rule() {
        let allow = PermissionMode::Allow;
        let deny = PermissionMode::Deny;
        let child = PermissionConfig {
            tools: Some(ToolPermissionConfig {
                default: Some(allow),
                names: BTreeMap::from([("agena.tasks.run".to_string(), allow)]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let parent = PermissionConfig {
            tools: Some(ToolPermissionConfig {
                default: Some(allow),
                names: BTreeMap::from([("agena.tasks.run".to_string(), deny)]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let principal = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&child)
        .expect("child policy")
        .try_apply_permission_ceiling(&parent)
        .expect("parent ceiling");

        assert!(matches!(
            principal.authorize_tool_name("agena.tasks.run"),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn invalid_effective_permission_fails_closed() {
        let invalid = PermissionConfig {
            path: Some(PathPermissionConfig {
                rules: IndexMap::from([(
                    "<unknown>/secret".to_string(),
                    PathAccessRuleConfig::Modes(PathAccessModes {
                        read: Some(PermissionMode::Allow),
                        write: Some(PermissionMode::Allow),
                    }),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let principal = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .apply_permission_config_or_self(&invalid);

        assert!(matches!(
            principal.authorize_tool_name("agena.fs.read"),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn broader_parent_path_deny_beats_more_specific_child_allow() {
        let allow = PermissionMode::Allow;
        let deny = PermissionMode::Deny;
        let child = PermissionConfig {
            path: Some(PathPermissionConfig {
                workspace: Some(PathAccessModes {
                    read: Some(allow),
                    write: Some(allow),
                }),
                rules: IndexMap::from([(
                    "secret/file.txt".to_string(),
                    PathAccessRuleConfig::Modes(PathAccessModes {
                        read: Some(allow),
                        write: Some(allow),
                    }),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let parent = PermissionConfig {
            path: Some(PathPermissionConfig {
                workspace: Some(PathAccessModes {
                    read: Some(allow),
                    write: Some(allow),
                }),
                rules: IndexMap::from([(
                    "secret/**".to_string(),
                    PathAccessRuleConfig::Modes(PathAccessModes {
                        read: Some(deny),
                        write: Some(deny),
                    }),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let principal = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&child)
        .expect("child policy")
        .try_apply_permission_ceiling(&parent)
        .expect("parent ceiling");
        let workspace = std::path::Path::new("/workspace");

        assert!(matches!(
            principal.authorize_path_access(
                AccessKind::Write,
                workspace,
                &workspace.join("secret/file.txt"),
            ),
            PermissionDecision::Deny { .. }
        ));
    }
}
