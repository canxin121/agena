//! Runtime-owned LSP plugin configuration values.

use std::collections::BTreeMap;

use agena_plugin_host::PluginsConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const LSP_PLUGIN_ID: &str = "agena.lsp";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// LSP server configuration.
pub struct LspConfig {
    pub defaults: LspServerDefaultsConfig,
    pub servers: BTreeMap<String, LspServerConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Default settings applied to all LSP servers.
pub struct LspServerDefaultsConfig {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    #[schemars(schema_with = "agena_plugin_host::sdk::bounded_json_schema")]
    pub initialization_options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Configuration of one LSP server.
pub struct LspServerConfig {
    pub process: LspServerProcessConfig,
    pub routing: LspServerRoutingConfig,
    pub session: LspServerSessionConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Process settings of an LSP server.
pub struct LspServerProcessConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Routing settings of an LSP server.
pub struct LspServerRoutingConfig {
    #[serde(default)]
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Per-session settings of an LSP server.
pub struct LspServerSessionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "agena_plugin_host::sdk::bounded_json_schema")]
    pub initialization_options: Option<serde_json::Value>,
}

pub fn lsp_config_from_plugins(plugins: &PluginsConfig) -> Result<LspConfig, String> {
    let Some(configured_plugin) = plugins.list.get(LSP_PLUGIN_ID) else {
        return Ok(LspConfig::default());
    };
    if configured_plugin.disabled() || configured_plugin.config().is_null() {
        return Ok(LspConfig::default());
    }
    serde_json::from_value(configured_plugin.config().clone())
        .map_err(|error| format!("plugins.list.\"{LSP_PLUGIN_ID}\".settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::LspConfig;

    #[test]
    fn default_lsp_configuration_is_empty() {
        assert!(LspConfig::default().servers.is_empty());
    }
}
