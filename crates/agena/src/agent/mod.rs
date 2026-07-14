use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::permission::{
    AccessKind, AccessSelector, NetworkPermissionPolicy, NetworkTarget, PermissionConfigError,
    PermissionDecision, PermissionMode, PermissionPolicy, ToolPermissionPolicy,
};
use crate::plugin::sdk::ToolTag;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct PermissionConfig {
    #[serde(default, skip_serializing_if = "path_permission_is_empty")]
    pub path: Option<PathPermissionConfig>,
    #[serde(default, skip_serializing_if = "network_permission_is_empty")]
    pub network: Option<NetworkPermissionConfig>,
    #[serde(default, skip_serializing_if = "tool_permission_is_empty")]
    pub tools: Option<ToolPermissionConfig>,
}

impl PermissionConfig {
    pub fn global_default() -> Self {
        let ask = PermissionMode::Ask;
        Self {
            path: Some(PathPermissionConfig {
                workspace: Some(PathAccessModes {
                    read: Some(PermissionMode::Allow),
                    write: Some(ask),
                }),
                external: Some(PathAccessModes {
                    read: Some(ask),
                    write: Some(ask),
                }),
                ..Default::default()
            }),
            network: Some(NetworkPermissionConfig {
                internet: Some(ask),
                private: Some(ask),
                loopback: Some(ask),
                ..Default::default()
            }),
            tools: Some(ToolPermissionConfig {
                default: Some(ask),
                tags: BTreeMap::from([("filesystem_read".to_string(), PermissionMode::Allow)]),
                ..Default::default()
            }),
        }
    }

    pub fn is_empty(&self) -> bool {
        path_permission_is_empty(&self.path)
            && network_permission_is_empty(&self.network)
            && tool_permission_is_empty(&self.tools)
    }

    pub fn merge_from(&mut self, overlay: Self) {
        merge_permission_section(&mut self.path, overlay.path);
        merge_permission_section(&mut self.network, overlay.network);
        merge_permission_section(&mut self.tools, overlay.tools);
    }

    pub fn merged_with(&self, overlay: &Self) -> Self {
        let mut merged = self.clone();
        merged.merge_from(overlay.clone());
        merged
    }

    pub fn apply_to_permission_policy(
        &self,
        base: PermissionPolicy,
    ) -> Result<PermissionPolicy, PermissionConfigError> {
        match self.path.as_ref() {
            Some(path) => path.apply_to_permission_policy(base),
            None => Ok(base),
        }
    }

    pub fn apply_to_tool_permission_policy(
        &self,
        base: ToolPermissionPolicy,
    ) -> Result<ToolPermissionPolicy, PermissionConfigError> {
        match self.tools.as_ref() {
            Some(tools) => tools.apply_to_tool_permission_policy(base),
            None => Ok(base),
        }
    }

    pub fn apply_to_network_permission_policy(
        &self,
        base: NetworkPermissionPolicy,
    ) -> Result<NetworkPermissionPolicy, PermissionConfigError> {
        match self.network.as_ref() {
            Some(network) => network.apply_to_network_permission_policy(base),
            None => Ok(base),
        }
    }
}

pub type AgentPermissionConfig = PermissionConfig;

fn path_permission_is_empty(config: &Option<PathPermissionConfig>) -> bool {
    config.as_ref().is_none_or(PathPermissionConfig::is_empty)
}

fn network_permission_is_empty(config: &Option<NetworkPermissionConfig>) -> bool {
    config
        .as_ref()
        .is_none_or(NetworkPermissionConfig::is_empty)
}

fn tool_permission_is_empty(config: &Option<ToolPermissionConfig>) -> bool {
    config.as_ref().is_none_or(ToolPermissionConfig::is_empty)
}

fn merge_permission_section<T: PermissionSection>(current: &mut Option<T>, overlay: Option<T>) {
    let Some(overlay) = overlay else {
        return;
    };
    match current.as_mut() {
        Some(current) => current.merge_from(overlay),
        None => *current = Some(overlay),
    }
}

trait PermissionSection {
    fn merge_from(&mut self, overlay: Self);
}

impl PermissionSection for PathPermissionConfig {
    fn merge_from(&mut self, overlay: Self) {
        PathPermissionConfig::merge_from(self, overlay);
    }
}

