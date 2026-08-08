use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of a config hook.
pub struct ConfigInput {
    pub current: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to plugin configuration by a hook.
pub struct ConfigPatch {
    /// Sparse object deep-merged into the current config.
    /// Only keys present are overwritten; absent keys are left unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<serde_json::Value>,
}
