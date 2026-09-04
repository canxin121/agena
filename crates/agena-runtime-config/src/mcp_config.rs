//! Neutral MCP configuration values parsed alongside runtime configuration.

use std::collections::BTreeMap;
use std::path::PathBuf;

use agena_plugin_host::{PluginPackage, PluginsConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MCP_PLUGIN_ID: &str = "agena.mcp";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// MCP server configuration.
pub struct McpConfig {
    pub runtime: McpRuntimeConfig,
    pub servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Runtime settings for MCP clients.
pub struct McpRuntimeConfig {
    pub token_store: McpTokenStoreConfig,
    pub reconnect: McpReconnectConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Token store settings for MCP OAuth.
pub struct McpTokenStoreConfig {
    pub enabled: bool,
}

impl Default for McpTokenStoreConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Automatic reconnect behaviour for configured but temporarily unavailable
/// MCP servers. Values are milliseconds and are bounded by the runtime before
/// constructing the connection supervisor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpReconnectConfig {
    pub enabled: bool,
    #[schemars(range(min = 1, max = 60000))]
    pub initial_delay_ms: u64,
    #[schemars(range(min = 1, max = 600000))]
    pub max_delay_ms: u64,
    #[schemars(range(min = 10, max = 60000))]
    pub poll_interval_ms: u64,
}

impl Default for McpReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            initial_delay_ms: 1_000,
            max_delay_ms: 30_000,
            poll_interval_ms: 500,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
/// Configuration of one MCP server.
pub enum McpServerConfig {
    /// Launches an MCP server as a child process and communicates over stdio.
    Stdio {
        process: McpStdioProcessConfig,
        #[serde(default)]
        tools: McpToolPolicyConfig,
    },
    /// Connects to a streamable HTTP MCP server.
    Http {
        endpoint: McpHttpEndpointConfig,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<McpHttpAuthConfig>,
        #[serde(default)]
        tools: McpToolPolicyConfig,
    },
}

/// Server-local MCP tool admission policy. It is evaluated in the connection
/// manager (initial discovery, change refresh, search and invocation), rather
/// than merely hiding a name from the model. `exclude` always wins over
/// `include`; patterns support `*` as a wildcard.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpToolPolicyConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Process settings of a stdio MCP server.
pub struct McpStdioProcessConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// HTTP endpoint settings of an MCP server.
pub struct McpHttpEndpointConfig {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
/// HTTP authentication settings of an MCP server.
pub enum McpHttpAuthConfig {
    /// Uses an inline bearer token value.
    Bearer { token: String },
    /// Reads the bearer token from an environment variable.
    BearerFromEnv { env: String },
    /// Resolves the bearer token from the configured secret store.
    BearerFromStore,
    /// OAuth credentials and any dynamically registered client identity live
    /// exclusively in the configured secret store. Configuration carries only
    /// the requested public scopes.
    /// Uses OAuth credentials managed by the configured secret store.
    OAuth {
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// Sends explicit custom HTTP headers.
    Custom { headers: BTreeMap<String, String> },
}

pub fn mcp_config_from_plugins(plugins: &PluginsConfig) -> Result<McpConfig, String> {
    let Some(configured_plugin) = plugins.list.get(MCP_PLUGIN_ID) else {
        return Ok(McpConfig::default());
    };
    if configured_plugin.disabled()
        || !matches!(configured_plugin.package, PluginPackage::Static { .. })
        || configured_plugin.settings().is_null()
    {
        return Ok(McpConfig::default());
    }
    serde_json::from_value(configured_plugin.settings().clone())
        .map_err(|error| format!("plugins.list.\"{MCP_PLUGIN_ID}\".settings: {error}"))
}
