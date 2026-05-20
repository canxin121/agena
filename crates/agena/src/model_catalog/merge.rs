use super::*;

pub(super) fn provider_priority(provider_id: &str, resolution: Option<&ConfigResolution>) -> i32 {
    let Some(resolution) = resolution else {
        return 0;
    };
    let Some(provider) = resolution.config.providers.get(provider_id) else {
        return 0;
    };
    provider
        .adapters
        .values()
        .filter(|adapter| adapter.enabled)
        .map(|adapter| match &adapter.definition {
            ProviderAdapterDefinition::Anthropic(_) => 500,
            ProviderAdapterDefinition::Gemini(_) => 500,
            ProviderAdapterDefinition::OpenAi(config) => match config.options.capability_family {
                Some(ProviderCapabilityFamilyConfig::OpenAi) | None => 450,
                Some(ProviderCapabilityFamilyConfig::Anthropic)
                | Some(ProviderCapabilityFamilyConfig::Gemini) => 350,
                Some(ProviderCapabilityFamilyConfig::Bedrock)
                | Some(ProviderCapabilityFamilyConfig::Gitlab) => 200,
            },
            ProviderAdapterDefinition::AmazonBedrock(_) => 200,
            ProviderAdapterDefinition::Gitlab(_) => 150,
            ProviderAdapterDefinition::Ollama(_) => 50,
        })
        .max()
        .unwrap_or_default()
}

pub fn catalog_definition_from_model(model: &Model) -> CatalogModelDefinition {
    CatalogModelDefinition {
        lifecycle: model.metadata.lifecycle,
        context_window_tokens: model.metadata.limits.context_window_tokens,
        max_input_tokens: model.metadata.limits.max_input_tokens,
        max_output_tokens: model.metadata.limits.max_output_tokens,
        description: model.metadata.description.clone(),
        knowledge_cutoff: model.metadata.knowledge_cutoff.clone(),
        release_date: model.metadata.release_date.clone(),
        last_updated: model.metadata.last_updated.clone(),
        open_weights: model.metadata.open_weights,
        default_thinking_mode: model.metadata.default_thinking_mode.clone(),
        supports_parallel_tool_calls: model.metadata.supports_parallel_tool_calls,
        supports_verbosity: model.metadata.supports_verbosity,
        default_verbosity: model.metadata.default_verbosity.clone(),
        default_temperature: model.metadata.default_temperature.clone(),
        default_top_p: model.metadata.default_top_p.clone(),
        default_top_k: model.metadata.default_top_k,
        assistant_reasoning_interleaved: model.metadata.assistant_reasoning_interleaved,
        assistant_reasoning_field: model.metadata.assistant_reasoning_field.clone(),
        output_modalities: model.metadata.output_modalities.clone(),
        pricing: model.metadata.pricing.clone(),
        display_name: model.display_name.clone(),
        origin: None,
        thinking_modes: model
            .thinking_modes
            .iter()
            .map(|(name, mode)| {
                (
                    name.clone(),
                    ConfiguredModelThinkingMode {
                        display_name: mode.display_name.clone(),
                        description: mode.description.clone(),
                        thinking: mode.thinking.clone(),
                        request_override: mode.request_override.clone(),
                        adapter_overrides: mode.adapter_overrides.clone(),
                        disabled: false,
                    },
                )
            })
            .collect(),
        speed_modes: model
            .speed_modes
            .iter()
            .map(|(name, mode)| {
                (
                    name.clone(),
                    ConfiguredModelSpeedMode {
                        display_name: mode.display_name.clone(),
                        description: mode.description.clone(),
                        request_override: mode.request_override.clone(),
                        adapter_overrides: mode.adapter_overrides.clone(),
                        disabled: false,
                    },
                )
            })
            .collect(),
        capabilities: capability_patch_from_model(&model.capabilities),
        source_priority: CatalogDefinitionSourcePriority::default(),
    }
}

