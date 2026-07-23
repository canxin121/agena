use std::collections::BTreeMap;

use agena_domain::{ModelPricing, ModelSpeedModeRequestOverride};

use crate::{
    CapabilitySelectionPatch, CatalogDefinitionSourcePriority, CatalogModelDefinition,
    ConfiguredModeDefault, ConfiguredModelModeMap, ConfiguredModelSpeedMode,
    ConfiguredModelThinkingMode, ModelCapabilityPatch, ModelCatalogDocument,
};

trait CatalogConfiguredMode {
    fn is_default(&self) -> Option<bool>;
    fn is_default_mut(&mut self) -> &mut Option<bool>;
    fn display_name(&self) -> &Option<String>;
    fn display_name_mut(&mut self) -> &mut Option<String>;
    fn description(&self) -> &Option<String>;
    fn description_mut(&mut self) -> &mut Option<String>;
    fn request_override(&self) -> &ModelSpeedModeRequestOverride;
    fn request_override_mut(&mut self) -> &mut ModelSpeedModeRequestOverride;
    fn adapter_overrides(&self) -> &BTreeMap<String, ModelSpeedModeRequestOverride>;
    fn adapter_overrides_mut(&mut self) -> &mut BTreeMap<String, ModelSpeedModeRequestOverride>;
    fn disabled(&self) -> bool;
    fn disabled_mut(&mut self) -> &mut bool;
}

