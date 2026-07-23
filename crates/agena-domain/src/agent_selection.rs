use serde::{Deserialize, Serialize};

/// Optional provider/model-mode defaults carried by agent and provider profiles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSelectionConfig {
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

impl AgentSelectionConfig {
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

/// Optional exact model-facing tool allowlist for an agent profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentToolsConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
}

impl AgentToolsConfig {
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty()
    }
}
