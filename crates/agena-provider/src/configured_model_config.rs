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
    #[serde(default)]
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
        let agena_tools = match fields.remove("agena_tools") {
            Some(value) => serde_json::from_value(value).map_err(D::Error::custom)?,
            None => AgenaToolsConfig::default(),
        };
        let definition =
            serde_json::from_value(serde_json::Value::Object(fields)).map_err(D::Error::custom)?;
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
    fn a_bare_model_route_exposes_tools_and_an_explicit_disabled_route_stays_disabled() {
        let default: ResolvedProviderModelConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(default.agena_tools.mode, AgenaToolMode::ProviderProtocol);
        assert_eq!(
            ResolvedProviderModelConfig::default().agena_tools,
            default.agena_tools
        );
        let disabled: ResolvedProviderModelConfig =
            serde_json::from_str(r#"{"agena_tools":{"mode":"disabled"}}"#).unwrap();
        assert_eq!(disabled.agena_tools.mode, AgenaToolMode::Disabled);
        assert_eq!(
            serde_json::to_value(disabled).unwrap()["agena_tools"]["mode"],
            "disabled"
        );
    }
}
