use anyhow::anyhow;

pub(super) fn local_model_catalog_summary(
    catalog: &agena::model_catalog::ModelCatalogResponse,
) -> LocalModelCatalogResponse {
    LocalModelCatalogResponse {
        refreshing: false,
        last_refresh_at: catalog.last_refresh_at,
        last_successful_source: catalog.last_successful_source,
        last_error: catalog.last_error.clone(),
        model_count: catalog.models.len(),
    }
}

pub(super) fn local_model_catalog_models(
    catalog: &agena::model_catalog::ModelCatalogResponse,
) -> Vec<CatalogModelResource> {
    catalog
        .models
        .iter()
        .cloned()
        .map(|model| CatalogModelResource::from_record(model, catalog.last_successful_source))
        .collect()
}

pub(super) fn local_model_catalog_model_search_text(model: &CatalogModelResource) -> String {
    let thinking_mode_text = model
        .thinking_modes
        .iter()
        .flat_map(|(name, mode)| {
            [
                name.clone(),
                mode.display_name.clone().unwrap_or_default(),
                mode.description.clone().unwrap_or_default(),
                mode.thinking
                    .as_ref()
                    .and_then(|value| serde_json::to_string(value).ok())
                    .unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n");
    let speed_mode_text = model
        .speed_modes
        .iter()
        .flat_map(|(name, mode)| {
            [
                name.clone(),
                mode.display_name.clone().unwrap_or_default(),
                mode.description.clone().unwrap_or_default(),
                serde_json::to_string(&mode.request_override).unwrap_or_default(),
                serde_json::to_string(&mode.adapter_overrides).unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n");

    [
        model.model_id.clone(),
        model.display_name.clone().unwrap_or_default(),
        model.origin.clone().unwrap_or_default(),
        model.description.clone().unwrap_or_default(),
        model
            .context_window_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        model
            .max_input_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        model
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        match model.source {
            ModelCatalogSourceKind::Generated => "generated".to_owned(),
            ModelCatalogSourceKind::Cache => "cache".to_owned(),
        },
        model.source_label.clone().unwrap_or_default(),
        model
            .lifecycle
            .map(|value| match value {
                agena::model::ModelLifecycle::Active => "active",
                agena::model::ModelLifecycle::Preview => "preview",
                agena::model::ModelLifecycle::Beta => "beta",
                agena::model::ModelLifecycle::Alpha => "alpha",
                agena::model::ModelLifecycle::Experimental => "experimental",
                agena::model::ModelLifecycle::Deprecated => "deprecated",
            })
            .unwrap_or_default()
            .to_owned(),
        thinking_mode_text,
        speed_mode_text,
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase()
}

pub(super) fn preferred_catalog_model_for_model_id<'a>(
    models: &'a [CatalogModelResource],
    model_id: &str,
) -> Option<&'a CatalogModelResource> {
    preferred_catalog_model_for_lookup_ids(models, &[model_id.to_owned()])
}

pub(super) fn preferred_catalog_model_for_lookup_ids<'a>(
    models: &'a [CatalogModelResource],
    model_ids: &[String],
) -> Option<&'a CatalogModelResource> {
    let lookup_ids = model_ids
        .iter()
        .map(|model_id| model_id.trim())
        .filter(|model_id| !model_id.is_empty())
        .collect::<Vec<_>>();
    models
        .iter()
        .filter(|model| {
            lookup_ids
                .iter()
                .any(|model_id| model.model_id == *model_id)
        })
        .min_by_key(|model| model.model_id.as_str())
}

pub(super) fn preferred_catalog_model_for_provider_model<'a>(
    models: &'a [CatalogModelResource],
    provider_model: &ProviderModel,
) -> Option<&'a CatalogModelResource> {
    preferred_catalog_model_for_lookup_ids(
        models,
        &[
            provider_model.id.to_string(),
            catalog_lookup_id_for_provider_model(provider_model),
        ],
    )
}

pub(super) fn catalog_lookup_id_for_model_id(model_id: &str) -> String {
    agena::model_catalog::canonical_model_catalog_id(model_id)
}

pub(super) fn catalog_lookup_id_for_provider_model(provider_model: &ProviderModel) -> String {
    provider_model
        .catalog_model_id
        .as_ref()
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| catalog_lookup_id_for_model_id(provider_model.id.as_ref()))
}

