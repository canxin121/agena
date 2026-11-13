//! Explicit projections from public model metadata into editable provider
//! configuration. There is no runtime access or persistence in this module.

use super::CapabilitySupportResource;
use super::CatalogModelResource;
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

/// Resolve the catalog entry a route is explicitly matched to.
///
/// A pinned id is an exact catalog key: it must not fall back to the route's
/// model-id candidates, because the whole point of the pin is that the
/// automatic match chose the wrong entry.
pub fn pinned_catalog_model<'a>(
    models: &'a [super::CatalogModelResource],
    pinned_model_id: Option<&str>,
) -> Option<&'a super::CatalogModelResource> {
    let pinned_model_id = pinned_model_id?.trim();
    if pinned_model_id.is_empty() {
        return None;
    }
    models
        .iter()
        .find(|model| model.model_id == pinned_model_id)
}

/// Resolve the catalog entry for one route: an explicit match first, then the
/// automatic candidates derived from the route's model id.
pub fn catalog_model_for_route<'a>(
    models: &'a [super::CatalogModelResource],
    pinned_model_id: Option<&str>,
    lookup_ids: &[String],
) -> Option<&'a super::CatalogModelResource> {
    pinned_catalog_model(models, pinned_model_id)
        .or_else(|| preferred_catalog_model_for_lookup_ids(models, lookup_ids))
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

/// Point an editable provider-model value at one catalog entry.
///
/// The route records only `catalog_model_id`: the entry then supplies the
/// definition and any field the user adds next to it overrides that field. The
/// route's operational settings (enablement, native compaction, tool mode) are
/// carried through untouched, and so are unknown fields the editor round-trips.
pub fn apply_catalog_template_to_model_value(
    model_value: serde_json::Value,
    catalog_model: &CatalogModelResource,
) -> Result<serde_json::Value, serde_json::Error> {
    let JsonValue::Object(mut value) = model_value else {
        return serde_json::to_value(model_value);
    };
    // Selecting an entry starts from "this entry only": definition fields the
    // route still carried from an earlier edit would otherwise freeze values
    // the user meant to follow.
    for key in ROUTE_DEFINITION_FIELDS {
        value.remove(*key);
    }
    let catalog_model_id = catalog_model.model_id.trim();
    if !catalog_model_id.is_empty() {
        value.insert(
            "catalog_model_id".to_owned(),
            JsonValue::String(catalog_model_id.to_owned()),
        );
    }
    Ok(JsonValue::Object(value))
}

/// Drop the route's catalog match so it follows the catalog's automatic
/// resolution again. The route's own definition fields are left in place: they
/// become plain overrides of whatever the automatic match resolves to.
pub fn clear_catalog_template_from_model_value(
    model_value: serde_json::Value,
) -> Result<serde_json::Value, serde_json::Error> {
    let JsonValue::Object(mut value) = model_value else {
        return serde_json::to_value(model_value);
    };
    value.remove("catalog_model_id");
    Ok(JsonValue::Object(value))
}

/// Definition fields a catalog selection owns. Operational policy (enablement,
/// compaction, tool mode) is deliberately absent: choosing an entry must not
/// reset how the route runs.
pub const ROUTE_DEFINITION_FIELDS: &[&str] = &[
    "lifecycle",
    "context_window_tokens",
    "max_input_tokens",
    "max_output_tokens",
    "display_name",
    "description",
    "knowledge_cutoff",
    "release_date",
    "last_updated",
    "open_weights",
    "supports_parallel_tool_calls",
    "supports_verbosity",
    "default_verbosity",
    "verbosity_levels",
    "provider_default_verbosity",
    "default_temperature",
    "default_top_p",
    "default_top_k",
    "assistant_reasoning_interleaved",
    "assistant_reasoning_field",
    "output_modalities",
    "pricing",
    "thinking_modes",
    "speed_modes",
    "input",
    "features",
];
#[cfg(test)]
mod tests {
    use super::{
        CatalogModelResource, apply_catalog_template_to_model_value,
        clear_catalog_template_from_model_value,
    };
    use serde_json::json;

    fn catalog_entry() -> CatalogModelResource {
        serde_json::from_value(json!({
            "model_id": "selected-model",
            "source": "generated",
            "display_name": "Selected name",
            "context_window_tokens": 200000
        }))
        .unwrap()
    }

    #[test]
    fn selecting_a_catalog_entry_records_only_its_id() {
        // Selecting an entry starts the route over on that entry: definition
        // fields from an earlier edit are dropped so the entry stays the
        // source and a catalog refresh still reaches the route.
        let catalog = catalog_entry();
        let value = json!({
            "enabled": false,
            "native_compaction": false,
            "agena_tools": { "mode": "disabled" },
            "display_name": "Previous name",
            "context_window_tokens": 1000
        });

        let selected = apply_catalog_template_to_model_value(value, &catalog).unwrap();
        assert_eq!(selected["catalog_model_id"], json!("selected-model"));
        assert!(selected.get("display_name").is_none());
        assert!(selected.get("context_window_tokens").is_none());
        // Operational policy is not a catalog field and must survive.
        assert_eq!(selected["enabled"], json!(false));
        assert_eq!(selected["native_compaction"], json!(false));
        assert_eq!(selected["agena_tools"]["mode"], json!("disabled"));
        assert!(selected.get("manual_overrides").is_none());
    }

    #[test]
    fn clearing_the_catalog_entry_removes_the_id() {
        let catalog = catalog_entry();
        let selected = apply_catalog_template_to_model_value(json!({}), &catalog).unwrap();
        let cleared = clear_catalog_template_from_model_value(selected).unwrap();
        assert!(cleared.get("catalog_model_id").is_none());
    }
}
