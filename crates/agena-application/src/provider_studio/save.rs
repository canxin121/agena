//! Provider Studio save/delete operations and their helpers, migrated from
//! `agena-tui-backend/src/backend_provider/selection.rs` and
//! `backend_provider/settings.rs`, plus `parse_credential_issuer` /
//! `non_empty` / `trimmed_owned` from `backend_util.rs` and the JSON config
//! helpers from `backend_config.rs`.
//!
//! The operations are free functions taking `&Application`; the `impl
//! Application` surface in `application_provider_studio.rs` delegates to them.
//! Function names and internal logic are preserved from the original backend;
//! only the receiver changed and error types were normalized per the R7 brief.

use anyhow::{Context, anyhow};
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use super::catalog::{
    apply_provider_auth_required_adapter_defaults_to_json_adapters,
    apply_provider_auth_required_adapter_defaults_to_json_value,
    build_provider_auth_patch_value_for_save, build_provider_patch_value_for_save,
    ensure_provider_model_entry, merge_provider_model_adapter_patch_for_save, optional_non_empty,
    preferred_catalog_model_for_provider_model, provider_adapter_settings_path,
    provider_defaults_adapter, provider_defaults_point_to,
    provider_model_catalog_lookup_candidates, provider_model_json_for_model_id,
    provider_model_overlay_to_json, provider_model_selection_contains,
    provider_model_settings_path, provider_settings_path, quoted_settings_segment,
    required_provider_save_field, required_trimmed, resolve_provider_defaults_from_value_for_save,
};
use super::draft_auth_data::{
    ProviderStudioSaveError, ProviderStudioSaveField, ProviderStudioSaveResult,
};
use super::draft_config::ProviderConfigDraft;
use crate::Application;
use crate::provider_queries::provider_adapter_models_response;
use agena_domain::ProviderId;

pub(crate) fn parse_credential_issuer(
    value: &str,
) -> anyhow::Result<agena_provider::CredentialIssuer> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai_chatgpt" => Ok(agena_provider::CredentialIssuer::OpenaiChatgpt),
        "github_copilot" => Ok(agena_provider::CredentialIssuer::GithubCopilot),
        "gitlab" => Ok(agena_provider::CredentialIssuer::Gitlab),
        "google_adc" => Ok(agena_provider::CredentialIssuer::GoogleAdc),
        "sap_ai_core" => Ok(agena_provider::CredentialIssuer::SapAiCore),
        _ => Err(anyhow!(
            "unsupported credential issuer `{}`; expected openai_chatgpt, github_copilot, gitlab, google_adc, or sap_ai_core",
            value.trim()
        )),
    }
}

pub(crate) fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

pub(crate) fn trimmed_owned(value: &str) -> Option<String> {
    non_empty(Some(value)).map(ToOwned::to_owned)
}

// ---------------------------------------------------------------------------
// Config JSON helpers (migrated from backend_config.rs)
// ---------------------------------------------------------------------------

/// Resolve a `plugins.list.<id>.config.*` path into its plugin id and the
/// remaining config segments. Returns `None` when the path is not a plugin
/// config target.
pub(crate) fn plugin_config_setting_target(
    path: &str,
) -> anyhow::Result<Option<(String, Vec<String>)>> {
    let segments =
        agena_domain::parse_json_path(path).map_err(|error| anyhow!(error.to_string()))?;
    if segments.len() < 4
        || segments.first().is_none_or(|segment| segment != "plugins")
        || segments.get(1).is_none_or(|segment| segment != "list")
        || segments.get(3).is_none_or(|segment| segment != "config")
    {
        return Ok(None);
    }
    Ok(Some((segments[2].clone(), segments[4..].to_vec())))
}

pub(crate) fn default_static_plugin_record() -> JsonValue {
    json!({
        "enabled": true,
        "package": { "kind": "static" },
        "config": null
    })
}