pub(super) fn provider_model_json_for_model_id(
    catalog_entries: &[CatalogModelResource],
    model_id: &str,
    provider_model: Option<&ProviderModel>,
) -> JsonValue {
    provider_model_overlay_to_json(provider_model_overlay_for_model_id(
        catalog_entries,
        model_id,
        provider_model,
    ))
}

pub(super) fn provider_model_overlay_for_model_id(
    catalog_entries: &[CatalogModelResource],
    model_id: &str,
    provider_model: Option<&ProviderModel>,
) -> ProviderModelOverlay {
    let mut overlay = preferred_catalog_model_for_model_id(catalog_entries, model_id)
        .or_else(|| {
            let lookup_id = catalog_lookup_id_for_model_id(model_id);
            (lookup_id != model_id)
                .then(|| preferred_catalog_model_for_model_id(catalog_entries, lookup_id.as_str()))
                .flatten()
        })
        .map(catalog_model_to_provider_model_overlay)
        .or_else(|| {
            provider_model.and_then(|provider_model| {
                preferred_catalog_model_for_provider_model(catalog_entries, provider_model)
                    .map(catalog_model_to_provider_model_overlay)
                    .or_else(|| Some(provider_model_to_provider_model_overlay(provider_model)))
            })
        })
        .unwrap_or_default();
    if let Some(provider_model) = provider_model {
        overlay.native_compaction = provider_model.native_compaction;
    }
    overlay
}

pub(super) fn provider_model_overlay_to_json(overlay: ProviderModelOverlay) -> JsonValue {
    match serde_json::to_value(overlay) {
        Ok(JsonValue::Object(mut value)) => {
            if matches!(value.get("enabled"), Some(JsonValue::Bool(true))) {
                value.remove("enabled");
            }
            JsonValue::Object(value)
        }
        Ok(other) => other,
        Err(_) => JsonValue::Object(JsonMap::new()),
    }
}

pub(super) fn catalog_model_to_provider_model_overlay(
    model: &CatalogModelResource,
) -> ProviderModelOverlay {
    provider_model_overlay_from_catalog_definition(&catalog_model_to_catalog_definition(model))
}

pub(super) fn catalog_model_to_catalog_definition(
    model: &CatalogModelResource,
) -> CatalogModelDefinition {
    CatalogModelDefinition::from_fields(
        model.lifecycle,
        model.context_window_tokens,
        model.max_input_tokens,
        model.max_output_tokens,
        model.description.clone(),
        model.knowledge_cutoff.clone(),
        model.release_date.clone(),
        model.last_updated.clone(),
        model.open_weights,
        model.supports_parallel_tool_calls,
        model.supports_verbosity,
        model.default_verbosity.clone(),
        model.default_temperature.clone(),
        model.default_top_p.clone(),
        model.default_top_k,
        model.assistant_reasoning_interleaved,
        model.assistant_reasoning_field.clone(),
        model.output_modalities.clone(),
        model.pricing.clone(),
        model.display_name.clone(),
        model.origin.clone(),
        model.thinking_modes.clone(),
        model.speed_modes.clone(),
        sanitized_catalog_capability_patch(&model.capabilities),
    )
}

pub(super) fn sanitized_catalog_capability_patch(
    patch: &agena::provider::ModelCapabilityPatch,
) -> agena::provider::ModelCapabilityPatch {
    let mut patch = patch.clone();
    patch.input = sanitize_selection_patch(patch.input.take());
    patch.features = sanitize_selection_patch(patch.features.take());

    patch
}

pub(super) fn sanitize_selection_patch<T: Clone + PartialEq>(
    patch: Option<agena::provider::CapabilitySelectionPatch<T>>,
) -> Option<agena::provider::CapabilitySelectionPatch<T>> {
    let patch = patch?;
    match patch {
        agena::provider::CapabilitySelectionPatch::Supported(mut supported) => {
            dedupe_vec(&mut supported);
            agena::provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
                supported,
                Vec::new(),
            )
        }
        agena::provider::CapabilitySelectionPatch::Patch(mut values) => {
            dedupe_vec(&mut values.supported);
            dedupe_vec(&mut values.unsupported);
            values
                .unsupported
                .retain(|value| !values.supported.contains(value));
            agena::provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
                values.supported,
                values.unsupported,
            )
        }
    }
}

pub(super) fn provider_model_to_provider_model_overlay(
    model: &ProviderModel,
) -> ProviderModelOverlay {
    let mut overlay =
        provider_model_overlay_from_catalog_definition(&catalog_definition_from_model(model));
    overlay.native_compaction = model.native_compaction;
    overlay
}

