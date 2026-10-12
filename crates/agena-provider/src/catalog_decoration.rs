//! Pure model-catalog baseline decoration.
//!
//! These helpers combine stable catalog definitions with provider-visible
//! model values. They deliberately do not know how a concrete provider lists
//! models; runtime adapters supply those values at their composition boundary.

use std::collections::BTreeMap;

use agena_domain::{Model, ModelCapabilities, ModelMetadata, ModelSpeedMode, ModelThinkingMode};

use crate::{
    CatalogModelDefinition, ConfiguredModelDefinition, ConfiguredModelModeMap,
    ConfiguredModelSpeedMode, ConfiguredModelThinkingMode,
};

pub fn catalog_definition_to_provider_definition(
    definition: &CatalogModelDefinition,
) -> ConfiguredModelDefinition {
    definition.clone().into_configured_definition()
}

pub fn apply_catalog_definition_as_baseline(
    definition: &CatalogModelDefinition,
    capability_fallback: &ModelCapabilities,
    metadata_fallback: &ModelMetadata,
    model: Model,
) -> Model {
    let configured = catalog_definition_to_provider_definition(definition);
    apply_configured_definition_as_baseline(
        &configured,
        capability_fallback,
        metadata_fallback,
        model,
    )
}

/// Apply a catalog definition as the authority for a route that follows the
/// catalog: every field the catalog defines wins over the route's own values and
/// the route only contributes what the catalog leaves undefined. Mode maps are
/// still unioned so a route may add selectors the catalog does not list.
///
/// This is the opposite precedence of [`apply_catalog_definition_as_baseline`]
/// and is what makes a catalog refresh take effect on the next model listing
/// without re-applying the catalog entry by hand.
pub fn apply_catalog_definition_as_authority(
    definition: &CatalogModelDefinition,
    capability_fallback: &ModelCapabilities,
    metadata_fallback: &ModelMetadata,
    mut model: Model,
) -> Model {
    let configured = catalog_definition_to_provider_definition(definition);
    let catalog_capabilities = configured.capabilities.apply_to(ModelCapabilities::default());
    let route_capabilities = model
        .capabilities
        .clone()
        .merged_with_fallbacks_from(capability_fallback);
    model.capabilities = catalog_capabilities.merged_with_fallbacks_from(&route_capabilities);

    let catalog_metadata = configured.metadata();
    let route_metadata = model
        .metadata
        .clone()
        .merged_with_fallbacks_from(metadata_fallback);
    model.metadata = catalog_metadata.merged_with_fallbacks_from(&route_metadata);

    if let Some(display_name) = configured.display_name.clone() {
        model.display_name = Some(display_name);
    }
    model.thinking_modes =
        merge_catalog_baseline_thinking_modes(model.thinking_modes, &configured.thinking_modes);
    model.speed_modes =
        merge_catalog_baseline_speed_modes(model.speed_modes, &configured.speed_modes);
    model
}

pub fn apply_configured_definition_as_baseline(
    configured: &ConfiguredModelDefinition,
    capability_fallback: &ModelCapabilities,
    metadata_fallback: &ModelMetadata,
    mut model: Model,
) -> Model {
    let catalog_capabilities = configured
        .capabilities
        .apply_to(ModelCapabilities::default());
    let primary_capabilities = if model.capabilities.is_default_placeholder() {
        capability_fallback.clone()
    } else {
        model
            .capabilities
            .clone()
            .merged_with_fallbacks_from(capability_fallback)
    };
    model.capabilities = primary_capabilities.merged_with_fallbacks_from(&catalog_capabilities);

    let catalog_metadata = configured.metadata();
    let primary_metadata = model
        .metadata
        .clone()
        .merged_with_fallbacks_from(metadata_fallback);
    model.metadata = primary_metadata.merged_with_fallbacks_from(&catalog_metadata);

    model.thinking_modes =
        merge_catalog_baseline_thinking_modes(model.thinking_modes, &configured.thinking_modes);
    model.speed_modes =
        merge_catalog_baseline_speed_modes(model.speed_modes, &configured.speed_modes);
    model
}

trait CatalogBaselineMode {
    fn is_default(&self) -> bool;
    fn set_default(&mut self, is_default: bool);
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
}

