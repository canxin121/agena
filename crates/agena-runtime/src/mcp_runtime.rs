//! Runtime-owned MCP bridge configuration and connection composition.

use std::sync::Arc;

use agena_mcp_client::{KeyringTokenStore, McpConnectionManager, ReconnectPolicy, ServerSpec};
use agena_plugin_host::{PluginPackage, PluginsConfig};
use agena_runtime_config::{McpHttpAuthConfig, McpServerConfig};

// Keep the runtime composition boundary and plugin schema on one configuration
// definition. These were historically duplicated, making it possible for the
// UI schema and the runtime parser to disagree about supported MCP fields.
pub(crate) use agena_runtime_config::{
    MCP_PLUGIN_ID, McpConfig, McpRuntimeConfig, mcp_config_from_plugins,
};

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
    workspace_root: &std::path::Path,
) -> Arc<McpConnectionManager> {
    let mut manager = McpConnectionManager::new(client_name.into(), client_version.into())
        .with_roots([workspace_root.to_path_buf()]);
    if config.runtime.token_store.enabled {
        manager.set_token_store(Arc::new(KeyringTokenStore::new()));
    }

    let manager = Arc::new(manager);
    for (name, server_config) in &config.servers {
        let manager = Arc::clone(&manager);
        let name = name.clone();
        let spec = match server_config {
            McpServerConfig::Stdio { process, tools } => ServerSpec::Stdio {
                command: process.command.clone(),
                args: process.args.clone(),
                env: process
                    .env
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                cwd: process.cwd.clone(),
                tool_policy: agena_mcp_client::McpToolPolicy {
                    include: tools.include.clone(),
                    exclude: tools.exclude.clone(),
                },
            },
            McpServerConfig::Http {
                endpoint,
                auth,
                tools,
            } => {
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
                    tool_policy: agena_mcp_client::McpToolPolicy {
                        include: tools.include.clone(),
                        exclude: tools.exclude.clone(),
                    },
                }
            }
        };
        if let Err(error) = manager.add_server(&name, spec).await {
            tracing::warn!(target: "agena::mcp", "failed to connect MCP server '{name}': {error}");
        } else {
            tracing::info!(target: "agena::mcp", "connected MCP server '{name}'");
        }
    }
    if config.runtime.reconnect.enabled {
        manager.start_reconnect_supervisor(ReconnectPolicy::new(
            std::time::Duration::from_millis(config.runtime.reconnect.initial_delay_ms),
            std::time::Duration::from_millis(config.runtime.reconnect.max_delay_ms),
            std::time::Duration::from_millis(config.runtime.reconnect.poll_interval_ms),
        ));
    }
    manager
}

/// Resolve the static MCP bridge configuration and compose its manager when enabled.
pub async fn build_configured_mcp_manager(
    plugins: &PluginsConfig,
    client_name: impl Into<String>,
    client_version: impl Into<String>,
    workspace_root: &std::path::Path,
) -> Result<Option<Arc<McpConnectionManager>>, String> {
    let config = mcp_config_from_plugins(plugins)?;
    if !mcp_static_bridge_enabled(plugins) {
        return Ok(None);
    }
    Ok(Some(
        build_mcp_manager(&config, client_name, client_version, workspace_root).await,
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
        McpHttpAuthConfig::OAuth { scopes } => agena_mcp_client::HttpAuth::OAuth {
            scopes: scopes.clone(),
        },
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
    use agena_runtime_config::{McpConfig, McpReconnectConfig, McpTokenStoreConfig};

    #[test]
    fn token_store_defaults_to_enabled() {
        assert!(McpTokenStoreConfig::default().enabled);
        assert!(McpConfig::default().runtime.token_store.enabled);
        assert!(McpReconnectConfig::default().enabled);
        assert!(McpConfig::default().runtime.reconnect.enabled);
    }
}