pub(super) fn dedupe_vec<T: PartialEq>(values: &mut Vec<T>) {
    let mut index = 0;
    while index < values.len() {
        let mut next = index + 1;
        while next < values.len() {
            if values[index] == values[next] {
                values.remove(next);
            } else {
                next += 1;
            }
        }
        index += 1;
    }
}

pub(super) fn ensure_provider_model_entry(
    adapter_value: &mut JsonValue,
    model_id: &str,
    model_value: JsonValue,
) -> Result<()> {
    let adapter = adapter_value
        .as_object_mut()
        .ok_or_else(|| anyhow!("adapter patch must be an object"))?;
    adapter.insert("enabled".to_owned(), JsonValue::Bool(true));
    if !adapter.contains_key("models") {
        adapter.insert("models".to_owned(), JsonValue::Object(JsonMap::new()));
    }
    let models = adapter
        .get_mut("models")
        .and_then(JsonValue::as_object_mut)
        .ok_or_else(|| anyhow!("adapter models patch must be an object"))?;
    models.insert(model_id.to_owned(), model_value);
    Ok(())
}

pub(super) fn map_provider_adapter_models_config_error(
    error: agena::config::ConfigError,
) -> anyhow::Error {
    match error {
        agena::config::ConfigError::Validation(message)
        | agena::config::ConfigError::App(agena::AppError::Config(message)) => anyhow!(message),
        other => anyhow!(other.to_string()),
    }
}

pub(super) fn supported_provider_draft_adapter_list(auth_kind: &ProviderDraftAuthKind) -> String {
    let supported = auth_kind
        .adapter_rules()
        .iter()
        .map(|rule| rule.adapter_id)
        .collect::<Vec<_>>()
        .join(", ");
    if supported.is_empty() {
        "no adapters until auth details are selected".to_owned()
    } else {
        supported
    }
}

pub(super) fn parse_oauth_expires_at_ms(value: &str) -> Result<i64> {
    optional_non_empty(value)
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| anyhow!("expires_at_ms must be an integer"))
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

pub(super) fn build_provider_patch_value_for_save(
    draft: &ProviderConfigDraft,
    default_adapter: &str,
    default_model: Option<&str>,
    adapters: JsonValue,
    include_defaults: bool,
) -> std::result::Result<JsonValue, ProviderStudioSaveError> {
    let adapters = serde_json::from_value::<
        std::collections::BTreeMap<String, ProviderAdapterOverlay>,
    >(adapters)
    .map_err(ProviderStudioSaveError::other)?;
    let mut adapters = adapters;
    apply_provider_auth_required_adapter_defaults_to_overlay_adapters(draft, &mut adapters);
    let overlay = draft.to_provider_overlay_for_save(
        default_adapter,
        default_model,
        adapters,
        include_defaults,
    )?;
    serde_json::to_value(overlay).map_err(ProviderStudioSaveError::other)
}

pub(super) fn build_provider_auth_patch_value_for_save(
    draft: &ProviderConfigDraft,
) -> std::result::Result<JsonMap<String, JsonValue>, ProviderStudioSaveError> {
    serde_json::to_value(draft.to_auth_overlay_for_save()?)
        .map_err(ProviderStudioSaveError::other)
        .and_then(|value| match value {
            JsonValue::Object(object) => Ok(object),
            _ => Err(ProviderStudioSaveError::other(
                "provider auth overlay must serialize as an object",
            )),
        })
}

pub(super) fn provider_draft_protocol_paths_for_listing(
    draft: &ProviderConfigDraft,
) -> agena::config::ProviderProtocolPathsConfig {
    if matches!(draft.auth_kind, ProviderDraftAuthKind::ClineApi) {
        cline_api_protocol_paths()
    } else {
        agena::config::ProviderProtocolPathsConfig::default()
    }
}

pub(super) fn apply_known_provider_listing_defaults(
    draft: &ProviderConfigDraft,
    target: &mut agena::config::ProviderAdapterModelsTarget,
) {
    if !matches!(draft.auth_kind, ProviderDraftAuthKind::ClineApi) {
        return;
    }

    if let Some(adapter) = target.adapters.get_mut("openai_chat_completions")
        && let ProviderAdapterDefinition::OpenAiChatCompletions(config) = &mut adapter.definition
    {
        config.options.models_url = Some(CLINE_API_MODELS_URL.to_owned());
    }
}

