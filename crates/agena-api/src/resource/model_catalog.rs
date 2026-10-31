//! Explicit projections from public model metadata into editable provider
//! configuration. There is no runtime access or persistence in this module.

use super::CapabilitySupportResource;
use agena_domain::ModelInputModality;
use agena_provider::{
    CapabilitySelectionPatch, CatalogModelDefinition, ConfiguredModeDefault,
    ConfiguredModelModeMap, ConfiguredModelSpeedMode, ConfiguredModelThinkingMode,
    ModelCapabilityFeature, ModelCapabilityPatch,
};
use serde_json::Value as JsonValue;

pub fn preferred_catalog_model_for_lookup_ids<'a>(
    models: &'a [super::CatalogModelResource],
    model_ids: &[String],
) -> Option<&'a super::CatalogModelResource> {
    // The candidates are ordered from exact provider ID to broader aliases.
    // Keeping that order matters when both a dated snapshot and a moving
    // family alias exist in the catalog.
    model_ids.iter().find_map(|model_id| {
        let model_id = model_id.trim();
        models.iter().find(|model| model.model_id == model_id)
    })
}

pub fn preferred_catalog_model_for_provider_model<'a>(
    models: &'a [super::CatalogModelResource],
    provider_model: &super::ProviderModelResource,
) -> Option<&'a super::CatalogModelResource> {
    let lookup_ids = provider_model_catalog_lookup_candidates(provider_model);
    preferred_catalog_model_for_lookup_ids(models, &lookup_ids)
}

/// All catalog lookup candidates for a provider model, most specific first.
pub fn provider_model_catalog_lookup_candidates(
    provider_model: &super::ProviderModelResource,
) -> Vec<String> {
    let mut lookup_ids = Vec::new();
    for id in [
        provider_model.catalog_model_id.clone().unwrap_or_default(),
        provider_model.id.clone(),
    ] {
        if !id.trim().is_empty() {
            lookup_ids.extend(catalog_lookup_candidates(id.as_str()));
        }
    }
    lookup_ids
}

pub fn catalog_lookup_id_for_model_id(model_id: &str) -> String {
    agena_provider::normalized_catalog_model_id(model_id)
}

/// Lookup candidate ids for a provider model, most specific first. Provider
/// ids frequently carry a dated/versioned tail (`gpt-5-2025-07-07`,
/// `claude-opus-4-6-20250514`, `gemini-2.5-pro-preview-09-2025`) that the
/// catalog stores either with its own snapshot key or as the bare model
/// (`gpt-5`). We try the raw id, then progressively stripped versions, then
/// the normalized form of each so a dated provider id still resolves to the
/// canonical catalog entry. Catalog keys are never re-normalized, so distinct
/// dated entries stay distinct at load time.
pub fn catalog_lookup_candidates(model_id: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    // Raw id, then progressively stripped version/date tails.
    push_lookup_candidate(model_id, &mut candidates, &mut seen);
    let mut base = model_id.to_owned();
    while let Some(stripped) = strip_catalog_version_tail(&base) {
        push_lookup_candidate(stripped.as_str(), &mut candidates, &mut seen);
        base = stripped;
    }

    // Normalized forms of every candidate so raw/normalized aliases land on
    // the same key.
    let normalized = candidates
        .iter()
        .map(|candidate| catalog_lookup_id_for_model_id(candidate))
        .collect::<Vec<_>>();
    for id in normalized {
        push_lookup_candidate(id.as_str(), &mut candidates, &mut seen);
    }

    // The canonical rules can turn a dash date into a dot (`gpt-5-2025-07-07`
    // → `gpt-5.2025-07-07`); strip tails from the normalized forms too so the
    // bare `gpt-5` is reached as a final fallback.
    let normalized_bases = candidates
        .iter()
        .map(|candidate| catalog_lookup_id_for_model_id(candidate))
        .collect::<Vec<_>>();
    for normalized_base in normalized_bases {
        let mut base = normalized_base;
        while let Some(stripped) = strip_catalog_version_tail(&base) {
            push_lookup_candidate(stripped.as_str(), &mut candidates, &mut seen);
            base = stripped;
        }
    }
    candidates
}

