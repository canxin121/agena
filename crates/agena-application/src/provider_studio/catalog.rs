//! Application configuration updates and route policy for Provider Studio.

pub(crate) use agena_api::resource::model_catalog::*;
use agena_provider::draft::optional_non_empty;
use agena_provider::draft::{
    CLINE_API_MODELS_URL, ProviderConfigDraft, ProviderDraftAuthKind, ProviderStudioSaveError,
    ProviderStudioSaveField, ProviderStudioSaveValidationError,
};
use agena_provider::{
    CredentialIssuer, OpenAiResponsesBackendConfig, ProviderAdapterOverlay,
    ProviderCapabilityFamilyConfig,
};
use anyhow::anyhow;
use serde_json::{Map as JsonMap, Value as JsonValue};

pub(crate) fn build_provider_patch_value_for_save(
    draft: &ProviderConfigDraft,
    adapters: JsonValue,
) -> std::result::Result<JsonValue, ProviderStudioSaveError> {
    let adapters = serde_json::from_value::<
        std::collections::BTreeMap<String, ProviderAdapterOverlay>,
    >(adapters)
    .map_err(ProviderStudioSaveError::other)?;
    let mut adapters = adapters;
    apply_provider_auth_required_adapter_defaults_to_overlay_adapters(draft, &mut adapters);
    let overlay = draft.to_provider_overlay_for_save(adapters)?;
    serde_json::to_value(overlay).map_err(ProviderStudioSaveError::other)
}

pub(crate) fn build_provider_auth_patch_value_for_save(
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

pub(crate) fn apply_provider_auth_required_adapter_defaults_to_overlay_adapters(
    draft: &ProviderConfigDraft,
    adapters: &mut std::collections::BTreeMap<String, ProviderAdapterOverlay>,
) {
    for adapter_id in ["openai_responses", "openai_chat_completions"] {
        if let Some(adapter) = adapters.get_mut(adapter_id) {
            apply_provider_auth_required_adapter_defaults_to_overlay(draft, adapter_id, adapter);
        }
    }
}

pub(crate) fn apply_provider_auth_required_adapter_defaults_to_overlay(
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

pub(crate) fn apply_provider_auth_required_adapter_defaults_to_json_adapters(
    draft: &ProviderConfigDraft,
    adapters: &mut JsonMap<String, JsonValue>,
) -> std::result::Result<(), ProviderStudioSaveError> {
    for adapter_id in ["openai_responses", "openai_chat_completions"] {
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

pub(crate) fn apply_provider_auth_required_adapter_defaults_to_json_value(
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

pub(crate) fn apply_provider_auth_required_adapter_defaults_to_json_object(
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

pub(crate) fn provider_model_settings_path(
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

/// Return the stable model id used by Agena for one adapter route.
///
/// Anthropic model discovery can return CPA's reversible `claude-fable-5-dd-`
/// aliases, so Provider Studio uses the same stable identity as runtime config.
pub(crate) fn canonical_provider_model_id(adapter_id: &str, model_id: &str) -> String {
    if adapter_id == "anthropic" {
        agena_provider::normalize_anthropic_model_id(model_id)
    } else {
        model_id.to_owned()
    }
}

pub(crate) fn provider_adapter_settings_path(provider_id: &str, adapter_id: &str) -> String {
    format!(
        "providers.{}.adapters.{}",
        quoted_settings_segment(provider_id),
        quoted_settings_segment(adapter_id),
    )
}

pub(crate) fn provider_settings_path(provider_id: &str) -> String {
    format!("providers.{}", quoted_settings_segment(provider_id))
}

pub(crate) fn merge_provider_model_adapter_patch_for_save(
    existing_adapter: Option<JsonValue>,
    adapter_id: &str,
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
    // A complete model edit replaces the entry; prompts are configured on
    // prompt layers, not on provider model routes.
    models_object.insert(
        canonical_provider_model_id(adapter_id, model_id),
        model_value,
    );
    Ok(JsonValue::Object(adapter))
}

pub(crate) fn provider_model_selection_contains(
    selected_model_keys: &std::collections::BTreeSet<String>,
    adapter_id: &str,
    model_id: &str,
) -> bool {
    selected_model_keys.contains(format!("{adapter_id}\u{1f}{model_id}").as_str())
}

pub(crate) fn required_provider_save_field(
    value: &str,
    field: ProviderStudioSaveField,
) -> std::result::Result<&str, ProviderStudioSaveValidationError> {
    optional_non_empty(value).ok_or(ProviderStudioSaveValidationError::FieldRequired(field))
}

pub(crate) fn quoted_settings_segment(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Required non-empty trimmed value, erroring with a `{field} is required`
/// message otherwise.
pub(crate) fn required_trimmed<'a>(value: &'a str, field: &str) -> anyhow::Result<&'a str> {
    optional_non_empty(value).ok_or_else(|| anyhow!("{field} is required"))
}
