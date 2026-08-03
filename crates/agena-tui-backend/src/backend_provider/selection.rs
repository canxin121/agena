use agena_api::resource::ProviderModelResource;
use anyhow::{Context, anyhow};
use serde_json::json;

use crate::Result;
use crate::{
    Backend, CatalogModelResource, InspectorRow, JsonMap, JsonValue, ModelCatalogListResponse,
    ModelRef, ProviderAdapterModelsResource, ProviderAdapterModelsResponse, ProviderConfigDraft,
    ProviderDraftAuthActionResult, ProviderDraftAuthError, ProviderId, ProviderModel,
    ProviderStudioSaveError, ProviderStudioSaveField, ProviderStudioSaveResult, RunOptions,
    apply_provider_auth_required_adapter_defaults_to_json_adapters,
    build_provider_auth_patch_value_for_save, build_provider_patch_value_for_save,
    catalog_lookup_id_for_provider_model, continue_provider_draft_auth,
    ensure_provider_model_entry, optional_non_empty, preferred_catalog_model_for_provider_model,
    provider_model_json_for_model_id, provider_model_selection_contains,
    provider_model_settings_path, required_provider_save_field, required_trimmed,
    resolve_provider_defaults_from_value_for_save, start_provider_draft_auth, summarize_named_mode,
};

