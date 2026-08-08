#[derive(Debug, Clone, Serialize)]
/// Summary of one provider adapter.
pub struct ProviderAdapterSummaryResource {
    pub adapter_id: String,
    pub enabled: bool,
    pub configured_model_count: usize,
}

#[derive(Debug, Clone, Serialize)]
/// Default adapter and model of a provider.
pub struct ProviderDefaultsResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
/// Summary of a provider and its adapters.
pub struct ProviderSummaryResource {
    pub provider_id: String,
    pub defaults: ProviderDefaultsResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapters: Vec<ProviderAdapterSummaryResource>,
}

#[derive(Debug, Clone, Serialize)]
/// Models of a provider.
pub struct ProviderModelsResponse {
    pub provider_id: String,
    pub models: Vec<ProviderModel>,
}
use super::Serialize;
use agena_domain::Model as ProviderModel;
