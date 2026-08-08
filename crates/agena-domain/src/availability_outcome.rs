use serde::{Deserialize, Serialize};

/// Authority that made an execution capability unavailable. These are hard
/// runtime boundaries, not user permission-policy decisions and therefore
/// cannot be changed by approving the invocation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySourceKind {
    AgentProfile,
    ExecutionAccess,
    ModelProfile,
    RuntimeConfiguration,
    Platform,
    Build,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Describes why a capability is unavailable and whether the request is retryable.
pub struct CapabilityUnavailableResult {
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub reason: String,
    pub source: CapabilitySourceKind,
    /// Whether changing runtime/profile configuration and retrying can make
    /// the capability available. Approval alone never changes this value.
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Describes why a tool is unavailable, with suggestions.
pub struct ToolUnavailableResult {
    pub tool_name: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    /// A stable machine-readable registration/load source.
    pub source: String,
    pub retryable: bool,
}
