use std::collections::{BTreeMap, BTreeSet};

use agena_domain::{ModelLifecycle, ModelPricing};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    CatalogModelRecord, ConfiguredModelDefinition, ConfiguredModelModeMap,
    ConfiguredModelSpeedMode, ConfiguredModelThinkingMode, ModelCapabilityPatch,
    ModelCatalogSnapshotSourceKind, ProviderModelCatalog,
};

/// Stable provider catalog definition. The ranking sidecar is deliberately
/// excluded from serialization so persisted catalog data stays portable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CatalogModelDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_temperature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_p: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_interleaved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "ConfiguredModelModeMap::is_empty")]
    pub thinking_modes: ConfiguredModelModeMap<ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "ConfiguredModelModeMap::is_empty")]
    pub speed_modes: ConfiguredModelModeMap<ConfiguredModelSpeedMode>,
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatch,
    #[serde(skip, default)]
    pub source_priority: CatalogDefinitionSourcePriority,
}

impl CatalogModelDefinition {
    #[allow(clippy::too_many_arguments)]
    pub fn from_fields(
        lifecycle: Option<ModelLifecycle>,
        context_window_tokens: Option<u32>,
        max_input_tokens: Option<u32>,
        max_output_tokens: Option<u32>,
        description: Option<String>,
        knowledge_cutoff: Option<String>,
        release_date: Option<String>,
        last_updated: Option<String>,
        open_weights: Option<bool>,
        supports_parallel_tool_calls: Option<bool>,
        supports_verbosity: Option<bool>,
        default_verbosity: Option<String>,
        default_temperature: Option<String>,
        default_top_p: Option<String>,
        default_top_k: Option<u32>,
        assistant_reasoning_interleaved: Option<bool>,
        assistant_reasoning_field: Option<String>,
        output_modalities: Vec<String>,
        pricing: Option<ModelPricing>,
        display_name: Option<String>,
        origin: Option<String>,
        thinking_modes: ConfiguredModelModeMap<ConfiguredModelThinkingMode>,
        speed_modes: ConfiguredModelModeMap<ConfiguredModelSpeedMode>,
        capabilities: ModelCapabilityPatch,
    ) -> Self {
        Self {
            lifecycle,
            context_window_tokens,
            max_input_tokens,
            max_output_tokens,
            description,
            knowledge_cutoff,
            release_date,
            last_updated,
            open_weights,
            supports_parallel_tool_calls,
            supports_verbosity,
            default_verbosity,
            default_temperature,
            default_top_p,
            default_top_k,
            assistant_reasoning_interleaved,
            assistant_reasoning_field,
            output_modalities,
            pricing,
            display_name,
            origin,
            thinking_modes,
            speed_modes,
            capabilities,
            source_priority: CatalogDefinitionSourcePriority::default(),
        }
    }

    pub fn from_persisted_json(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value)
    }

    pub fn to_persisted_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }

    pub fn is_empty(&self) -> bool {
        self.lifecycle.is_none()
            && self.context_window_tokens.is_none()
            && self.max_input_tokens.is_none()
            && self.max_output_tokens.is_none()
            && self.description.is_none()
            && self.knowledge_cutoff.is_none()
            && self.release_date.is_none()
            && self.last_updated.is_none()
            && self.open_weights.is_none()
            && self.supports_parallel_tool_calls.is_none()
            && self.supports_verbosity.is_none()
            && self.default_verbosity.is_none()
            && self.default_temperature.is_none()
            && self.default_top_p.is_none()
            && self.default_top_k.is_none()
            && self.assistant_reasoning_interleaved.is_none()
            && self.assistant_reasoning_field.is_none()
            && self.output_modalities.is_empty()
            && self.pricing.is_none()
            && self.display_name.is_none()
            && self.origin.is_none()
            && self.thinking_modes.is_empty()
            && self.speed_modes.is_empty()
            && self.capabilities.is_empty()
    }

    pub fn into_configured_definition(self) -> ConfiguredModelDefinition {
        ConfiguredModelDefinition {
            lifecycle: self.lifecycle,
            context_window_tokens: self.context_window_tokens,
            max_input_tokens: self.max_input_tokens,
            max_output_tokens: self.max_output_tokens,
            display_name: self.display_name,
            description: self.description,
            knowledge_cutoff: self.knowledge_cutoff,
            release_date: self.release_date,
            last_updated: self.last_updated,
            open_weights: self.open_weights,
            supports_parallel_tool_calls: self.supports_parallel_tool_calls,
            supports_verbosity: self.supports_verbosity,
            default_verbosity: self.default_verbosity,
            default_temperature: self.default_temperature,
            default_top_p: self.default_top_p,
            default_top_k: self.default_top_k,
            assistant_reasoning_interleaved: self.assistant_reasoning_interleaved,
            assistant_reasoning_field: self.assistant_reasoning_field,
            output_modalities: self.output_modalities,
            pricing: self.pricing,
            thinking_modes: self.thinking_modes,
            speed_modes: self.speed_modes,
            capabilities: self.capabilities,
        }
    }
}

