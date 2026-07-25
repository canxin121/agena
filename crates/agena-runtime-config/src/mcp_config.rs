//! Neutral MCP configuration values parsed alongside runtime configuration.

use std::collections::BTreeMap;
use std::path::PathBuf;

use agena_plugin_host::{PluginPackage, PluginsConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MCP_PLUGIN_ID: &str = "agena.mcp";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpConfig {
    pub runtime: McpRuntimeConfig,
    pub servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpRuntimeConfig {
    pub token_store: McpTokenStoreConfig,
    pub reconnect: McpReconnectConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpTokenStoreConfig {
    pub enabled: bool,
    /// The preferred durable credential backend. `keyring` keeps credentials
    /// out of normal configuration and delegates protection to the operating
    /// system; `file` exists only for explicit compatibility use.
    pub backend: McpTokenStoreBackend,
    /// When the keyring is unavailable or has no entry, also consult the
    /// legacy chmod-600 file store. This is deliberately opt-in so headless
    /// deployments do not silently create a plaintext credential file.
    pub file_fallback: bool,
}

impl Default for McpTokenStoreConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: McpTokenStoreBackend::Keyring,
            file_fallback: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpTokenStoreBackend {
    #[default]
    Keyring,
    File,
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
pub enum McpServerConfig {
    Stdio {
        process: McpStdioProcessConfig,
        #[serde(default)]
        tools: McpToolPolicyConfig,
    },
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
pub struct McpHttpEndpointConfig {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpHttpAuthConfig {
    Bearer {
        token: String,
    },
    BearerFromEnv {
        env: String,
    },
    BearerFromStore,
    /// OAuth credentials and any dynamically registered client identity live
    /// exclusively in the configured secret store. Configuration carries only
    /// the requested public scopes.
    OAuth {
        #[serde(default)]
        scopes: Vec<String>,
    },
    Custom {
        headers: BTreeMap<String, String>,
    },
}

pub fn mcp_config_from_plugins(plugins: &PluginsConfig) -> Result<McpConfig, String> {
    let Some(configured_plugin) = plugins.list.get(MCP_PLUGIN_ID) else {
        return Ok(McpConfig::default());
    };
    if configured_plugin.disabled()
        || !matches!(configured_plugin.package, PluginPackage::Static { .. })
        || configured_plugin.config().is_null()
    {
        return Ok(McpConfig::default());
    }
    serde_json::from_value(configured_plugin.config().clone())
        .map_err(|error| format!("plugins.list.\"{MCP_PLUGIN_ID}\".config: {error}"))
}
