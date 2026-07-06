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
                Some(ProviderCapabilityFamilyConfig::OpenAi)
                | Some(ProviderCapabilityFamilyConfig::OpenAiCompatible)
                | None => 450,
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
        input: CapabilitySelectionPatch::optional_from_supported_unsupported(
            supported_inputs,
            unsupported_inputs,
        ),
        features: CapabilitySelectionPatch::optional_from_supported_unsupported(
            supported_features,
            unsupported_features,
        ),
        ..ModelCapabilityPatch::default()
    }
}

trait CatalogConfiguredMode {
    fn display_name(&self) -> &Option<String>;
    fn display_name_mut(&mut self) -> &mut Option<String>;
    fn description(&self) -> &Option<String>;
    fn description_mut(&mut self) -> &mut Option<String>;
    fn request_override(&self) -> &crate::model::ModelSpeedModeRequestOverride;
    fn request_override_mut(&mut self) -> &mut crate::model::ModelSpeedModeRequestOverride;
    fn adapter_overrides(&self) -> &BTreeMap<String, crate::model::ModelSpeedModeRequestOverride>;
    fn adapter_overrides_mut(
        &mut self,
    ) -> &mut BTreeMap<String, crate::model::ModelSpeedModeRequestOverride>;
    fn disabled(&self) -> bool;
    fn disabled_mut(&mut self) -> &mut bool;
}

macro_rules! impl_catalog_configured_mode {
    ($ty:path) => {
        impl CatalogConfiguredMode for $ty {
            fn display_name(&self) -> &Option<String> {
                &self.display_name
            }

            fn display_name_mut(&mut self) -> &mut Option<String> {
                &mut self.display_name
            }

            fn description(&self) -> &Option<String> {
                &self.description
            }

            fn description_mut(&mut self) -> &mut Option<String> {
                &mut self.description
            }

            fn request_override(&self) -> &crate::model::ModelSpeedModeRequestOverride {
                &self.request_override
            }

            fn request_override_mut(&mut self) -> &mut crate::model::ModelSpeedModeRequestOverride {
                &mut self.request_override
            }

            fn adapter_overrides(
                &self,
            ) -> &BTreeMap<String, crate::model::ModelSpeedModeRequestOverride> {
                &self.adapter_overrides
            }

            fn adapter_overrides_mut(
                &mut self,
            ) -> &mut BTreeMap<String, crate::model::ModelSpeedModeRequestOverride> {
                &mut self.adapter_overrides
            }

            fn disabled(&self) -> bool {
                self.disabled
            }

            fn disabled_mut(&mut self) -> &mut bool {
                &mut self.disabled
            }
        }
    };
}

impl_catalog_configured_mode!(ConfiguredModelThinkingMode);
impl_catalog_configured_mode!(ConfiguredModelSpeedMode);

fn fill_missing_option<T: Clone>(current: &mut Option<T>, next: &Option<T>) {
    if current.is_none() {
        *current = next.clone();
    }
}

fn merge_catalog_mode_maps<Mode>(
    current: &mut BTreeMap<String, Mode>,
    next: &BTreeMap<String, Mode>,
    merge_mode: impl Fn(&mut Mode, &Mode),
) where
    Mode: Clone,
{
    for (name, mode) in next {
        current
            .entry(name.clone())
            .and_modify(|existing| merge_mode(existing, mode))
            .or_insert_with(|| mode.clone());
    }
}

fn merge_mode_adapter_overrides_fill_missing(
    current: &mut BTreeMap<String, crate::model::ModelSpeedModeRequestOverride>,
    next: &BTreeMap<String, crate::model::ModelSpeedModeRequestOverride>,
) {
    for (adapter_id, override_patch) in next {
        let current_patch = current.entry(adapter_id.clone()).or_default();
        merge_speed_mode_request_override_fill_missing(current_patch, override_patch);
    }
}

fn merge_mode_adapter_overrides_override(
    current: &mut BTreeMap<String, crate::model::ModelSpeedModeRequestOverride>,
    next: &BTreeMap<String, crate::model::ModelSpeedModeRequestOverride>,
) {
    for (adapter_id, override_patch) in next {
        let merged = current
            .get(adapter_id)
            .cloned()
            .unwrap_or_default()
            .merged_with(override_patch);
        current.insert(adapter_id.clone(), merged);
    }
}

