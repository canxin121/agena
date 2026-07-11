pub(in crate::app) fn provider_model_config_field_label_key(
    field: ProviderModelConfigField,
) -> &'static str {
    match field {
        ProviderModelConfigField::ModelId => "provider-model-field-model-id",
        ProviderModelConfigField::Enabled => "provider-model-field-enabled",
        ProviderModelConfigField::DisplayName => "provider-model-field-display-name",
        ProviderModelConfigField::Lifecycle => "provider-model-field-lifecycle",
        ProviderModelConfigField::ContextWindowTokens => "provider-model-field-context-window",
        ProviderModelConfigField::MaxInputTokens => "provider-model-field-max-input",
        ProviderModelConfigField::MaxOutputTokens => "provider-model-field-max-output",
        ProviderModelConfigField::InputModalities => "provider-model-field-input-modalities",
        ProviderModelConfigField::Features => "provider-model-field-features",
        ProviderModelConfigField::OutputModalities => "provider-model-field-output-modalities",
        ProviderModelConfigField::Description => "provider-model-field-description",
        ProviderModelConfigField::NativeTools => "provider-model-field-native-tools",
    }
}

pub(in crate::app) fn provider_native_tools_available_preset_for_adapter(
    adapter_id: &str,
) -> Option<ProviderNativeToolsPreset> {
    match adapter_id.trim() {
        "openai" => Some(ProviderNativeToolsPreset::OpenAiHostedDefaults),
        "anthropic" => Some(ProviderNativeToolsPreset::AnthropicHostedDefaults),
        "gemini" => Some(ProviderNativeToolsPreset::GeminiHostedDefaults),
        _ => None,
    }
}

pub(in crate::app) fn provider_native_tools_preset_label(
    i18n: &I18n,
    preset: ProviderNativeToolsPreset,
) -> String {
    match preset {
        ProviderNativeToolsPreset::Disabled => {
            ui_text::t(i18n, "provider-native-tools-disabled-label")
        }
        ProviderNativeToolsPreset::OpenAiHostedDefaults => {
            ui_text::t(i18n, "provider-native-tools-openai-label")
        }
        ProviderNativeToolsPreset::AnthropicHostedDefaults => {
            ui_text::t(i18n, "provider-native-tools-anthropic-label")
        }
        ProviderNativeToolsPreset::GeminiHostedDefaults => {
            ui_text::t(i18n, "provider-native-tools-gemini-label")
        }
        ProviderNativeToolsPreset::Custom => ui_text::t(i18n, "provider-native-tools-custom-label"),
    }
}

pub(in crate::app) fn provider_model_overlay_to_json_local(
    overlay: agena::config::ProviderModelOverlay,
) -> std::result::Result<JsonValue, String> {
    if overlay.enabled && overlay.native_tools.is_empty() && overlay.definition.is_empty() {
        return Ok(JsonValue::Object(JsonMap::new()));
    }
    match serde_json::to_value(overlay).map_err(|error| error.to_string())? {
        JsonValue::Object(mut object) => {
            if matches!(object.get("enabled"), Some(JsonValue::Bool(true))) {
                object.remove("enabled");
            }
            Ok(JsonValue::Object(object))
        }
        other => Ok(other),
    }
}

pub(in crate::app) fn trimmed_owned_local(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(in crate::app) fn parse_optional_u32_field(
    value: &str,
    field: &'static str,
) -> std::result::Result<Option<u32>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<u32>()
        .map(Some)
        .map_err(|_| format!("{field} must be an unsigned integer"))
}

pub(in crate::app) fn parse_optional_model_lifecycle(
    value: &str,
) -> std::result::Result<Option<agena::model::ModelLifecycle>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    serde_json::from_value::<agena::model::ModelLifecycle>(JsonValue::String(value.to_owned()))
        .map(Some)
        .map_err(|_| format!("unsupported lifecycle `{value}`"))
}

pub(in crate::app) fn model_lifecycle_token(value: agena::model::ModelLifecycle) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub(in crate::app) fn split_csv_tokens(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(in crate::app) fn parse_bool_token(value: &str) -> std::result::Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "enabled" => Ok(true),
        "false" | "no" | "0" | "disabled" => Ok(false),
        other => Err(format!("unsupported boolean `{other}`")),
    }
}

pub(in crate::app) fn parse_model_input_modality(
    value: &str,
) -> Option<agena::model::ModelInputModality> {
    match value.trim() {
        "text" => Some(agena::model::ModelInputModality::Text),
        "image" => Some(agena::model::ModelInputModality::Image),
        "document" => Some(agena::model::ModelInputModality::Document),
        "audio" => Some(agena::model::ModelInputModality::Audio),
        "video" => Some(agena::model::ModelInputModality::Video),
        "file" => Some(agena::model::ModelInputModality::File),
        _ => None,
    }
}

pub(in crate::app) fn parse_model_input_modality_set(
    value: &str,
) -> std::result::Result<BTreeSet<String>, String> {
    let mut parsed = BTreeSet::new();
    for token in split_csv_tokens(value) {
        if parse_model_input_modality(token.as_str()).is_none() {
            return Err(format!("unsupported input modality `{token}`"));
        }
        parsed.insert(token);
    }
    Ok(parsed)
}

pub(in crate::app) fn parse_model_capability_feature(
    value: &str,
) -> Option<agena::provider::ModelCapabilityFeature> {
    match value.trim() {
        "tool_calling" => Some(agena::provider::ModelCapabilityFeature::ToolCalling),
        "streaming" => Some(agena::provider::ModelCapabilityFeature::Streaming),
        "reasoning" => Some(agena::provider::ModelCapabilityFeature::Reasoning),
        "structured_output" => Some(agena::provider::ModelCapabilityFeature::StructuredOutput),
        "temperature" => Some(agena::provider::ModelCapabilityFeature::Temperature),
        _ => None,
    }
}

pub(in crate::app) fn parse_model_capability_feature_set(
    value: &str,
) -> std::result::Result<BTreeSet<String>, String> {
    let mut parsed = BTreeSet::new();
    for token in split_csv_tokens(value) {
        if parse_model_capability_feature(token.as_str()).is_none() {
            return Err(format!("unsupported model feature `{token}`"));
        }
        parsed.insert(token);
    }
    Ok(parsed)
}
use super::{
    BTreeSet, I18n, JsonMap, JsonValue, ProviderModelConfigField, ProviderNativeToolsPreset,
    ui_text,
};
