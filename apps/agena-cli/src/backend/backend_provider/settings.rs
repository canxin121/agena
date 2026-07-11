use anyhow::{Context, anyhow};
use serde_json::json;

use crate::backend::Result;
use crate::backend::{
    Backend, ConfigSettingsEditOptions, ConfigSettingsEditResponse, ConfigSettingsGetInput,
    ConfigSettingsPatchInput, ConfigSettingsPathInput, ConfigSettingsSetInput, JsonMap, JsonValue,
    ProcessEnvironment, ProviderAdapterModelsResponse, ProviderConfigDraft, ProviderModelOverlay,
    ProviderStudioSaveError, ProviderStudioSaveField, ProviderStudioSaveResult,
    apply_provider_auth_required_adapter_defaults_to_json_value,
    build_provider_auth_patch_value_for_save, list_provider_adapter_models_with_config,
    merge_provider_model_adapter_patch_for_save, optional_non_empty, patch_file_settings,
    provider_adapter_settings_path, provider_defaults_adapter, provider_defaults_point_to,
    provider_model_overlay_to_json, provider_settings_path, quoted_settings_segment,
    read_file_setting, required_provider_save_field, resolve_provider_defaults_from_value_for_save,
    set_file_setting,
};

impl Backend {
    pub(super) fn read_file_provider_settings(
        &self,
        provider_id: &str,
    ) -> Result<Option<JsonValue>> {
        let configured = read_file_setting(
            self.runtime.config_resolution().meta.config_path.clone(),
            ConfigSettingsGetInput {
                target: ConfigSettingsPathInput {
                    path: Some(provider_settings_path(provider_id)),
                },
                source: agena::config::ConfigSettingsSource::File,
            },
        )
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to read configured provider")?
        .value;
        if configured.is_null() {
            Ok(None)
        } else {
            Ok(Some(configured))
        }
    }

