#[derive(Debug, Clone, Serialize)]
/// Inspection details of a loaded plugin.
pub struct PluginInspectResponse {
    pub plugin: agena_plugin_host::PluginInspect,
}

fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Clone, Serialize)]
/// Neutral plugin surface with the tool registry generation it reflects.
pub struct PluginSurfaceCatalogResponse {
    pub catalog: agena_plugin_host::PluginSurfaceCatalog,
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
/// Context of a user-driven plugin operation/tool request.
pub struct PluginOperationRequestContext {
    #[serde(default = "empty_object")]
    pub input: serde_json::Value,
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
/// Request to invoke a registered plugin tool explicitly.
pub struct PluginToolInvokeRequest {
    pub tool: String,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(flatten)]
    pub context: PluginOperationRequestContext,
}

#[derive(Debug, Clone, Serialize)]
/// Complete settings state rendered by every plugin workbench client.
pub struct PluginSettingsResponse {
    pub plugin_id: String,
    pub contract: agena_plugin_host::sdk::SettingsContract,
    pub defaults: serde_json::Value,
    pub configured: serde_json::Value,
    pub effective: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PluginSettingsDiagnostic>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
/// One safe, field-addressed settings diagnostic.
pub struct PluginSettingsDiagnostic {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
/// Replace the complete plugin-owned configuration value.
pub struct PluginSettingsUpdateRequest {
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
/// Persisted settings and Runtime reload outcome.
pub struct PluginSettingsUpdateResponse {
    pub settings: PluginSettingsResponse,
    pub reload_required: bool,
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
