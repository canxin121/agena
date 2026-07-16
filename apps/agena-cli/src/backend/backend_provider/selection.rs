use anyhow::{Context, anyhow};
use serde_json::json;

use crate::backend::Result;
use crate::backend::{
    Backend, CatalogModelResource, ConfigSettingsGetInput, ConfigSettingsPathInput, InspectorRow,
    JsonMap, JsonValue, ModelCapabilities, ModelCatalogListResponse, ModelCatalogProviderRecord,
    ModelId, ModelMetadata, ModelRef, ProviderAdapterModelsResource, ProviderAdapterModelsResponse,
    ProviderConfigDraft, ProviderDraftAuthActionResult, ProviderDraftAuthError, ProviderId,
    ProviderModel, ProviderStudioSaveError, ProviderStudioSaveField, ProviderStudioSaveResult,
    RunOptions, apply_provider_auth_required_adapter_defaults_to_json_adapters,
    apply_provider_tools_defaults_to_model_value, build_provider_auth_patch_value_for_save,
    build_provider_patch_value_for_save, catalog_lookup_id_for_provider_model,
    continue_provider_draft_auth, decorate_provider_models, ensure_provider_model_entry,
    local_model_catalog_model_search_text, local_model_catalog_models, local_model_catalog_summary,
    map_provider_adapter_models_config_error, normalize_limit, optional_non_empty,
    preferred_catalog_model_for_provider_model, provider_model_json_for_model_id,
    provider_model_selection_contains, provider_model_settings_path, read_file_setting,
    required_provider_save_field, required_trimmed, resolve_provider_defaults_from_value_for_save,
    saved_provider_adapter_models_target, start_provider_draft_auth, summarize_named_mode,
};