pub(crate) fn plugin_record_for_config_edit(
    sources: &crate::dto::ConfigJsonSources,
    plugin_id: &str,
) -> JsonValue {
    let path = format!("plugins.list.{}", quoted_settings_segment(plugin_id));
    agena_domain::get_json_path(&sources.file, Some(path.as_str()))
        .ok()
        .filter(|value| value.is_object())
        .or_else(|| {
            agena_domain::get_json_path(&sources.effective, Some(path.as_str()))
                .ok()
                .filter(|value| value.is_object())
        })
        .unwrap_or_else(default_static_plugin_record)
}

pub(crate) fn normalize_plugin_record_for_config_edit(
    record: &mut JsonValue,
) -> anyhow::Result<&mut JsonValue> {
    if !record.is_object() {
        *record = default_static_plugin_record();
    }
    let object = record
        .as_object_mut()
        .ok_or_else(|| anyhow!("plugin config record must be an object"))?;
    object
        .entry("enabled".to_owned())
        .or_insert(JsonValue::Bool(true));
    object
        .entry("package".to_owned())
        .or_insert_with(|| json!({ "kind": "static" }));
    Ok(object
        .entry("config".to_owned())
        .or_insert_with(|| JsonValue::Object(JsonMap::new())))
}

pub(crate) fn set_nested_json_value(root: &mut JsonValue, segments: &[String], value: JsonValue) {
    if segments.is_empty() {
        *root = value;
        return;
    }
    if !root.is_object() {
        *root = JsonValue::Object(JsonMap::new());
    }
    let mut cursor = root;
    for segment in &segments[..segments.len().saturating_sub(1)] {
        let object = cursor.as_object_mut().expect("nested settings object");
        cursor = object
            .entry(segment.clone())
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        if !cursor.is_object() {
            *cursor = JsonValue::Object(JsonMap::new());
        }
    }
    let object = cursor.as_object_mut().expect("nested settings object");
    object.insert(segments[segments.len() - 1].clone(), value);
}

pub(crate) fn remove_nested_json_value(root: &mut JsonValue, segments: &[String]) -> bool {
    if segments.is_empty() {
        let deleted = !root.is_null();
        *root = JsonValue::Null;
        return deleted;
    }
    let mut cursor = root;
    for segment in &segments[..segments.len().saturating_sub(1)] {
        let Some(next) = cursor
            .as_object_mut()
            .and_then(|object| object.get_mut(segment.as_str()))
        else {
            return false;
        };
        cursor = next;
    }
    cursor
        .as_object_mut()
        .and_then(|object| object.remove(segments[segments.len() - 1].as_str()))
        .is_some()
}

// ---------------------------------------------------------------------------
// Provider Studio operations
// ---------------------------------------------------------------------------

pub(crate) fn provider_config_draft(
    app: &Application,
    provider_id: Option<&str>,
) -> std::result::Result<ProviderConfigDraft, crate::ApplicationError> {
    let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
        let mut draft = ProviderConfigDraft::new_empty();
        draft.normalize_shape();
        return Ok(draft);
    };

    let provider = app
        .provider_catalog()
        .configured_editor(&ProviderId::new(provider_id))
        .ok_or_else(|| {
            crate::ApplicationError::internal(format!("provider not found: {provider_id}"))
        })?;
    Ok(ProviderConfigDraft::from_configured_editor(provider))
}

pub(crate) fn configured_provider_adapter_ids(
    app: &Application,
    provider_id: Option<&str>,
) -> std::collections::BTreeSet<String> {
    let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return std::collections::BTreeSet::new();
    };
    app.provider_catalog()
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