fn push_lookup_candidate(
    value: &str,
    candidates: &mut Vec<String>,
    seen: &mut std::collections::BTreeSet<String>,
) {
    let trimmed = value.trim().to_owned();
    if !trimmed.is_empty() && seen.insert(trimmed.clone()) {
        candidates.push(trimmed);
    }
}

/// Strip one trailing version/date segment from a model id, if present.
/// Matches `-YYYYMMDD`, `-YYYY-MM-DD`, `-MM-YYYY`, `-YYYYMM`, `-YYYY`, and
/// `-latest` tails (e.g. `claude-opus-4-6-20250514` → `claude-opus-4-6`).
fn strip_catalog_version_tail(model_id: &str) -> Option<String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tail_start = trimmed.rfind('-')?;
    let tail = &trimmed[tail_start + 1..];
    let is_version_segment = tail == "latest"
        || tail.chars().all(|ch| ch.is_ascii_digit()) && matches!(tail.len(), 4 | 6 | 8)
        || tail.chars().all(|ch| ch.is_ascii_digit() || ch == '-')
            && tail.matches('-').count() == 1
            && matches!(tail.len(), 7 | 9 | 10);
    if is_version_segment {
        let stripped = trimmed[..tail_start].trim().to_owned();
        (!stripped.is_empty()).then_some(stripped)
    } else {
        None
    }
}

pub fn provider_model_json_for_model_id(
    catalog_entries: &[super::CatalogModelResource],
    model_id: &str,
    provider_model: Option<&super::ProviderModelResource>,
) -> Result<JsonValue, serde_json::Error> {
    provider_model_overlay_to_json(provider_model_overlay_for_model_id(
        catalog_entries,
        model_id,
        provider_model,
    ))
}

pub fn provider_model_overlay_for_model_id(
    catalog_entries: &[super::CatalogModelResource],
    model_id: &str,
    provider_model: Option<&super::ProviderModelResource>,
) -> agena_provider::ResolvedProviderModelConfig {
    let provider_id = provider_model.map(|model| model.provider_id.as_str());
    let mut overlay = preferred_catalog_model_for_lookup_ids(
        catalog_entries,
        &catalog_lookup_candidates(model_id),
    )
    .map(|model| catalog_model_to_provider_model_overlay(model, provider_id))
    .or_else(|| {
        provider_model.and_then(|provider_model| {
            preferred_catalog_model_for_provider_model(catalog_entries, provider_model)
                .map(|model| {
                    catalog_model_to_provider_model_overlay(
                        model,
                        Some(&provider_model.provider_id),
                    )
                })
                .or_else(|| Some(provider_model_to_provider_model_overlay(provider_model)))
        })
    })
    .unwrap_or_default();
    if let Some(provider_model) = provider_model {
        overlay.native_compaction = provider_model.native_compaction;
    }
    overlay
}

pub fn provider_model_overlay_to_json(
    overlay: agena_provider::ResolvedProviderModelConfig,
) -> Result<JsonValue, serde_json::Error> {
    match serde_json::to_value(overlay) {
        Ok(JsonValue::Object(mut value)) => {
            if matches!(value.get("enabled"), Some(JsonValue::Bool(true))) {
                value.remove("enabled");
            }
            Ok(JsonValue::Object(value))
        }
        Ok(other) => Ok(other),
        Err(error) => Err(error),
    }
}