macro_rules! impl_catalog_baseline_mode {
    ($ty:path) => {
        impl CatalogBaselineMode for $ty {
            fn is_default(&self) -> bool {
                self.is_default
            }

            fn set_default(&mut self, is_default: bool) {
                self.is_default = is_default;
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
        }
    };
}

impl_catalog_baseline_mode!(ModelThinkingMode);
impl_catalog_baseline_mode!(ModelSpeedMode);

fn fill_missing_option<T: Clone>(current: &mut Option<T>, next: &Option<T>) {
    if current.is_none() {
        *current = next.clone();
    }
}

fn merge_baseline_adapter_overrides(
    primary: &mut BTreeMap<String, agena_domain::ModelSpeedModeRequestOverride>,
    baseline: &BTreeMap<String, agena_domain::ModelSpeedModeRequestOverride>,
) {
    for (adapter_id, override_patch) in baseline {
        match primary.remove(adapter_id.as_str()) {
            Some(existing) => {
                primary.insert(adapter_id.clone(), override_patch.merged_with(&existing));
            }
            None => {
                primary.insert(adapter_id.clone(), override_patch.clone());
            }
        }
    }
}

fn merge_catalog_baseline_mode<Mode>(
    mut primary: Mode,
    baseline: &Mode,
    merge_extra: impl Fn(&mut Mode, &Mode),
) -> Mode
where
    Mode: CatalogBaselineMode,
{
    fill_missing_option(primary.display_name_mut(), baseline.display_name());
    fill_missing_option(primary.description_mut(), baseline.description());
    merge_extra(&mut primary, baseline);
    let merged = baseline
        .request_override()
        .merged_with(primary.request_override());
    *primary.request_override_mut() = merged;
    merge_baseline_adapter_overrides(
        primary.adapter_overrides_mut(),
        baseline.adapter_overrides(),
    );
    primary
}

fn retain_mode_map_default<Mode: CatalogBaselineMode>(
    modes: &mut BTreeMap<String, Mode>,
    preferred: Option<&str>,
) {
    let fallback = preferred.map(ToOwned::to_owned).or_else(|| {
        modes
            .iter()
            .find(|(_, mode)| mode.is_default())
            .map(|(name, _)| name.clone())
    });
    for (name, mode) in modes {
        mode.set_default(fallback.as_deref() == Some(name.as_str()));
    }
}

pub fn merge_catalog_baseline_thinking_modes(
    primary: Vec<ModelThinkingMode>,
    baseline: &ConfiguredModelModeMap<ConfiguredModelThinkingMode>,
) -> Vec<ModelThinkingMode> {
    let primary_default = primary
        .iter()
        .find(|mode| mode.is_default)
        .and_then(|mode| mode.selector().map(|selector| selector.into_owned()));
    let mut modes = primary
        .into_iter()
        .filter_map(|mode| {
            let selector = mode.selector()?.into_owned();
            Some((selector, mode))
        })
        .collect::<BTreeMap<_, _>>();

    for (name, configured) in baseline.iter() {
        if configured.disabled {
            continue;
        }
        let baseline_mode = crate::configured_thinking_mode_to_model(name, configured);
        let mode = match modes.remove(name) {
            Some(primary_mode) => {
                merge_catalog_baseline_mode(primary_mode, &baseline_mode, |primary, baseline| {
                    fill_missing_option(&mut primary.preset, &baseline.preset);
                    fill_missing_option(&mut primary.thinking, &baseline.thinking);
                })
            }
            None => baseline_mode,
        };
        modes.insert(name.to_string(), mode);
    }

    let default = primary_default
        .as_deref()
        .or_else(|| baseline.default.mode());
    retain_mode_map_default(&mut modes, default);
    modes.into_values().collect()
}

pub fn merge_catalog_baseline_speed_modes(
    primary: BTreeMap<String, ModelSpeedMode>,
    baseline: &ConfiguredModelModeMap<ConfiguredModelSpeedMode>,
) -> BTreeMap<String, ModelSpeedMode> {
    let primary_default = primary
        .iter()
        .find(|(_, mode)| mode.is_default)
        .map(|(name, _)| name.clone());
    let mut modes = primary;
    for (name, configured) in baseline.iter() {
        if configured.disabled {
            continue;
        }
        let Some(baseline_mode) = configured.apply_to_mode(None) else {
            continue;
        };
        let mode = match modes.remove(name) {
            Some(primary_mode) => {
                merge_catalog_baseline_mode(primary_mode, &baseline_mode, |_, _| {})
            }
            None => baseline_mode,
        };
        modes.insert(name.clone(), mode);
    }

    let default = primary_default
        .as_deref()
        .or_else(|| baseline.default.mode());
    retain_mode_map_default(&mut modes, default);
    modes
}

#[cfg(test)]
mod tests {
    use super::{
        apply_catalog_definition_as_authority, apply_catalog_definition_as_baseline,
        apply_configured_definition_as_baseline,
    };
    use agena_domain::Model;

    use crate::{CatalogModelDefinition, ConfiguredModelDefinition};

    fn selectors(model: &Model) -> Vec<String> {
        let mut selectors: Vec<String> = model
            .thinking_modes
            .iter()
            .filter_map(|mode| mode.selector())
            .map(|selector| selector.into_owned())
            .collect();
        selectors.sort();
        selectors
    }

    #[test]
    fn catalog_thinking_modes_are_the_only_source_for_a_catalogued_model() {
        // Shape copied from the published catalog entry of a real model: the
        // effort values OpenAI publishes, and nothing else.
        let definition: CatalogModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": {
                "low": { "display_name": "Think Low", "strategy": "effort", "effort": "low" },
                "medium": { "display_name": "Think Medium", "strategy": "effort", "effort": "medium" },
                "high": { "display_name": "Think High", "strategy": "effort", "effort": "high" },
                "xhigh": { "display_name": "Think Xhigh", "strategy": "effort", "effort": "xhigh" },
                "max": { "display_name": "Think Max", "strategy": "effort", "effort": "max" }
            }
        }))
        .expect("catalog definition");

        let decorated = apply_catalog_definition_as_baseline(
            &definition,
            &Default::default(),
            &Default::default(),
            Model::new("openai", "gpt-6.1-sol"),
        );

        assert_eq!(
            selectors(&decorated),
            ["high", "low", "max", "medium", "xhigh"]
        );
    }

    #[test]
    fn a_model_id_alone_never_adds_thinking_modes() {
        let decorated = apply_catalog_definition_as_baseline(
            &CatalogModelDefinition::default(),
            &Default::default(),
            &Default::default(),
            Model::new("openai", "gpt-6.1-sol"),
        );

        assert!(decorated.thinking_modes.is_empty());
    }

    #[test]
    fn catalog_authority_replaces_a_stale_route_copy() {
        // The route still carries the values copied when the catalog entry was
        // applied by hand; a catalog refresh must win, not the old copy.
        let definition: CatalogModelDefinition = serde_json::from_value(serde_json::json!({
            "context_window_tokens": 1_050_000,
            "max_output_tokens": 128_000,
            "display_name": "GPT-6.1 Sol",
            "thinking_modes": {
                "max": { "display_name": "Think Max", "strategy": "effort", "effort": "max" }
            }
        }))
        .expect("catalog definition");
        let mut model = Model::new("openai", "gpt-6.1-sol");
        model.metadata.limits.context_window_tokens = Some(900_000);
        model.metadata.limits.max_output_tokens = Some(64_000);
        model.display_name = Some("Old name".to_owned());

        let decorated = apply_catalog_definition_as_authority(
            &definition,
            &Default::default(),
            &Default::default(),
            model,
        );

        assert_eq!(decorated.metadata.limits.context_window_tokens, Some(1_050_000));
        assert_eq!(decorated.metadata.limits.max_output_tokens, Some(128_000));
        assert_eq!(decorated.display_name.as_deref(), Some("GPT-6.1 Sol"));
        assert_eq!(selectors(&decorated), ["max"]);
    }

    #[test]
    fn catalog_authority_keeps_route_values_the_catalog_leaves_open() {
        let definition: CatalogModelDefinition = serde_json::from_value(serde_json::json!({
            "context_window_tokens": 1_050_000
        }))
        .expect("catalog definition");
        let mut model = Model::new("openai", "gpt-6.1-sol");
        model.metadata.limits.max_output_tokens = Some(64_000);

        let decorated = apply_catalog_definition_as_authority(
            &definition,
            &Default::default(),
            &Default::default(),
            model,
        );

        assert_eq!(decorated.metadata.limits.context_window_tokens, Some(1_050_000));
        assert_eq!(decorated.metadata.limits.max_output_tokens, Some(64_000));
    }

    #[test]
    fn configured_definition_fills_missing_model_metadata_without_overwriting_primary() {
        let configured = ConfiguredModelDefinition {
            context_window_tokens: Some(128_000),
            description: Some("catalog description".to_owned()),
            ..Default::default()
        };
        let mut model = Model::new("provider", "model");
        model.metadata.description = Some("primary description".to_owned());

        let decorated = apply_configured_definition_as_baseline(
            &configured,
            &Default::default(),
            &Default::default(),
            model,
        );

        assert_eq!(
            decorated.metadata.limits.context_window_tokens,
            Some(128_000)
        );
        assert_eq!(
            decorated.metadata.description.as_deref(),
            Some("primary description")
        );
    }
}
