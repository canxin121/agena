#[derive(Debug, Clone, Serialize)]
/// Inspection details of a loaded plugin.
pub struct PluginInspectResponse {
    pub plugin: agena_plugin_host::PluginInspect,
}

#[derive(Debug, Clone, Serialize)]
/// Plugin UI catalog with the tool registry generation it reflects.
pub struct PluginUiCatalogResponse {
    pub catalog: agena_plugin_host::PluginUiCatalog,
    pub tool_registry_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_registry_last_event:
        Option<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Deserialize)]
/// Context of a plugin UI request.
pub struct PluginUiRequestContext {
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub session_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to invoke a tool from plugin UI.
pub struct PluginUiInvokeToolRequest {
    pub tool: String,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(flatten)]
    pub context: PluginUiRequestContext,
}

#[derive(Debug, Clone, Serialize)]
/// Log records of a plugin.
pub struct PluginLogListResponse {
    pub plugin_id: String,
    pub logs: Vec<agena_plugin_host::PluginLogRecord>,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for listing plugin log records.
pub struct PluginLogListQuery {
    #[serde(default)]
    pub after_seq: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
}
use super::{Deserialize, Serialize};