pub fn catalog_model_to_provider_model_overlay(
    model: &super::CatalogModelResource,
    provider_id: Option<&str>,
) -> agena_provider::ResolvedProviderModelConfig {
    let definition = catalog_model_to_catalog_definition(model);
    let mut overlay = agena_provider::provider_model_overlay_from_catalog_definition(&definition);
    // Catalog verbosity is provider-scoped. Resolve only the profile belonging
    // to the route being edited; never flatten another provider's profile into
    // this route's saved configuration.
    overlay.definition.supports_verbosity = None;
    overlay.definition.default_verbosity = None;
    overlay.definition.verbosity_levels.clear();
    overlay.definition.provider_default_verbosity = None;
    if let Some(profile) = provider_id.and_then(|id| definition.verbosity_by_provider.get(id)) {
        overlay.definition.supports_verbosity = Some(profile.supported);
        if profile.supported {
            overlay.definition.verbosity_levels = profile.levels.clone();
            overlay.definition.provider_default_verbosity = profile.provider_default.clone();
        }
    }
    overlay
}

pub fn catalog_model_to_catalog_definition(
    model: &super::CatalogModelResource,
) -> CatalogModelDefinition {
    let mut definition = CatalogModelDefinition::from_fields(
        model.lifecycle,
        model.context_window_tokens,
        model.max_input_tokens,
        model.max_output_tokens,
        model.description.clone(),
        model.knowledge_cutoff.clone(),
        model.release_date.clone(),
        model.last_updated.clone(),
        model.open_weights,
        model.supports_parallel_tool_calls,
        model.supports_verbosity,
        model.default_verbosity.clone(),
        model.default_temperature.clone(),
        model.default_top_p.clone(),
        model.default_top_k,
        model.assistant_reasoning_interleaved,
        model.assistant_reasoning_field.clone(),
        model.output_modalities.clone(),
        model.pricing.clone(),
        model.display_name.clone(),
        model.origin.clone(),
        model.thinking_modes.clone(),
        model.speed_modes.clone(),
        sanitized_catalog_capability_patch(&model.capabilities.clone()),
    );
    definition.provider_sources = model.provider_sources.clone();
    definition.verbosity_by_provider = model.verbosity_by_provider.clone();
    definition
}

pub fn sanitized_catalog_capability_patch(
    patch: &agena_provider::ModelCapabilityPatch,
) -> agena_provider::ModelCapabilityPatch {
    let mut patch = patch.clone();
    patch.input = sanitize_selection_patch(patch.input.take());
    patch.features = sanitize_selection_patch(patch.features.take());

    patch
}

pub fn sanitize_selection_patch<T: Clone + PartialEq>(
    patch: Option<agena_provider::CapabilitySelectionPatch<T>>,
) -> Option<agena_provider::CapabilitySelectionPatch<T>> {
    let patch = patch?;
    match patch {
        agena_provider::CapabilitySelectionPatch::Supported(mut supported) => {
            dedupe_vec(&mut supported);
            agena_provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
                supported,
                Vec::new(),
            )
        }
        agena_provider::CapabilitySelectionPatch::Patch(mut values) => {
            dedupe_vec(&mut values.supported);
            dedupe_vec(&mut values.unsupported);
            values
                .unsupported
                .retain(|value| !values.supported.contains(value));
            agena_provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
                values.supported,
                values.unsupported,
            )
        }
    }
}

