use super::*;

pub fn catalog_definition_to_provider_definition(
    definition: &CatalogModelDefinition,
) -> ConfiguredModelDefinition {
    definition.clone().into_configured_definition()
}

pub fn decorate_provider_models(
    provider: &dyn ModelProvider,
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
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    model_id: ModelId,
    mut model: Model,
) -> Model {
    let matched_catalog_id = catalog_match_model_id_for_raw(model_id.as_str());
    if let Some(catalog_model_id) = matched_catalog_id {
        model.catalog_model_id = Some(ModelId::new(catalog_model_id));
    }

    if let Some(display_name) = catalog_definition_for_model_id(provider_record, model_id.as_str())
        .and_then(|definition| definition.display_name.clone())
    {
        model.display_name = Some(display_name);
    }

    if let Some(configured) = catalog_definition_for_model_id(provider_record, model_id.as_str())
        .cloned()
        .map(|definition| catalog_definition_to_provider_definition(&definition))
    {
        let adapter_id = model.adapter_id.clone();
        configured.apply_to_model(
            model,
            &provider.model_capabilities_for_adapter(adapter_id.as_ref(), &model_id),
            &provider_model_metadata(provider, adapter_id.as_ref(), &model_id),
        )
    } else {
        model
    }
}

fn provider_model_metadata(
    provider: &dyn ModelProvider,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> crate::model::ModelMetadata {
    provider.model_metadata_for_adapter(adapter_id, model)
}

fn provider_model_thinking_modes(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> BTreeMap<String, crate::model::ModelThinkingMode> {
    let mut modes = provider.model_thinking_modes_for_adapter(adapter_id, model);
    if let Some(configured) = catalog_definition_for_model_id(provider_record, model.as_str())
        .cloned()
        .map(|definition| catalog_definition_to_provider_definition(&definition))
    {
        for (name, configured_mode) in &configured.thinking_modes {
            match configured_mode.apply_to_mode(modes.get(name)) {
                Some(mode) => {
                    modes.insert(name.clone(), mode);
                }
                None => {
                    modes.remove(name);
                }
            }
        }
    }
    modes
}

fn provider_model_speed_modes(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> BTreeMap<String, crate::model::ModelSpeedMode> {
    let mut modes = provider.model_speed_modes_for_adapter(adapter_id, model);
    if let Some(configured) = catalog_definition_for_model_id(provider_record, model.as_str())
        .cloned()
        .map(|definition| catalog_definition_to_provider_definition(&definition))
    {
        for (name, configured_mode) in &configured.speed_modes {
            match configured_mode.apply_to_mode(modes.get(name)) {
                Some(mode) => {
                    modes.insert(name.clone(), mode);
                }
                None => {
                    modes.remove(name);
                }
            }
        }
    }
    modes
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
