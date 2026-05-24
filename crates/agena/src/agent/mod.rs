use std::{collections::BTreeMap, path::Path};

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
                base = base.with_workspace_read_default(mode);
            }
            if let Some(mode) = workspace.write {
                base = base.with_workspace_write_default(mode);
            }
        }
        if let Some(external) = self.external.as_ref() {
            if let Some(mode) = external.read {
                base = base.with_external_read_default(mode);
            }
            if let Some(mode) = external.write {
                base = base.with_external_write_default(mode);
            }
        }
        for (pattern, access) in &self.rules {
            let modes = access.to_modes()?;
            let trimmed = pattern.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(mode) = modes.read {
                base = base.with_path_pattern_rule(AccessSelector::Read, mode, trimmed)?;
            }
            if let Some(mode) = modes.write {
                base = base.with_path_pattern_rule(AccessSelector::Write, mode, trimmed)?;
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
            base = base.with_internet_default(mode);
        }
        if let Some(mode) = self.private {
            base = base.with_private_default(mode);
        }
        if let Some(mode) = self.loopback {
            base = base.with_loopback_default(mode);
        }
        for (pattern, mode) in &self.rules {
            let trimmed = pattern.trim();
            if trimmed.is_empty() {
                continue;
            }
            base = base.with_rule(trimmed, *mode)?;
        }
        Ok(base)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ToolPermissionConfig {
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
        self.tags.is_empty()
            && self.names.is_empty()
            && self.plugin.is_empty()
            && self.rules.is_empty()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        self.tags.extend(overlay.tags);
        self.names.extend(overlay.names);
        self.plugin.extend(overlay.plugin);
        self.rules.extend(overlay.rules);
    }

    pub fn apply_to_tool_permission_policy(
        &self,
        mut base: ToolPermissionPolicy,
    ) -> Result<ToolPermissionPolicy, PermissionConfigError> {
        for (tag, mode) in &self.tags {
            if let Some(tag) = ToolTag::from_tag(tag) {
                base = base.with_tag_mode(tag, *mode);
            }
        }
        for (tool_name, mode) in self.names.iter().chain(self.plugin.iter()) {
            let name = tool_name.trim();
            if name.is_empty() {
                continue;
            }
            base = base.with_tool_mode(name.to_string(), *mode);
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
        ToolPermissionRules::Mode(mode) => Ok(base.with_tool_mode(tool_name.to_string(), *mode)),
        ToolPermissionRules::Ordered(entries) => {
            if tool_name == "bash" {
                for (pattern, mode) in sorted_rule_entries(entries) {
                    let trimmed = pattern.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == "*" {
                        base = base.with_tool_mode(tool_name.to_string(), mode);
                    } else {
                        base = base.with_bash_overlay_rule(trimmed, mode);
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
                    base = base.with_tool_mode(tool_name.to_string(), mode);
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
}

impl Agent {
    pub fn new(name: impl Into<String>, permission_policy: PermissionPolicy) -> Self {
        let name = name.into();
        Self {
            description: None,
            prompt: None,
            disable: false,
            name,
            permission_policy,
            network_policy: NetworkPermissionPolicy::allow_all(),
            tool_policy: ToolPermissionPolicy::allow_all(),
        }
    }

    pub fn with_tool_policy(mut self, tool_policy: ToolPermissionPolicy) -> Self {
        self.tool_policy = tool_policy;
        self
    }

    pub fn with_allowed_tools<I, S>(mut self, allowed_tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let current_policy = self.tool_policy.clone();
        let mut tool_policy = ToolPermissionPolicy::new(PermissionMode::Deny);
        for tool_name in allowed_tools {
            let name = tool_name.as_ref().trim();
            if name.is_empty() {
                continue;
            }
            let mode = current_policy
                .check_tool_name(name)
                .into_mode()
                .unwrap_or(PermissionMode::Allow);
            tool_policy = tool_policy.with_tool_mode(name.to_string(), mode);
        }
        for rule in current_policy.bash_deny_rules() {
            tool_policy = tool_policy
                .with_bash_deny_pattern(rule.pattern().to_string())
                .expect("existing bash deny pattern should remain valid");
        }
        for rule in current_policy.bash_pattern_rules() {
            tool_policy = tool_policy
                .with_bash_pattern_rule(rule.pattern().to_string(), rule.mode())
                .expect("existing bash rule should remain valid");
        }
        for rule in current_policy.bash_overlay_rules() {
            tool_policy =
                tool_policy.with_bash_overlay_rule(rule.pattern().to_string(), rule.mode());
        }
        self.tool_policy = tool_policy;
        self
    }

    pub fn try_with_permission_config(
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

    pub fn with_permission_config(self, config: &PermissionConfig) -> Self {
        match self.clone().try_with_permission_config(config) {
            Ok(agent) => agent,
            Err(err) => {
                tracing::warn!(
                    target: "agena::agent",
                    agent = %self.name,
                    "ignoring invalid agent permission config at runtime: {err}"
                );
                self
            }
        }
    }

    pub fn authorize_tool(
        &self,
        tool_name: &str,
        command: Option<&str>,
        tags: &[ToolTag],
    ) -> PermissionDecision {
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
            };
        }
        self.tool_policy.check_tool(tool_name, command, tags)
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
        self.network_policy.check_connect(target)
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
        self.permission_policy
            .check_access(access, workspace_root, target_path)
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

trait PermissionDecisionExt {
    fn into_mode(self) -> Option<PermissionMode>;
}

impl PermissionDecisionExt for PermissionDecision {
    fn into_mode(self) -> Option<PermissionMode> {
        match self {
            PermissionDecision::Allow => Some(PermissionMode::Allow),
            PermissionDecision::Ask { .. } => Some(PermissionMode::Ask),
            PermissionDecision::Deny { .. } => Some(PermissionMode::Deny),
        }
    }
}