pub fn provider_model_to_provider_model_overlay(
    model: &super::ProviderModelResource,
) -> agena_provider::ResolvedProviderModelConfig {
    let metadata = &model.metadata;
    let lifecycle = metadata.lifecycle;
    let definition = CatalogModelDefinition::from_fields(
        lifecycle,
        metadata.context_window_tokens,
        metadata.max_input_tokens,
        metadata.max_output_tokens,
        metadata.description.clone(),
        metadata.knowledge_cutoff.clone(),
        metadata.release_date.clone(),
        metadata.last_updated.clone(),
        metadata.open_weights,
        metadata.supports_parallel_tool_calls,
        None,
        None,
        metadata.default_temperature.clone(),
        metadata.default_top_p.clone(),
        metadata.default_top_k,
        metadata.assistant_reasoning_interleaved,
        metadata.assistant_reasoning_field.clone(),
        metadata.output_modalities.clone(),
        // Pricing is display metadata; configuration generation never needs it.
        None,
        model.display_name.clone(),
        None,
        provider_model_thinking_modes_to_configured(model),
        provider_model_speed_modes_to_configured(model),
        provider_model_capabilities_to_patch(model),
    );
    let mut overlay = agena_provider::provider_model_overlay_from_catalog_definition(&definition);
    // This fallback is used for a live provider model that has no catalog
    // match, so its metadata is already scoped to the selected provider.
    overlay.definition.supports_verbosity = metadata.supports_verbosity;
    overlay.definition.default_verbosity = metadata.default_verbosity.clone();
    overlay.definition.verbosity_levels = metadata.verbosity_levels.clone();
    overlay.definition.provider_default_verbosity = metadata.provider_default_verbosity.clone();
    overlay.native_compaction = model.native_compaction;
    overlay
}

/// Convert the live model's thinking modes into a configured mode map. Uses the
/// provider's `From<Vec<ConfiguredModelThinkingMode>>` impl, which derives each
/// mode's selector, marks the map default, and persists the flat
/// strategy/effort/budget form (the `#[serde(skip)]` `preset`/`thinking` fields
/// never reach disk), so the saved config round-trips like a catalog overlay.
fn provider_model_thinking_modes_to_configured(
    model: &super::ProviderModelResource,
) -> ConfiguredModelModeMap<ConfiguredModelThinkingMode> {
    model
        .thinking_modes
        .iter()
        .map(|mode| ConfiguredModelThinkingMode {
            is_default: mode.is_default.then_some(true),
            display_name: mode.display_name.clone(),
            description: mode.description.clone(),
            preset: mode.preset.clone(),
            thinking: mode.thinking.clone(),
            strategy: None,
            effort: None,
            budget_tokens: None,
            display: None,
            request_override: mode.request_override.clone(),
            adapter_overrides: mode
                .adapter_overrides
                .iter()
                .map(|(adapter, override_patch)| (adapter.clone(), override_patch.clone()))
                .collect(),
            disabled: false,
        })
        .collect::<Vec<_>>()
        .into()
}

/// Convert the live model's speed modes into a configured mode map, mirroring
/// the thinking-mode impl's default handling (the `is_default` entry becomes
/// the map default).
fn provider_model_speed_modes_to_configured(
    model: &super::ProviderModelResource,
) -> ConfiguredModelModeMap<ConfiguredModelSpeedMode> {
    let mut result = ConfiguredModelModeMap::default();
    for (name, mode) in &model.speed_modes {
        if mode.is_default {
            result.default = ConfiguredModeDefault::Mode(name.clone());
        }
        result.modes.insert(
            name.clone(),
            ConfiguredModelSpeedMode {
                is_default: mode.is_default.then_some(true),
                display_name: mode.display_name.clone(),
                description: mode.description.clone(),
                request_override: mode.request_override.clone(),
                adapter_overrides: mode
                    .adapter_overrides
                    .iter()
                    .map(|(adapter, override_patch)| (adapter.clone(), override_patch.clone()))
                    .collect(),
                disabled: false,
            },
        );
    }
    result
}

