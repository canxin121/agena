use std::collections::BTreeMap;

use crate::{
    CatalogDefinitionSourcePriority, CatalogModelDefinition, ConfiguredModelSpeedMode,
    ConfiguredModelThinkingMode, ModelCatalogDocument, merge_capability_patch, merge_model_pricing,
    merge_speed_mode_request_override_fill_missing, merge_unique,
};

trait CatalogConfiguredMode {
    fn is_default(&self) -> Option<bool>;
    fn is_default_mut(&mut self) -> &mut Option<bool>;
    fn display_name(&self) -> &Option<String>;
    fn display_name_mut(&mut self) -> &mut Option<String>;
    fn description(&self) -> &Option<String>;
    fn description_mut(&mut self) -> &mut Option<String>;
    fn request_override(&self) -> &agena_domain::ModelSpeedModeRequestOverride;
    fn request_override_mut(&mut self) -> &mut agena_domain::ModelSpeedModeRequestOverride;
    fn adapter_overrides(&self) -> &BTreeMap<String, agena_domain::ModelSpeedModeRequestOverride>;
    fn adapter_overrides_mut(
        &mut self,
    ) -> &mut BTreeMap<String, agena_domain::ModelSpeedModeRequestOverride>;
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

            fn request_override(&self) -> &agena_domain::ModelSpeedModeRequestOverride {
                &self.request_override
            }

            fn request_override_mut(&mut self) -> &mut agena_domain::ModelSpeedModeRequestOverride {
                &mut self.request_override
            }

            fn adapter_overrides(
                &self,
            ) -> &BTreeMap<String, agena_domain::ModelSpeedModeRequestOverride> {
                &self.adapter_overrides
            }

            fn adapter_overrides_mut(
                &mut self,
            ) -> &mut BTreeMap<String, agena_domain::ModelSpeedModeRequestOverride> {
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

fn merge_mode_default(
    current: &mut crate::ConfiguredModeDefault,
    next: &crate::ConfiguredModeDefault,
) {
    if matches!(current, crate::ConfiguredModeDefault::Inherit) {
        *current = next.clone();
    }
}

fn merge_catalog_mode_groups<Mode: Clone + CatalogConfiguredMode>(
    current: &mut crate::ConfiguredModelModeMap<Mode>,
    next: &crate::ConfiguredModelModeMap<Mode>,
    merge_mode: impl Fn(&mut Mode, &Mode),
) {
    merge_catalog_mode_maps(&mut current.modes, &next.modes, merge_mode);
    merge_mode_default(&mut current.default, &next.default);
}

fn merge_mode_adapter_overrides_fill_missing(
    current: &mut BTreeMap<String, agena_domain::ModelSpeedModeRequestOverride>,
    next: &BTreeMap<String, agena_domain::ModelSpeedModeRequestOverride>,
) {
    for (adapter_id, override_patch) in next {
        let current_patch = current.entry(adapter_id.clone()).or_default();
        merge_speed_mode_request_override_fill_missing(current_patch, override_patch);
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

fn merge_catalog_speed_mode_fill_missing(
    current: &mut ConfiguredModelSpeedMode,
    next: &ConfiguredModelSpeedMode,
) {
    merge_catalog_configured_mode_fill_missing(current, next, |_current, _next| {});
}

pub fn merge_public_source_catalog_document(
    current: &mut BTreeMap<String, CatalogModelDefinition>,
    next: ModelCatalogDocument,
) {
    for (model_id, definition) in next.models {
        current
            .entry(model_id)
            .and_modify(|existing| merge_public_source_catalog_definition(existing, &definition))
            .or_insert(definition);
    }
}

pub fn merge_public_source_catalog_definition(
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
    merge_catalog_mode_groups(
        &mut current.thinking_modes,
        &next.thinking_modes,
        merge_catalog_thinking_mode,
    );
    merge_catalog_mode_groups(
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

#[cfg(test)]
mod tests {
    use super::merge_public_source_catalog_document;
    use crate::{CatalogDefinitionSourcePriority, CatalogModelDefinition, ModelCatalogDocument};
    use std::collections::BTreeMap;

    #[test]
    fn public_source_merge_uses_the_higher_priority_limit_and_keeps_fallback_data() {
        let mut models = BTreeMap::from([(
            "model".to_owned(),
            CatalogModelDefinition {
                context_window_tokens: Some(4_096),
                description: Some("primary description".to_owned()),
                source_priority: CatalogDefinitionSourcePriority {
                    limits_priority: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        )]);
        merge_public_source_catalog_document(
            &mut models,
            ModelCatalogDocument {
                models: BTreeMap::from([(
                    "model".to_owned(),
                    CatalogModelDefinition {
                        context_window_tokens: Some(8_192),
                        origin: Some("fallback origin".to_owned()),
                        source_priority: CatalogDefinitionSourcePriority {
                            limits_priority: 2,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )]),
            },
        );

        let merged = models.get("model").expect("merged model");
        assert_eq!(merged.context_window_tokens, Some(8_192));
        assert_eq!(merged.description.as_deref(), Some("primary description"));
        assert_eq!(merged.origin.as_deref(), Some("fallback origin"));
        assert_eq!(merged.source_priority.limits_priority, 2);
    }
}
