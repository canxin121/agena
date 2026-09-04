//! Complete persisted configuration for one provider model route.
//!
//! Provider routes only decide how the five fixed Agena Tool API gateway
//! functions are transported. Ordinary execution tools never become provider
//! declarations, and provider-service capabilities live in ordinary plugins
//! such as `agena.openai`.

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
    /// Transport mode for the fixed five-function Agena Tool API. Removed
    /// `direct` and `provider_native` members are rejected when loading config.
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
        for removed_key in ["provider_tools", "provider_native_tools", "native_tools"] {
            if fields.contains_key(removed_key) {
                return Err(D::Error::custom(format!(
                    "unknown field `{removed_key}`; provider service capabilities are ordinary plugins such as `agena.openai`"
                )));
            }
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
            Some(serde_json::Value::Object(value)) => {
                for removed_field in ["direct", "provider_native"] {
                    if value.contains_key(removed_field) {
                        return Err(D::Error::custom(format!(
                            "unknown field `agena_tools.{removed_field}`; only the five Tool API gateway functions use the provider tool protocol"
                        )));
                    }
                }
                serde_json::from_value(serde_json::Value::Object(value))
                    .map_err(D::Error::custom)?
            }
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