macro_rules! impl_catalog_configured_mode {
    ($ty:path) => {
        impl CatalogConfiguredMode for $ty {
            fn is_default(&self) -> Option<bool> {
                self.is_default
            }
            fn is_default_mut(&mut self) -> &mut Option<bool> {
                &mut self.is_default
            }
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
            fn request_override(&self) -> &ModelSpeedModeRequestOverride {
                &self.request_override
            }
            fn request_override_mut(&mut self) -> &mut ModelSpeedModeRequestOverride {
                &mut self.request_override
            }
            fn adapter_overrides(&self) -> &BTreeMap<String, ModelSpeedModeRequestOverride> {
                &self.adapter_overrides
            }
            fn adapter_overrides_mut(
                &mut self,
            ) -> &mut BTreeMap<String, ModelSpeedModeRequestOverride> {
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

fn merge_mode_default(current: &mut ConfiguredModeDefault, next: &ConfiguredModeDefault) {
    if matches!(current, ConfiguredModeDefault::Inherit) {
        *current = next.clone();
    }
}

fn merge_catalog_mode_maps<Mode>(
    current: &mut BTreeMap<String, Mode>,
    next: &BTreeMap<String, Mode>,
    merge_mode: impl Fn(&mut Mode, &Mode),
) where
    Mode: Clone + CatalogConfiguredMode,
{
    let mut default_name = current
        .iter()
        .find(|(_, mode)| mode.is_default() == Some(true))
        .map(|(name, _)| name.clone());
    for (name, mode) in next {
        let mut mode = mode.clone();
        if mode.is_default() == Some(true) {
            if default_name.is_some() && default_name.as_ref() != Some(name) {
                *mode.is_default_mut() = None;
            } else {
                default_name = Some(name.clone());
            }
        }
        current
            .entry(name.clone())
            .and_modify(|existing| merge_mode(existing, &mode))
            .or_insert(mode);
    }
}

fn merge_catalog_mode_groups<Mode: Clone + CatalogConfiguredMode>(
    current: &mut ConfiguredModelModeMap<Mode>,
    next: &ConfiguredModelModeMap<Mode>,
    merge_mode: impl Fn(&mut Mode, &Mode),
) {
    merge_catalog_mode_maps(&mut current.modes, &next.modes, merge_mode);
    merge_mode_default(&mut current.default, &next.default);
}

fn merge_mode_adapter_overrides_fill_missing(
    current: &mut BTreeMap<String, ModelSpeedModeRequestOverride>,
    next: &BTreeMap<String, ModelSpeedModeRequestOverride>,
) {
    for (adapter_id, override_patch) in next {
        merge_speed_mode_request_override_fill_missing(
            current.entry(adapter_id.clone()).or_default(),
            override_patch,
        );
    }
}

fn merge_mode_adapter_overrides_override(
    current: &mut BTreeMap<String, ModelSpeedModeRequestOverride>,
    next: &BTreeMap<String, ModelSpeedModeRequestOverride>,
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
    if current.is_default().is_none() {
        *current.is_default_mut() = next.is_default();
    }
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
    if current.is_default().is_none() {
        *current.is_default_mut() = next.is_default();
    }
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

fn merge_catalog_thinking_mode(
    current: &mut ConfiguredModelThinkingMode,
    next: &ConfiguredModelThinkingMode,
) {
    merge_catalog_configured_mode_fill_missing(current, next, |current, next| {
        fill_missing_option(&mut current.preset, &next.preset);
        fill_missing_option(&mut current.thinking, &next.thinking);
        fill_missing_option(&mut current.strategy, &next.strategy);
        fill_missing_option(&mut current.effort, &next.effort);
        fill_missing_option(&mut current.budget_tokens, &next.budget_tokens);
        fill_missing_option(&mut current.display, &next.display);
    });
}

fn merge_catalog_speed_mode(
    current: &mut ConfiguredModelSpeedMode,
    next: &ConfiguredModelSpeedMode,
) {
    merge_catalog_configured_mode_override(current, next, |_, _| {});
}

/// Merges a lower-priority definition into a primary definition without
/// replacing populated primary fields.
pub fn merge_catalog_definition(
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
    fill_missing_option(&mut current.description, &next.description);
    fill_missing_option(&mut current.knowledge_cutoff, &next.knowledge_cutoff);
    fill_missing_option(&mut current.release_date, &next.release_date);
    fill_missing_option(&mut current.last_updated, &next.last_updated);
    if current.open_weights.is_none() {
        current.open_weights = next.open_weights;
    }
    if current.supports_parallel_tool_calls.is_none() {
        current.supports_parallel_tool_calls = next.supports_parallel_tool_calls;
    }
    if current.supports_verbosity.is_none() {
        current.supports_verbosity = next.supports_verbosity;
    }
    fill_missing_option(&mut current.default_verbosity, &next.default_verbosity);
    fill_missing_option(&mut current.default_temperature, &next.default_temperature);
    fill_missing_option(&mut current.default_top_p, &next.default_top_p);
    if current.default_top_k.is_none() {
        current.default_top_k = next.default_top_k;
    }
    if current.assistant_reasoning_interleaved.is_none() {
        current.assistant_reasoning_interleaved = next.assistant_reasoning_interleaved;
    }
    fill_missing_option(
        &mut current.assistant_reasoning_field,
        &next.assistant_reasoning_field,
    );
    merge_unique(&mut current.output_modalities, &next.output_modalities);
    merge_model_pricing(&mut current.pricing, next.pricing.as_ref());
    fill_missing_option(&mut current.display_name, &next.display_name);
    fill_missing_option(&mut current.origin, &next.origin);
    merge_catalog_mode_groups(
        &mut current.thinking_modes,
        &next.thinking_modes,
        merge_catalog_thinking_mode,
    );
    merge_catalog_mode_maps(
        &mut current.speed_modes.modes,
        &next.speed_modes.modes,
        merge_catalog_speed_mode,
    );
    merge_mode_default(&mut current.speed_modes.default, &next.speed_modes.default);
    merge_capability_patch(&mut current.capabilities, &next.capabilities);
    merge_source_priority(&mut current.source_priority, &next.source_priority);
}

/// Merges one live provider document into a mutable catalog, keeping the
/// incoming definition as primary when duplicate model IDs are encountered.
pub fn merge_live_provider_catalog_document(
    current: &mut BTreeMap<String, CatalogModelDefinition>,
    next: ModelCatalogDocument,
) {
    for (model_id, definition) in next.models {
        current
            .entry(model_id)
            .and_modify(|existing| {
                let mut merged = definition.clone();
                merge_catalog_definition(&mut merged, existing);
                *existing = merged;
            })
            .or_insert(definition);
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

/// Fill a request override with fields absent from the primary value.
pub fn merge_speed_mode_request_override_fill_missing(
    current: &mut ModelSpeedModeRequestOverride,
    next: &ModelSpeedModeRequestOverride,
) {
    for (key, value) in &next.headers {
        current
            .headers
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    merge_json_patch_maps_fill_missing(&mut current.body_patch, &next.body_patch);
}

pub fn merge_json_patch_maps_fill_missing(
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

pub fn merge_json_value_fill_missing(current: &mut serde_json::Value, next: &serde_json::Value) {
    if let (serde_json::Value::Object(current), serde_json::Value::Object(next)) = (current, next) {
        for (key, value) in next {
            match current.get_mut(key) {
                Some(existing) => merge_json_value_fill_missing(existing, value),
                None => {
                    current.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

pub fn merge_capability_patch(current: &mut ModelCapabilityPatch, next: &ModelCapabilityPatch) {
    merge_selection_patch(&mut current.input, next.input.as_ref());
    merge_selection_patch(&mut current.features, next.features.as_ref());
}

pub fn merge_selection_patch<T: Clone + PartialEq>(
    current: &mut Option<CapabilitySelectionPatch<T>>,
    next: Option<&CapabilitySelectionPatch<T>>,
) {
    let Some(next) = next else { return };
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

pub fn merge_unique<T: Clone + PartialEq>(current: &mut Vec<T>, next: &[T]) {
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
        if !opposite.contains(value) && !current.contains(value) {
            current.push(value.clone());
        }
    }
}

pub fn merge_model_pricing(current: &mut Option<ModelPricing>, next: Option<&ModelPricing>) {
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

#[cfg(test)]
mod tests {
    use super::{
        merge_json_value_fill_missing, merge_live_provider_catalog_document, merge_model_pricing,
        merge_unique,
    };
    use crate::{CatalogModelDefinition, ModelCatalogDocument};
    use agena_domain::ModelPricing;
    use std::collections::BTreeMap;

    #[test]
    fn merge_primitives_keep_primary_values() {
        let mut json = serde_json::json!({"name":"primary","nested":{"keep":true}});
        merge_json_value_fill_missing(
            &mut json,
            &serde_json::json!({"name":"fallback","nested":{"add":42}}),
        );
        assert_eq!(json["name"], "primary");
        assert_eq!(json["nested"]["add"], 42);
        let mut items = vec!["primary", "shared"];
        merge_unique(&mut items, &["shared", "fallback"]);
        assert_eq!(items, vec!["primary", "shared", "fallback"]);
        let mut pricing = Some(ModelPricing {
            input_usd_per_million_tokens: Some("1".to_owned()),
            ..Default::default()
        });
        merge_model_pricing(
            &mut pricing,
            Some(&ModelPricing {
                output_usd_per_million_tokens: Some("2".to_owned()),
                ..Default::default()
            }),
        );
        assert_eq!(
            pricing.unwrap().input_usd_per_million_tokens.as_deref(),
            Some("1")
        );
    }

    #[test]
    fn live_provider_document_merge_prefers_the_later_definition() {
        let mut models = BTreeMap::from([(
            "model".to_owned(),
            CatalogModelDefinition {
                display_name: Some("first".to_owned()),
                description: Some("fallback description".to_owned()),
                ..Default::default()
            },
        )]);
        merge_live_provider_catalog_document(
            &mut models,
            ModelCatalogDocument {
                models: BTreeMap::from([(
                    "model".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("later".to_owned()),
                        ..Default::default()
                    },
                )]),
            },
        );
        let merged = models.get("model").expect("merged model");
        assert_eq!(merged.display_name.as_deref(), Some("later"));
        assert_eq!(merged.description.as_deref(), Some("fallback description"));
    }
}
