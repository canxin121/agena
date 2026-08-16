#[derive(Debug, Clone, Serialize)]
/// Inspection details of a loaded plugin.
pub struct PluginInspectResponse {
    pub plugin: agena_plugin_host::PluginInspect,
}

#[derive(Debug, Clone, Serialize)]
/// Plugin UI catalog with the tool registry generation it reflects.
pub struct PluginUiCatalogResponse {
    pub catalog: agena_plugin_host::PluginUiCatalog,
    /// Registered tools available to permission-policy editors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_tools: Vec<PermissionToolCatalogResource>,
    /// Recent plugin notification intents, newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notifications: Vec<agena_plugin_host::HostNotification>,
    /// Built-in and plugin-contributed transcript activity kinds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity_kinds: Vec<agena_domain::ActivityKind>,
    pub tool_registry_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_registry_last_event:
        Option<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Presentation-neutral registered tool metadata used by permission UIs.
pub struct PermissionToolCatalogResource {
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
/// Context of a plugin UI request.
pub struct PluginUiRequestContext {
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub session_id: Option<i64>,
    /// Slash spelling and raw argument text are presentation context exposed
    /// to command hooks. They never participate in authorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    #[serde(default)]
    pub raw: String,
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