    pub async fn save_provider_model_value(
        &self,
        draft: ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        model_value: JsonValue,
        set_default: bool,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_provider_save_field(
            draft.provider_id.as_str(),
            ProviderStudioSaveField::ProviderId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let adapter_id =
            required_provider_save_field(adapter_id, ProviderStudioSaveField::AdapterId)
                .map_err(ProviderStudioSaveError::Validation)?;
        let model_id = required_provider_save_field(model_id, ProviderStudioSaveField::ModelId)
            .map_err(ProviderStudioSaveError::Validation)?;
        let JsonValue::Object(_) = &model_value else {
            return Err(ProviderStudioSaveError::ProviderModelConfigMustBeObject);
        };
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, &[adapter_id.to_owned()]);
        draft
            .validate_for_adapters_for_save(&effective_adapter_ids)
            .map_err(ProviderStudioSaveError::Validation)?;
        let default_adapter = if set_default {
            adapter_id
        } else {
            optional_non_empty(draft.default_adapter.as_str()).unwrap_or(adapter_id)
        };
        let default_model = if set_default {
            model_id
        } else {
            optional_non_empty(draft.default_model.as_str()).unwrap_or(model_id)
        };
        let include_defaults = set_default || draft.source_provider_id.is_none();
        let existing_adapter = draft
            .source_provider_id
            .as_deref()
            .map(|provider_id| {
                read_file_setting(
                    self.runtime.config_resolution().meta.config_path.clone(),
                    ConfigSettingsGetInput {
                        target: ConfigSettingsPathInput {
                            path: Some(provider_adapter_settings_path(provider_id, adapter_id)),
                        },
                        source: agena::config::ConfigSettingsSource::File,
                    },
                )
                .map_err(ProviderStudioSaveError::other)
                .map(|response| response.value)
            })
            .transpose()?;
        let model_overlay = serde_json::from_value::<ProviderModelOverlay>(model_value)
            .map_err(ProviderStudioSaveError::other)?;
        let mut adapter_patch = merge_provider_model_adapter_patch_for_save(
            existing_adapter,
            model_id,
            provider_model_overlay_to_json(model_overlay),
        )?;
        apply_provider_auth_required_adapter_defaults_to_json_value(
            &draft,
            adapter_id,
            &mut adapter_patch,
        )?;
        let mut provider_patch = JsonMap::new();
        provider_patch.insert("enabled".to_owned(), JsonValue::Bool(true));
        provider_patch.insert(
            "auth".to_owned(),
            JsonValue::Object(build_provider_auth_patch_value_for_save(&draft)?),
        );
        provider_patch.insert(
            "adapters".to_owned(),
            json!({
                adapter_id: adapter_patch,
            }),
        );
        if include_defaults {
            provider_patch.insert(
                "defaults".to_owned(),
                json!({
                    "adapter": default_adapter,
                    "model": default_model,
                }),
            );
        }
        self.patch_provider_settings(provider_id, JsonValue::Object(provider_patch))
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::ConfiguredModelSaved {
            provider_id: provider_id.to_owned(),
            adapter_id: adapter_id.to_owned(),
            model_id: model_id.to_owned(),
        })
    }

    pub async fn delete_provider_model(
        &self,
        draft: ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_provider_save_field(
            draft.provider_id.as_str(),
            ProviderStudioSaveField::ProviderId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let adapter_id =
            required_provider_save_field(adapter_id, ProviderStudioSaveField::AdapterId)
                .map_err(ProviderStudioSaveError::Validation)?;
        let model_id = required_provider_save_field(model_id, ProviderStudioSaveField::ModelId)
            .map_err(ProviderStudioSaveError::Validation)?;
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, &[adapter_id.to_owned()]);
        draft
            .validate_for_adapters_for_save(&effective_adapter_ids)
            .map_err(ProviderStudioSaveError::Validation)?;

        let mut provider_value = self
            .read_file_provider_settings(provider_id)
            .map_err(ProviderStudioSaveError::other)?
            .ok_or(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject)?;
        let provider_object = provider_value
            .as_object_mut()
            .ok_or(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject)?;
        let updates_default = provider_defaults_point_to(provider_object, adapter_id, model_id);
        let current_default_adapter = provider_defaults_adapter(provider_object)
            .map(ToOwned::to_owned)
            .or_else(|| optional_non_empty(draft.default_adapter.as_str()).map(ToOwned::to_owned))
            .unwrap_or_else(|| adapter_id.to_owned());
        let next_default = {
            let adapters = provider_object
                .get_mut("adapters")
                .and_then(JsonValue::as_object_mut)
                .ok_or(ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject)?;
            let adapter = adapters
                .get_mut(adapter_id)
                .and_then(JsonValue::as_object_mut)
                .ok_or_else(|| ProviderStudioSaveError::ProviderAdapterMustBeObject {
                    adapter_id: adapter_id.to_owned(),
                })?;
            let models = adapter
                .get_mut("models")
                .and_then(JsonValue::as_object_mut)
                .ok_or(ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject)?;
            models.remove(model_id);

            if updates_default {
                Some(resolve_provider_defaults_from_value_for_save(
                    adapters,
                    Some(current_default_adapter.as_str()),
                    None,
                )?)
            } else {
                None
            }
        };

        if let Some((next_adapter, next_model)) = next_default {
            let mut defaults = JsonMap::new();
            defaults.insert("adapter".to_owned(), JsonValue::String(next_adapter));
            if let Some(next_model) = next_model {
                defaults.insert("model".to_owned(), JsonValue::String(next_model));
            }
            provider_object.insert("defaults".to_owned(), JsonValue::Object(defaults));
        }

        self.set_provider_settings(provider_id, provider_value)
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::ModelDeleted {
            provider_id: provider_id.to_owned(),
            adapter_id: adapter_id.to_owned(),
            model_id: model_id.to_owned(),
        })
    }

    pub async fn delete_provider(
        &self,
        provider_id: &str,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let provider_id =
            required_provider_save_field(provider_id, ProviderStudioSaveField::ProviderId)
                .map_err(ProviderStudioSaveError::Validation)?;
        let provider_value = self
            .read_file_provider_settings(provider_id)
            .map_err(ProviderStudioSaveError::other)?
            .ok_or(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject)?;
        if !provider_value.is_object() {
            return Err(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject);
        }

        let configured_default_provider = read_file_setting(
            self.runtime.config_resolution().meta.config_path.clone(),
            ConfigSettingsGetInput {
                target: ConfigSettingsPathInput {
                    path: Some("providers.default".to_owned()),
                },
                source: agena::config::ConfigSettingsSource::File,
            },
        )
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to read configured default provider")
        .map_err(ProviderStudioSaveError::other)?
        .value;
        let clears_default_provider = configured_default_provider
            .as_str()
            .map(str::trim)
            .is_some_and(|configured| configured == provider_id);
        if clears_default_provider {
            self.delete_config_setting("providers.default")
                .await
                .map_err(ProviderStudioSaveError::other)?;
        }
        self.delete_config_setting(provider_settings_path(provider_id).as_str())
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::ProviderDeleted {
            provider_id: provider_id.to_owned(),
        })
    }

    pub async fn delete_provider_adapter(
        &self,
        draft: ProviderConfigDraft,
        adapter_id: &str,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_provider_save_field(
            draft.provider_id.as_str(),
            ProviderStudioSaveField::ProviderId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let adapter_id =
            required_provider_save_field(adapter_id, ProviderStudioSaveField::AdapterId)
                .map_err(ProviderStudioSaveError::Validation)?;

        let mut provider_value = self
            .read_file_provider_settings(provider_id)
            .map_err(ProviderStudioSaveError::other)?
            .ok_or(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject)?;
        let provider_object = provider_value
            .as_object_mut()
            .ok_or(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject)?;
        let (removed_model_count, delete_provider_after) = {
            let adapters = provider_object
                .get_mut("adapters")
                .and_then(JsonValue::as_object_mut)
                .ok_or(ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject)?;
            let removed_adapter_value = adapters.remove(adapter_id).ok_or_else(|| {
                ProviderStudioSaveError::ProviderAdapterMustBeObject {
                    adapter_id: adapter_id.to_owned(),
                }
            })?;
            let removed_adapter = removed_adapter_value.as_object().ok_or_else(|| {
                ProviderStudioSaveError::ProviderAdapterMustBeObject {
                    adapter_id: adapter_id.to_owned(),
                }
            })?;
            let removed_model_count = match removed_adapter.get("models") {
                Some(JsonValue::Object(models)) => models.len(),
                Some(JsonValue::Null) | None => 0,
                Some(_) => {
                    return Err(
                        ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject,
                    );
                }
            };
            (removed_model_count, adapters.is_empty())
        };
        if delete_provider_after {
            return self.delete_provider(provider_id).await;
        }

        let requested_default_adapter = provider_defaults_adapter(provider_object)
            .filter(|candidate| *candidate != adapter_id)
            .or_else(|| {
                optional_non_empty(draft.default_adapter.as_str())
                    .filter(|candidate| *candidate != adapter_id)
            });
        let (next_adapter, next_model) = {
            let adapters = provider_object
                .get("adapters")
                .and_then(JsonValue::as_object)
                .ok_or(ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject)?;
            resolve_provider_defaults_from_value_for_save(
                adapters,
                requested_default_adapter,
                optional_non_empty(draft.default_model.as_str()),
            )?
        };
        let mut defaults = JsonMap::new();
        defaults.insert("adapter".to_owned(), JsonValue::String(next_adapter));
        if let Some(next_model) = next_model {
            defaults.insert("model".to_owned(), JsonValue::String(next_model));
        }
        provider_object.insert("defaults".to_owned(), JsonValue::Object(defaults));

        self.set_provider_settings(provider_id, provider_value)
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::AdapterDeleted {
            provider_id: provider_id.to_owned(),
            adapter_id: adapter_id.to_owned(),
            removed_model_count,
        })
    }

    pub(super) async fn list_provider_adapter_models_with_target(
        &self,
        target: agena::config::ProviderAdapterModelsTarget,
    ) -> Result<ProviderAdapterModelsResponse> {
        let resolution = self.runtime.config_resolution();
        let adapter_models = list_provider_adapter_models_with_config(
            &resolution.config,
            &target,
            &ProcessEnvironment,
        )
        .await
        .context("failed to list provider adapter models")?;
        Ok(ProviderAdapterModelsResponse {
            provider_id: adapter_models.provider_id,
            adapters: adapter_models
                .adapters
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    pub(super) async fn patch_provider_settings(
        &self,
        provider_id: &str,
        provider_patch: JsonValue,
    ) -> Result<ConfigSettingsEditResponse> {
        let config_path = self.runtime.config_resolution().meta.config_path.clone();
        let response = patch_file_settings(
            config_path,
            ConfigSettingsPatchInput {
                target: ConfigSettingsPathInput {
                    path: Some("providers".to_owned()),
                },
                changes: json!({
                    provider_id: provider_patch,
                }),
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            },
        )
        .map_err(|error| anyhow!("failed to patch provider settings: {error}"))?;

        if response.reload_required {
            self.runtime
                .reload()
                .await
                .context("failed to reload runtime after provider settings change")?;
        }
        Ok(response)
    }

    pub(super) async fn set_provider_settings(
        &self,
        provider_id: &str,
        provider_value: JsonValue,
    ) -> Result<ConfigSettingsEditResponse> {
        let config_path = self.runtime.config_resolution().meta.config_path.clone();
        let response = set_file_setting(
            config_path,
            ConfigSettingsSetInput {
                path: format!("providers.{}", quoted_settings_segment(provider_id)),
                value: provider_value,
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            },
        )
        .map_err(|error| anyhow!("failed to save provider settings: {error}"))?;

        if response.reload_required {
            self.runtime
                .reload()
                .await
                .context("failed to reload runtime after provider settings change")?;
        }
        Ok(response)
    }
}