impl Backend {
    pub fn provider_config_draft(&self, provider_id: Option<&str>) -> Result<ProviderConfigDraft> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            let mut draft = ProviderConfigDraft::new_empty();
            draft.normalize_shape();
            return Ok(draft);
        };

        let provider = self
            .application
            .provider_catalog()
            .configured_editor(&ProviderId::new(provider_id))
            .ok_or_else(|| anyhow!("provider not found: {provider_id}"))?;
        Ok(ProviderConfigDraft::from_configured_editor(provider))
    }

    pub async fn start_provider_draft_auth(
        &self,
        draft: ProviderConfigDraft,
    ) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
        start_provider_draft_auth(
            self.application.runtime_draft_authentication().as_ref(),
            draft,
        )
        .await
    }

    pub async fn continue_provider_draft_auth(
        &self,
        draft: ProviderConfigDraft,
    ) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
        continue_provider_draft_auth(
            self.application.runtime_draft_authentication().as_ref(),
            draft,
        )
        .await
    }

    pub(super) fn configured_provider_adapter_ids(
        &self,
        provider_id: Option<&str>,
    ) -> std::collections::BTreeSet<String> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return std::collections::BTreeSet::new();
        };
        self.application
            .provider_catalog()
            .configured_routing(&ProviderId::new(provider_id))
            .map(|provider| {
                provider
                    .adapters
                    .into_iter()
                    .map(|adapter| adapter.adapter_id)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn configured_provider_model_routes(
        &self,
        provider_id: Option<&str>,
    ) -> Vec<(String, String)> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Vec::new();
        };
        self.application
            .provider_catalog()
            .configured_routing(&ProviderId::new(provider_id))
            .into_iter()
            .flat_map(|provider| provider.adapters)
            .filter(|adapter| adapter.enabled)
            .flat_map(|adapter| {
                let adapter_id = adapter.adapter_id;
                adapter
                    .model_ids
                    .into_iter()
                    .map(move |model_id| (adapter_id.clone(), model_id))
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
        self.application
            .provider_catalog()
            .configured_routing(&ProviderId::new(provider_id))
            .into_iter()
            .flat_map(|provider| provider.adapters)
            .map(|adapter| ProviderAdapterModelsResource {
                adapter_id: adapter.adapter_id.clone(),
                enabled: adapter.enabled,
                resolved_base_url: None,
                models: adapter
                    .model_ids
                    .into_iter()
                    .map(|model_id| {
                        ProviderModelResource::configured(adapter.adapter_id.as_str(), model_id)
                    })
                    .collect(),
                failure: None,
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
        self.application
            .provider_catalog()
            .configured_local_models(&ProviderId::new(provider_id))
            .map_err(|error| anyhow!(error.to_string()))
    }

    pub fn model_display_name(&self, model: &ModelRef) -> Option<String> {
        preferred_model_display_name(
            self.list_local_provider_models(model.provider_id.as_ref())
                .ok()?,
            model,
        )
    }

    pub fn list_model_catalog_models(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<ModelCatalogListResponse> {
        Ok(self.application.list_model_catalog(query, offset, limit))
    }

    pub fn lookup_model_catalog_models(&self, model_ids: &[String]) -> Vec<CatalogModelResource> {
        self.application.lookup_model_catalog_models(model_ids)
    }

    pub fn resolved_model_for_run_options(&self, request: &RunOptions) -> Result<ModelRef> {
        if let Some(model) = request.model.as_ref() {
            return match model.adapter_id.as_deref() {
                Some(adapter_id) => {
                    ModelRef::try_new_with_adapter(&model.provider_id, adapter_id, &model.model_id)
                }
                None => ModelRef::try_new(&model.provider_id, &model.model_id),
            }
            .context("run option contains an invalid model reference");
        }

        self.application
            .provider_catalog()
            .default_model()
            .map_err(|error| anyhow!(error.to_string()))?
            .ok_or_else(|| anyhow!("no providers configured"))
    }

    pub fn runtime_thinking_mode_rows(&self, request: &RunOptions) -> Result<Vec<InspectorRow>> {
        let model = self.resolved_model_for_run_options(request)?;
        self.model_thinking_mode_rows(&model)
    }

    pub fn model_thinking_mode_rows(&self, model: &ModelRef) -> Result<Vec<InspectorRow>> {
        let mut modes = self
            .application
            .provider_catalog()
            .model_execution_options(model)
            .map_err(|error| anyhow!(error.to_string()))?
            .thinking_modes;
        modes.sort_by(agena_domain::compare_thinking_mode_strength);
        Ok(modes
            .into_iter()
            .filter_map(|mode| {
                Some(InspectorRow {
                    label: mode.selector()?.into_owned(),
                    detail: summarize_named_mode(
                        mode.display_name.as_deref(),
                        mode.description.as_deref(),
                    ),
                })
            })
            .collect())
    }

    pub fn runtime_speed_mode_rows(&self, request: &RunOptions) -> Result<Vec<InspectorRow>> {
        let model = self.resolved_model_for_run_options(request)?;
        self.model_speed_mode_rows(&model)
    }

    pub fn model_speed_mode_rows(&self, model: &ModelRef) -> Result<Vec<InspectorRow>> {
        let mut rows = self
            .application
            .provider_catalog()
            .model_execution_options(model)
            .map_err(|error| anyhow!(error.to_string()))?
            .speed_modes
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
        let model = self.resolved_model_for_run_options(request)?;
        self.model_verbosity_values(&model)
    }

    pub fn model_verbosity_values(&self, model: &ModelRef) -> Result<Vec<String>> {
        let metadata = self
            .application
            .provider_catalog()
            .model_execution_options(model)
            .map_err(|error| anyhow!(error.to_string()))?
            .metadata;
        Ok(metadata.supported_verbosity_levels_for_model(&model.model_id))
    }

    pub async fn refresh_model_catalog(&self) -> Result<()> {
        self.application
            .request_model_catalog_refresh()
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(())
    }

    pub async fn list_draft_provider_adapter_models(
        &self,
        draft: &ProviderConfigDraft,
        adapter_ids: &[String],
    ) -> Result<ProviderAdapterModelsResponse> {
        let mut draft = draft.clone();
        draft.normalize_shape();
        let request = draft.build_listing_request(adapter_ids)?;
        let adapter_models = self
            .application
            .provider_catalog()
            .list_draft_adapter_models(request)
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(self.provider_adapter_models_response(adapter_models))
    }

    pub async fn list_saved_provider_adapter_models(
        &self,
        provider_id: &str,
        adapter_ids: &[String],
    ) -> Result<ProviderAdapterModelsResponse> {
        let provider_id = provider_id.trim();
        let adapter_models = self
            .application
            .provider_catalog()
            .list_saved_adapter_models(&ProviderId::new(provider_id), adapter_ids.to_vec())
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(self.provider_adapter_models_response(adapter_models))
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
        if let Some(source_provider_id) = draft.source_provider_id.as_deref()
            && source_provider_id != provider_id
            && self
                .read_file_provider_settings(provider_id)
                .map_err(ProviderStudioSaveError::other)?
                .is_some()
        {
            return Err(ProviderStudioSaveError::other(anyhow!(
                "provider `{provider_id}` already exists; rename it to a different id"
            )));
        }
        let requested_default_adapter = optional_non_empty(draft.default_adapter.as_str())
            .map(str::to_owned)
            .or_else(|| {
                selected_adapter_ids
                    .iter()
                    .map(String::as_str)
                    .find_map(optional_non_empty)
                    .map(ToOwned::to_owned)
            });
        let requested_default_model =
            optional_non_empty(draft.default_model.as_str()).map(str::to_owned);
        if draft.default_adapter.trim().is_empty()
            && let Some(default_adapter) = requested_default_adapter.as_ref()
        {
            draft.default_adapter = default_adapter.clone();
        }
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

        // Editing an existing provider under a new id is a rename: start from
        // the source provider's full file value so fields the draft does not
        // manage (such as `network`) are carried to the new key instead of
        // being dropped. For a plain save the source id equals the target id
        // and this reads the provider being edited.
        let existing_base_id = draft.source_provider_id.as_deref().unwrap_or(provider_id);
        let mut provider_value = self
            .read_file_provider_settings(existing_base_id)
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
            if adapter_models.failure.is_some() || !selected.contains(adapter_id) {
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
            let existing_models = adapter_object
                .get("models")
                .and_then(JsonValue::as_object)
                .cloned()
                .unwrap_or_default();
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
                    let generated = provider_model_json_for_model_id(
                        &catalog_entries,
                        model.id.as_ref(),
                        Some(model),
                    );
                    (
                        model.id.to_string(),
                        preserve_existing_model_execution_policy(
                            generated,
                            existing_models.get(model.id.as_str()),
                        ),
                    )
                })
                .collect::<JsonMap<_, _>>();
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
                        .find(|model| model.id == default_model)
                        .cloned()
                });
            let default_model_value = provider_model_json_for_model_id(
                &catalog_entries,
                default_model,
                default_provider_model.as_ref(),
            );
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
            .await?;
        if let Some(source_provider_id) = draft.source_provider_id.as_deref()
            && source_provider_id != provider_id
        {
            // The provider was renamed. Drop the old key and re-point every
            // `providers.default` / `default_selection.provider` reference to
            // the new id in one atomic patch so the file keeps validating.
            self.rename_provider_references(source_provider_id, provider_id)
                .await?;
        }
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
        let existing_models = self
            .read_file_provider_settings(provider_id)
            .map_err(ProviderStudioSaveError::other)?
            .as_ref()
            .and_then(JsonValue::as_object)
            .and_then(|provider| provider.get("adapters"))
            .and_then(JsonValue::as_object)
            .and_then(|adapters| adapters.get(adapter_id))
            .and_then(JsonValue::as_object)
            .and_then(|adapter| adapter.get("models"))
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let configured_models = adapter_models
            .models
            .iter()
            .map(|model| {
                let generated = provider_model_json_for_model_id(
                    &catalog_entries,
                    model.id.as_ref(),
                    Some(model),
                );
                (
                    model.id.to_string(),
                    preserve_existing_model_execution_policy(
                        generated,
                        existing_models.get(model.id.as_str()),
                    ),
                )
            })
            .collect::<JsonMap<_, _>>();
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
            .await?;
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
        provider_model: Option<&ProviderModelResource>,
    ) -> Result<JsonValue> {
        let adapter_id = required_trimmed(adapter_id, "adapter_id")?;
        let model_id = required_trimmed(model_id, "model_id")?;
        if let Some(provider_id) = draft.source_provider_id.as_deref() {
            let path = provider_model_settings_path(provider_id, adapter_id, model_id);
            let configured = self
                .application
                .runtime_config_settings()
                .read_file_settings(agena_runtime::ConfigSettingsGetInput {
                    target: agena_runtime::ConfigSettingsPathInput { path: Some(path) },
                    source: agena_runtime::ConfigSettingsSource::File,
                })
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

    /// Re-point the file-level `providers.default` and
    /// `providers.default_selection.provider` references from `source` to
    /// `target`, and drop the old `providers.<source>` key, in a single atomic
    /// patch. The new provider config is written under `target` by the caller
    /// first, so the resulting document still validates.
    async fn rename_provider_references(
        &self,
        source: &str,
        target: &str,
    ) -> std::result::Result<(), ProviderStudioSaveError> {
        let read_path = |path: &str| {
            self.application
                .runtime_config_settings()
                .read_file_settings(agena_runtime::ConfigSettingsGetInput {
                    target: agena_runtime::ConfigSettingsPathInput {
                        path: Some(path.to_owned()),
                    },
                    source: agena_runtime::ConfigSettingsSource::File,
                })
                .map(|response| response.value)
                .map_err(ProviderStudioSaveError::from)
        };
        let default_provider = read_path("providers.default")?;
        let default_selection = read_path("providers.default_selection")?;

        let mut changes = JsonMap::new();
        changes.insert(source.to_owned(), JsonValue::Null);
        if default_provider.as_str().map(str::trim) == Some(source) {
            changes.insert("default".to_owned(), JsonValue::String(target.to_owned()));
        }
        if let Some(selection) = default_selection.as_object() {
            let mut selection = selection.clone();
            if selection
                .get("provider")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                == Some(source)
            {
                selection.insert("provider".to_owned(), JsonValue::String(target.to_owned()));
                changes.insert("default_selection".to_owned(), JsonValue::Object(selection));
            }
        }
        if changes.is_empty() {
            return Ok(());
        }
        self.patch_provider_settings_root(JsonValue::Object(changes))
            .await?;
        Ok(())
    }

    /// Persist `providers.default` and `providers.default_selection` in one
    /// atomic patch. Writing them as two separate edits would leave the file in
    /// a partially updated state if the second edit failed validation (the
    /// default provider would already be persisted without the selection).
    pub async fn set_provider_default_selection(
        &self,
        provider_id: &str,
        selection: JsonValue,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse> {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            return Err(anyhow!("provider id is required"));
        }
        let mut changes = JsonMap::new();
        changes.insert("default".to_owned(), JsonValue::String(provider_id.to_owned()));
        changes.insert("default_selection".to_owned(), selection);
        let response = self
            .application
            .runtime_config_settings()
            .patch_file_settings(agena_runtime::ConfigSettingsPatchInput {
                target: agena_runtime::ConfigSettingsPathInput {
                    path: Some("providers".to_owned()),
                },
                changes: JsonValue::Object(changes),
                options: agena_runtime::ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })
            .context("failed to set provider default selection")?;

        if response.reload_required {
            self.application
                .runtime_control()
                .reload()
                .await
                .context("failed to reload runtime after provider default selection change")?;
        }
        Ok(response)
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

fn preserve_existing_model_execution_policy(
    mut generated: JsonValue,
    existing: Option<&JsonValue>,
) -> JsonValue {
    let Some(existing_model) = existing.and_then(JsonValue::as_object) else {
        return generated;
    };
    let Some(generated_model) = generated.as_object_mut() else {
        return generated;
    };
    for field in ["agena_tools", "native_compaction"] {
        if let Some(value) = existing_model.get(field).cloned() {
            generated_model.insert(field.to_owned(), value);
        }
    }
    generated
}

fn preferred_model_display_name(models: Vec<ProviderModel>, model: &ModelRef) -> Option<String> {
    models
        .into_iter()
        .find(|candidate| {
            candidate.id == model.model_id
                && model
                    .adapter_id
                    .as_ref()
                    .is_none_or(|adapter_id| candidate.adapter_id.as_ref() == Some(adapter_id))
        })
        .and_then(|candidate| candidate.display_name)
        .map(|display_name| display_name.trim().to_owned())
        .filter(|display_name| !display_name.is_empty())
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
    use super::{
        apply_provider_adapter_selection, build_provider_adapter_matches_patch,
        preferred_model_display_name, preserve_existing_model_execution_policy,
        resolve_provider_defaults_from_value_for_save,
    };
    use crate::{
        JsonMap, ModelRef, ProviderConfigDraft, ProviderDraftAuthKind,
        ProviderDraftSecretSourceKind, ProviderModel,
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
    fn model_status_prefers_the_display_name_for_the_selected_adapter() {
        let mut first = ProviderModel::new("provider", "shared-model");
        first.adapter_id = Some(agena_domain::AdapterId::new("adapter-a"));
        first.display_name = Some("First Display".to_owned());
        let mut selected = ProviderModel::new("provider", "shared-model");
        selected.adapter_id = Some(agena_domain::AdapterId::new("adapter-b"));
        selected.display_name = Some("  Selected Display  ".to_owned());

        assert_eq!(
            preferred_model_display_name(
                vec![first, selected],
                &ModelRef::new_with_adapter("provider", "adapter-b", "shared-model"),
            )
            .as_deref(),
            Some("Selected Display")
        );
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

    #[test]
    fn refreshed_model_preserves_explicit_execution_policy() {
        let generated = json!({
            "display_name": "Refreshed",
            "agena_tools": { "mode": "provider_protocol" }
        });
        let existing = json!({
            "display_name": "Old",
            "native_compaction": false,
            "agena_tools": {
                "mode": "prompt_envelope",
                "provider_native": { "hosted": { "web_search": true } }
            }
        });

        let merged = preserve_existing_model_execution_policy(generated, Some(&existing));

        assert_eq!(merged["display_name"], "Refreshed");
        assert_eq!(merged["native_compaction"], false);
        assert_eq!(merged["agena_tools"], existing["agena_tools"]);
    }

    #[test]
    fn new_model_keeps_capability_derived_agena_tool_policy() {
        let generated = json!({
            "agena_tools": { "mode": "disabled" }
        });

        let merged = preserve_existing_model_execution_policy(generated.clone(), None);

        assert_eq!(merged, generated);
    }

    #[test]
    fn defaults_resolution_falls_back_when_requested_adapter_was_disabled() {
        let mut adapters = JsonMap::new();
        adapters.insert(
            "openai_chat_completions".to_owned(),
            json!({ "enabled": false }),
        );
        adapters.insert("anthropic".to_owned(), json!({ "enabled": true }));

        let (default_adapter, default_model) =
            resolve_provider_defaults_from_value_for_save(&adapters, Some("openai_chat_completions"), None)
                .expect("defaults resolve");

        assert_eq!(default_adapter, "anthropic");
        assert_eq!(default_model, None);
    }

    #[test]
    fn defaults_resolution_prefers_enabled_requested_adapter() {
        let mut adapters = JsonMap::new();
        adapters.insert("anthropic".to_owned(), json!({ "enabled": true }));
        adapters.insert("gemini".to_owned(), json!({ "enabled": true }));

        let (default_adapter, _) =
            resolve_provider_defaults_from_value_for_save(&adapters, Some("gemini"), None)
                .expect("defaults resolve");

        assert_eq!(default_adapter, "gemini");
    }

    #[test]
    fn defaults_resolution_errors_when_no_adapter_is_enabled() {
        let mut adapters = JsonMap::new();
        adapters.insert(
            "openai_responses".to_owned(),
            json!({ "enabled": false }),
        );

        let error = resolve_provider_defaults_from_value_for_save(&adapters, None, None)
            .expect_err("no enabled adapter must be rejected");
        assert!(matches!(
            error,
            crate::ProviderStudioSaveError::Validation(_)
        ));
    }
}
