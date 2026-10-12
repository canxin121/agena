//! Catalog baseline decoration over a runtime-neutral provider view.

use std::collections::{BTreeMap, BTreeSet};

use agena_domain::{
    AdapterId, Model, ModelCapabilities, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode,
    ProviderId,
};

use crate::{
    CatalogModelDefinition, ModelCatalogProviderRecord, apply_catalog_definition_as_authority,
    apply_catalog_definition_as_baseline, catalog_definition_to_provider_definition,
    merge_catalog_baseline_speed_modes, merge_catalog_baseline_thinking_modes,
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
///
/// Routes that follow the catalog (the default) take every value the catalog
/// defines, so a catalog refresh lands on the next listing. A route listed in
/// `manual_overrides` keeps its own values and the catalog only fills gaps.
pub fn decorate_provider_models(
    provider: &dyn CatalogModelDecorationSource,
    provider_record: &ModelCatalogProviderRecord,
    manual_overrides: &BTreeSet<String>,
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
        *model = decorate_provider_model(
            provider,
            provider_record,
            manual_overrides,
            model.id.clone(),
            model.clone(),
        );
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
            manual_overrides,
            model_id,
            base,
        ));
    }
    models
}

fn decorate_provider_model(
    provider: &dyn CatalogModelDecorationSource,
    provider_record: &ModelCatalogProviderRecord,
    manual_overrides: &BTreeSet<String>,
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
        let capabilities = provider.model_capabilities_for_adapter(adapter_id.as_ref(), &model_id);
        let metadata = provider.model_metadata_for_adapter(adapter_id.as_ref(), &model_id);
        if manual_overrides.contains(model_id.as_ref())
            || model
                .catalog_model_id
                .as_ref()
                .is_some_and(|id| manual_overrides.contains(id.as_ref()))
        {
            apply_catalog_definition_as_baseline(definition, &capabilities, &metadata, model)
        } else {
            apply_catalog_definition_as_authority(definition, &capabilities, &metadata, model)
        }
    } else {
        model
    }
}

fn apply_catalog_display_name_as_fallback(model: &mut Model, definition: &CatalogModelDefinition) {
    if let Some(display_name) = &definition.display_name
        && model
            .display_name
            .as_deref()
            .is_none_or(|current| current == model.id.as_ref())
    {
        model.display_name = Some(display_name.clone());
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

#[cfg(test)]
mod tests {
    use super::apply_catalog_display_name_as_fallback;
    use crate::{CatalogModelDefinition, ModelCatalogProviderRecord, decorate_provider_models};
    use agena_domain::{
        AdapterId, Model, ModelCapabilities, ModelId, ModelMetadata, ModelSpeedMode,
        ModelThinkingMode,
    };
    use std::collections::{BTreeMap, BTreeSet};

    struct StubSource;

    impl crate::CatalogModelDecorationSource for StubSource {
        fn provider_id(&self) -> &str {
            "openai"
        }

        fn catalog_model_id_for_raw(&self, raw_model_id: &str) -> Option<String> {
            Some(raw_model_id.to_owned())
        }

        fn native_compaction_enabled_for_adapter(
            &self,
            _adapter_id: Option<&AdapterId>,
            _model: &ModelId,
        ) -> bool {
            true
        }

        fn model_capabilities_for_adapter(
            &self,
            _adapter_id: Option<&AdapterId>,
            _model: &ModelId,
        ) -> ModelCapabilities {
            ModelCapabilities::default()
        }

        fn model_metadata_for_adapter(
            &self,
            _adapter_id: Option<&AdapterId>,
            _model: &ModelId,
        ) -> ModelMetadata {
            let mut metadata = ModelMetadata::default();
            metadata.limits.context_window_tokens = Some(900_000);
            metadata
        }

        fn model_thinking_modes_for_adapter(
            &self,
            _adapter_id: Option<&AdapterId>,
            _model: &ModelId,
        ) -> Vec<ModelThinkingMode> {
            Vec::new()
        }

        fn model_speed_modes_for_adapter(
            &self,
            _adapter_id: Option<&AdapterId>,
            _model: &ModelId,
        ) -> BTreeMap<String, ModelSpeedMode> {
            BTreeMap::new()
        }
    }

    fn catalog_record() -> ModelCatalogProviderRecord {
        ModelCatalogProviderRecord {
            models: BTreeMap::from([(
                "gpt-6.1-sol".to_owned(),
                CatalogModelDefinition {
                    context_window_tokens: Some(1_050_000),
                    ..Default::default()
                },
            )]),
            appendable_model_ids: Default::default(),
        }
    }

    fn route_model() -> Model {
        let mut model = Model::new("openai", "gpt-6.1-sol");
        model.metadata.limits.context_window_tokens = Some(900_000);
        model
    }

    #[test]
    fn a_listed_route_takes_the_refreshed_catalog_values() {
        let decorated = decorate_provider_models(
            &StubSource,
            &catalog_record(),
            &BTreeSet::new(),
            vec![route_model()],
        );

        assert_eq!(
            decorated[0].metadata.limits.context_window_tokens,
            Some(1_050_000),
            "a route that follows the catalog must pick up catalog updates"
        );
    }

    #[test]
    fn a_manual_route_keeps_its_own_values() {
        let manual = BTreeSet::from(["gpt-6.1-sol".to_owned()]);
        let decorated =
            decorate_provider_models(&StubSource, &catalog_record(), &manual, vec![route_model()]);

        assert_eq!(
            decorated[0].metadata.limits.context_window_tokens,
            Some(900_000),
            "manual_overrides must keep the route's own value"
        );
    }

    #[test]
    fn catalog_name_replaces_raw_id_placeholder_but_keeps_provider_name() {
        let definition = CatalogModelDefinition {
            display_name: Some("DeepSeek V4.1 Flash".to_owned()),
            ..Default::default()
        };
        let mut placeholder = Model::new("cline", "cline-pass/deepseek-v4.1-flash");
        placeholder.display_name = Some(placeholder.id.to_string());
        apply_catalog_display_name_as_fallback(&mut placeholder, &definition);
        assert_eq!(
            placeholder.display_name.as_deref(),
            Some("DeepSeek V4.1 Flash")
        );

        let mut provider_named = Model::new("cline", "cline-pass/deepseek-v4.1-flash");
        provider_named.display_name = Some("Provider label".to_owned());
        apply_catalog_display_name_as_fallback(&mut provider_named, &definition);
        assert_eq!(
            provider_named.display_name.as_deref(),
            Some("Provider label")
        );
    }
}