fn merge_catalog_configured_mode_fill_missing<Mode: CatalogConfiguredMode>(
    current: &mut Mode,
    next: &Mode,
    merge_extra: impl Fn(&mut Mode, &Mode),
) {
    fill_missing_option(current.display_name_mut(), next.display_name());
    fill_missing_option(current.description_mut(), next.description());
    merge_extra(current, next);
    merge_speed_mode_request_override_fill_missing(
        current.request_override_mut(),
        next.request_override(),
    );
    merge_mode_adapter_overrides_fill_missing(
        current.adapter_overrides_mut(),
        next.adapter_overrides(),
    );
    *current.disabled_mut() |= next.disabled();
}

fn merge_catalog_configured_mode_override<Mode: CatalogConfiguredMode>(
    current: &mut Mode,
    next: &Mode,
    merge_extra: impl Fn(&mut Mode, &Mode),
) {
    fill_missing_option(current.display_name_mut(), next.display_name());
    fill_missing_option(current.description_mut(), next.description());
    merge_extra(current, next);
    let merged = current
        .request_override()
        .merged_with(next.request_override());
    *current.request_override_mut() = merged;
    merge_mode_adapter_overrides_override(
        current.adapter_overrides_mut(),
        next.adapter_overrides(),
    );
    *current.disabled_mut() |= next.disabled();
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
    merge_catalog_mode_maps(
        &mut current.thinking_modes,
        &next.thinking_modes,
        merge_catalog_thinking_mode,
    );
    merge_catalog_mode_maps(
        &mut current.speed_modes,
        &next.speed_modes,
        merge_catalog_speed_mode,
    );
    merge_capability_patch(&mut current.capabilities, &next.capabilities);
    merge_source_priority(&mut current.source_priority, &next.source_priority);
}

pub(super) fn merge_catalog_thinking_mode(
    current: &mut ConfiguredModelThinkingMode,
    next: &ConfiguredModelThinkingMode,
) {
    merge_catalog_configured_mode_fill_missing(current, next, |current, next| {
        fill_missing_option(&mut current.thinking, &next.thinking);
    });
}

pub(super) fn merge_catalog_speed_mode(
    current: &mut ConfiguredModelSpeedMode,
    next: &ConfiguredModelSpeedMode,
) {
    merge_catalog_configured_mode_override(current, next, |_current, _next| {});
}

pub(super) fn merge_catalog_speed_mode_fill_missing(
    current: &mut ConfiguredModelSpeedMode,
    next: &ConfiguredModelSpeedMode,
) {
    merge_catalog_configured_mode_fill_missing(current, next, |_current, _next| {});
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
    merge_limit_field(
        &mut current.context_window_tokens,
        next.context_window_tokens,
        current.source_priority.limits_priority,
        next.source_priority.limits_priority,
    );
    merge_limit_field(
        &mut current.max_input_tokens,
        next.max_input_tokens,
        current.source_priority.limits_priority,
        next.source_priority.limits_priority,
    );
    merge_limit_field(
        &mut current.max_output_tokens,
        next.max_output_tokens,
        current.source_priority.limits_priority,
        next.source_priority.limits_priority,
    );
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
    merge_catalog_mode_maps(
        &mut current.thinking_modes,
        &next.thinking_modes,
        merge_catalog_thinking_mode,
    );
    merge_catalog_mode_maps(
        &mut current.speed_modes,
        &next.speed_modes,
        merge_catalog_speed_mode_fill_missing,
    );
    merge_capability_patch(&mut current.capabilities, &next.capabilities);
    merge_source_priority(&mut current.source_priority, &next.source_priority);
}

fn merge_limit_field(
    current: &mut Option<u32>,
    next: Option<u32>,
    current_priority: i32,
    next_priority: i32,
) {
    if current.is_none() || (next.is_some() && next_priority > current_priority) {
        *current = next.or(*current);
    }
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
    merge_selection_patch(&mut current.input, next.input.as_ref());
    merge_selection_patch(&mut current.features, next.features.as_ref());
}

pub(super) fn merge_selection_patch<T: Clone + PartialEq>(
    current: &mut Option<CapabilitySelectionPatch<T>>,
    next: Option<&CapabilitySelectionPatch<T>>,
) {
    let Some(next) = next else {
        return;
    };
    let Some(current_patch) = current.as_ref() else {
        *current = Some(next.clone());
        return;
    };

    let mut supported = current_patch.supported().to_vec();
    let mut unsupported = current_patch.unsupported().to_vec();

    merge_unique_without_conflicts(&mut supported, &unsupported, next.supported());
    merge_unique_without_conflicts(&mut unsupported, &supported, next.unsupported());

    *current =
        CapabilitySelectionPatch::optional_from_supported_unsupported(supported, unsupported);
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