pub(super) fn apply_provider_auth_required_adapter_defaults_to_overlay_adapters(
    draft: &ProviderConfigDraft,
    adapters: &mut std::collections::BTreeMap<String, ProviderAdapterOverlay>,
) {
    for adapter_id in [
        "openai_responses",
        "openai_chat_completions",
        "openai_realtime",
    ] {
        if let Some(adapter) = adapters.get_mut(adapter_id) {
            apply_provider_auth_required_adapter_defaults_to_overlay(draft, adapter_id, adapter);
        }
    }
}

pub(super) fn apply_provider_auth_required_adapter_defaults_to_overlay(
    draft: &ProviderConfigDraft,
    adapter_id: &str,
    adapter: &mut ProviderAdapterOverlay,
) {
    if matches!(draft.auth_kind, ProviderDraftAuthKind::ClineApi)
        && adapter_id == "openai_chat_completions"
    {
        adapter.models_url = Some(CLINE_API_MODELS_URL.to_owned());
    }

    match (draft.auth_kind.credential_issuer(), adapter_id) {
        (Some(CredentialIssuer::OpenaiChatgpt), "openai_responses") => {
            adapter.backend = Some(OpenAiResponsesBackendConfig::ChatgptCodex);
        }
        (Some(CredentialIssuer::GoogleAdc), "openai_chat_completions") => {
            adapter.capability_family = Some(ProviderCapabilityFamilyConfig::Gemini);
        }
        _ => {}
    }
}

pub(super) fn apply_provider_auth_required_adapter_defaults_to_json_adapters(
    draft: &ProviderConfigDraft,
    adapters: &mut JsonMap<String, JsonValue>,
) -> std::result::Result<(), ProviderStudioSaveError> {
    for adapter_id in [
        "openai_responses",
        "openai_chat_completions",
        "openai_realtime",
    ] {
        if let Some(adapter_value) = adapters.get_mut(adapter_id) {
            apply_provider_auth_required_adapter_defaults_to_json_value(
                draft,
                adapter_id,
                adapter_value,
            )?;
        }
    }
    Ok(())
}

pub(super) fn apply_provider_auth_required_adapter_defaults_to_json_value(
    draft: &ProviderConfigDraft,
    adapter_id: &str,
    adapter_value: &mut JsonValue,
) -> std::result::Result<(), ProviderStudioSaveError> {
    let adapter_object = adapter_value.as_object_mut().ok_or_else(|| {
        ProviderStudioSaveError::ProviderAdapterMustBeObject {
            adapter_id: adapter_id.to_owned(),
        }
    })?;
    apply_provider_auth_required_adapter_defaults_to_json_object(draft, adapter_id, adapter_object);
    Ok(())
}

pub(super) fn apply_provider_auth_required_adapter_defaults_to_json_object(
    draft: &ProviderConfigDraft,
    adapter_id: &str,
    adapter: &mut JsonMap<String, JsonValue>,
) {
    if matches!(draft.auth_kind, ProviderDraftAuthKind::ClineApi)
        && adapter_id == "openai_chat_completions"
    {
        adapter.insert(
            "models_url".to_owned(),
            JsonValue::String(CLINE_API_MODELS_URL.to_owned()),
        );
    }

    match (draft.auth_kind.credential_issuer(), adapter_id) {
        (Some(CredentialIssuer::OpenaiChatgpt), "openai_responses") => {
            adapter.insert(
                "backend".to_owned(),
                JsonValue::String("chatgpt_codex".to_owned()),
            );
        }
        (Some(CredentialIssuer::GoogleAdc), "openai_chat_completions") => {
            adapter.insert(
                "capability_family".to_owned(),
                JsonValue::String("gemini".to_owned()),
            );
        }
        _ => {}
    }
}

pub(super) fn provider_model_settings_path(
    provider_id: &str,
    adapter_id: &str,
    model_id: &str,
) -> String {
    format!(
        "providers.{}.adapters.{}.models.{}",
        quoted_settings_segment(provider_id),
        quoted_settings_segment(adapter_id),
        quoted_settings_segment(model_id),
    )
}

pub(super) fn provider_adapter_settings_path(provider_id: &str, adapter_id: &str) -> String {
    format!(
        "providers.{}.adapters.{}",
        quoted_settings_segment(provider_id),
        quoted_settings_segment(adapter_id),
    )
}

pub(super) fn provider_settings_path(provider_id: &str) -> String {
    format!("providers.{}", quoted_settings_segment(provider_id))
}