/// In-memory merge/curation sidecar. It is never serialized into catalog
/// records and is provider-contract data rather than a core runtime value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogDefinitionSourcePriority {
    pub sort_priority: i32,
    pub descriptive_priority: i32,
    pub limits_priority: i32,
    pub capability_priority: i32,
    pub semantics_priority: i32,
    pub pricing_priority: i32,
    pub mode_priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCatalogProviderRecord {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, CatalogModelDefinition>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub appendable_model_ids: BTreeSet<String>,
}

impl ModelCatalogProviderRecord {
    pub fn into_provider_catalog(self) -> ProviderModelCatalog {
        ProviderModelCatalog {
            models: self
                .models
                .into_iter()
                .map(|(id, definition)| (id, definition.into_configured_definition()))
                .collect(),
            appendable_model_ids: self.appendable_model_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ModelCatalogDocument {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, CatalogModelDefinition>,
}

impl ModelCatalogDocument {
    pub fn model_ids(&self) -> BTreeSet<String> {
        self.models.keys().cloned().collect()
    }
    pub fn model_record(&self) -> ModelCatalogProviderRecord {
        ModelCatalogProviderRecord {
            models: self.models.clone(),
            appendable_model_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCatalogSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_source: Option<ModelCatalogSnapshotSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default)]
    pub official: ModelCatalogDocument,
}

impl ModelCatalogSnapshot {
    pub fn merged_models(&self) -> ModelCatalogProviderRecord {
        self.official.model_record()
    }
    pub fn models(&self) -> Vec<CatalogModelRecord> {
        let mut models: Vec<_> = self
            .official
            .models
            .iter()
            .map(|(id, d)| Self::model_record(id, d))
            .collect();
        models.sort_by(|left, right| left.model_id.cmp(&right.model_id));
        models
    }
    pub fn model_record(model_id: &str, definition: &CatalogModelDefinition) -> CatalogModelRecord {
        CatalogModelRecord {
            model_id: model_id.to_owned(),
            display_name: definition.display_name.clone(),
            origin: definition.origin.clone(),
            lifecycle: definition.lifecycle,
            context_window_tokens: definition.context_window_tokens,
            max_input_tokens: definition.max_input_tokens,
            max_output_tokens: definition.max_output_tokens,
            description: definition.description.clone(),
            knowledge_cutoff: definition.knowledge_cutoff.clone(),
            release_date: definition.release_date.clone(),
            last_updated: definition.last_updated.clone(),
            open_weights: definition.open_weights,
            supports_parallel_tool_calls: definition.supports_parallel_tool_calls,
            supports_verbosity: definition.supports_verbosity,
            default_verbosity: definition.default_verbosity.clone(),
            default_temperature: definition.default_temperature.clone(),
            default_top_p: definition.default_top_p.clone(),
            default_top_k: definition.default_top_k,
            assistant_reasoning_interleaved: definition.assistant_reasoning_interleaved,
            assistant_reasoning_field: definition.assistant_reasoning_field.clone(),
            output_modalities: definition.output_modalities.clone(),
            pricing: definition.pricing.clone(),
            thinking_modes: definition.thinking_modes.clone(),
            speed_modes: definition.speed_modes.clone(),
            capabilities: definition.capabilities.clone(),
        }
    }
    pub fn model_ids(&self) -> Vec<String> {
        self.official.model_ids().into_iter().collect()
    }
    pub fn to_response(&self) -> ModelCatalogResponse {
        ModelCatalogResponse {
            last_refresh_at: self.last_refresh_at,
            last_successful_source: self.last_successful_source,
            last_error: self.last_error.clone(),
            models: self.models(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_source: Option<ModelCatalogSnapshotSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub models: Vec<CatalogModelRecord>,
}

#[cfg(test)]
mod tests {
    use super::{CatalogDefinitionSourcePriority, CatalogModelDefinition};
    #[test]
    fn persisted_definition_excludes_ranking_sidecar() {
        let definition = CatalogModelDefinition {
            source_priority: CatalogDefinitionSourcePriority {
                sort_priority: 42,
                ..Default::default()
            },
            ..Default::default()
        };
        let value = definition.to_persisted_json().expect("encode definition");
        let restored =
            CatalogModelDefinition::from_persisted_json(value.clone()).expect("decode definition");
        assert_eq!(
            restored.source_priority,
            CatalogDefinitionSourcePriority::default()
        );
        assert!(value.get("source_priority").is_none());
    }
}
