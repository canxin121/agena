//! Explicit projection between internal catalog records and the public API.
//! Public catalog DTOs are defined only in `agena-api`.

use agena_api::resource::*;
use agena_provider::{CatalogModelRecord, ModelCatalogSnapshotSourceKind};

pub(crate) fn catalog_model_resource_from_record(
    value: CatalogModelRecord,
    last_successful_source: Option<ModelCatalogSnapshotSourceKind>,
) -> CatalogModelResource {
    let source = match last_successful_source.unwrap_or(ModelCatalogSnapshotSourceKind::Generated) {
        ModelCatalogSnapshotSourceKind::Generated => ModelCatalogSourceKind::Generated,
        ModelCatalogSnapshotSourceKind::Cache => ModelCatalogSourceKind::Cache,
    };
    let source_label = Some(str::to_owned(match source {
        ModelCatalogSourceKind::Generated => "generated catalog",
        ModelCatalogSourceKind::Cache => "cached catalog",
    }));

    CatalogModelResource {
        model_id: value.model_id,
        provider_sources: value.provider_sources,
        verbosity_by_provider: value.verbosity_by_provider,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_projection_preserves_rest_shape_and_roundtrips_configuration() {
        let record: CatalogModelRecord = serde_json::from_value(json!({
            "model_id": "fixture-model",
            "provider_sources": ["fixture"],
            "verbosity_by_provider": { "fixture": { "supported": true, "levels": ["low", "high"], "provider_default": "low" } },
            "lifecycle": "preview",
            "pricing": {"input_usd_per_million_tokens": "0.123400"},
            "input": {"supported": ["text", "image"], "unsupported": ["audio"]},
            "features": ["tool_calling", "reasoning"],
            "thinking_modes": {
                "default": "custom",
                "custom": { "strategy": "adaptive", "effort": "high", "display": "summarized", "request_override": { "body_patch": { "nested": {"value": null} } } }
            },
            "speed_modes": {
                "default": null,
                "fast": { "request_override": { "headers": {"x-mode": "fast"} }, "disabled": true }
            }
        })).unwrap();
        let resource = catalog_model_resource_from_record(
            record.clone(),
            Some(ModelCatalogSnapshotSourceKind::Cache),
        );
        let mut expected = serde_json::to_value(&record).unwrap();
        expected["source"] = json!("cache");
        expected["source_label"] = json!("cached catalog");
        assert_eq!(serde_json::to_value(&resource).unwrap(), expected);
        assert_eq!(resource.thinking_modes, record.thinking_modes);
        assert_eq!(resource.speed_modes, record.speed_modes);
        assert_eq!(resource.capabilities, record.capabilities);
    }
}
