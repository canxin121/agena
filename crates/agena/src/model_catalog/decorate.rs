use super::*;
use crate::model::{ModelMetadata, ProviderId};

pub fn catalog_definition_to_provider_definition(
    definition: &CatalogModelDefinition,
) -> ConfiguredModelDefinition {
    definition.clone().into_configured_definition()
}

pub(crate) fn apply_catalog_display_name_as_fallback(
    model: &mut Model,
    definition: &CatalogModelDefinition,
) {
    if model.display_name.is_none() {
        model.display_name = definition.display_name.clone();
    }
}

pub(crate) fn apply_catalog_definition_as_baseline(
    definition: &CatalogModelDefinition,
    capability_fallback: &ModelCapabilities,
    metadata_fallback: &ModelMetadata,
    mut model: Model,
) -> Model {
    let configured = catalog_definition_to_provider_definition(definition);
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

pub fn decorate_provider_models(
    provider: &dyn ModelRuntime,
    provider_record: &ModelCatalogProviderRecord,
    mut models: Vec<Model>,
) -> Vec<Model> {
    let mut listed = BTreeSet::new();
    let mut listed_catalog_ids = BTreeSet::new();

    for model in &mut models {
        listed.insert(model.id.to_string());
        if let Some(catalog_model_id) = catalog_match_model_id_for_raw(model.id.as_str()) {
            listed_catalog_ids.insert(catalog_model_id.clone());
            model.catalog_model_id = Some(ModelId::new(catalog_model_id));
        }
        *model =
            decorate_provider_model(provider, provider_record, model.id.clone(), model.clone());
    }

    for model_id in &provider_record.appendable_model_ids {
        if listed.contains(model_id.as_str())
            || listed_catalog_ids.contains(model_id.as_str())
            || models.iter().any(|model| {
                model.id.as_str() == model_id.as_str()
                    || model.catalog_model_id.as_ref().map(ModelId::as_str)
                        == Some(model_id.as_str())
            })
        {
            continue;
        }

        let model_id = ModelId::new(model_id.clone());
        let base = Model {
            provider_id: ProviderId::new(provider.id()),
            adapter_id: None,
            id: ModelId::new(model_id.as_str()),
            catalog_model_id: Some(model_id.clone()),
            display_name: None,
            capabilities: provider.model_capabilities_for_adapter(None, &model_id),
            metadata: provider_model_metadata(provider, None, &model_id),
            thinking_modes: provider_model_thinking_modes(
                provider,
                provider_record,
                None,
                &model_id,
            ),
            speed_modes: provider_model_speed_modes(provider, provider_record, None, &model_id),
        };
        models.push(decorate_provider_model(
            provider,
            provider_record,
            model_id,
            base,
        ));
    }

    models
}

fn decorate_provider_model(
    provider: &dyn ModelRuntime,
    provider_record: &ModelCatalogProviderRecord,
    model_id: ModelId,
    mut model: Model,
) -> Model {
    let matched_catalog_id = catalog_match_model_id_for_raw(model_id.as_str());
    if let Some(catalog_model_id) = matched_catalog_id {
        model.catalog_model_id = Some(ModelId::new(catalog_model_id));
    }

    if let Some(definition) = catalog_definition_for_model_id(provider_record, model_id.as_str()) {
        apply_catalog_display_name_as_fallback(&mut model, definition);
        let adapter_id = model.adapter_id.clone();
        apply_catalog_definition_as_baseline(
            definition,
            &provider.model_capabilities_for_adapter(adapter_id.as_ref(), &model_id),
            &provider_model_metadata(provider, adapter_id.as_ref(), &model_id),
            model,
        )
    } else {
        model
    }
}

fn provider_model_metadata(
    provider: &dyn ModelRuntime,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> crate::model::ModelMetadata {
    provider.model_metadata_for_adapter(adapter_id, model)
}

fn provider_model_thinking_modes(
    provider: &dyn ModelRuntime,
    provider_record: &ModelCatalogProviderRecord,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> BTreeMap<String, crate::model::ModelThinkingMode> {
    let modes = provider.model_thinking_modes_for_adapter(adapter_id, model);
    if let Some(definition) = catalog_definition_for_model_id(provider_record, model.as_str()) {
        merge_catalog_baseline_thinking_modes(
            modes,
            &catalog_definition_to_provider_definition(definition).thinking_modes,
        )
    } else {
        modes
    }
}

fn provider_model_speed_modes(
    provider: &dyn ModelRuntime,
    provider_record: &ModelCatalogProviderRecord,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> BTreeMap<String, crate::model::ModelSpeedMode> {
    let modes = provider.model_speed_modes_for_adapter(adapter_id, model);
    if let Some(definition) = catalog_definition_for_model_id(provider_record, model.as_str()) {
        merge_catalog_baseline_speed_modes(
            modes,
            &catalog_definition_to_provider_definition(definition).speed_modes,
        )
    } else {
        modes
    }
}

trait CatalogBaselineMode {
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
}

macro_rules! impl_catalog_baseline_mode {
    ($ty:path) => {
        impl CatalogBaselineMode for $ty {
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
        }
    };
}

impl_catalog_baseline_mode!(crate::model::ModelThinkingMode);
impl_catalog_baseline_mode!(crate::model::ModelSpeedMode);

trait ConfiguredCatalogBaselineMode<Mode> {
    fn disabled(&self) -> bool;
    fn apply_to_empty(&self) -> Option<Mode>;
}

impl ConfiguredCatalogBaselineMode<crate::model::ModelThinkingMode>
    for ConfiguredModelThinkingMode
{
    fn disabled(&self) -> bool {
        self.disabled
    }

    fn apply_to_empty(&self) -> Option<crate::model::ModelThinkingMode> {
        self.apply_to_mode(None)
    }
}

impl ConfiguredCatalogBaselineMode<crate::model::ModelSpeedMode> for ConfiguredModelSpeedMode {
    fn disabled(&self) -> bool {
        self.disabled
    }

    fn apply_to_empty(&self) -> Option<crate::model::ModelSpeedMode> {
        self.apply_to_mode(None)
    }
}

fn fill_missing_option<T: Clone>(current: &mut Option<T>, next: &Option<T>) {
    if current.is_none() {
        *current = next.clone();
    }
}

fn merge_baseline_adapter_overrides(
    primary: &mut BTreeMap<String, crate::model::ModelSpeedModeRequestOverride>,
    baseline: &BTreeMap<String, crate::model::ModelSpeedModeRequestOverride>,
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

fn merge_catalog_baseline_mode<Mode, ConfiguredMode>(
    mut primary: Mode,
    baseline: &ConfiguredMode,
    merge_extra: impl Fn(&mut Mode, &Mode),
) -> Mode
where
    Mode: CatalogBaselineMode,
    ConfiguredMode: ConfiguredCatalogBaselineMode<Mode>,
{
    if let Some(mode) = baseline.apply_to_empty() {
        fill_missing_option(primary.display_name_mut(), mode.display_name());
        fill_missing_option(primary.description_mut(), mode.description());
        merge_extra(&mut primary, &mode);
        let merged = mode
            .request_override()
            .merged_with(primary.request_override());
        *primary.request_override_mut() = merged;
        merge_baseline_adapter_overrides(primary.adapter_overrides_mut(), mode.adapter_overrides());
    }
    primary
}

fn merge_catalog_baseline_modes<Mode, ConfiguredMode>(
    mut primary: BTreeMap<String, Mode>,
    baseline: &BTreeMap<String, ConfiguredMode>,
    merge_mode: impl Fn(Mode, &ConfiguredMode) -> Mode,
) -> BTreeMap<String, Mode>
where
    ConfiguredMode: ConfiguredCatalogBaselineMode<Mode>,
{
    for (name, configured) in baseline {
        if configured.disabled() {
            continue;
        }
        match primary.remove(name) {
            Some(mode) => {
                primary.insert(name.clone(), merge_mode(mode, configured));
            }
            None => {
                if let Some(mode) = configured.apply_to_empty() {
                    primary.insert(name.clone(), mode);
                }
            }
        }
    }
    primary
}

pub(crate) fn merge_catalog_baseline_thinking_modes(
    primary: BTreeMap<String, crate::model::ModelThinkingMode>,
    baseline: &BTreeMap<String, ConfiguredModelThinkingMode>,
) -> BTreeMap<String, crate::model::ModelThinkingMode> {
    merge_catalog_baseline_modes(primary, baseline, |mode, configured| {
        merge_catalog_baseline_mode(mode, configured, |primary, baseline| {
            fill_missing_option(&mut primary.thinking, &baseline.thinking);
        })
    })
}

pub(crate) fn merge_catalog_baseline_speed_modes(
    primary: BTreeMap<String, crate::model::ModelSpeedMode>,
    baseline: &BTreeMap<String, ConfiguredModelSpeedMode>,
) -> BTreeMap<String, crate::model::ModelSpeedMode> {
    merge_catalog_baseline_modes(primary, baseline, |mode, configured| {
        merge_catalog_baseline_mode(mode, configured, |_primary, _baseline| {})
    })
}

fn catalog_definition_for_model_id<'a>(
    provider_record: &'a ModelCatalogProviderRecord,
    raw_model_id: &str,
) -> Option<&'a CatalogModelDefinition> {
    provider_record.models.get(raw_model_id).or_else(|| {
        catalog_match_model_id_for_raw(raw_model_id)
            .as_ref()
            .and_then(|catalog_model_id| provider_record.models.get(catalog_model_id))
    })
}

fn catalog_match_model_id_for_raw(raw_model_id: &str) -> Option<String> {
    let canonical = canonical_model_catalog_id(raw_model_id);
    let trimmed = canonical.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
