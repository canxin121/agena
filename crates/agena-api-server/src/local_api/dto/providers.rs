#[derive(Debug, Clone, Serialize)]
pub struct ProviderAdapterSummaryResource {
    pub adapter_id: String,
    pub enabled: bool,
    pub configured_model_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderNativeToolBindingResource {
    pub tool: String,
    pub route: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderNativeToolsSummaryResource {
    pub active: bool,
    pub model_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<ProviderNativeToolBindingResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderDefaultsResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderSummaryResource {
    pub provider_id: String,
    pub defaults: ProviderDefaultsResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapters: Vec<ProviderAdapterSummaryResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_native_tools: Option<ProviderNativeToolsSummaryResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderModelsResponse {
    pub provider_id: String,
    pub models: Vec<ProviderModel>,
}
use super::{ProviderModel, Serialize};