impl PermissionSection for NetworkPermissionConfig {
    fn merge_from(&mut self, overlay: Self) {
        NetworkPermissionConfig::merge_from(self, overlay);
    }
}

impl PermissionSection for ToolPermissionConfig {
    fn merge_from(&mut self, overlay: Self) {
        ToolPermissionConfig::merge_from(self, overlay);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct PathPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathAccessModes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<PathAccessModes>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub rules: IndexMap<String, PathAccessRuleConfig>,
}

impl PathPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.workspace.is_none() && self.external.is_none() && self.rules.is_empty()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        if let Some(workspace) = overlay.workspace {
            match self.workspace.as_mut() {
                Some(current) => current.merge_from(workspace),
                None => self.workspace = Some(workspace),
            }
        }
        if let Some(external) = overlay.external {
            match self.external.as_mut() {
                Some(current) => current.merge_from(external),
                None => self.external = Some(external),
            }
        }
        self.rules.extend(overlay.rules);
    }

    pub fn apply_to_permission_policy(
        &self,
        mut base: PermissionPolicy,
    ) -> Result<PermissionPolicy, PermissionConfigError> {
        if let Some(workspace) = self.workspace.as_ref() {
            if let Some(mode) = workspace.read {
                base.workspace_read_default = mode;
            }
            if let Some(mode) = workspace.write {
                base.workspace_write_default = mode;
            }
        }
        if let Some(external) = self.external.as_ref() {
            if let Some(mode) = external.read {
                base.external_read_default = mode;
            }
            if let Some(mode) = external.write {
                base.external_write_default = mode;
            }
        }
        for (pattern, access) in &self.rules {
            let modes = access.to_modes()?;
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct PathAccessModes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<PermissionMode>,
}

impl PathAccessModes {
    pub fn merge_from(&mut self, overlay: Self) {
        if overlay.read.is_some() {
            self.read = overlay.read;
        }
        if overlay.write.is_some() {
            self.write = overlay.write;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathAccessRuleConfig {
    Modes(PathAccessModes),
    Shorthand(String),
}

impl PathAccessRuleConfig {
    fn to_modes(&self) -> Result<PathAccessModes, PermissionConfigError> {
        match self {
            Self::Modes(modes) => Ok(modes.clone()),
            Self::Shorthand(value) => path_access_shorthand(value),
        }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct NetworkPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internet: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loopback: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub rules: IndexMap<String, PermissionMode>,
}

impl NetworkPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.internet.is_none()
            && self.private.is_none()
            && self.loopback.is_none()
            && self.rules.is_empty()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        if overlay.internet.is_some() {
            self.internet = overlay.internet;
        }
        if overlay.private.is_some() {
            self.private = overlay.private;
        }
        if overlay.loopback.is_some() {
            self.loopback = overlay.loopback;
        }
        self.rules.extend(overlay.rules);
    }

    pub fn apply_to_network_permission_policy(
        &self,
        mut base: NetworkPermissionPolicy,
    ) -> Result<NetworkPermissionPolicy, PermissionConfigError> {
        if let Some(mode) = self.internet {
            base.internet_default = mode;
        }
        if let Some(mode) = self.private {
            base.private_default = mode;
        }
        if let Some(mode) = self.loopback {
            base.loopback_default = mode;
        }
        for (pattern, mode) in &self.rules {
            let trimmed = pattern.trim();
            if trimmed.is_empty() {
                continue;
            }
            base.add_rule(trimmed, *mode)?;
        }
        Ok(base)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ToolPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, PermissionMode>,
    #[serde(default, rename = "names", skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, PermissionMode>,
    #[serde(default, skip)]
    pub plugin: BTreeMap<String, PermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, ToolPermissionRules>,
}

impl ToolPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.default.is_none()
            && self.tags.is_empty()
            && self.names.is_empty()
            && self.plugin.is_empty()
            && self.rules.is_empty()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        if overlay.default.is_some() {
            self.default = overlay.default;
        }
        self.tags.extend(overlay.tags);
        self.names.extend(overlay.names);
        self.plugin.extend(overlay.plugin);
        self.rules.extend(overlay.rules);
    }

    pub fn apply_to_tool_permission_policy(
        &self,
        mut base: ToolPermissionPolicy,
    ) -> Result<ToolPermissionPolicy, PermissionConfigError> {
        if let Some(mode) = self.default {
            base.default_mode = mode;
        }
        for (tag, mode) in &self.tags {
            if let Some(tag) = ToolTag::from_tag(tag) {
                base.tag_modes.insert(tag.as_ref().to_string(), *mode);
            }
        }
        for (tool_name, mode) in self.names.iter().chain(self.plugin.iter()) {
            let name = tool_name.trim();
            if name.is_empty() {
                continue;
            }
            base.tool_modes.insert(name.to_string(), *mode);
        }
        for (tool_name, rules) in &self.rules {
            let name = tool_name.trim();
            if name.is_empty() {
                continue;
            }
            base = apply_tool_permission_rules(base, name, rules)?;
        }
        Ok(base)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolPermissionRules {
    Mode(PermissionMode),
    Ordered(IndexMap<String, PermissionMode>),
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
            if matches!(tool_name, "bash" | "shell" | "agena.process.run") {
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
pub struct Agent {
    pub name: String,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub disable: bool,
    pub permission_policy: PermissionPolicy,
    pub network_policy: NetworkPermissionPolicy,
    pub tool_policy: ToolPermissionPolicy,
    allowed_tool_names: Option<HashSet<String>>,
    permission_ceiling_policy: Option<PermissionPolicy>,
    network_ceiling_policy: Option<NetworkPermissionPolicy>,
    tool_ceiling_policy: Option<ToolPermissionPolicy>,
}

impl Agent {
    pub fn new(
        name: impl Into<String>,
        permission_policy: PermissionPolicy,
        tool_policy: ToolPermissionPolicy,
    ) -> Self {
        let name = name.into();
        Self {
            description: None,
            prompt: None,
            disable: false,
            name,
            permission_policy,
            network_policy: NetworkPermissionPolicy::allow_all(),
            tool_policy,
            allowed_tool_names: None,
            permission_ceiling_policy: None,
            network_ceiling_policy: None,
            tool_ceiling_policy: None,
        }
    }

    pub fn restricted_to_allowed_tools<I, S>(mut self, allowed_tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.allowed_tool_names = Some(
            allowed_tools
                .into_iter()
                .map(|name| name.as_ref().trim().to_string())
                .filter(|name| !name.is_empty())
                .collect(),
        );
        self
    }

    fn tool_is_in_allowlist(&self, names: &[&str]) -> bool {
        let Some(allowed) = self.allowed_tool_names.as_ref() else {
            return true;
        };
        if names.iter().any(|name| allowed.contains(*name)) {
            return true;
        }
        let allow_gateway = allowed.iter().any(|name| {
            !matches!(
                name.as_str(),
                "__agena_compaction_no_tools__" | "__agena_no_tools__"
            )
        });
        allow_gateway && names.iter().any(|name| is_model_tools_gateway_name(name))
    }

    pub fn try_apply_permission_config(
        mut self,
        config: &PermissionConfig,
    ) -> Result<Self, PermissionConfigError> {
        if config.is_empty() {
            return Ok(self);
        }
        self.permission_policy =
            config.apply_to_permission_policy(self.permission_policy.clone())?;
        self.network_policy =
            config.apply_to_network_permission_policy(self.network_policy.clone())?;
        self.tool_policy = config.apply_to_tool_permission_policy(self.tool_policy.clone())?;
        Ok(self)
    }

    pub fn apply_permission_config_or_self(self, config: &PermissionConfig) -> Self {
        match self.clone().try_apply_permission_config(config) {
            Ok(agent) => agent,
            Err(err) => {
                tracing::error!(
                    target: "agena::agent",
                    agent = %self.name,
                    "refusing invalid agent permission config at runtime: {err}"
                );
                // Effective permissions are a security boundary. Persisted or
                // imported runtime state can still reach this layer even when
                // normal configuration validation was bypassed, so malformed
                // policy must never fall back to the base agent's privileges.
                let mut denied = self;
                denied.disable = true;
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
        self.permission_ceiling_policy =
            Some(config.apply_to_permission_policy(PermissionPolicy::allow_all())?);
        self.network_ceiling_policy =
            Some(config.apply_to_network_permission_policy(NetworkPermissionPolicy::allow_all())?);
        self.tool_ceiling_policy =
            Some(config.apply_to_tool_permission_policy(ToolPermissionPolicy::allow_all())?);
        Ok(self)
    }

    pub fn apply_permission_ceiling_or_self(self, config: &PermissionConfig) -> Self {
        match self.clone().try_apply_permission_ceiling(config) {
            Ok(agent) => agent,
            Err(err) => {
                tracing::error!(
                    target: "agena::agent",
                    agent = %self.name,
                    "refusing invalid permission ceiling at runtime: {err}"
                );
                // Invalid boundaries must fail closed rather than silently
                // granting the child the unrestricted base agent.
                let mut denied = self;
                denied.disable = true;
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
        if self.disable || !self.tool_is_in_allowlist(&[tool_name]) {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' cannot access tool '{tool_name}'", self.name),
            };
        }
        let decision = self.tool_policy.check_tool(tool_name, command, tags);
        match self.tool_ceiling_policy.as_ref() {
            Some(ceiling) => {
                restrictive_decision(decision, ceiling.check_tool(tool_name, command, tags))
            }
            None => decision,
        }
    }

    pub fn authorize_tool_names(
        &self,
        tool_names: &[&str],
        command: Option<&str>,
        tags: &[ToolTag],
    ) -> PermissionDecision {
        if self.disable || !self.tool_is_in_allowlist(tool_names) {
            return PermissionDecision::Deny {
                reason: format!(
                    "agent '{}' cannot access tool '{}'",
                    self.name,
                    tool_names.first().copied().unwrap_or("tool")
                ),
            };
        }
        let decision = self
            .tool_policy
            .check_tool_with_names(tool_names, command, tags);
        match self.tool_ceiling_policy.as_ref() {
            Some(ceiling) => restrictive_decision(
                decision,
                ceiling.check_tool_with_names(tool_names, command, tags),
            ),
            None => decision,
        }
    }

    pub fn authorize_tool_name(&self, tool_name: &str) -> PermissionDecision {
        self.authorize_tool_tags(tool_name, &[])
    }

    pub fn authorize_tool_tags(&self, tool_name: &str, tags: &[ToolTag]) -> PermissionDecision {
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
            };
        }
        self.authorize_tool(tool_name, None, tags)
    }

    pub fn authorize_network_connect(&self, target: &NetworkTarget) -> PermissionDecision {
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
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
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
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
        (PermissionDecision::Allow, PermissionDecision::Allow) => PermissionDecision::Allow,
    }
}

fn is_model_tools_gateway_name(name: &str) -> bool {
    matches!(
        name,
        "tools_list"
            | "tools_search"
            | "tools_help"
            | "tools_tags"
            | "tools_call"
            | "agena.tools.list"
            | "agena.tools.search"
            | "agena.tools.help"
            | "agena.tools.tags"
            | "agena.tools.call"
    )
}

#[cfg(test)]
mod permission_ceiling_tests {
    use super::*;

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
        let agent = Agent::new(
            "child",
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&child)
        .expect("child policy")
        .try_apply_permission_ceiling(&parent)
        .expect("parent ceiling");

        assert!(matches!(
            agent.authorize_tool_name("agena.tasks.run"),
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
        let agent = Agent::new(
            "invalid",
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .apply_permission_config_or_self(&invalid);

        assert!(matches!(
            agent.authorize_tool_name("agena.fs.read"),
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
        let agent = Agent::new(
            "child",
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&child)
        .expect("child policy")
        .try_apply_permission_ceiling(&parent)
        .expect("parent ceiling");
        let workspace = std::path::Path::new("/workspace");

        assert!(matches!(
            agent.authorize_path_access(
                AccessKind::Write,
                workspace,
                &workspace.join("secret/file.txt"),
            ),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn empty_intersection_sentinel_hides_gateway_tools() {
        let agent = Agent::new(
            "child",
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .restricted_to_allowed_tools(["__agena_no_tools__"]);

        assert!(matches!(
            agent.authorize_tool_name("tools_search"),
            PermissionDecision::Deny { .. }
        ));
    }
}

#[derive(Debug, Error)]
pub enum AgentPolicyError {
    #[error("agent '{agent_name}' cannot use tool '{tool_name}': {reason}")]
    ToolDenied {
        agent_name: String,
        tool_name: String,
        reason: String,
    },
    #[error("agent '{agent_name}' has invalid permission config: {reason}")]
    InvalidPermissionConfig { agent_name: String, reason: String },
}