pub(super) fn capability_patch_from_model(
    capabilities: &ModelCapabilities,
) -> ModelCapabilityPatch {
    let mut supported_inputs = Vec::new();
    let mut unsupported_inputs = Vec::new();
    for (modality, support) in [
        (ModelInputModality::Text, capabilities.text_input),
        (ModelInputModality::Image, capabilities.image_input),
        (ModelInputModality::Document, capabilities.document_input),
        (ModelInputModality::Audio, capabilities.audio_input),
        (ModelInputModality::Video, capabilities.video_input),
        (ModelInputModality::File, capabilities.file_input),
    ] {
        match support {
            CapabilitySupport::Supported if !matches!(modality, ModelInputModality::Text) => {
                supported_inputs.push(modality);
            }
            CapabilitySupport::Unsupported => unsupported_inputs.push(modality),
            _ => {}
        }
    }

    let mut supported_features = Vec::new();
    let mut unsupported_features = Vec::new();
    for (feature, support) in [
        (
            ModelCapabilityFeature::ToolCalling,
            capabilities.tool_calling,
        ),
        (ModelCapabilityFeature::Streaming, capabilities.streaming),
        (ModelCapabilityFeature::Reasoning, capabilities.reasoning),
        (
            ModelCapabilityFeature::StructuredOutput,
            capabilities.structured_output,
        ),
        (
            ModelCapabilityFeature::Temperature,
            capabilities.temperature_supported,
        ),
    ] {
        match support {
            CapabilitySupport::Supported
                if !matches!(feature, ModelCapabilityFeature::Temperature) =>
            {
                supported_features.push(feature);
            }
            CapabilitySupport::Unsupported => unsupported_features.push(feature),
            _ => {}
        }
    }

    ModelCapabilityPatch {
        input: (!supported_inputs.is_empty() || !unsupported_inputs.is_empty()).then_some(
            InputCapabilityPatch::Patch(InputCapabilityPatchBody {
                supported: supported_inputs,
                unsupported: unsupported_inputs,
            }),
        ),
        features: (!supported_features.is_empty() || !unsupported_features.is_empty()).then_some(
            FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                supported: supported_features,
                unsupported: unsupported_features,
            }),
        ),
        ..ModelCapabilityPatch::default()
    }
}

