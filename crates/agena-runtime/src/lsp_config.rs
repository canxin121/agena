//! Runtime-owned LSP plugin configuration values.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use agena_plugin_host::PluginsConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const LSP_PLUGIN_ID: &str = "agena.lsp";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LspConfig {
    pub defaults: LspServerDefaultsConfig,
    pub servers: BTreeMap<String, LspServerConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LspServerDefaultsConfig {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub initialization_options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LspServerConfig {
    pub process: LspServerProcessConfig,
    pub routing: LspServerRoutingConfig,
    pub session: LspServerSessionConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LspServerProcessConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LspServerRoutingConfig {
    #[serde(default)]
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LspServerSessionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
        .map_err(|error| format!("plugins.list.\"{LSP_PLUGIN_ID}\".config: {error}"))
}

/// Compose the optional LSP registry and its background registration guard.
///
/// Runtime owns plugin configuration parsing, enablement, LSP registry setup,
/// and registration-task lifecycle. The process composition layer supplies only
/// its identity strings and workspace path.
pub fn compose_lsp_services(
    plugins: &PluginsConfig,
    workspace_root: &Path,
    originator: impl Into<String>,
    package_version: impl Into<String>,
) -> Result<
    (
        Option<Arc<agena_lsp::LspRegistry>>,
        Option<Arc<crate::AbortOnDrop>>,
    ),
    String,
> {
    let config = lsp_config_from_plugins(plugins)?;
    let enabled = plugins
        .list
        .get(LSP_PLUGIN_ID)
        .is_some_and(|entry| !entry.disabled());
    if !enabled {
        return Ok((None, None));
    }

    let registry = Arc::new(agena_lsp::LspRegistry::new(
        workspace_root.to_path_buf(),
        originator.into(),
        package_version.into(),
    ));
    let defaults = config.defaults;
    let entries = config.servers.into_iter().collect::<Vec<_>>();
    let registration_registry = Arc::clone(&registry);
    let registration = crate::spawn_registration_batch(entries, move |(name, entry)| {
        let registry = Arc::clone(&registration_registry);
        let defaults = defaults.clone();
        async move {
            registry
                .register(lsp_runtime_spec(name.clone(), &entry, &defaults))
                .await;
            tracing::info!(target: "agena::lsp", "registered LSP server '{name}' (lazy-spawn)");
        }
    });
    Ok((Some(registry), Some(Arc::new(registration))))
}

fn lsp_runtime_spec(
    name: String,
    config: &LspServerConfig,
    defaults: &LspServerDefaultsConfig,
) -> agena_lsp::LspServerSpec {
    let mut env = defaults.env.clone();
    env.extend(config.process.env.clone());
    let root_markers = if config.routing.root_markers.is_empty() {
        defaults.root_markers.clone()
    } else {
        config.routing.root_markers.clone()
    };
    agena_lsp::LspServerSpec {
        name,
        command: config.process.command.clone(),
        args: config.process.args.clone(),
        env,
        file_extensions: config.routing.file_extensions.clone(),
        root_markers,
        initialization_options: config
            .session
            .initialization_options
            .clone()
            .or_else(|| defaults.initialization_options.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::LspConfig;

    #[test]
    fn default_lsp_configuration_is_empty() {
        assert!(LspConfig::default().servers.is_empty());
    }
}
