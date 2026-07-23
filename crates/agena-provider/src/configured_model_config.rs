//! Complete persisted configuration for one provider model route.
//!
//! This value is schema-neutral: configuration loaders may parse it, while
//! catalog, runtime, and presentation consumers can use the same provider
//! contract without depending on a concrete configuration implementation.

use serde::{Deserialize, Serialize};

use crate::{AgenaToolsConfig, ConfiguredModelDefinition, ProviderNativeToolBinding};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProviderModelConfig {
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Whether this model route may use a provider-native conversation
    /// compaction endpoint before falling back to Agena's text summarizer.
    /// This is execution policy rather than intrinsic model capability.
    #[serde(skip_serializing_if = "is_true")]
    pub native_compaction: bool,
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
        for legacy_key in ["provider_tools", "provider_native_tools", "native_tools"] {
            if fields.contains_key(legacy_key) {
                return Err(D::Error::custom(format!(
                    "unknown field `{legacy_key}`; provider-native tools belong under `agena_tools.provider_native`"
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
        let agena_tools = fields
            .remove("agena_tools")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .unwrap_or_default();
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

impl ResolvedProviderModelConfig {
    pub fn provider_native_tool_bindings(&self) -> Vec<ProviderNativeToolBinding> {
        self.agena_tools.provider_native.bindings()
    }
}

fn is_true(value: &bool) -> bool {
    *value
}

#[cfg(test)]
mod tests {
    use super::ResolvedProviderModelConfig;

    #[test]
    fn defaults_and_rejects_legacy_native_tool_aliases() {
        let config: ResolvedProviderModelConfig =
            serde_json::from_value(serde_json::json!({})).expect("minimal configured model");
        assert!(config.enabled);
        assert!(config.native_compaction);

        let error = serde_json::from_value::<ResolvedProviderModelConfig>(serde_json::json!({
            "provider_native_tools": {}
        }))
        .expect_err("legacy native-tool alias must remain rejected");
        assert!(error.to_string().contains("agena_tools.provider_native"));
    }
}
