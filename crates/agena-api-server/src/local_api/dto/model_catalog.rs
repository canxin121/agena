use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalogResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_successful_source: Option<ModelCatalogEntrySourceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub entry_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalogListResponse {
    pub summary: ModelCatalogResponse,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_origins: Vec<String>,
    pub items: Vec<ModelCatalogEntryResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalogLookupResponse {
    pub items: Vec<ModelCatalogEntryResource>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogSourceKind {
    Generated,
    Cache,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalogEntryResource {
    pub model_id: String,
    pub source: ModelCatalogSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<agena::model::ModelLifecycle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_thinking_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_temperature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_top_p: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_interleaved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<agena::model::ModelPricing>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub thinking_modes:
        std::collections::BTreeMap<String, agena::provider::ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub speed_modes: std::collections::BTreeMap<String, agena::provider::ConfiguredModelSpeedMode>,
    #[serde(flatten)]
    pub capabilities: agena::provider::ModelCapabilityPatch,
}

impl From<ModelCatalogEntryRecord> for ModelCatalogEntryResource {
    fn from(value: ModelCatalogEntryRecord) -> Self {
        Self::from_record(value, None)
    }
}

impl ModelCatalogEntryResource {
    pub fn from_record(
        value: ModelCatalogEntryRecord,
        last_successful_source: Option<ModelCatalogEntrySourceKind>,
    ) -> Self {
        let source = match last_successful_source.unwrap_or(ModelCatalogEntrySourceKind::Generated) {
            ModelCatalogEntrySourceKind::Generated => ModelCatalogSourceKind::Generated,
            ModelCatalogEntrySourceKind::Cache => ModelCatalogSourceKind::Cache,
        };
        let source_label = Some(str::to_owned(match source {
            ModelCatalogSourceKind::Generated => "generated catalog",
            ModelCatalogSourceKind::Cache => "cached catalog",
        }));

        Self {
            model_id: value.model_id,
            source,
            source_label,
            display_name: value.display_name,
            origin: value.origin,
            lifecycle: value.lifecycle,
            context_window_tokens: value.context_window_tokens,
            max_input_tokens: value.max_input_tokens,
            max_output_tokens: value.max_output_tokens,
            description: value.description,
            knowledge_cutoff: value.knowledge_cutoff,
            release_date: value.release_date,
            last_updated: value.last_updated,
            open_weights: value.open_weights,
            default_thinking_mode: value.default_thinking_mode,
            supports_parallel_tool_calls: value.supports_parallel_tool_calls,
            supports_verbosity: value.supports_verbosity,
            default_verbosity: value.default_verbosity,
            default_temperature: value.default_temperature,
            default_top_p: value.default_top_p,
            default_top_k: value.default_top_k,
            assistant_reasoning_interleaved: value.assistant_reasoning_interleaved,
            assistant_reasoning_field: value.assistant_reasoning_field,
            output_modalities: value.output_modalities,
            pricing: value.pricing,
            thinking_modes: value.thinking_modes,
            speed_modes: value.speed_modes,
            capabilities: value.capabilities,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelCatalogLookupRequest {
    #[serde(default)]
    pub model_ids: Vec<String>,
}
