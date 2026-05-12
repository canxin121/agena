use std::{collections::BTreeMap, path::Path};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::message::FirstPartyToolInput;
use crate::permission::{
    AccessKind, AccessSelector, ExecutionMode, PermissionConfigError, PermissionDecision,
    PermissionMode, PermissionPolicy, PermissionRule, ToolPermissionPolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AgentMode {
    #[default]
    Primary,
    Subagent,
    All,
}

impl AgentMode {
    pub const fn is_primary(&self) -> bool {
        matches!(self, Self::Primary)
    }

    pub const fn allows_root(self) -> bool {
        matches!(self, Self::Primary | Self::All)
    }

    pub const fn allows_subagent(self) -> bool {
        matches!(self, Self::Subagent | Self::All)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentTemperature(pub f32);

impl Eq for AgentTemperature {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentRunConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<AgentTemperature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
}

impl AgentRunConfig {
    pub fn is_empty(&self) -> bool {
        self.temperature.is_none() && self.max_output_tokens.is_none() && self.steps.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_read: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_write: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_external_directory: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<ExecutionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<AgentPermissionRules>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<AgentPermissionRules>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_directory: Option<AgentPermissionRules>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_rules: BTreeMap<String, AgentPermissionRules>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bash_rules: Vec<AgentBashRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bash_deny_patterns: Vec<String>,
}

impl AgentPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.default_read.is_none()
            && self.default_write.is_none()
            && self.default_external_directory.is_none()
            && self.execution_mode.is_none()
            && self.tools.is_empty()
            && self.read.is_none()
            && self.write.is_none()
            && self.external_directory.is_none()
            && self.tool_rules.is_empty()
            && self.bash_rules.is_empty()
            && self.bash_deny_patterns.is_empty()
    }

    pub fn merged_with(&self, overlay: &Self) -> Self {
        let mut merged = self.clone();
        if let Some(mode) = overlay.default_read {
            merged.default_read = Some(mode);
        }
        if let Some(mode) = overlay.default_write {
            merged.default_write = Some(mode);
        }
        if let Some(mode) = overlay.default_external_directory {
            merged.default_external_directory = Some(mode);
        }
        if let Some(mode) = overlay.execution_mode {
            merged.execution_mode = Some(mode);
        }
        merged.tools.extend(overlay.tools.clone());
        if let Some(rules) = overlay.read.as_ref() {
            merged.read = Some(rules.clone());
        }
        if let Some(rules) = overlay.write.as_ref() {
            merged.write = Some(rules.clone());
        }
        if let Some(rules) = overlay.external_directory.as_ref() {
            merged.external_directory = Some(rules.clone());
        }
        merged.tool_rules.extend(overlay.tool_rules.clone());
        if !overlay.bash_rules.is_empty() {
            merged.bash_rules = overlay.bash_rules.clone();
        }
        if !overlay.bash_deny_patterns.is_empty() {
            merged.bash_deny_patterns = overlay.bash_deny_patterns.clone();
        }
        merged
    }

    pub fn apply_to_permission_policy(
        &self,
        mut base: PermissionPolicy,
    ) -> Result<PermissionPolicy, PermissionConfigError> {
        if let Some(mode) = self.default_read {
            base = base.with_default_read(mode);
        }
        if let Some(mode) = self.default_write {
            base = base.with_default_write(mode);
        }
        if let Some(mode) = self.default_external_directory {
            base = base.with_external_directory_default(mode);
        }
        if let Some(rules) = self.read.as_ref() {
            base = apply_path_rules(base, AccessSelector::Read, rules)?;
        }
        if let Some(rules) = self.write.as_ref() {
            base = apply_path_rules(base, AccessSelector::Write, rules)?;
        }
        if let Some(rules) = self.external_directory.as_ref() {
            base = apply_path_rules(base, AccessSelector::ExternalDirectory, rules)?;
        }
        Ok(base)
    }

    pub fn apply_to_tool_permission_policy(
        &self,
        mut base: ToolPermissionPolicy,
    ) -> Result<ToolPermissionPolicy, PermissionConfigError> {
        if let Some(mode) = self.execution_mode {
            base = base.with_execution_mode(mode);
        }
        for (tool_name, mode) in &self.tools {
            let name = tool_name.trim();
            if name.is_empty() {
                continue;
            }
            base = base.with_tool_mode(name.to_string(), *mode);
        }
        for (tool_name, rules) in &self.tool_rules {
            let name = tool_name.trim();
            if name.is_empty() {
                continue;
            }
            base = apply_tool_rules(base, name, rules)?;
        }
        for rule in &self.bash_rules {
            base = base.with_bash_pattern_rule(rule.pattern.clone(), rule.mode)?;
        }
        for pattern in &self.bash_deny_patterns {
            base = base.with_bash_deny_pattern(pattern.clone())?;
        }
        Ok(base)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentBashRule {
    pub pattern: String,
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentPermissionRules {
    Mode(PermissionMode),
    Ordered(IndexMap<String, PermissionMode>),
}

impl AgentPermissionRules {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Mode(_) => false,
            Self::Ordered(map) => map.is_empty(),
        }
    }
}

fn apply_path_rules(
    mut base: PermissionPolicy,
    selector: AccessSelector,
    rules: &AgentPermissionRules,
) -> Result<PermissionPolicy, PermissionConfigError> {
    match rules {
        AgentPermissionRules::Mode(mode) => {
            let rule = match selector {
                AccessSelector::Read | AccessSelector::Write => {
                    PermissionRule::path_wildcard(selector, *mode, "*")
                }
                AccessSelector::ExternalDirectory => PermissionRule::external_only(selector, *mode),
                AccessSelector::Any => PermissionRule::path_wildcard(selector, *mode, "*"),
            };
            Ok(base.with_rule(rule))
        }
        AgentPermissionRules::Ordered(entries) => {
            for (pattern, mode) in sorted_rule_entries(entries) {
                let trimmed = pattern.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let rule = match selector {
                    AccessSelector::ExternalDirectory if trimmed == "*" => {
                        PermissionRule::external_only(selector, mode)
                    }
                    _ => PermissionRule::path_wildcard(selector, mode, trimmed),
                };
                base = base.with_rule(rule);
            }
            Ok(base)
        }
    }
}

fn apply_tool_rules(
    mut base: ToolPermissionPolicy,
    tool_name: &str,
    rules: &AgentPermissionRules,
) -> Result<ToolPermissionPolicy, PermissionConfigError> {
    match rules {
        AgentPermissionRules::Mode(mode) => {
            if tool_name == "bash" {
                Ok(base.with_tool_mode(tool_name.to_string(), *mode))
            } else {
                Ok(base.with_tool_mode(tool_name.to_string(), *mode))
            }
        }
        AgentPermissionRules::Ordered(entries) => {
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
    pub mode: AgentMode,
    pub prompt: Option<String>,
    pub disable: bool,
    pub permission_policy: PermissionPolicy,
    pub tool_policy: ToolPermissionPolicy,
}

impl Agent {
    pub fn new(name: impl Into<String>, permission_policy: PermissionPolicy) -> Self {
        let name = name.into();
        Self {
            description: None,
            mode: AgentMode::Primary,
            prompt: None,
            disable: false,
            name,
            permission_policy,
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
        let mut tool_policy = ToolPermissionPolicy::new(PermissionMode::Deny)
            .with_execution_mode(current_policy.execution_mode());
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
        for rule in current_policy.bash_rules() {
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
        config: &AgentPermissionConfig,
    ) -> Result<Self, PermissionConfigError> {
        if config.is_empty() {
            return Ok(self);
        }
        self.permission_policy =
            config.apply_to_permission_policy(self.permission_policy.clone())?;
        self.tool_policy = config.apply_to_tool_permission_policy(self.tool_policy.clone())?;
        Ok(self)
    }

    pub fn with_permission_config(self, config: &AgentPermissionConfig) -> Self {
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

    pub fn authorize_first_party_tool(&self, input: &FirstPartyToolInput) -> PermissionDecision {
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
            };
        }
        self.tool_policy.check_first_party(input)
    }

    pub fn authorize_tool_name(&self, tool_name: &str) -> PermissionDecision {
        self.authorize_tool_call(tool_name, false)
    }

    pub fn authorize_tool_call(&self, tool_name: &str, sensitive: bool) -> PermissionDecision {
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
            };
        }
        self.tool_policy.check_tool(tool_name, None, sensitive)
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

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::Path};

    use crate::message::{BashToolInput, FirstPartyToolInput, ReadToolInput};
    use crate::permission::{AccessKind, PermissionDecision, PermissionMode, PermissionPolicy};
    use indexmap::IndexMap;

    use super::{Agent, AgentMode, AgentPermissionConfig, AgentPermissionRules};

    #[test]
    fn new_agent_has_reasonable_defaults() {
        let agent = Agent::new("build", PermissionPolicy::allow_all());

        assert_eq!(agent.name, "build");
        assert_eq!(agent.description, None);
        assert_eq!(agent.mode, AgentMode::Primary);
        assert_eq!(agent.prompt, None);
        assert!(!agent.disable);
    }

    #[test]
    fn agent_fields_can_be_set_directly() {
        let mut agent = Agent::new("explore", PermissionPolicy::allow_all());
        agent.description = Some("Read-only explorer".to_string());
        agent.mode = AgentMode::Subagent;
        agent.prompt = Some("You are a focused exploration agent.".to_string());
        agent.disable = true;

        assert_eq!(agent.description.as_deref(), Some("Read-only explorer"));
        assert_eq!(agent.mode, AgentMode::Subagent);
        assert_eq!(
            agent.prompt.as_deref(),
            Some("You are a focused exploration agent.")
        );
        assert!(agent.disable);
    }

    #[test]
    fn disabled_agent_denies_first_party_tools() {
        let mut agent = Agent::new("build", PermissionPolicy::allow_all());
        agent.disable = true;
        let input = FirstPartyToolInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });

        match agent.authorize_first_party_tool(&input) {
            crate::permission::PermissionDecision::Deny { reason } => {
                assert!(reason.contains("disabled"));
            }
            other => panic!("expected deny decision for disabled agent, got {other:?}"),
        }
    }

    #[test]
    fn disabled_agent_denies_path_access() {
        let mut agent = Agent::new("build", PermissionPolicy::allow_all());
        agent.disable = true;

        match agent.authorize_path_access(AccessKind::Read, Path::new("."), Path::new("README.md"))
        {
            crate::permission::PermissionDecision::Deny { reason } => {
                assert!(reason.contains("disabled"));
            }
            other => panic!("expected deny decision for disabled agent, got {other:?}"),
        }
    }

    #[test]
    fn agent_permission_config_overrides_tool_and_path_policy() {
        let agent = Agent::new("planner", PermissionPolicy::allow_all())
            .try_with_permission_config(&AgentPermissionConfig {
                default_read: Some(PermissionMode::Allow),
                default_write: Some(PermissionMode::Deny),
                default_external_directory: Some(PermissionMode::Ask),
                execution_mode: Some(crate::permission::ExecutionMode::Ask),
                tools: BTreeMap::from([("bash".to_string(), PermissionMode::Ask)]),
                ..AgentPermissionConfig::default()
            })
            .expect("permission config compiles");

        match agent.authorize_first_party_tool(&FirstPartyToolInput::Bash(BashToolInput {
            command: "git status".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        })) {
            PermissionDecision::Ask { .. } => {}
            other => panic!("expected ask for bash, got {other:?}"),
        }

        match agent.authorize_path_access(
            AccessKind::Write,
            Path::new("/workspace"),
            Path::new("/workspace/file.txt"),
        ) {
            PermissionDecision::Deny { .. } => {}
            other => panic!("expected deny for write, got {other:?}"),
        }
    }

    #[test]
    fn ordered_path_rules_use_last_match_wins() {
        let agent = Agent::new("explore", PermissionPolicy::allow_all())
            .try_with_permission_config(&AgentPermissionConfig {
                read: Some(AgentPermissionRules::Ordered(IndexMap::from([
                    ("*".to_string(), PermissionMode::Allow),
                    ("*.env".to_string(), PermissionMode::Ask),
                ]))),
                ..AgentPermissionConfig::default()
            })
            .expect("permission config compiles");

        match agent.authorize_path_access(
            AccessKind::Read,
            Path::new("/workspace/repo"),
            Path::new("local.env"),
        ) {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("*.env"));
            }
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    #[test]
    fn external_directory_rules_can_allow_specific_absolute_path() {
        let agent = Agent::new("plan", PermissionPolicy::allow_all())
            .try_with_permission_config(&AgentPermissionConfig {
                default_external_directory: Some(PermissionMode::Deny),
                external_directory: Some(AgentPermissionRules::Ordered(IndexMap::from([
                    ("*".to_string(), PermissionMode::Ask),
                    ("/tmp/allowed/**".to_string(), PermissionMode::Allow),
                ]))),
                ..AgentPermissionConfig::default()
            })
            .expect("permission config compiles");

        assert_eq!(
            agent.authorize_path_access(
                AccessKind::Read,
                Path::new("/workspace/repo"),
                Path::new("/tmp/allowed/file.txt"),
            ),
            PermissionDecision::Allow
        );

        match agent.authorize_path_access(
            AccessKind::Read,
            Path::new("/workspace/repo"),
            Path::new("/tmp/elsewhere/file.txt"),
        ) {
            PermissionDecision::Ask { .. } => {}
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    #[test]
    fn bash_overlay_rules_survive_allowed_tool_filtering() {
        let agent = Agent::new("planner", PermissionPolicy::allow_all())
            .try_with_permission_config(&AgentPermissionConfig {
                tool_rules: BTreeMap::from([(
                    "bash".to_string(),
                    AgentPermissionRules::Ordered(IndexMap::from([
                        ("git push *".to_string(), PermissionMode::Deny),
                        ("git *".to_string(), PermissionMode::Allow),
                        ("*".to_string(), PermissionMode::Ask),
                    ])),
                )]),
                ..AgentPermissionConfig::default()
            })
            .expect("permission config compiles")
            .with_allowed_tools(["bash"]);

        assert_eq!(
            agent.authorize_first_party_tool(&FirstPartyToolInput::Bash(BashToolInput {
                command: "git status".to_string(),
                description: String::new(),
                timeout_ms: None,
                workdir: None,
            })),
            PermissionDecision::Allow
        );

        match agent.authorize_first_party_tool(&FirstPartyToolInput::Bash(BashToolInput {
            command: "git push origin main".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        })) {
            PermissionDecision::Deny { .. } => {}
            other => panic!("expected deny decision, got {other:?}"),
        }
    }
}
