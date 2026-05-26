use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct PluginInspectResponse {
    pub plugin: agena::plugin::PluginInspect,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginUiCatalogResponse {
    pub catalog: agena::plugin::PluginUiCatalog,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginUiRequestContext {
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub session_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginUiInvokeToolRequest {
    pub tool: String,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(flatten)]
    pub context: PluginUiRequestContext,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginLogListResponse {
    pub plugin_id: String,
    pub logs: Vec<agena::plugin::PluginLogRecord>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PluginLogListQuery {
    #[serde(default)]
    pub after_seq: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
}
