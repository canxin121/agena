//! Catalog baseline decoration over a runtime-neutral provider view.

use std::collections::{BTreeMap, BTreeSet};

use agena_domain::{
    AdapterId, Model, ModelCapabilities, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode,
    ProviderId,
};

use crate::{
    CatalogModelDefinition, ModelCatalogProviderRecord, apply_catalog_definition_as_baseline,
    catalog_definition_to_provider_definition, merge_catalog_baseline_speed_modes,
    merge_catalog_baseline_thinking_modes,
};

/// The small provider-specific read view required by catalog decoration.
/// Concrete SDK/runtime implementations adapt themselves at composition time.
pub trait CatalogModelDecorationSource: Send + Sync {
    fn provider_id(&self) -> &str;
    fn catalog_model_id_for_raw(&self, raw_model_id: &str) -> Option<String>;
    fn native_compaction_enabled_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool;
    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities;
    fn model_metadata_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelMetadata;
    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Vec<ModelThinkingMode>;
    fn model_speed_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelSpeedMode>;
}

/// Merge configured catalog baselines into models returned by a provider.
pub fn decorate_provider_models(
    provider: &dyn CatalogModelDecorationSource,
    provider_record: &ModelCatalogProviderRecord,
    mut models: Vec<Model>,
) -> Vec<Model> {
    let mut listed = BTreeSet::new();
    let mut listed_catalog_ids = BTreeSet::new();
    for model in &mut models {
        listed.insert(model.id.to_string());
        if let Some(catalog_model_id) = provider.catalog_model_id_for_raw(model.id.as_ref()) {
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
                model.id.as_ref() == model_id
                    || model.catalog_model_id.as_ref().map(AsRef::<str>::as_ref)
                        == Some(model_id.as_str())
            })
        {
            continue;
        }
        let model_id = ModelId::new(model_id.clone());
        let base = Model {
            provider_id: ProviderId::new(provider.provider_id()),
            adapter_id: None,
            id: ModelId::new(model_id.as_ref()),
            catalog_model_id: Some(model_id.clone()),
            display_name: None,
            native_compaction: provider.native_compaction_enabled_for_adapter(None, &model_id),
            capabilities: provider.model_capabilities_for_adapter(None, &model_id),
            metadata: provider.model_metadata_for_adapter(None, &model_id),
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
    provider: &dyn CatalogModelDecorationSource,
    provider_record: &ModelCatalogProviderRecord,
    model_id: ModelId,
    mut model: Model,
) -> Model {
    if let Some(catalog_model_id) = provider.catalog_model_id_for_raw(model_id.as_ref()) {
        model.catalog_model_id = Some(ModelId::new(catalog_model_id));
    }
    if let Some(definition) =
        catalog_definition_for_model_id(provider, provider_record, model_id.as_ref())
    {
        apply_catalog_display_name_as_fallback(&mut model, definition);
        let adapter_id = model.adapter_id.clone();
        apply_catalog_definition_as_baseline(
            definition,
            &provider.model_capabilities_for_adapter(adapter_id.as_ref(), &model_id),
            &provider.model_metadata_for_adapter(adapter_id.as_ref(), &model_id),
            model,
        )
    } else {
        model
    }
}

fn apply_catalog_display_name_as_fallback(model: &mut Model, definition: &CatalogModelDefinition) {
    if model.display_name.is_none() {
        model.display_name = definition.display_name.clone();
    }
}
fn provider_model_thinking_modes(
    provider: &dyn CatalogModelDecorationSource,
    provider_record: &ModelCatalogProviderRecord,
    adapter_id: Option<&AdapterId>,
    model: &ModelId,
) -> Vec<ModelThinkingMode> {
    let modes = provider.model_thinking_modes_for_adapter(adapter_id, model);
    match catalog_definition_for_model_id(provider, provider_record, model.as_ref()) {
        Some(definition) => merge_catalog_baseline_thinking_modes(
            modes,
            &catalog_definition_to_provider_definition(definition).thinking_modes,
        ),
        None => modes,
    }
}
fn provider_model_speed_modes(
    provider: &dyn CatalogModelDecorationSource,
    provider_record: &ModelCatalogProviderRecord,
    adapter_id: Option<&AdapterId>,
    model: &ModelId,
) -> BTreeMap<String, ModelSpeedMode> {
    let modes = provider.model_speed_modes_for_adapter(adapter_id, model);
    match catalog_definition_for_model_id(provider, provider_record, model.as_ref()) {
        Some(definition) => merge_catalog_baseline_speed_modes(
            modes,
            &catalog_definition_to_provider_definition(definition).speed_modes,
        ),
        None => modes,
    }
}
fn catalog_definition_for_model_id<'a>(
    provider: &'a dyn CatalogModelDecorationSource,
    provider_record: &'a ModelCatalogProviderRecord,
    raw_model_id: &str,
) -> Option<&'a CatalogModelDefinition> {
    provider_record.models.get(raw_model_id).or_else(|| {
        provider
            .catalog_model_id_for_raw(raw_model_id)
            .as_ref()
            .and_then(|id| provider_record.models.get(id))
    })
}
