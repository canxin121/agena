use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionMode {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AgentPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<AgentPathPermissionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<AgentNetworkPermissionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<AgentToolPermissionConfig>,
}

impl AgentPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.path.is_none() && self.network.is_none() && self.tools.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AgentPathPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<AgentPathAccessModes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<AgentPathAccessModes>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub rules: IndexMap<String, AgentPathAccessRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AgentPathAccessModes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<AgentPermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<AgentPermissionMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AgentPathAccessRule {
    Modes(AgentPathAccessModes),
    Shorthand(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AgentNetworkPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internet: Option<AgentPermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<AgentPermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loopback: Option<AgentPermissionMode>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub rules: IndexMap<String, AgentPermissionMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AgentToolPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<AgentPermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, AgentPermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, AgentPermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugin: BTreeMap<String, AgentPermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, AgentToolPermissionRules>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AgentToolPermissionRules {
    Mode(AgentPermissionMode),
    Ordered(IndexMap<String, AgentPermissionMode>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostAgentSelectionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

impl HostAgentSelectionConfig {
    pub fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.adapter.is_none()
            && self.model.is_none()
            && self.thinking_mode.is_none()
            && self.speed_mode.is_none()
            && self.verbosity.is_none()
            && self.parallel_tool_calls.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAgentDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "AgentPermissionConfig::is_empty")]
    pub permission: AgentPermissionConfig,
    #[serde(default, skip_serializing_if = "HostAgentSelectionConfig::is_empty")]
    pub defaults: HostAgentSelectionConfig,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAgentRegisterRequest {
    pub agent: HostAgentDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAgentRemoveRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostAgentRemoveResponse {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostAgentListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<HostAgentDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAgentGetRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostAgentGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<HostAgentDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostAgentSwitchRequest {
    /// The target agent profile. When omitted or blank, the explicit runtime
    /// agent selection is cleared and the session falls back to its base
    /// model/system/tool context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Defaults to the callback session. Plugins may pass a session id for
    /// session-level orchestration outside a tool invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    /// Push the current agent before switching so a later restore can return
    /// to it.
    #[serde(default)]
    pub push_previous: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAgentSwitchResponse {
    pub session_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_agent: Option<String>,
    #[serde(default)]
    pub stack_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostAgentRestoreRequest {
    /// Defaults to the callback session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAgentRestoreResponse {
    pub session_id: i64,
    #[serde(default)]
    pub restored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_agent: Option<String>,
    #[serde(default)]
    pub stack_depth: usize,
}
