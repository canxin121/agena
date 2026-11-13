//! Catalog baseline decoration over a runtime-neutral provider view.

use std::collections::{BTreeMap, BTreeSet};

use agena_domain::{
    AdapterId, Model, ModelCapabilities, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode,
    ProviderId,
};

use crate::{
    CatalogModelDefinition, ConfiguredModelDefinition, ModelCatalogProviderRecord,
    apply_configured_definition_as_baseline, catalog_definition_to_provider_definition,
    merge_catalog_baseline_speed_modes, merge_catalog_baseline_thinking_modes,
};

/// The small provider-specific read view required by catalog decoration.
/// Concrete SDK/runtime implementations adapt themselves at composition time.
pub trait CatalogModelDecorationSource: Send + Sync {
    fn provider_id(&self) -> &str;
    fn catalog_source_id(&self) -> String {
        self.provider_id().to_owned()
    }
    fn catalog_model_id_for_raw(&self, raw_model_id: &str) -> Option<String>;
    /// Catalog entry this route was explicitly matched to, if any. A pinned
    /// entry outranks the automatic match derived from the model id, so a
    /// corrected match keeps applying to the route it was chosen for.
    fn pinned_catalog_model_id(
        &self,
        _adapter_id: Option<&AdapterId>,
        _model: &ModelId,
    ) -> Option<String> {
        None
    }
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

/// Merge the catalog entry a route resolves to into models returned by a
/// provider.
///
/// Precedence is the same for every route: the route's own saved fields win,
/// then the catalog entry supplies everything the route leaves unset, then the
/// provider's live listing fills whatever the catalog does not define. Writing
/// only `catalog_model_id` therefore mirrors that entry and keeps tracking a
/// catalog refresh; adding fields next to it overrides just those fields.
pub fn decorate_provider_models(
    provider: &dyn CatalogModelDecorationSource,
    provider_record: &ModelCatalogProviderRecord,
    mut models: Vec<Model>,
) -> Vec<Model> {
    let mut listed = BTreeSet::new();
    let mut listed_catalog_ids = BTreeSet::new();
    for model in &mut models {
        listed.insert(model.id.to_string());
        let catalog_model_id = provider
            .pinned_catalog_model_id(model.adapter_id.as_ref(), &model.id)
            .or_else(|| provider.catalog_model_id_for_raw(model.id.as_ref()));
        if let Some(catalog_model_id) = catalog_model_id {
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
    // An explicit match wins over the automatic one so a corrected catalog
    // selection keeps addressing this route instead of being re-derived from
    // its provider model id on every listing.
    let adapter_id = model.adapter_id.clone();
    let catalog_model_id = provider
        .pinned_catalog_model_id(adapter_id.as_ref(), &model_id)
        .or_else(|| provider.catalog_model_id_for_raw(model_id.as_ref()));
    if let Some(catalog_model_id) = catalog_model_id {
        model.catalog_model_id = Some(ModelId::new(catalog_model_id));
    }
    if let Some(definition) =
        route_catalog_definition(provider, provider_record, adapter_id.as_ref(), &model_id)
    {
        apply_catalog_display_name_as_fallback(&mut model, definition);
        let capabilities = provider.model_capabilities_for_adapter(adapter_id.as_ref(), &model_id);
        let metadata = provider.model_metadata_for_adapter(adapter_id.as_ref(), &model_id);
        let configured = configured_definition_for_provider(definition, provider);
        apply_configured_definition_as_baseline(&configured, &capabilities, &metadata, model)
    } else {
        model
    }
}

fn configured_definition_for_provider(
    definition: &CatalogModelDefinition,
    provider: &dyn CatalogModelDecorationSource,
) -> ConfiguredModelDefinition {
    let mut configured = definition.clone().into_configured_definition();
    // Global catalog values must never leak onto a different provider route.
    // The user's request default remains in provider configuration.
    configured.supports_verbosity = None;
    configured.default_verbosity = None;
    configured.verbosity_levels.clear();
    configured.provider_default_verbosity = None;
    if let Some(profile) = definition
        .verbosity_by_provider
        .get(&provider.catalog_source_id())
        && profile.supported
    {
        configured.supports_verbosity = Some(true);
        configured.verbosity_levels = profile.levels.clone();
        configured.provider_default_verbosity = profile.provider_default.clone();
    }
    configured
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
    match route_catalog_definition(provider, provider_record, adapter_id, model) {
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
    match route_catalog_definition(provider, provider_record, adapter_id, model) {
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

/// Resolve the catalog definition for one route, honouring an explicit match.
///
/// A route pinned to a catalog entry always reads that entry. Otherwise the
/// raw model id wins, then its canonical form, so the automatic match behaves
/// exactly as it did before pins existed.
fn route_catalog_definition<'a>(
    provider: &'a dyn CatalogModelDecorationSource,
    provider_record: &'a ModelCatalogProviderRecord,
    adapter_id: Option<&AdapterId>,
    model_id: &ModelId,
) -> Option<&'a CatalogModelDefinition> {
    if let Some(pinned) = provider.pinned_catalog_model_id(adapter_id, model_id) {
        return provider_record.models.get(pinned.as_str());
    }
    catalog_definition_for_model_id(provider, provider_record, model_id.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CatalogModelDefinition, ModelCatalogProviderRecord};
    use std::collections::BTreeMap;

    struct Fixture {
        pinned: Option<String>,
    }

    impl CatalogModelDecorationSource for Fixture {
        fn provider_id(&self) -> &str {
            "fixture"
        }
        fn catalog_model_id_for_raw(&self, raw_model_id: &str) -> Option<String> {
            Some(raw_model_id.to_owned())
        }
        fn pinned_catalog_model_id(
            &self,
            _adapter_id: Option<&AdapterId>,
            _model: &ModelId,
        ) -> Option<String> {
            self.pinned.clone()
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
            ModelMetadata::default()
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

    fn record() -> ModelCatalogProviderRecord {
        let mut models = BTreeMap::new();
        models.insert(
            "route-model".to_owned(),
            CatalogModelDefinition {
                display_name: Some("Automatic name".to_owned()),
                ..Default::default()
            },
        );
        models.insert(
            "chosen-entry".to_owned(),
            CatalogModelDefinition {
                display_name: Some("Chosen name".to_owned()),
                ..Default::default()
            },
        );
        ModelCatalogProviderRecord {
            models,
            appendable_model_ids: Default::default(),
        }
    }

    fn decorated_name(pinned: Option<&str>) -> Option<String> {
        let fixture = Fixture {
            pinned: pinned.map(ToOwned::to_owned),
        };
        let base = Model::new("fixture", "route-model");
        let decorated = decorate_provider_models(&fixture, &record(), vec![base]);
        decorated
            .into_iter()
            .next()
            .and_then(|model| model.display_name)
    }

    #[test]
    fn an_unpinned_route_uses_the_catalog_match_for_its_model_id() {
        assert_eq!(decorated_name(None).as_deref(), Some("Automatic name"));
    }

    #[test]
    fn a_pinned_route_reads_the_entry_it_was_matched_to() {
        assert_eq!(
            decorated_name(Some("chosen-entry")).as_deref(),
            Some("Chosen name")
        );
    }
}
