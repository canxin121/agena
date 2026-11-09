use agena_assistant::IntegrationSpec;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationAction {
    List,
    Status,
    Propose,
    Pause,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, agena_macros::ToolInput)]
#[serde(deny_unknown_fields)]
pub struct IntegrationRequest {
    pub action: IntegrationAction,
    /// Exact quotation from the current direct user, required for propose/pause.
    #[serde(default)]
    pub user_quote: Option<String>,
    #[serde(default)]
    pub binding_id: Option<String>,
    #[serde(default)]
    pub expected_revision: Option<i64>,
    #[serde(default)]
    pub spec: Option<IntegrationSpec>,
}
