use super::*;
use crate::model::ModelMetadata;

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
            .with_fallbacks_from(capability_fallback)
    };
    model.capabilities = primary_capabilities.with_fallbacks_from(&catalog_capabilities);

    let catalog_metadata = configured.metadata();
    let primary_metadata = model
        .metadata
        .clone()
        .with_fallbacks_from(metadata_fallback);
    model.metadata = primary_metadata.with_fallbacks_from(&catalog_metadata);

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
        let base = Model::new(provider.id(), model_id.as_str())
            .with_catalog_model_id(model_id.as_str())
            .with_capabilities(provider.model_capabilities_for_adapter(None, &model_id))
            .with_metadata(provider_model_metadata(provider, None, &model_id))
            .with_thinking_modes(provider_model_thinking_modes(
                provider,
                provider_record,
                None,
                &model_id,
            ))
            .with_speed_modes(provider_model_speed_modes(
                provider,
                provider_record,
                None,
                &model_id,
            ));
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

pub(crate) fn merge_catalog_baseline_thinking_modes(
    mut primary: BTreeMap<String, crate::model::ModelThinkingMode>,
    baseline: &BTreeMap<String, ConfiguredModelThinkingMode>,
) -> BTreeMap<String, crate::model::ModelThinkingMode> {
    for (name, configured) in baseline {
        if configured.disabled {
            continue;
        }
        match primary.remove(name) {
            Some(mode) => {
                primary.insert(
                    name.clone(),
                    merge_catalog_baseline_thinking_mode(mode, configured),
                );
            }
            None => {
                if let Some(mode) = configured.apply_to_mode(None) {
                    primary.insert(name.clone(), mode);
                }
            }
        }
    }
    primary
}

pub(crate) fn merge_catalog_baseline_speed_modes(
    mut primary: BTreeMap<String, crate::model::ModelSpeedMode>,
    baseline: &BTreeMap<String, ConfiguredModelSpeedMode>,
) -> BTreeMap<String, crate::model::ModelSpeedMode> {
    for (name, configured) in baseline {
        if configured.disabled {
            continue;
        }
        match primary.remove(name) {
            Some(mode) => {
                primary.insert(
                    name.clone(),
                    merge_catalog_baseline_speed_mode(mode, configured),
                );
            }
            None => {
                if let Some(mode) = configured.apply_to_mode(None) {
                    primary.insert(name.clone(), mode);
                }
            }
        }
    }
    primary
}

fn merge_catalog_baseline_thinking_mode(
    mut primary: crate::model::ModelThinkingMode,
    baseline: &ConfiguredModelThinkingMode,
) -> crate::model::ModelThinkingMode {
    if let Some(mode) = baseline.apply_to_mode(None) {
        if primary.display_name.is_none() {
            primary.display_name = mode.display_name;
        }
        if primary.description.is_none() {
            primary.description = mode.description;
        }
        if primary.thinking.is_none() {
            primary.thinking = mode.thinking;
        }
        primary.request_override = mode.request_override.merged_with(&primary.request_override);
        for (adapter_id, override_patch) in mode.adapter_overrides {
            match primary.adapter_overrides.remove(adapter_id.as_str()) {
                Some(existing) => {
                    primary
                        .adapter_overrides
                        .insert(adapter_id, override_patch.merged_with(&existing));
                }
                None => {
                    primary.adapter_overrides.insert(adapter_id, override_patch);
                }
            }
        }
    }
    primary
}

fn merge_catalog_baseline_speed_mode(
    mut primary: crate::model::ModelSpeedMode,
    baseline: &ConfiguredModelSpeedMode,
) -> crate::model::ModelSpeedMode {
    if let Some(mode) = baseline.apply_to_mode(None) {
        if primary.display_name.is_none() {
            primary.display_name = mode.display_name;
        }
        if primary.description.is_none() {
            primary.description = mode.description;
        }
        primary.request_override = mode.request_override.merged_with(&primary.request_override);
        for (adapter_id, override_patch) in mode.adapter_overrides {
            match primary.adapter_overrides.remove(adapter_id.as_str()) {
                Some(existing) => {
                    primary
                        .adapter_overrides
                        .insert(adapter_id, override_patch.merged_with(&existing));
                }
                None => {
                    primary.adapter_overrides.insert(adapter_id, override_patch);
                }
            }
        }
    }
    primary
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_display_name_only_fills_missing_value() {
        let definition = CatalogModelDefinition {
            display_name: Some("Catalog".to_owned()),
            ..CatalogModelDefinition::default()
        };

        let mut live_model = Model::new("test", "model").with_display_name("Live");
        apply_catalog_display_name_as_fallback(&mut live_model, &definition);
        assert_eq!(live_model.display_name.as_deref(), Some("Live"));

        let mut missing_display_name = Model::new("test", "model");
        apply_catalog_display_name_as_fallback(&mut missing_display_name, &definition);
        assert_eq!(
            missing_display_name.display_name.as_deref(),
            Some("Catalog")
        );
    }
}
