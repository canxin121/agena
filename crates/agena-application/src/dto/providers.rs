#[derive(Debug, Clone, Serialize)]
pub struct ProviderAdapterSummaryResource {
    pub adapter_id: String,
    pub enabled: bool,
    pub configured_model_count: usize,
}

/// Transition-only DTO retained for internal callers compiled against the old
/// catalog shape. It is no longer serialized from provider summaries.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderNativeToolBindingResource {
    pub tool: String,
    pub route: String,
}

/// Transition-only DTO retained for source compatibility.
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
    /// Provider service tools are ordinary plugins and are not part of a model
    /// provider summary. Kept only so old application constructors compile.
    #[serde(skip)]
    pub provider_native_tools: Option<ProviderNativeToolsSummaryResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderModelsResponse {
    pub provider_id: String,
    pub models: Vec<ProviderModel>,
}
use super::Serialize;
use agena_domain::Model as ProviderModel;
