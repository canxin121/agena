//! Runtime-owned MCP bridge configuration and connection composition.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use agena_mcp_client::{FileTokenStore, McpConnectionManager, ServerSpec, TokenStore};
use agena_plugin_host::{PluginPackage, PluginsConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Stable id used to find the configured static MCP bridge plugin.
pub const MCP_PLUGIN_ID: &str = "agena.mcp";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpConfig {
    pub runtime: McpRuntimeConfig,
    /// Map of `<server_name> -> <transport spec>`.
    pub servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpRuntimeConfig {
    pub token_store: McpTokenStoreConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpTokenStoreConfig {
    pub enabled: bool,
}

impl Default for McpTokenStoreConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpServerConfig {
    Stdio {
        process: McpStdioProcessConfig,
    },
    Http {
        endpoint: McpHttpEndpointConfig,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<McpHttpAuthConfig>,
    },
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
    Bearer { token: String },
    BearerFromEnv { env: String },
    BearerFromStore,
    Custom { headers: BTreeMap<String, String> },
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

pub fn mcp_static_bridge_enabled(plugins: &PluginsConfig) -> bool {
    plugins
        .list
        .get(MCP_PLUGIN_ID)
        .is_some_and(|configured_plugin| {
            !configured_plugin.disabled()
                && matches!(configured_plugin.package, PluginPackage::Static { .. })
        })
}

pub async fn build_mcp_manager(
    config: &McpConfig,
    client_name: impl Into<String>,
    client_version: impl Into<String>,
) -> Arc<McpConnectionManager> {
    let mut manager = McpConnectionManager::new(client_name.into(), client_version.into());
    if config.runtime.token_store.enabled {
        match FileTokenStore::open_default() {
            Ok(store) => manager.set_token_store(Arc::new(store) as Arc<dyn TokenStore>),
            Err(error) => tracing::warn!(
                target: "agena::mcp",
                "failed to open default token store: {error}"
            ),
        }
    }

    let manager = Arc::new(manager);
    for (name, server_config) in &config.servers {
        let manager = Arc::clone(&manager);
        let name = name.clone();
        let spec = match server_config {
            McpServerConfig::Stdio { process } => ServerSpec::Stdio {
                command: process.command.clone(),
                args: process.args.clone(),
                env: process
                    .env
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                cwd: process.cwd.clone(),
            },
            McpServerConfig::Http { endpoint, auth } => {
                let Some(url) = parse_server_url(name.as_str(), endpoint.url.as_str()) else {
                    continue;
                };
                ServerSpec::Http {
                    url,
                    headers: endpoint
                        .headers
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                    auth: mcp_http_auth(auth.as_ref()),
                }
            }
        };
        if let Err(error) = manager.add_server(&name, spec).await {
            tracing::warn!(target: "agena::mcp", "failed to connect MCP server '{name}': {error}");
        } else {
            tracing::info!(target: "agena::mcp", "connected MCP server '{name}'");
        }
    }
    manager
}

/// Resolve the static MCP bridge configuration and compose its manager when enabled.
pub async fn build_configured_mcp_manager(
    plugins: &PluginsConfig,
    client_name: impl Into<String>,
    client_version: impl Into<String>,
) -> Result<Option<Arc<McpConnectionManager>>, String> {
    let config = mcp_config_from_plugins(plugins)?;
    if !mcp_static_bridge_enabled(plugins) {
        return Ok(None);
    }
    Ok(Some(
        build_mcp_manager(&config, client_name, client_version).await,
    ))
}

fn parse_server_url(name: &str, value: &str) -> Option<url::Url> {
    match url::Url::parse(value) {
        Ok(url) => Some(url),
        Err(error) => {
            tracing::warn!(target: "agena::mcp", "skipping mcp server '{name}': invalid url '{value}': {error}");
            None
        }
    }
}

fn mcp_http_auth(config: Option<&McpHttpAuthConfig>) -> Option<agena_mcp_client::HttpAuth> {
    config.map(|config| match config {
        McpHttpAuthConfig::Bearer { token } => agena_mcp_client::HttpAuth::Bearer(token.clone()),
        McpHttpAuthConfig::BearerFromEnv { env } => {
            agena_mcp_client::HttpAuth::BearerFromEnv(env.clone())
        }
        McpHttpAuthConfig::BearerFromStore => agena_mcp_client::HttpAuth::BearerFromStore,
        McpHttpAuthConfig::Custom { headers } => agena_mcp_client::HttpAuth::Custom(
            headers
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::{McpConfig, McpTokenStoreConfig};

    #[test]
    fn token_store_defaults_to_enabled() {
        assert!(McpTokenStoreConfig::default().enabled);
        assert!(McpConfig::default().runtime.token_store.enabled);
    }
}