pub(super) fn merge_catalog_definition(
    current: &mut CatalogModelDefinition,
    next: &CatalogModelDefinition,
) {
    if current.lifecycle.is_none() {
        current.lifecycle = next.lifecycle;
    }
    if current.context_window_tokens.is_none() {
        current.context_window_tokens = next.context_window_tokens;
    }
    if current.max_input_tokens.is_none() {
        current.max_input_tokens = next.max_input_tokens;
    }
    if current.max_output_tokens.is_none() {
        current.max_output_tokens = next.max_output_tokens;
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    if current.knowledge_cutoff.is_none() {
        current.knowledge_cutoff = next.knowledge_cutoff.clone();
    }
    if current.release_date.is_none() {
        current.release_date = next.release_date.clone();
    }
    if current.last_updated.is_none() {
        current.last_updated = next.last_updated.clone();
    }
    if current.open_weights.is_none() {
        current.open_weights = next.open_weights;
    }
    if current.default_thinking_mode.is_none() {
        current.default_thinking_mode = next.default_thinking_mode.clone();
    }
    if current.supports_parallel_tool_calls.is_none() {
        current.supports_parallel_tool_calls = next.supports_parallel_tool_calls;
    }
    if current.supports_verbosity.is_none() {
        current.supports_verbosity = next.supports_verbosity;
    }
    if current.default_verbosity.is_none() {
        current.default_verbosity = next.default_verbosity.clone();
    }
    if current.default_temperature.is_none() {
        current.default_temperature = next.default_temperature.clone();
    }
    if current.default_top_p.is_none() {
        current.default_top_p = next.default_top_p.clone();
    }
    if current.default_top_k.is_none() {
        current.default_top_k = next.default_top_k;
    }
    if current.assistant_reasoning_interleaved.is_none() {
        current.assistant_reasoning_interleaved = next.assistant_reasoning_interleaved;
    }
    if current.assistant_reasoning_field.is_none() {
        current.assistant_reasoning_field = next.assistant_reasoning_field.clone();
    }
    merge_unique(&mut current.output_modalities, &next.output_modalities);
    merge_model_pricing(&mut current.pricing, next.pricing.as_ref());
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.origin.is_none() {
        current.origin = next.origin.clone();
    }
    for (name, mode) in &next.thinking_modes {
        current
            .thinking_modes
            .entry(name.clone())
            .and_modify(|existing| merge_catalog_thinking_mode(existing, mode))
            .or_insert_with(|| mode.clone());
    }
    for (name, mode) in &next.speed_modes {
        current
            .speed_modes
            .entry(name.clone())
            .and_modify(|existing| merge_catalog_speed_mode(existing, mode))
            .or_insert_with(|| mode.clone());
    }
    merge_capability_patch(&mut current.capabilities, &next.capabilities);
    merge_source_priority(&mut current.source_priority, &next.source_priority);
}

pub(super) fn merge_catalog_thinking_mode(
    current: &mut ConfiguredModelThinkingMode,
    next: &ConfiguredModelThinkingMode,
) {
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    if current.thinking.is_none() {
        current.thinking = next.thinking.clone();
    }
    merge_speed_mode_request_override_fill_missing(
        &mut current.request_override,
        &next.request_override,
    );
    for (adapter_id, override_patch) in &next.adapter_overrides {
        let current_patch = current
            .adapter_overrides
            .entry(adapter_id.clone())
            .or_default();
        merge_speed_mode_request_override_fill_missing(current_patch, override_patch);
    }
    current.disabled |= next.disabled;
}

pub(super) fn merge_catalog_speed_mode(
    current: &mut ConfiguredModelSpeedMode,
    next: &ConfiguredModelSpeedMode,
) {
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    current.request_override = current.request_override.merged_with(&next.request_override);
    for (adapter_id, override_patch) in &next.adapter_overrides {
        let merged = current
            .adapter_overrides
            .get(adapter_id)
            .cloned()
            .unwrap_or_default()
            .merged_with(override_patch);
        current.adapter_overrides.insert(adapter_id.clone(), merged);
    }
    current.disabled |= next.disabled;
}

pub(super) fn merge_catalog_speed_mode_fill_missing(
    current: &mut ConfiguredModelSpeedMode,
    next: &ConfiguredModelSpeedMode,
) {
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    merge_speed_mode_request_override_fill_missing(
        &mut current.request_override,
        &next.request_override,
    );
    for (adapter_id, override_patch) in &next.adapter_overrides {
        let current_patch = current
            .adapter_overrides
            .entry(adapter_id.clone())
            .or_default();
        merge_speed_mode_request_override_fill_missing(current_patch, override_patch);
    }
    current.disabled |= next.disabled;
}

pub(super) fn merge_live_provider_catalog_document(
    current: &mut ModelCatalogDocument,
    next: ModelCatalogDocument,
) {
    for (model_id, definition) in next.models {
        current
            .models
            .entry(model_id)
            .and_modify(|existing| {
                let mut merged = definition.clone();
                merge_catalog_definition(&mut merged, existing);
                *existing = merged;
            })
            .or_insert(definition);
    }
}

pub(super) fn merge_public_source_catalog_document(
    current: &mut ModelCatalogDocument,
    next: ModelCatalogDocument,
) {
    for (model_id, definition) in next.models {
        current
            .models
            .entry(model_id)
            .and_modify(|existing| merge_public_source_catalog_definition(existing, &definition))
            .or_insert(definition);
    }
}

pub(super) fn merge_public_source_catalog_definition(
    current: &mut CatalogModelDefinition,
    next: &CatalogModelDefinition,
) {
    if current.lifecycle.is_none() {
        current.lifecycle = next.lifecycle;
    }
    if current.context_window_tokens.is_none() {
        current.context_window_tokens = next.context_window_tokens;
    }
    if current.max_input_tokens.is_none() {
        current.max_input_tokens = next.max_input_tokens;
    }
    if current.max_output_tokens.is_none() {
        current.max_output_tokens = next.max_output_tokens;
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    if current.knowledge_cutoff.is_none() {
        current.knowledge_cutoff = next.knowledge_cutoff.clone();
    }
    if current.release_date.is_none() {
        current.release_date = next.release_date.clone();
    }
    if current.last_updated.is_none() {
        current.last_updated = next.last_updated.clone();
    }
    if current.open_weights.is_none() {
        current.open_weights = next.open_weights;
    }
    if current.default_thinking_mode.is_none() {
        current.default_thinking_mode = next.default_thinking_mode.clone();
    }
    if current.supports_parallel_tool_calls.is_none() {
        current.supports_parallel_tool_calls = next.supports_parallel_tool_calls;
    }
    if current.supports_verbosity.is_none() {
        current.supports_verbosity = next.supports_verbosity;
    }
    if current.default_verbosity.is_none() {
        current.default_verbosity = next.default_verbosity.clone();
    }
    if current.default_temperature.is_none() {
        current.default_temperature = next.default_temperature.clone();
    }
    if current.default_top_p.is_none() {
        current.default_top_p = next.default_top_p.clone();
    }
    if current.default_top_k.is_none() {
        current.default_top_k = next.default_top_k;
    }
    if current.assistant_reasoning_interleaved.is_none() {
        current.assistant_reasoning_interleaved = next.assistant_reasoning_interleaved;
    }
    if current.assistant_reasoning_field.is_none() {
        current.assistant_reasoning_field = next.assistant_reasoning_field.clone();
    }
    merge_unique(&mut current.output_modalities, &next.output_modalities);
    merge_model_pricing(&mut current.pricing, next.pricing.as_ref());
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.origin.is_none() {
        current.origin = next.origin.clone();
    }
    for (name, mode) in &next.thinking_modes {
        current
            .thinking_modes
            .entry(name.clone())
            .and_modify(|existing| merge_catalog_thinking_mode(existing, mode))
            .or_insert_with(|| mode.clone());
    }
    for (name, mode) in &next.speed_modes {
        current
            .speed_modes
            .entry(name.clone())
            .and_modify(|existing| merge_catalog_speed_mode_fill_missing(existing, mode))
            .or_insert_with(|| mode.clone());
    }
    merge_capability_patch(&mut current.capabilities, &next.capabilities);
    merge_source_priority(&mut current.source_priority, &next.source_priority);
}

fn merge_source_priority(
    current: &mut CatalogDefinitionSourcePriority,
    next: &CatalogDefinitionSourcePriority,
) {
    current.sort_priority = current.sort_priority.max(next.sort_priority);
    current.descriptive_priority = current.descriptive_priority.max(next.descriptive_priority);
    current.limits_priority = current.limits_priority.max(next.limits_priority);
    current.capability_priority = current.capability_priority.max(next.capability_priority);
    current.semantics_priority = current.semantics_priority.max(next.semantics_priority);
    current.pricing_priority = current.pricing_priority.max(next.pricing_priority);
    current.mode_priority = current.mode_priority.max(next.mode_priority);
}

pub(super) fn merge_speed_mode_request_override_fill_missing(
    current: &mut crate::model::ModelSpeedModeRequestOverride,
    next: &crate::model::ModelSpeedModeRequestOverride,
) {
    for (key, value) in &next.headers {
        current
            .headers
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    merge_json_patch_maps_fill_missing(&mut current.body_patch, &next.body_patch);
}

pub(super) fn merge_json_patch_maps_fill_missing(
    current: &mut BTreeMap<String, serde_json::Value>,
    next: &BTreeMap<String, serde_json::Value>,
) {
    for (key, value) in next {
        match current.get_mut(key) {
            Some(existing) => merge_json_value_fill_missing(existing, value),
            None => {
                current.insert(key.clone(), value.clone());
            }
        }
    }
}

pub(super) fn merge_json_value_fill_missing(
    current: &mut serde_json::Value,
    next: &serde_json::Value,
) {
    if let (serde_json::Value::Object(current_map), serde_json::Value::Object(next_map)) =
        (current, next)
    {
        for (key, value) in next_map {
            match current_map.get_mut(key) {
                Some(existing) => merge_json_value_fill_missing(existing, value),
                None => {
                    current_map.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

pub(super) fn merge_capability_patch(
    current: &mut ModelCapabilityPatch,
    next: &ModelCapabilityPatch,
) {
    merge_input_patch(&mut current.input, next.input.as_ref());
    merge_feature_patch(&mut current.features, next.features.as_ref());
    if current.text_input.is_none() {
        current.text_input = next.text_input;
    }
    if current.image_input.is_none() {
        current.image_input = next.image_input;
    }
    if current.document_input.is_none() {
        current.document_input = next.document_input;
    }
    if current.audio_input.is_none() {
        current.audio_input = next.audio_input;
    }
    if current.video_input.is_none() {
        current.video_input = next.video_input;
    }
    if current.file_input.is_none() {
        current.file_input = next.file_input;
    }
    if current.tool_calling.is_none() {
        current.tool_calling = next.tool_calling;
    }
    if current.streaming.is_none() {
        current.streaming = next.streaming;
    }
    if current.reasoning.is_none() {
        current.reasoning = next.reasoning;
    }
    if current.structured_output.is_none() {
        current.structured_output = next.structured_output;
    }
    if current.temperature_supported.is_none() {
        current.temperature_supported = next.temperature_supported;
    }
}

pub(super) fn merge_input_patch(
    current: &mut Option<InputCapabilityPatch>,
    next: Option<&InputCapabilityPatch>,
) {
    let Some(next) = next else {
        return;
    };
    let Some(current_patch) = current.as_mut() else {
        *current = Some(next.clone());
        return;
    };

    let mut supported = match current_patch {
        InputCapabilityPatch::Supported(values) => values.clone(),
        InputCapabilityPatch::Patch(values) => values.supported.clone(),
    };
    let mut unsupported = match current_patch {
        InputCapabilityPatch::Supported(_) => Vec::new(),
        InputCapabilityPatch::Patch(values) => values.unsupported.clone(),
    };

    match next {
        InputCapabilityPatch::Supported(values) => {
            merge_unique_without_conflicts(&mut supported, &unsupported, values);
        }
        InputCapabilityPatch::Patch(values) => {
            merge_unique_without_conflicts(&mut supported, &unsupported, &values.supported);
            merge_unique_without_conflicts(&mut unsupported, &supported, &values.unsupported);
        }
    }

    *current_patch = if unsupported.is_empty() {
        InputCapabilityPatch::Supported(supported)
    } else {
        InputCapabilityPatch::Patch(InputCapabilityPatchBody {
            supported,
            unsupported,
        })
    };
}

pub(super) fn merge_feature_patch(
    current: &mut Option<FeatureCapabilityPatch>,
    next: Option<&FeatureCapabilityPatch>,
) {
    let Some(next) = next else {
        return;
    };
    let Some(current_patch) = current.as_mut() else {
        *current = Some(next.clone());
        return;
    };

    let mut supported = match current_patch {
        FeatureCapabilityPatch::Supported(values) => values.clone(),
        FeatureCapabilityPatch::Patch(values) => values.supported.clone(),
    };
    let mut unsupported = match current_patch {
        FeatureCapabilityPatch::Supported(_) => Vec::new(),
        FeatureCapabilityPatch::Patch(values) => values.unsupported.clone(),
    };

    match next {
        FeatureCapabilityPatch::Supported(values) => {
            merge_unique_without_conflicts(&mut supported, &unsupported, values);
        }
        FeatureCapabilityPatch::Patch(values) => {
            merge_unique_without_conflicts(&mut supported, &unsupported, &values.supported);
            merge_unique_without_conflicts(&mut unsupported, &supported, &values.unsupported);
        }
    }

    *current_patch = if unsupported.is_empty() {
        FeatureCapabilityPatch::Supported(supported)
    } else {
        FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
            supported,
            unsupported,
        })
    };
}

pub(super) fn merge_unique<T: Clone + PartialEq>(current: &mut Vec<T>, next: &[T]) {
    for value in next {
        if !current.contains(value) {
            current.push(value.clone());
        }
    }
}

fn merge_unique_without_conflicts<T: Clone + PartialEq>(
    current: &mut Vec<T>,
    opposite: &[T],
    next: &[T],
) {
    for value in next {
        if opposite.contains(value) || current.contains(value) {
            continue;
        }
        current.push(value.clone());
    }
}

pub(super) fn merge_model_pricing(current: &mut Option<ModelPricing>, next: Option<&ModelPricing>) {
    match (current.as_mut(), next) {
        (None, Some(next)) => *current = Some(next.clone()),
        (Some(current), Some(next)) => {
            if current.input_usd_per_million_tokens.is_none() {
                current.input_usd_per_million_tokens = next.input_usd_per_million_tokens.clone();
            }
            if current.output_usd_per_million_tokens.is_none() {
                current.output_usd_per_million_tokens = next.output_usd_per_million_tokens.clone();
            }
            if current.cache_read_usd_per_million_tokens.is_none() {
                current.cache_read_usd_per_million_tokens =
                    next.cache_read_usd_per_million_tokens.clone();
            }
            if current.cache_write_usd_per_million_tokens.is_none() {
                current.cache_write_usd_per_million_tokens =
                    next.cache_write_usd_per_million_tokens.clone();
            }
            for tier in &next.tiers {
                match current.tiers.iter_mut().find(|existing| {
                    existing.tier_type == tier.tier_type && existing.size_tokens == tier.size_tokens
                }) {
                    Some(existing) => {
                        if existing.input_usd_per_million_tokens.is_none() {
                            existing.input_usd_per_million_tokens =
                                tier.input_usd_per_million_tokens.clone();
                        }
                        if existing.output_usd_per_million_tokens.is_none() {
                            existing.output_usd_per_million_tokens =
                                tier.output_usd_per_million_tokens.clone();
                        }
                        if existing.cache_read_usd_per_million_tokens.is_none() {
                            existing.cache_read_usd_per_million_tokens =
                                tier.cache_read_usd_per_million_tokens.clone();
                        }
                        if existing.cache_write_usd_per_million_tokens.is_none() {
                            existing.cache_write_usd_per_million_tokens =
                                tier.cache_write_usd_per_million_tokens.clone();
                        }
                    }
                    None => current.tiers.push(tier.clone()),
                }
            }
        }
        _ => {}
    }
}
