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
pub struct ResolvedProviderModelConfig {
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Whether this model route may use a dedicated conversation compaction
    /// endpoint before falling back to Agena's text summarizer.
    #[serde(skip_serializing_if = "is_true")]
    pub native_compaction: bool,
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
    use super::*;
    use crate::AgenaToolMode;

    #[test]
    fn inherited_routes_assume_tools_and_preserve_inheritance_on_save() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"agena_tools": {}}),
            serde_json::json!({"features": ["tool_calling"]}),
            serde_json::json!({"features": ["streaming"]}),
        ] {
            let default: ResolvedProviderModelConfig = serde_json::from_value(value).unwrap();
            assert_eq!(default.agena_tools.mode, AgenaToolMode::ProviderProtocol);
            assert_eq!(
                ResolvedProviderModelConfig::default().agena_tools,
                default.agena_tools
            );
            let saved = serde_json::to_value(default).unwrap();
            assert!(saved.get("agena_tools").is_none());
            let reloaded: ResolvedProviderModelConfig = serde_json::from_value(saved).unwrap();
            assert_eq!(reloaded.agena_tools.mode, AgenaToolMode::ProviderProtocol);
            assert!(reloaded.agena_tools.is_inherited());
        }
    }

    #[test]
    fn explicit_negative_capability_disables_an_inherited_route() {
        let value = serde_json::json!({"features": {"unsupported": ["tool_calling"]}});
        let config: ResolvedProviderModelConfig = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(config.agena_tools.mode, AgenaToolMode::Disabled);
        assert!(config.agena_tools.is_inherited());
        let saved = serde_json::to_value(config).unwrap();
        assert_eq!(saved, value);
        let reloaded: ResolvedProviderModelConfig = serde_json::from_value(saved).unwrap();
        assert_eq!(reloaded.agena_tools.mode, AgenaToolMode::Disabled);
    }

    #[test]
    fn explicit_user_mode_wins_over_capability_metadata_and_round_trips() {
        let disabled: ResolvedProviderModelConfig = serde_json::from_value(serde_json::json!({
            "agena_tools": {"mode": "disabled"},
            "features": ["tool_calling"],
        }))
        .unwrap();
        assert_eq!(disabled.agena_tools.mode, AgenaToolMode::Disabled);
        assert_eq!(
            serde_json::to_value(disabled).unwrap()["agena_tools"]["mode"],
            "disabled"
        );
        let enabled: ResolvedProviderModelConfig = serde_json::from_value(serde_json::json!({
            "agena_tools": {"mode": "provider_protocol"},
            "features": {"unsupported": ["tool_calling"]},
        }))
        .unwrap();
        assert_eq!(enabled.agena_tools.mode, AgenaToolMode::ProviderProtocol);
        assert_eq!(
            enabled.agena_tools.configured_mode(),
            Some(AgenaToolMode::ProviderProtocol)
        );
        let reloaded: ResolvedProviderModelConfig =
            serde_json::from_value(serde_json::to_value(enabled.clone()).unwrap()).unwrap();
        assert_eq!(reloaded, enabled);
    }

    #[test]
    fn invalid_explicit_tool_modes_are_errors_instead_of_inherited_defaults() {
        for tools in [
            serde_json::json!({"mode": null}),
            serde_json::json!({"mode": "unknown"}),
            serde_json::json!({"mod": "disabled"}),
        ] {
            assert!(
                serde_json::from_value::<ResolvedProviderModelConfig>(
                    serde_json::json!({"agena_tools": tools})
                )
                .is_err()
            );
        }
    }
}