impl Backend {
    pub fn provider_config_draft(&self, provider_id: Option<&str>) -> Result<ProviderConfigDraft> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            let mut draft = ProviderConfigDraft::new_empty();
            draft.normalize_shape();
            return Ok(draft);
        };

        let snapshot = self.runtime.current_snapshot();
        let provider = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow!("provider not found: {provider_id}"))?;
        Ok(ProviderConfigDraft::from_resolved(provider_id, provider))
    }

    pub async fn start_provider_draft_auth(
        &self,
        draft: ProviderConfigDraft,
    ) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
        start_provider_draft_auth(draft).await
    }

    pub async fn continue_provider_draft_auth(
        &self,
        draft: ProviderConfigDraft,
    ) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
        continue_provider_draft_auth(draft).await
    }

    pub(super) fn configured_provider_adapter_ids(
        &self,
        provider_id: Option<&str>,
    ) -> std::collections::BTreeSet<String> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return std::collections::BTreeSet::new();
        };
        let snapshot = self.runtime.current_snapshot();
        snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
            .map(|provider| provider.adapters.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn configured_provider_model_routes(
        &self,
        provider_id: Option<&str>,
    ) -> Vec<(String, String)> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Vec::new();
        };
        let snapshot = self.runtime.current_snapshot();
        let Some(provider) = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
        else {
            return Vec::new();
        };
        provider
            .models
            .keys()
            .filter_map(|route| {
                route
                    .split_once('/')
                    .filter(|(adapter_id, _)| {
                        provider
                            .adapters
                            .get(*adapter_id)
                            .map(|adapter| adapter.enabled)
                            .unwrap_or(false)
                    })
                    .map(|(adapter_id, model_id)| (adapter_id.to_owned(), model_id.to_owned()))
            })
            .collect()
    }

    pub fn configured_provider_adapter_models(
        &self,
        provider_id: Option<&str>,
    ) -> Vec<ProviderAdapterModelsResource> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Vec::new();
        };
        let snapshot = self.runtime.current_snapshot();
        let Some(provider) = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
        else {
            return Vec::new();
        };

        let mut adapter_ids = provider.adapters.keys().cloned().collect::<Vec<_>>();
        adapter_ids.sort();
        adapter_ids
            .into_iter()
            .map(|adapter_id| {
                let mut model_ids = provider
                    .models
                    .keys()
                    .filter_map(|route| {
                        route
                            .split_once('/')
                            .and_then(|(route_adapter_id, model_id)| {
                                (route_adapter_id == adapter_id).then(|| model_id.to_owned())
                            })
                    })
                    .collect::<Vec<_>>();
                model_ids.sort();
                ProviderAdapterModelsResource {
                    adapter_id: adapter_id.clone(),
                    enabled: provider
                        .adapters
                        .get(adapter_id.as_str())
                        .map(|adapter| adapter.enabled)
                        .unwrap_or(true),
                    resolved_base_url: None,
                    models: model_ids
                        .into_iter()
                        .map(|model_id| ProviderModel::new(adapter_id.as_str(), model_id))
                        .collect(),
                    error: None,
                }
            })
            .collect()
    }

    pub(super) fn effective_provider_draft_adapter_ids(
        &self,
        draft: &ProviderConfigDraft,
        extra_adapter_ids: &[String],
    ) -> std::collections::BTreeSet<String> {
        let mut adapter_ids =
            self.configured_provider_adapter_ids(draft.source_provider_id.as_deref());
        adapter_ids.extend(
            extra_adapter_ids
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        );
        if let Some(default_adapter) = optional_non_empty(draft.default_adapter.as_str()) {
            adapter_ids.insert(default_adapter.to_owned());
        }
        adapter_ids
    }

    pub fn list_local_provider_models(&self, provider_id: &str) -> Result<Vec<ProviderModel>> {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            return Ok(Vec::new());
        }

        let snapshot = self.runtime.current_snapshot();
        let Some(configured) = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
        else {
            return Ok(Vec::new());
        };

        let mut enabled_adapter_ids = configured
            .adapters
            .iter()
            .filter(|(_, adapter)| adapter.enabled)
            .map(|(adapter_id, _)| adapter_id.clone())
            .collect::<Vec<_>>();
        enabled_adapter_ids.sort();

        let default_adapter = configured
            .defaults
            .adapter
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|adapter_id| {
                configured
                    .adapters
                    .get(*adapter_id)
                    .map(|adapter| adapter.enabled)
                    .unwrap_or(false)
            })
            .map(ToOwned::to_owned)
            .or_else(|| (enabled_adapter_ids.len() == 1).then(|| enabled_adapter_ids[0].clone()));

        let mut seen = std::collections::BTreeSet::new();
        let mut models = Vec::new();

        for route in configured.models.keys() {
            let Some((adapter_id, model_id)) = route.split_once('/') else {
                continue;
            };
            let adapter_id = adapter_id.trim();
            let model_id = model_id.trim();
            if adapter_id.is_empty()
                || model_id.is_empty()
                || !configured
                    .adapters
                    .get(adapter_id)
                    .map(|adapter| adapter.enabled)
                    .unwrap_or(false)
            {
                continue;
            }
            if seen.insert((adapter_id.to_owned(), model_id.to_owned())) {
                models.push(ProviderModel {
                    provider_id: ProviderId::new(provider_id),
                    adapter_id: Some(agena::model::AdapterId::new(adapter_id)),
                    id: ModelId::new(model_id),
                    catalog_model_id: None,
                    display_name: None,
                    capabilities: ModelCapabilities::default(),
                    metadata: ModelMetadata::default(),
                    thinking_modes: std::collections::BTreeMap::new(),
                    speed_modes: std::collections::BTreeMap::new(),
                });
            }
        }

        if let Some(default_model) = configured
            .defaults
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let default_key = (
                default_adapter.clone().unwrap_or_default(),
                default_model.to_owned(),
            );
            if seen.insert(default_key) {
                let model = default_adapter
                    .as_deref()
                    .map(|adapter_id| ProviderModel {
                        provider_id: ProviderId::new(provider_id),
                        adapter_id: Some(agena::model::AdapterId::new(adapter_id)),
                        id: ModelId::new(default_model),
                        catalog_model_id: None,
                        display_name: None,
                        capabilities: ModelCapabilities::default(),
                        metadata: ModelMetadata::default(),
                        thinking_modes: std::collections::BTreeMap::new(),
                        speed_modes: std::collections::BTreeMap::new(),
                    })
                    .unwrap_or_else(|| ProviderModel::new(provider_id, default_model));
                models.push(model);
            }
        }

        let provider = snapshot
            .provider_registry()
            .get(provider_id)
            .ok_or_else(|| anyhow!("provider not found: {provider_id}"))?;
        let provider_record = snapshot
            .model_catalog()
            .effective_provider_record(&enabled_adapter_ids)
            .unwrap_or_default();
        let local_provider_record = ModelCatalogProviderRecord {
            models: provider_record.models,
            appendable_model_ids: Default::default(),
        };

        Ok(decorate_provider_models(
            provider.as_ref(),
            &local_provider_record,
            models,
        ))
    }

    pub fn list_model_catalog_models(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<ModelCatalogListResponse> {
        let snapshot = self.runtime.current_snapshot();
        let catalog = snapshot.model_catalog_response();
        let summary = local_model_catalog_summary(&catalog);
        let models = local_model_catalog_models(&catalog);
        let search = query.trim().to_lowercase();
        let available_origins = {
            let mut origins = models
                .iter()
                .filter_map(|model| {
                    let origin = model.origin.clone().unwrap_or_default();
                    let trimmed = origin.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_owned())
                })
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            origins.sort();
            origins
        };
        let filtered = models
            .into_iter()
            .filter(|model| {
                search.is_empty() || local_model_catalog_model_search_text(model).contains(&search)
            })
            .collect::<Vec<_>>();
        let total = filtered.len();
        let limit = normalize_limit(Some(limit as u64)) as usize;
        let items = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        Ok(ModelCatalogListResponse {
            summary,
            total,
            offset,
            limit,
            available_origins,
            items,
        })
    }

    pub fn lookup_model_catalog_models(&self, model_ids: &[String]) -> Vec<CatalogModelResource> {
        let requested = model_ids
            .iter()
            .flat_map(|model_id| {
                let raw = model_id.trim().to_owned();
                if raw.is_empty() {
                    return Vec::new();
                }
                let canonical = agena::model_catalog::canonical_model_catalog_id(raw.as_str());
                if canonical.is_empty() || canonical == raw {
                    vec![raw]
                } else {
                    vec![raw, canonical]
                }
            })
            .collect::<std::collections::BTreeSet<_>>();
        let snapshot = self.runtime.current_snapshot();
        let catalog = snapshot.model_catalog_response();
        local_model_catalog_models(&catalog)
            .into_iter()
            .filter(|model| requested.contains(model.model_id.as_str()))
            .collect()
    }

    pub fn resolved_model_for_run_options(&self, request: &RunOptions) -> Result<ModelRef> {
        if let Some(model) = request.model.as_ref() {
            return Ok(model.clone());
        }

        self.runtime
            .current_snapshot()
            .resolve_default_model()
            .context("failed to resolve default model selection")?
            .ok_or_else(|| anyhow!("no providers configured"))
    }

    pub fn runtime_thinking_mode_rows(&self, request: &RunOptions) -> Result<Vec<InspectorRow>> {
        let snapshot = self.runtime.current_snapshot();
        let registry = snapshot.provider_registry();
        let model = self.resolved_model_for_run_options(request)?;
        let mut rows = registry
            .model_thinking_modes(&model)
            .context("failed to resolve think modes for current model")?
            .into_iter()
            .map(|(name, mode)| InspectorRow {
                label: name,
                detail: summarize_named_mode(
                    mode.display_name.as_deref(),
                    mode.description.as_deref(),
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(rows)
    }

    pub fn runtime_speed_mode_rows(&self, request: &RunOptions) -> Result<Vec<InspectorRow>> {
        let snapshot = self.runtime.current_snapshot();
        let registry = snapshot.provider_registry();
        let model = self.resolved_model_for_run_options(request)?;
        let mut rows = registry
            .model_speed_modes(&model)
            .context("failed to resolve speed modes for current model")?
            .into_iter()
            .map(|(name, mode)| InspectorRow {
                label: name,
                detail: summarize_named_mode(
                    mode.display_name.as_deref(),
                    mode.description.as_deref(),
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(rows)
    }

    pub fn runtime_verbosity_values(&self, request: &RunOptions) -> Result<Vec<String>> {
        let snapshot = self.runtime.current_snapshot();
        let registry = snapshot.provider_registry();
        let model = self.resolved_model_for_run_options(request)?;
        let metadata = registry
            .model_metadata(&model)
            .context("failed to resolve verbosity metadata for current model")?;
        Ok(metadata.supported_verbosity_levels_for_model(&model.model_id))
    }

    pub async fn refresh_model_catalog(&self) -> Result<()> {
        let snapshot = self.runtime.current_snapshot();
        let source_providers = snapshot.catalog_source_provider_registry();
        snapshot
            .model_catalog()
            .refresh_from_registry(
                source_providers.as_ref(),
                Some(snapshot.config_resolution()),
            )
            .await
            .context("failed to refresh model catalog")?;
        Ok(())
    }

    pub async fn list_draft_provider_adapter_models(
        &self,
        draft: &ProviderConfigDraft,
        adapter_ids: &[String],
    ) -> Result<ProviderAdapterModelsResponse> {
        let mut draft = draft.clone();
        draft.normalize_shape();
        let target = draft.build_listing_target(adapter_ids)?;
        self.list_provider_adapter_models_with_target(target).await
    }

    pub async fn list_saved_provider_adapter_models(
        &self,
        provider_id: &str,
        adapter_ids: &[String],
    ) -> Result<ProviderAdapterModelsResponse> {
        let provider_id = provider_id.trim();
        let snapshot = self.runtime.current_snapshot();
        let resolved = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow!("provider not found: {provider_id}"))?;
        let target = saved_provider_adapter_models_target(provider_id, resolved, adapter_ids)
            .map_err(map_provider_adapter_models_config_error)?;
        self.list_provider_adapter_models_with_target(target).await
    }

    pub async fn save_provider_draft(
        &self,
        draft: ProviderConfigDraft,
        adapter_model_lists: &[ProviderAdapterModelsResource],
        selected_adapter_ids: &[String],
        selected_model_keys: &std::collections::BTreeSet<String>,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_provider_save_field(
            draft.provider_id.as_str(),
            ProviderStudioSaveField::ProviderId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let requested_default_adapter =
            optional_non_empty(draft.default_adapter.as_str()).map(str::to_owned);
        let requested_default_model =
            optional_non_empty(draft.default_model.as_str()).map(str::to_owned);
        let effective_adapter_ids = selected_adapter_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        draft
            .validate_for_adapters_for_save(&effective_adapter_ids)
            .map_err(ProviderStudioSaveError::Validation)?;

        let catalog_entries = self.lookup_model_catalog_models(
            &adapter_model_lists
                .iter()
                .flat_map(|adapter_models| {
                    adapter_models
                        .models
                        .iter()
                        .map(catalog_lookup_id_for_provider_model)
                })
                .chain(requested_default_model.iter().cloned())
                .collect::<Vec<_>>(),
        );
        let selected = selected_adapter_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<std::collections::BTreeSet<_>>();

        let mut provider_value = self
            .read_file_provider_settings(provider_id)
            .map_err(ProviderStudioSaveError::other)?
            .unwrap_or_else(|| JsonValue::Object(JsonMap::new()));
        let provider_object = provider_value
            .as_object_mut()
            .ok_or(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject)?;
        let mut adapters = provider_object
            .remove("adapters")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let mut known_adapter_ids =
            self.configured_provider_adapter_ids(draft.source_provider_id.as_deref());
        known_adapter_ids.extend(effective_adapter_ids.iter().cloned());
        apply_provider_adapter_selection(&mut adapters, &known_adapter_ids, &selected)?;

        for adapter_models in adapter_model_lists {
            let adapter_id = adapter_models.adapter_id.as_str();
            if adapter_models.error.is_some() || !selected.contains(adapter_id) {
                continue;
            }

            let mut adapter_value = adapters
                .remove(adapter_id)
                .unwrap_or_else(|| JsonValue::Object(JsonMap::new()));
            let adapter_object = adapter_value.as_object_mut().ok_or_else(|| {
                ProviderStudioSaveError::ProviderAdapterMustBeObject {
                    adapter_id: adapter_id.to_owned(),
                }
            })?;
            let configured_models = adapter_models
                .models
                .iter()
                .filter(|model| {
                    provider_model_selection_contains(
                        selected_model_keys,
                        adapter_id,
                        model.id.as_ref(),
                    )
                })
                .map(|model| {
                    let mut model_value = provider_model_json_for_model_id(
                        &catalog_entries,
                        model.id.as_ref(),
                        Some(model),
                    );
                    apply_provider_tools_defaults_to_model_value(
                        &draft,
                        adapter_id,
                        &mut model_value,
                    )?;
                    Ok((model.id.to_string(), model_value))
                })
                .collect::<std::result::Result<JsonMap<_, _>, ProviderStudioSaveError>>()?;
            adapter_object.insert("enabled".to_owned(), JsonValue::Bool(true));
            adapter_object.insert("models".to_owned(), JsonValue::Object(configured_models));
            adapters.insert(adapter_id.to_owned(), adapter_value);
        }

        apply_provider_auth_required_adapter_defaults_to_json_adapters(&draft, &mut adapters)?;

        let (default_adapter, default_model) = resolve_provider_defaults_from_value_for_save(
            &adapters,
            requested_default_adapter.as_deref(),
            requested_default_model.as_deref(),
        )?;

        if let Some(default_model) = default_model.as_deref() {
            let default_provider_model = adapter_model_lists
                .iter()
                .find(|adapter_models| adapter_models.adapter_id == default_adapter)
                .and_then(|adapter_models| {
                    adapter_models
                        .models
                        .iter()
                        .find(|model| model.id.as_ref() == default_model)
                        .cloned()
                });
            let default_model_value = provider_model_json_for_model_id(
                &catalog_entries,
                default_model,
                default_provider_model.as_ref(),
            );
            let mut default_model_value = default_model_value;
            apply_provider_tools_defaults_to_model_value(
                &draft,
                default_adapter.as_str(),
                &mut default_model_value,
            )?;
            adapters
                .entry(default_adapter.clone())
                .or_insert_with(|| json!({ "enabled": true }));
            ensure_provider_model_entry(
                adapters
                    .get_mut(default_adapter.as_str())
                    .expect("default adapter must exist"),
                default_model,
                default_model_value,
            )
            .map_err(ProviderStudioSaveError::other)?;
        }

        provider_object.insert("enabled".to_owned(), JsonValue::Bool(true));
        let mut defaults = JsonMap::new();
        defaults.insert(
            "adapter".to_owned(),
            JsonValue::String(default_adapter.clone()),
        );
        if let Some(default_model) = default_model.as_ref() {
            defaults.insert("model".to_owned(), JsonValue::String(default_model.clone()));
        }
        provider_object.insert("defaults".to_owned(), JsonValue::Object(defaults));
        provider_object.insert(
            "auth".to_owned(),
            JsonValue::Object(build_provider_auth_patch_value_for_save(&draft)?),
        );
        provider_object.insert("adapters".to_owned(), JsonValue::Object(adapters));
        self.set_provider_settings(provider_id, provider_value)
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::ProviderDraftSaved {
            provider_id: provider_id.to_owned(),
            default_adapter,
            default_model,
        })
    }

    pub async fn save_provider_adapter_matches(
        &self,
        draft: ProviderConfigDraft,
        adapter_models: ProviderAdapterModelsResource,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_provider_save_field(
            draft.provider_id.as_str(),
            ProviderStudioSaveField::ProviderId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let adapter_id = required_provider_save_field(
            adapter_models.adapter_id.as_str(),
            ProviderStudioSaveField::AdapterId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, &[adapter_id.to_owned()]);
        draft
            .validate_for_adapters_for_save(&effective_adapter_ids)
            .map_err(ProviderStudioSaveError::Validation)?;
        let catalog_entries = self.lookup_model_catalog_models(
            &adapter_models
                .models
                .iter()
                .map(catalog_lookup_id_for_provider_model)
                .collect::<Vec<_>>(),
        );
        let configured_models = adapter_models
            .models
            .iter()
            .map(|model| {
                let mut model_value = provider_model_json_for_model_id(
                    &catalog_entries,
                    model.id.as_ref(),
                    Some(model),
                );
                apply_provider_tools_defaults_to_model_value(&draft, adapter_id, &mut model_value)?;
                Ok((model.id.to_string(), model_value))
            })
            .collect::<std::result::Result<JsonMap<_, _>, ProviderStudioSaveError>>()?;
        let matched_model_count = adapter_models
            .models
            .iter()
            .filter(|model| {
                preferred_catalog_model_for_provider_model(&catalog_entries, model).is_some()
            })
            .count();
        let provider_patch =
            build_provider_adapter_matches_patch(&draft, adapter_id, configured_models)?;
        self.patch_provider_settings(provider_id, provider_patch)
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::AdapterMatchesSaved {
            provider_id: provider_id.to_owned(),
            adapter_id: adapter_id.to_owned(),
            listed_model_count: adapter_models.models.len(),
            matched_model_count,
        })
    }

    pub fn provider_model_draft_value(
        &self,
        draft: &ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        provider_model: Option<&ProviderModel>,
    ) -> Result<JsonValue> {
        let adapter_id = required_trimmed(adapter_id, "adapter_id")?;
        let model_id = required_trimmed(model_id, "model_id")?;
        if let Some(provider_id) = draft.source_provider_id.as_deref() {
            let path = provider_model_settings_path(provider_id, adapter_id, model_id);
            let configured = read_file_setting(
                self.runtime.config_resolution().meta.config_path.clone(),
                ConfigSettingsGetInput {
                    target: ConfigSettingsPathInput { path: Some(path) },
                    source: agena::config::ConfigSettingsSource::File,
                },
            )
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to read configured provider model")?
            .value;
            if !configured.is_null() {
                return Ok(configured);
            }
        }

        let catalog_entries = self.lookup_model_catalog_models(
            &[model_id.to_owned()]
                .into_iter()
                .chain(provider_model.map(catalog_lookup_id_for_provider_model))
                .collect::<Vec<_>>(),
        );
        Ok(provider_model_json_for_model_id(
            &catalog_entries,
            model_id,
            provider_model,
        ))
    }
}

fn apply_provider_adapter_selection(
    adapters: &mut JsonMap<String, JsonValue>,
    known_adapter_ids: &std::collections::BTreeSet<String>,
    selected_adapter_ids: &std::collections::BTreeSet<&str>,
) -> std::result::Result<(), ProviderStudioSaveError> {
    for adapter_id in known_adapter_ids {
        adapters
            .entry(adapter_id.clone())
            .or_insert_with(|| json!({}));
    }
    for (adapter_id, adapter_value) in adapters {
        let adapter_object = adapter_value.as_object_mut().ok_or_else(|| {
            ProviderStudioSaveError::ProviderAdapterMustBeObject {
                adapter_id: adapter_id.clone(),
            }
        })?;
        adapter_object.insert(
            "enabled".to_owned(),
            JsonValue::Bool(selected_adapter_ids.contains(adapter_id.as_str())),
        );
    }
    Ok(())
}

fn build_provider_adapter_matches_patch(
    draft: &ProviderConfigDraft,
    adapter_id: &str,
    configured_models: JsonMap<String, JsonValue>,
) -> std::result::Result<JsonValue, ProviderStudioSaveError> {
    // Provider drafts are built from the fully resolved configuration,
    // while partial adapter saves patch the writable file layer. The file
    // layer may not contain `defaults` yet (for example when the defaults
    // came from another config layer), so every provider patch must carry
    // the visible draft defaults instead of relying on an existing object.
    build_provider_patch_value_for_save(
        draft,
        optional_non_empty(draft.default_adapter.as_str()).unwrap_or(adapter_id),
        optional_non_empty(draft.default_model.as_str()),
        json!({
            adapter_id: {
                "enabled": true,
                "models": configured_models,
            }
        }),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::{apply_provider_adapter_selection, build_provider_adapter_matches_patch};
    use crate::backend::{
        JsonMap, ProviderConfigDraft, ProviderDraftAuthKind, ProviderDraftSecretSourceKind,
    };
    use serde_json::json;
    use std::collections::BTreeSet;

    #[test]
    fn adapter_selection_is_authoritative_and_materializes_disabled_overrides() {
        let mut adapters = JsonMap::from_iter([(
            "openai_responses".to_owned(),
            json!({ "enabled": true, "models_url": "https://example.test/models" }),
        )]);
        let known = BTreeSet::from([
            "openai_responses".to_owned(),
            "openai_chat_completions".to_owned(),
        ]);
        let selected = BTreeSet::from(["openai_chat_completions"]);

        apply_provider_adapter_selection(&mut adapters, &known, &selected)
            .expect("adapter selection should apply");

        assert_eq!(adapters["openai_responses"]["enabled"], false);
        assert_eq!(
            adapters["openai_responses"]["models_url"],
            "https://example.test/models"
        );
        assert_eq!(adapters["openai_chat_completions"]["enabled"], true);
    }

    #[test]
    fn selected_adapter_patch_materializes_visible_resolved_defaults() {
        let mut draft = ProviderConfigDraft::new_empty();
        draft.source_provider_id = Some("jiuuij".to_owned());
        draft.provider_id = "jiuuij".to_owned();
        draft.auth_kind = ProviderDraftAuthKind::Api;
        draft.auth.base_url = "https://jiuuij.example/v1".to_owned();
        draft.auth.secret_source_kind = ProviderDraftSecretSourceKind::Inline;
        draft.auth.secret_source_value = "test-key".to_owned();
        draft.default_adapter = "openai_chat_completions".to_owned();
        draft.default_model = "grok-4.3-fast".to_owned();

        let patch = build_provider_adapter_matches_patch(
            &draft,
            "openai_chat_completions",
            JsonMap::from_iter([("grok-4.3-fast".to_owned(), json!({}))]),
        )
        .expect("adapter patch should serialize");

        assert_eq!(patch["defaults"]["adapter"], "openai_chat_completions");
        assert_eq!(patch["defaults"]["model"], "grok-4.3-fast");
        assert!(
            patch["adapters"]["openai_chat_completions"]["models"]["grok-4.3-fast"].is_object()
        );
    }
}
