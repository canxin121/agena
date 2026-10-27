//! External MCP connection management contracts shared by clients and plugins.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpConnectionScope {
    Global,
    #[default]
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpConnectionSummary {
    pub id: String,
    pub name: String,
    pub scope: McpConnectionScope,
    pub transport: String,
    pub enabled: bool,
    pub effective: bool,
    pub state: String,
    pub tool_count: usize,
    pub revision: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpConnectionDetail {
    #[serde(flatten)]
    pub summary: McpConnectionSummary,
    pub config: Value,
    pub secret_paths: Vec<String>,
    pub protocol_version: Option<String>,
    pub server_info: Option<Value>,
    pub capabilities: Option<Value>,
    pub oauth_health: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpConnectionCatalog {
    pub connections: Vec<McpConnectionSummary>,
    pub workspace_root: String,
    pub global_path: String,
    pub workspace_path: String,
    pub bridge_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpMutationResult {
    pub saved: bool,
    pub connection: Option<McpConnectionDetail>,
    pub warning: Option<String>,
    pub reload_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpTestResult {
    pub ok: bool,
    pub elapsed_ms: u64,
    pub tool_count: usize,
    pub protocol_version: Option<String>,
    pub server_info: Option<Value>,
    pub capabilities: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpCapabilityPage {
    pub kind: String,
    pub items: Vec<Value>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpImportCandidate {
    pub name: String,
    pub config: Value,
    pub secret_paths: Vec<String>,
    pub valid: bool,
    pub errors: Vec<String>,
    pub conflict: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpImportPreview {
    pub revision: String,
    pub candidates: Vec<McpImportCandidate>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpImportResult {
    pub installed: Vec<McpConnectionSummary>,
    pub warning: Option<String>,
    pub reload_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpConnectionCreateRequest {
    pub name: String,
    #[serde(default)]
    pub scope: McpConnectionScope,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpConnectionUpdateRequest {
    pub config: Value,
    #[serde(default)]
    pub preserve_secrets: Vec<String>,
    pub expected_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpConnectionEnabledRequest {
    pub enabled: bool,
    pub expected_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpBridgeEnabledRequest {
    pub enabled: bool,
    #[serde(default)]
    pub scope: McpConnectionScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpImportPreviewRequest {
    #[serde(default)]
    pub scope: McpConnectionScope,
    pub document: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpImportRequest {
    #[serde(default)]
    pub scope: McpConnectionScope,
    pub document: Value,
    pub selected: Vec<String>,
    #[serde(default)]
    pub replace: bool,
    pub expected_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpCredentialRequest {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthStartRequest {
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthCompleteRequest {
    pub flow_id: String,
    pub callback_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpOAuthStart {
    pub flow_id: String,
    pub authorization_url: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpResourceReadRequest {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpPromptGetRequest {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Map<String, Value>,
}
