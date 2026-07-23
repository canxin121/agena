use agena_domain::{CapabilitySupport, Model, ModelCapabilities, ModelInputModality};

use crate::{
    CapabilitySelectionPatch, CatalogDefinitionSourcePriority, CatalogModelDefinition,
    ConfiguredModeDefault, ConfiguredModelModeMap, ConfiguredModelSpeedMode,
    ConfiguredModelThinkingMode, ModelCapabilityFeature, ModelCapabilityPatch,
};

/// Projects the stable domain model value into the provider catalog contract.
///
/// This deliberately has no runtime, configuration, HTTP, or persistence
/// dependency, so catalog construction can share it without depending on core.
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
            .map(|mode| ConfiguredModelThinkingMode {
                is_default: mode.is_default.then_some(true),
                preset: mode.preset.clone(),
                display_name: mode.display_name.clone(),
                description: mode.description.clone(),
                thinking: mode.thinking.clone(),
                request_override: mode.request_override.clone(),
                adapter_overrides: mode.adapter_overrides.clone(),
                disabled: false,
                strategy: None,
                effort: None,
                budget_tokens: None,
                display: None,
            })
            .collect::<Vec<_>>()
            .into(),
        speed_modes: ConfiguredModelModeMap {
            default: model
                .speed_modes
                .iter()
                .find_map(|(name, mode)| {
                    mode.is_default
                        .then(|| ConfiguredModeDefault::Mode(name.clone()))
                })
                .unwrap_or_default(),
            modes: model
                .speed_modes
                .iter()
                .map(|(name, mode)| {
                    (
                        name.clone(),
                        ConfiguredModelSpeedMode {
                            is_default: mode.is_default.then_some(true),
                            display_name: mode.display_name.clone(),
                            description: mode.description.clone(),
                            request_override: mode.request_override.clone(),
                            adapter_overrides: mode.adapter_overrides.clone(),
                            disabled: false,
                        },
                    )
                })
                .collect(),
        },
        capabilities: capability_patch_from_model(&model.capabilities),
        source_priority: CatalogDefinitionSourcePriority::default(),
    }
}

pub fn capability_patch_from_model(capabilities: &ModelCapabilities) -> ModelCapabilityPatch {
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
    }
}