pub(super) fn merge_provider_model_adapter_patch_for_save(
    existing_adapter: Option<JsonValue>,
    model_id: &str,
    model_value: JsonValue,
) -> std::result::Result<JsonValue, ProviderStudioSaveError> {
    let mut adapter = match existing_adapter {
        Some(JsonValue::Object(object)) => object,
        Some(JsonValue::Null) | None => JsonMap::new(),
        Some(_) => {
            return Err(ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject);
        }
    };
    adapter.insert("enabled".to_owned(), JsonValue::Bool(true));
    let models = adapter
        .entry("models".to_owned())
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    let Some(models_object) = models.as_object_mut() else {
        return Err(ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject);
    };
    models_object.insert(model_id.to_owned(), model_value);
    Ok(JsonValue::Object(adapter))
}

pub(super) fn provider_defaults_point_to(
    provider: &JsonMap<String, JsonValue>,
    adapter_id: &str,
    model_id: &str,
) -> bool {
    let Some(defaults) = provider.get("defaults").and_then(JsonValue::as_object) else {
        return false;
    };
    defaults.get("adapter").and_then(JsonValue::as_str) == Some(adapter_id)
        && defaults.get("model").and_then(JsonValue::as_str) == Some(model_id)
}

pub(super) fn provider_defaults_adapter(provider: &JsonMap<String, JsonValue>) -> Option<&str> {
    provider
        .get("defaults")
        .and_then(JsonValue::as_object)
        .and_then(|defaults| defaults.get("adapter"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn provider_model_selection_contains(
    selected_model_keys: &std::collections::BTreeSet<String>,
    adapter_id: &str,
    model_id: &str,
) -> bool {
    selected_model_keys.contains(format!("{adapter_id}\u{1f}{model_id}").as_str())
}

pub(super) fn resolve_provider_defaults_from_value_for_save(
    adapters: &JsonMap<String, JsonValue>,
    requested_default_adapter: Option<&str>,
    requested_default_model: Option<&str>,
) -> std::result::Result<(String, Option<String>), ProviderStudioSaveError> {
    let default_adapter = requested_default_adapter
        .filter(|default_adapter| adapters.contains_key(*default_adapter))
        .map(ToOwned::to_owned)
        .ok_or(ProviderStudioSaveError::Validation(
            ProviderStudioSaveValidationError::FieldRequired(
                ProviderStudioSaveField::DefaultAdapter,
            ),
        ))?;

    if let Some(default_model) = requested_default_model
        && provider_value_contains_model(adapters, default_adapter.as_str(), default_model)
    {
        return Ok((default_adapter, Some(default_model.to_owned())));
    }
    Ok((default_adapter, None))
}

pub(super) fn required_provider_save_field(
    value: &str,
    field: ProviderStudioSaveField,
) -> std::result::Result<&str, ProviderStudioSaveValidationError> {
    optional_non_empty(value).ok_or(ProviderStudioSaveValidationError::FieldRequired(field))
}

pub(super) fn provider_value_contains_model(
    adapters: &JsonMap<String, JsonValue>,
    adapter_id: &str,
    model_id: &str,
) -> bool {
    adapters
        .get(adapter_id)
        .and_then(|adapter| adapter.get("models"))
        .and_then(JsonValue::as_object)
        .is_some_and(|models| models.contains_key(model_id))
}

pub(super) fn quoted_settings_segment(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn credential_issuer_label(issuer: CredentialIssuer) -> &'static str {
    match issuer {
        CredentialIssuer::OpenaiChatgpt => "openai_chatgpt",
        CredentialIssuer::GithubCopilot => "github_copilot",
        CredentialIssuer::Gitlab => "gitlab",
        CredentialIssuer::GoogleAdc => "google_adc",
        CredentialIssuer::SapAiCore => "sap_ai_core",
    }
}
use crate::backend::Result;
use crate::backend::{
    CLINE_API_MODELS_URL, CatalogModelDefinition, CatalogModelResource, CredentialIssuer, JsonMap,
    JsonValue, LocalModelCatalogResponse, ModelCatalogSourceKind, OpenAiResponsesBackendConfig,
    ProviderAdapterDefinition, ProviderAdapterOverlay, ProviderCapabilityFamilyConfig,
    ProviderConfigDraft, ProviderDraftAuthKind, ProviderModel, ProviderModelOverlay,
    ProviderStudioSaveError, ProviderStudioSaveField, ProviderStudioSaveValidationError,
    catalog_definition_from_model, cline_api_protocol_paths, optional_non_empty,
    provider_model_overlay_from_catalog_definition,
};