pub(crate) fn effective_provider_draft_adapter_ids(
    app: &Application,
    draft: &ProviderConfigDraft,
    extra_adapter_ids: &[String],
) -> std::collections::BTreeSet<String> {
    let mut adapter_ids = configured_provider_adapter_ids(app, draft.source_provider_id.as_deref());
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

pub(crate) async fn list_draft_provider_adapter_models(
    app: &Application,
    draft: &ProviderConfigDraft,
    adapter_ids: &[String],
) -> std::result::Result<agena_api::resource::ProviderAdapterModelsResponse, crate::ApplicationError>
{
    let mut draft = draft.clone();
    draft.normalize_shape();
    let request = draft
        .build_listing_request(adapter_ids)
        .map_err(crate::ApplicationError::internal)?;
    let adapter_models = app
        .provider_catalog()
        .list_draft_adapter_models(request)
        .await
        .map_err(|error| crate::ApplicationError::internal(error.to_string()))?;
    Ok(provider_adapter_models_response(app, adapter_models))
}

pub(crate) async fn list_saved_provider_adapter_models(
    app: &Application,
    provider_id: &str,
    adapter_ids: &[String],
) -> std::result::Result<agena_api::resource::ProviderAdapterModelsResponse, crate::ApplicationError>
{
    let provider_id = provider_id.trim();
    let adapter_models = app
        .provider_catalog()
        .list_saved_adapter_models(&ProviderId::new(provider_id), adapter_ids.to_vec())
        .await
        .map_err(|error| crate::ApplicationError::internal(error.to_string()))?;
    Ok(provider_adapter_models_response(app, adapter_models))
}

pub(crate) async fn save_provider_draft(
    app: &Application,
    draft: ProviderConfigDraft,
    adapter_model_lists: &[agena_api::resource::ProviderAdapterModelsResource],
    selected_adapter_ids: &[String],
    selected_model_keys: &std::collections::BTreeSet<String>,
    model_config_values: &std::collections::BTreeMap<String, JsonValue>,
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
        && read_file_provider_settings(app, provider_id)
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

    let catalog_entries = app.lookup_model_catalog_models(
        &adapter_model_lists
            .iter()
            .flat_map(|adapter_models| {
                adapter_models
                    .models
                    .iter()
                    .flat_map(provider_model_catalog_lookup_candidates)
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
    let mut provider_value = read_file_provider_settings(app, existing_base_id)
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
        configured_provider_adapter_ids(app, draft.source_provider_id.as_deref());
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
                let configured = model_config_values
                    .get(&format!("{}\u{1f}{}", adapter_id, model.id))
                    .cloned()
                    .unwrap_or(generated);
                (
                    model.id.to_string(),
                    if model_config_values
                        .contains_key(&format!("{}\u{1f}{}", adapter_id, model.id))
                    {
                        configured
                    } else {
                        preserve_existing_model_config(
                            configured,
                            existing_models.get(model.id.as_str()),
                        )
                    },
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
        let default_model_key = format!("{}\u{1f}{}", default_adapter, default_model);
        let default_model_value = model_config_values
            .get(&default_model_key)
            .cloned()
            .unwrap_or_else(|| {
                provider_model_json_for_model_id(
                    &catalog_entries,
                    default_model,
                    default_provider_model.as_ref(),
                )
            });
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
    set_provider_settings(app, provider_id, provider_value).await?;
    if let Some(source_provider_id) = draft.source_provider_id.as_deref()
        && source_provider_id != provider_id
    {
        // The provider was renamed. Drop the old key and re-point every
        // `providers.default` / `default_selection.provider` reference to
        // the new id in one atomic patch so the file keeps validating.
        rename_provider_references(app, source_provider_id, provider_id).await?;
    }
    Ok(ProviderStudioSaveResult::ProviderDraftSaved {
        provider_id: provider_id.to_owned(),
        default_adapter,
        default_model,
    })
}

pub(crate) async fn save_provider_adapter_matches(
    app: &Application,
    draft: ProviderConfigDraft,
    adapter_models: agena_api::resource::ProviderAdapterModelsResource,
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
        effective_provider_draft_adapter_ids(app, &draft, &[adapter_id.to_owned()]);
    draft
        .validate_for_adapters_for_save(&effective_adapter_ids)
        .map_err(ProviderStudioSaveError::Validation)?;
    let catalog_entries = app.lookup_model_catalog_models(
        &adapter_models
            .models
            .iter()
            .flat_map(provider_model_catalog_lookup_candidates)
            .collect::<Vec<_>>(),
    );
    let existing_models = read_file_provider_settings(app, provider_id)
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
            let generated =
                provider_model_json_for_model_id(&catalog_entries, model.id.as_ref(), Some(model));
            (
                model.id.to_string(),
                preserve_existing_model_config(generated, existing_models.get(model.id.as_str())),
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
    patch_provider_settings(app, provider_id, provider_patch).await?;
    Ok(ProviderStudioSaveResult::AdapterMatchesSaved {
        provider_id: provider_id.to_owned(),
        adapter_id: adapter_id.to_owned(),
        listed_model_count: adapter_models.models.len(),
        matched_model_count,
    })
}

pub(crate) fn provider_model_draft_value(
    app: &Application,
    draft: &ProviderConfigDraft,
    adapter_id: &str,
    model_id: &str,
    provider_model: Option<&agena_api::resource::ProviderModelResource>,
) -> std::result::Result<JsonValue, crate::ApplicationError> {
    let adapter_id =
        required_trimmed(adapter_id, "adapter_id").map_err(crate::ApplicationError::internal)?;
    let model_id =
        required_trimmed(model_id, "model_id").map_err(crate::ApplicationError::internal)?;
    if let Some(provider_id) = draft.source_provider_id.as_deref() {
        let path = provider_model_settings_path(provider_id, adapter_id, model_id);
        let configured = app
            .runtime_config_settings()
            .read_file_settings(agena_runtime::ConfigSettingsGetInput {
                target: agena_runtime::ConfigSettingsPathInput { path: Some(path) },
                source: agena_runtime::ConfigSettingsSource::File,
            })
            .map_err(|error| {
                crate::ApplicationError::internal(format!(
                    "failed to read configured provider model: {error}"
                ))
            })?
            .value;
        if !configured.is_null() {
            return Ok(configured);
        }
    }

    let catalog_entries = app.lookup_model_catalog_models(
        &[model_id.to_owned()]
            .into_iter()
            .chain(
                provider_model
                    .map(provider_model_catalog_lookup_candidates)
                    .into_iter()
                    .flatten(),
            )
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
    app: &Application,
    source: &str,
    target: &str,
) -> std::result::Result<(), ProviderStudioSaveError> {
    let read_path = |path: &str| {
        app.runtime_config_settings()
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
    patch_provider_settings_root(app, JsonValue::Object(changes)).await?;
    Ok(())
}

/// Persist `providers.default` and `providers.default_selection` in one
/// atomic patch. Writing them as two separate edits would leave the file in
/// a partially updated state if the second edit failed validation (the
/// default provider would already be persisted without the selection).
pub(crate) async fn set_provider_default_selection(
    app: &Application,
    provider_id: &str,
    selection: JsonValue,
) -> std::result::Result<agena_runtime::ConfigSettingsEditResponse, crate::ApplicationError> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return Err(crate::ApplicationError::internal("provider id is required"));
    }
    let mut changes = JsonMap::new();
    changes.insert(
        "default".to_owned(),
        JsonValue::String(provider_id.to_owned()),
    );
    changes.insert("default_selection".to_owned(), selection);
    let response = app
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
        .map_err(|error| {
            crate::ApplicationError::internal(format!(
                "failed to set provider default selection: {error}"
            ))
        })?;

    if response.reload_required {
        app.runtime_control().reload().await.map_err(|error| {
            crate::ApplicationError::internal(format!(
                "failed to reload runtime after provider default selection change: {error}"
            ))
        })?;
    }
    Ok(response)
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

/// Keep user-authored model overrides when a provider-level save refreshes the
/// route list. The catalog is still the baseline for newly discovered models,
/// but re-saving a provider must not silently reset a model edited earlier in
/// the Model Studio (display name, limits, capabilities, modes, etc.).
fn preserve_existing_model_config(
    mut generated: JsonValue,
    existing: Option<&JsonValue>,
) -> JsonValue {
    let Some(existing_model) = existing.and_then(JsonValue::as_object) else {
        return generated;
    };
    let Some(generated_model) = generated.as_object_mut() else {
        return generated;
    };
    for (field, value) in existing_model {
        generated_model.insert(field.clone(), value.clone());
    }
    generated
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

// ---------------------------------------------------------------------------
// settings.rs operations
// ---------------------------------------------------------------------------

pub(crate) fn read_file_provider_settings(
    app: &Application,
    provider_id: &str,
) -> anyhow::Result<Option<JsonValue>> {
    let configured = app
        .runtime_config_settings()
        .read_file_settings(agena_runtime::ConfigSettingsGetInput {
            target: agena_runtime::ConfigSettingsPathInput {
                path: Some(provider_settings_path(provider_id)),
            },
            source: agena_runtime::ConfigSettingsSource::File,
        })
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to read configured provider")?
        .value;
    if configured.is_null() {
        Ok(None)
    } else {
        Ok(Some(configured))
    }
}

pub(crate) async fn save_provider_model_value(
    app: &Application,
    draft: ProviderConfigDraft,
    adapter_id: &str,
    model_id: &str,
    model_value: JsonValue,
) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
    let mut draft = draft;
    draft.normalize_shape();
    let provider_id = required_provider_save_field(
        draft.provider_id.as_str(),
        ProviderStudioSaveField::ProviderId,
    )
    .map_err(ProviderStudioSaveError::Validation)?;
    let adapter_id = required_provider_save_field(adapter_id, ProviderStudioSaveField::AdapterId)
        .map_err(ProviderStudioSaveError::Validation)?;
    let model_id = required_provider_save_field(model_id, ProviderStudioSaveField::ModelId)
        .map_err(ProviderStudioSaveError::Validation)?;
    let JsonValue::Object(_) = &model_value else {
        return Err(ProviderStudioSaveError::ProviderModelConfigMustBeObject);
    };
    let effective_adapter_ids =
        effective_provider_draft_adapter_ids(app, &draft, &[adapter_id.to_owned()]);
    if draft.default_adapter.trim().is_empty() {
        draft.default_adapter = adapter_id.to_owned();
    }
    draft
        .validate_for_adapters_for_save(&effective_adapter_ids)
        .map_err(ProviderStudioSaveError::Validation)?;
    let current_provider =
        read_file_provider_settings(app, provider_id).map_err(ProviderStudioSaveError::other)?;
    let current_defaults = current_provider
        .as_ref()
        .and_then(JsonValue::as_object)
        .and_then(|provider| provider.get("defaults"))
        .and_then(JsonValue::as_object);
    let default_adapter = current_defaults
        .and_then(|defaults| defaults.get("adapter"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| optional_non_empty(draft.default_adapter.as_str()))
        .unwrap_or(adapter_id);
    let default_model = current_defaults
        .and_then(|defaults| defaults.get("model"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| optional_non_empty(draft.default_model.as_str()));
    let existing_adapter = draft
        .source_provider_id
        .as_deref()
        .map(|provider_id| {
            app.runtime_config_settings()
                .read_file_settings(agena_runtime::ConfigSettingsGetInput {
                    target: agena_runtime::ConfigSettingsPathInput {
                        path: Some(provider_adapter_settings_path(provider_id, adapter_id)),
                    },
                    source: agena_runtime::ConfigSettingsSource::File,
                })
                .map_err(ProviderStudioSaveError::other)
                .map(|response| response.value)
        })
        .transpose()?;
    let model_overlay =
        serde_json::from_value::<agena_provider::ResolvedProviderModelConfig>(model_value)
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
    // Always materialize the resolved draft defaults into the writable
    // provider patch. Otherwise an existing provider whose defaults came
    // from another config layer fails standalone file validation even
    // though Provider Studio visibly shows a selected default adapter.
    let mut defaults = JsonMap::new();
    defaults.insert(
        "adapter".to_owned(),
        JsonValue::String(default_adapter.to_owned()),
    );
    if let Some(default_model) = default_model {
        defaults.insert(
            "model".to_owned(),
            JsonValue::String(default_model.to_owned()),
        );
    }
    provider_patch.insert("defaults".to_owned(), JsonValue::Object(defaults));
    patch_provider_settings(app, provider_id, JsonValue::Object(provider_patch)).await?;
    Ok(ProviderStudioSaveResult::ConfiguredModelSaved {
        provider_id: provider_id.to_owned(),
        adapter_id: adapter_id.to_owned(),
        model_id: model_id.to_owned(),
    })
}

pub(crate) async fn delete_provider_model(
    app: &Application,
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
    let adapter_id = required_provider_save_field(adapter_id, ProviderStudioSaveField::AdapterId)
        .map_err(ProviderStudioSaveError::Validation)?;
    let model_id = required_provider_save_field(model_id, ProviderStudioSaveField::ModelId)
        .map_err(ProviderStudioSaveError::Validation)?;
    let effective_adapter_ids =
        effective_provider_draft_adapter_ids(app, &draft, &[adapter_id.to_owned()]);
    if draft.default_adapter.trim().is_empty() {
        draft.default_adapter = adapter_id.to_owned();
    }
    draft
        .validate_for_adapters_for_save(&effective_adapter_ids)
        .map_err(ProviderStudioSaveError::Validation)?;

    let mut provider_value = read_file_provider_settings(app, provider_id)
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

    set_provider_settings(app, provider_id, provider_value).await?;
    Ok(ProviderStudioSaveResult::ModelDeleted {
        provider_id: provider_id.to_owned(),
        adapter_id: adapter_id.to_owned(),
        model_id: model_id.to_owned(),
    })
}

pub(crate) async fn delete_provider(
    app: &Application,
    provider_id: &str,
) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
    let provider_id =
        required_provider_save_field(provider_id, ProviderStudioSaveField::ProviderId)
            .map_err(ProviderStudioSaveError::Validation)?;
    let provider_value = read_file_provider_settings(app, provider_id)
        .map_err(ProviderStudioSaveError::other)?
        .ok_or(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject)?;
    if !provider_value.is_object() {
        return Err(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject);
    }

    // Removing a provider that `providers.default` or
    // `providers.default_selection` still references would fail the
    // post-edit config validation. Clear every reference to the deleted
    // provider in the same atomic patch so the resulting file validates.
    let configured_default_provider = app
        .runtime_config_settings()
        .read_file_settings(agena_runtime::ConfigSettingsGetInput {
            target: agena_runtime::ConfigSettingsPathInput {
                path: Some("providers.default".to_owned()),
            },
            source: agena_runtime::ConfigSettingsSource::File,
        })
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to read configured default provider")
        .map_err(ProviderStudioSaveError::other)?
        .value;
    let configured_default_selection = app
        .runtime_config_settings()
        .read_file_settings(agena_runtime::ConfigSettingsGetInput {
            target: agena_runtime::ConfigSettingsPathInput {
                path: Some("providers.default_selection".to_owned()),
            },
            source: agena_runtime::ConfigSettingsSource::File,
        })
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to read configured default provider selection")
        .map_err(ProviderStudioSaveError::other)?
        .value;
    let clears_default_provider = configured_default_provider
        .as_str()
        .map(str::trim)
        .is_some_and(|configured| configured == provider_id);
    let default_selection_points_at_provider = configured_default_selection
        .as_object()
        .and_then(|selection| selection.get("provider"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .is_some_and(|configured| configured == provider_id);

    let mut changes = JsonMap::new();
    changes.insert(provider_id.to_owned(), JsonValue::Null);
    if clears_default_provider {
        changes.insert("default".to_owned(), JsonValue::Null);
    }
    if default_selection_points_at_provider {
        changes.insert("default_selection".to_owned(), JsonValue::Null);
    }
    patch_provider_settings_root(app, JsonValue::Object(changes)).await?;
    Ok(ProviderStudioSaveResult::ProviderDeleted {
        provider_id: provider_id.to_owned(),
    })
}

pub(crate) async fn delete_provider_adapter(
    app: &Application,
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
    let adapter_id = required_provider_save_field(adapter_id, ProviderStudioSaveField::AdapterId)
        .map_err(ProviderStudioSaveError::Validation)?;

    let mut provider_value = read_file_provider_settings(app, provider_id)
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
                return Err(ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject);
            }
        };
        (removed_model_count, adapters.is_empty())
    };
    if delete_provider_after {
        return delete_provider(app, provider_id).await;
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

    set_provider_settings(app, provider_id, provider_value).await?;
    // The file-level `providers.default_selection` can still reference the
    // adapter just deleted (its `adapter` field). Resolution does not
    // reject that reference, so it would silently point at a missing
    // adapter at runtime. Clear the selection when it points at this
    // provider and the removed adapter so it falls back to the provider
    // defaults written above.
    clear_default_selection_for_removed_adapter(app, provider_id, adapter_id).await?;
    Ok(ProviderStudioSaveResult::AdapterDeleted {
        provider_id: provider_id.to_owned(),
        adapter_id: adapter_id.to_owned(),
        removed_model_count,
    })
}

/// Drop `providers.default_selection` when it references `provider_id` with
/// the removed `adapter_id`, so the effective selection falls back to the
/// provider defaults instead of pointing at a deleted adapter.
async fn clear_default_selection_for_removed_adapter(
    app: &Application,
    provider_id: &str,
    adapter_id: &str,
) -> std::result::Result<(), ProviderStudioSaveError> {
    let selection = app
        .runtime_config_settings()
        .read_file_settings(agena_runtime::ConfigSettingsGetInput {
            target: agena_runtime::ConfigSettingsPathInput {
                path: Some("providers.default_selection".to_owned()),
            },
            source: agena_runtime::ConfigSettingsSource::File,
        })
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to read configured default provider selection")
        .map_err(ProviderStudioSaveError::other)?
        .value;
    let Some(selection) = selection.as_object() else {
        return Ok(());
    };
    let references_removed = selection
        .get("provider")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .is_some_and(|configured| configured == provider_id)
        && selection
            .get("adapter")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .is_some_and(|configured| configured == adapter_id);
    if !references_removed {
        return Ok(());
    }
    let mut changes = JsonMap::new();
    changes.insert("default_selection".to_owned(), JsonValue::Null);
    patch_provider_settings_root(app, JsonValue::Object(changes)).await?;
    Ok(())
}

pub(crate) async fn patch_provider_settings(
    app: &Application,
    provider_id: &str,
    provider_patch: JsonValue,
) -> std::result::Result<agena_runtime::ConfigSettingsEditResponse, ProviderStudioSaveError> {
    let response = app.runtime_config_settings().patch_file_settings(
        agena_runtime::ConfigSettingsPatchInput {
            target: agena_runtime::ConfigSettingsPathInput {
                path: Some("providers".to_owned()),
            },
            changes: json!({
                provider_id: provider_patch,
            }),
            options: agena_runtime::ConfigSettingsEditOptions {
                dry_run: false,
                validate: true,
                reload: true,
            },
        },
    )?;

    if response.reload_required {
        app.runtime_control()
            .reload()
            .await
            .context("failed to reload runtime after provider settings change")
            .map_err(ProviderStudioSaveError::other)?;
    }
    Ok(response)
}

/// Patch the `providers` root map atomically. Null values delete keys, so a
/// single edit can remove a provider and every `default`/`default_selection`
/// reference to it before the resulting document is validated.
pub(crate) async fn patch_provider_settings_root(
    app: &Application,
    changes: JsonValue,
) -> std::result::Result<agena_runtime::ConfigSettingsEditResponse, ProviderStudioSaveError> {
    let response = app.runtime_config_settings().patch_file_settings(
        agena_runtime::ConfigSettingsPatchInput {
            target: agena_runtime::ConfigSettingsPathInput {
                path: Some("providers".to_owned()),
            },
            changes,
            options: agena_runtime::ConfigSettingsEditOptions {
                dry_run: false,
                validate: true,
                reload: true,
            },
        },
    )?;

    if response.reload_required {
        app.runtime_control()
            .reload()
            .await
            .context("failed to reload runtime after provider settings change")
            .map_err(ProviderStudioSaveError::other)?;
    }
    Ok(response)
}

pub(crate) async fn set_provider_settings(
    app: &Application,
    provider_id: &str,
    provider_value: JsonValue,
) -> std::result::Result<agena_runtime::ConfigSettingsEditResponse, ProviderStudioSaveError> {
    let response =
        app.runtime_config_settings()
            .set_file_setting(agena_runtime::ConfigSettingsSetInput {
                path: format!("providers.{}", quoted_settings_segment(provider_id)),
                value: provider_value,
                options: agena_runtime::ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })?;

    if response.reload_required {
        app.runtime_control()
            .reload()
            .await
            .context("failed to reload runtime after provider settings change")
            .map_err(ProviderStudioSaveError::other)?;
    }
    Ok(response)
}

/// Picks the effective thinking-mode selector for the composer status: the
/// mode the model marks as default, or the first listed mode when no default
/// is marked (catalog models commonly leave every mode unmarked). Returns
/// `None` only when the model exposes no thinking modes at all.
pub(crate) fn default_thinking_mode_selector(
    modes: &[agena_domain::ModelThinkingMode],
) -> Option<String> {
    modes
        .iter()
        .find(|mode| mode.is_default)
        .or_else(|| modes.first())
        .and_then(|mode| mode.selector().map(|selector| selector.into_owned()))
}

/// Picks the effective speed-mode name for the composer status. A speed mode
/// is only effective by default when the model explicitly marks it as such;
/// an unmarked speed catalog means that no speed override should be sent and
/// the provider/model native default should be used.
pub(crate) fn default_speed_mode_name(
    modes: &std::collections::BTreeMap<String, agena_domain::ModelSpeedMode>,
) -> Option<String> {
    modes
        .iter()
        .find(|(_, mode)| mode.is_default)
        .map(|(name, _)| name.clone())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::collections::BTreeMap;

    use super::{default_speed_mode_name, preserve_existing_model_config};

    #[test]
    fn provider_save_preserves_existing_model_overrides() {
        let generated = json!({
            "display_name": "Catalog name",
            "context_window_tokens": 128000,
            "features": ["tool_calling"],
        });
        let existing = json!({
            "display_name": "My name",
            "context_window_tokens": 64000,
            "agena_tools": { "mode": "disabled" },
        });

        assert_eq!(
            preserve_existing_model_config(generated, Some(&existing)),
            json!({
                "display_name": "My name",
                "context_window_tokens": 64000,
                "features": ["tool_calling"],
                "agena_tools": { "mode": "disabled" },
            })
        );
    }

    #[test]
    fn unmarked_speed_modes_do_not_create_an_override() {
        let mut modes = BTreeMap::new();
        modes.insert("fast".to_owned(), agena_domain::ModelSpeedMode::default());
        modes.insert("pro".to_owned(), agena_domain::ModelSpeedMode::default());

        assert_eq!(default_speed_mode_name(&modes), None);
    }

    #[test]
    fn explicitly_marked_speed_mode_is_used_as_the_default_override() {
        let mut modes = BTreeMap::new();
        modes.insert("fast".to_owned(), agena_domain::ModelSpeedMode::default());
        modes.insert(
            "pro".to_owned(),
            agena_domain::ModelSpeedMode {
                is_default: true,
                ..Default::default()
            },
        );

        assert_eq!(default_speed_mode_name(&modes).as_deref(), Some("pro"));
    }
}
