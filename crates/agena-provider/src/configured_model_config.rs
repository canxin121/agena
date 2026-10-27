//! Complete persisted configuration for one provider model route.
//!
//! Provider routes only decide how the declared Agena Tool API gateway
//! functions are transported. Ordinary execution tools never become provider
//! declarations, and provider-service capabilities live in ordinary plugins
//! such as `agena.chatgpt`.

use serde::{Deserialize, Serialize};

use crate::{AgenaToolsConfig, ConfiguredModelDefinition};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Resolved configuration of one provider model.
///
/// `thinking_modes` / `speed_modes` are the declared selectors of this model
/// route. The model catalog is the primary source; declare them here for a
/// model the catalog does not cover (or to override its entry), because the
/// runtime never infers modes from a model id.
pub struct ResolvedProviderModelConfig {
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Whether this model route may use a dedicated conversation compaction
    /// endpoint before falling back to Agena's text summarizer.
    #[serde(skip_serializing_if = "is_true")]
    pub native_compaction: bool,
    /// Additional instructions for this route. Omitted inherits; an empty string
    /// disables inherited additional instructions without changing Agena's prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_system_prompt: Option<String>,
    /// Transport mode for the fixed five-function Agena Tool API.
    #[serde(skip_serializing_if = "AgenaToolsConfig::is_inherited")]
    pub agena_tools: AgenaToolsConfig,
    #[serde(flatten)]
    pub definition: ConfiguredModelDefinition,
}

impl<'de> Deserialize<'de> for ResolvedProviderModelConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        let mut fields = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        const CURRENT_MODEL_FIELDS: &[&str] = &[
            "enabled",
            "native_compaction",
            "additional_system_prompt",
            "agena_tools",
            "lifecycle",
            "context_window_tokens",
            "max_input_tokens",
            "max_output_tokens",
            "display_name",
            "description",
            "knowledge_cutoff",
            "release_date",
            "last_updated",
            "open_weights",
            "supports_parallel_tool_calls",
            "supports_verbosity",
            "default_verbosity",
            "verbosity_levels",
            "provider_default_verbosity",
            "default_temperature",
            "default_top_p",
            "default_top_k",
            "assistant_reasoning_interleaved",
            "assistant_reasoning_field",
            "output_modalities",
            "pricing",
            "thinking_modes",
            "speed_modes",
            "input",
            "features",
            "manual_overrides",
        ];
        if let Some(unknown) = fields
            .keys()
            .find(|key| !CURRENT_MODEL_FIELDS.contains(&key.as_str()))
        {
            return Err(D::Error::custom(format!("unknown field `{unknown}`")));
        }
        let enabled = fields
            .remove("enabled")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .unwrap_or(true);
        let native_compaction = fields
            .remove("native_compaction")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .unwrap_or(true);
        let mut agena_tools: AgenaToolsConfig = match fields.remove("agena_tools") {
            Some(value) => serde_json::from_value(value).map_err(D::Error::custom)?,
            None => AgenaToolsConfig::default(),
        };
        let additional_system_prompt: Option<String> = fields
            .remove("additional_system_prompt")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .flatten();
        let definition: ConfiguredModelDefinition =
            serde_json::from_value(serde_json::Value::Object(fields)).map_err(D::Error::custom)?;
        if agena_tools.is_inherited() {
            agena_tools = AgenaToolsConfig::from_tool_calling_support(
                definition
                    .capabilities
                    .feature_support(crate::ModelCapabilityFeature::ToolCalling),
            );
        }
        Ok(Self {
            enabled,
            native_compaction,
            additional_system_prompt,
            agena_tools,
            definition,
        })
    }
}

impl Default for ResolvedProviderModelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            native_compaction: true,
            additional_system_prompt: None,
            agena_tools: AgenaToolsConfig::default(),
            definition: ConfiguredModelDefinition::default(),
        }
    }
}

fn is_true(value: &bool) -> bool {
    *value
}

#[cfg(test)]
mod tests {
    use super::ResolvedProviderModelConfig;
    use serde_json::json;

    #[test]
    fn additional_system_prompt_preserves_inheritance_empty_and_text() {
        for (input, expected) in [
            (json!({}), None),
            (json!({"additional_system_prompt": null}), None),
            (json!({"additional_system_prompt": ""}), Some("")),
            (
                json!({"additional_system_prompt": "  keep formatting\n"}),
                Some("  keep formatting\n"),
            ),
        ] {
            let configured: ResolvedProviderModelConfig = serde_json::from_value(input).unwrap();
            assert_eq!(configured.additional_system_prompt.as_deref(), expected);
            let serialized = serde_json::to_value(&configured).unwrap();
            assert_eq!(
                serialized.get("additional_system_prompt"),
                expected.map(|value| json!(value)).as_ref()
            );
            let round_trip: ResolvedProviderModelConfig =
                serde_json::from_value(serialized).unwrap();
            assert_eq!(round_trip.additional_system_prompt.as_deref(), expected);
        }
    }

    #[test]
    fn additional_system_prompt_rejects_non_string_values() {
        for value in [json!(false), json!(42), json!([]), json!({})] {
            assert!(
                serde_json::from_value::<ResolvedProviderModelConfig>(
                    json!({"additional_system_prompt": value})
                )
                .is_err()
            );
        }
    }
}