/// Map the flat per-feature capability resource to a capability patch. Only
/// `Supported`/`Unsupported` contribute; `Unknown` is omitted so it can later
/// be filled from the catalog during decoration.
fn provider_model_capabilities_to_patch(
    model: &super::ProviderModelResource,
) -> ModelCapabilityPatch {
    let capabilities = &model.capabilities;
    let mut input_supported = Vec::new();
    let mut input_unsupported = Vec::new();
    for (modality, support) in [
        (ModelInputModality::Text, &capabilities.text_input),
        (ModelInputModality::Image, &capabilities.image_input),
        (ModelInputModality::Document, &capabilities.document_input),
        (ModelInputModality::Audio, &capabilities.audio_input),
        (ModelInputModality::Video, &capabilities.video_input),
        (ModelInputModality::File, &capabilities.file_input),
    ] {
        push_capability_support(
            &mut input_supported,
            &mut input_unsupported,
            modality,
            support,
        );
    }
    let mut feature_supported = Vec::new();
    let mut feature_unsupported = Vec::new();
    for (feature, support) in [
        (
            ModelCapabilityFeature::ToolCalling,
            &capabilities.tool_calling,
        ),
        (ModelCapabilityFeature::Streaming, &capabilities.streaming),
        (ModelCapabilityFeature::Reasoning, &capabilities.reasoning),
        (
            ModelCapabilityFeature::StructuredOutput,
            &capabilities.structured_output,
        ),
        (
            ModelCapabilityFeature::Temperature,
            &capabilities.temperature_supported,
        ),
    ] {
        push_capability_support(
            &mut feature_supported,
            &mut feature_unsupported,
            feature,
            support,
        );
    }
    ModelCapabilityPatch {
        input: CapabilitySelectionPatch::optional_from_supported_unsupported(
            input_supported,
            input_unsupported,
        ),
        features: CapabilitySelectionPatch::optional_from_supported_unsupported(
            feature_supported,
            feature_unsupported,
        ),
    }
}

fn push_capability_support<T>(
    supported: &mut Vec<T>,
    unsupported: &mut Vec<T>,
    value: T,
    support: &CapabilitySupportResource,
) {
    match support {
        CapabilitySupportResource::Supported => supported.push(value),
        CapabilitySupportResource::Unsupported => unsupported.push(value),
        CapabilitySupportResource::Unknown => {}
    }
}

pub fn dedupe_vec<T: PartialEq>(values: &mut Vec<T>) {
    let mut index = 0;
    while index < values.len() {
        let mut next = index + 1;
        while next < values.len() {
            if values[index] == values[next] {
                values.remove(next);
            } else {
                next += 1;
            }
        }
        index += 1;
    }
}

/// Build an editable model configuration from public provider metadata when
/// the model has no explicit file-level override yet.
pub fn provider_model_draft_value_from_resource(
    model_id: &str,
    provider_model: Option<&super::ProviderModelResource>,
) -> Result<serde_json::Value, serde_json::Error> {
    provider_model_json_for_model_id(&[], model_id, provider_model)
}

/// Copy one selected catalog definition into an editable provider-model value.
/// The selected catalog ID is UI state only; the returned value contains just
/// ordinary model fields and keeps the route's operational settings.
pub fn apply_catalog_template_to_model_value(
    model_value: serde_json::Value,
    catalog_model: &super::CatalogModelResource,
) -> Result<serde_json::Value, serde_json::Error> {
    let mut configured: agena_provider::ResolvedProviderModelConfig =
        serde_json::from_value(model_value)?;
    configured.definition =
        catalog_model_to_catalog_definition(catalog_model).into_configured_definition();
    serde_json::to_value(configured)
}

#[cfg(test)]
mod tests {
    use super::apply_catalog_template_to_model_value;
    use serde_json::json;

    #[test]
    fn catalog_refresh_preserves_additional_system_prompt() {
        let catalog = serde_json::from_value(json!({
            "model_id": "catalog-model",
            "source": "generated",
            "display_name": "Refreshed name"
        }))
        .unwrap();
        for prompt in [None, Some(""), Some("  keep formatting\n")] {
            let mut value = json!({"display_name": "Previous name"});
            if let Some(prompt) = prompt {
                value["additional_system_prompt"] = json!(prompt);
            }
            let refreshed = apply_catalog_template_to_model_value(value, &catalog).unwrap();
            assert_eq!(refreshed["display_name"], json!("Refreshed name"));
            assert_eq!(
                refreshed.get("additional_system_prompt"),
                prompt.map(|value| json!(value)).as_ref()
            );
        }
    }
}
