use std::collections::BTreeSet;

use agena_tui::i18n::I18n;
use agena_tui_backend::ProviderNativeToolsPreset;
use serde_json::Value as JsonValue;

use crate::ProviderModelConfigField;

pub fn provider_model_config_field_label_key(field: ProviderModelConfigField) -> &'static str {
    match field {
        ProviderModelConfigField::ModelId => "provider-model-field-model-id",
        ProviderModelConfigField::Enabled => "provider-model-field-enabled",
        ProviderModelConfigField::NativeCompaction => "provider-model-field-native-compaction",
        ProviderModelConfigField::AgenaToolMode => "provider-model-field-agena-tool-mode",
        // Transition-only enum variant. The active model editor no longer
        // offers provider-native presets.
        ProviderModelConfigField::ProviderNativeTools => {
            "provider-model-field-provider-native-tools"
        }
        ProviderModelConfigField::DisplayName => "provider-model-field-display-name",
        ProviderModelConfigField::Lifecycle => "provider-model-field-lifecycle",
        ProviderModelConfigField::ContextWindowTokens => "provider-model-field-context-window",
        ProviderModelConfigField::MaxInputTokens => "provider-model-field-max-input",
        ProviderModelConfigField::MaxOutputTokens => "provider-model-field-max-output",
        ProviderModelConfigField::Features => "provider-model-field-features",
        ProviderModelConfigField::InputModalities => "provider-model-field-input-modalities",
        ProviderModelConfigField::OutputModalities => "provider-model-field-output-modalities",
        ProviderModelConfigField::ThinkingModes => "provider-model-field-thinking-modes",
        ProviderModelConfigField::SpeedModes => "provider-model-field-speed-modes",
        ProviderModelConfigField::Description => "provider-model-field-description",
    }
}

/// Provider service capabilities are configured as ordinary plugins, so model
/// adapters no longer advertise a provider-native preset.
pub fn provider_native_tools_available_preset_for_adapter(
    _adapter_id: &str,
) -> Option<ProviderNativeToolsPreset> {
    None
}

pub fn provider_native_tools_preset_label(
    i18n: &I18n,
    _preset: ProviderNativeToolsPreset,
) -> String {
    i18n.text("provider-native-tools-disabled-label")
}

pub fn provider_model_overlay_to_json_local(
    overlay: agena_provider::ResolvedProviderModelConfig,
) -> std::result::Result<JsonValue, String> {
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

pub fn trimmed_owned_local(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub fn parse_optional_u32_field(
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

pub fn parse_optional_model_lifecycle(
    value: &str,
) -> std::result::Result<Option<agena_domain::ModelLifecycle>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    serde_json::from_value::<agena_domain::ModelLifecycle>(JsonValue::String(value.to_owned()))
        .map(Some)
        .map_err(|_| format!("unsupported lifecycle `{value}`"))
}

pub fn model_lifecycle_token(value: agena_domain::ModelLifecycle) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub fn split_csv_tokens(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn parse_bool_token(value: &str) -> std::result::Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "enabled" => Ok(true),
        "false" | "no" | "0" | "disabled" => Ok(false),
        other => Err(format!("unsupported boolean `{other}`")),
    }
}

pub fn parse_model_input_modality(value: &str) -> Option<agena_domain::ModelInputModality> {
    match value.trim() {
        "text" => Some(agena_domain::ModelInputModality::Text),
        "image" => Some(agena_domain::ModelInputModality::Image),
        "document" => Some(agena_domain::ModelInputModality::Document),
        "audio" => Some(agena_domain::ModelInputModality::Audio),
        "video" => Some(agena_domain::ModelInputModality::Video),
        "file" => Some(agena_domain::ModelInputModality::File),
        _ => None,
    }
}

pub fn parse_model_input_modality_set(
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

pub fn parse_model_capability_feature(
    value: &str,
) -> Option<agena_provider::ModelCapabilityFeature> {
    match value.trim() {
        "tool_calling" => Some(agena_provider::ModelCapabilityFeature::ToolCalling),
        "streaming" => Some(agena_provider::ModelCapabilityFeature::Streaming),
        "reasoning" => Some(agena_provider::ModelCapabilityFeature::Reasoning),
        "structured_output" => Some(agena_provider::ModelCapabilityFeature::StructuredOutput),
        "temperature" => Some(agena_provider::ModelCapabilityFeature::Temperature),
        _ => None,
    }
}

pub fn parse_model_capability_feature_set(
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
