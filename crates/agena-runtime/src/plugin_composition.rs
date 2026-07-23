//! Runtime-owned concrete PluginHost composition.

use std::{path::Path, sync::Arc};

use agena_plugin_host::{
    PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};

/// Compose a PluginHost from resolved configuration and caller-supplied static plugins.
///
/// Static registrations remain at the process composition boundary because they
/// bind concrete tool implementations. Runtime owns all generic host build
/// policy, including previous-plugin transport reuse.
pub async fn compose_plugin_host(
    static_plugins: Vec<StaticPluginRegistration>,
    plugin_config: PluginsConfig,
    workspace_root: &Path,
    previous_host: Option<Arc<PluginHost>>,
    previous_config: Option<&PluginsConfig>,
    agena_version: impl Into<String>,
) -> Result<Arc<PluginHost>, agena_plugin_host::HostError> {
    let previous_plugins = previous_config
        .map(PluginHostBuildConfig::previous_plugins)
        .unwrap_or_default();
    PluginHost::new(PluginHostBuildConfig {
        static_plugins,
        config: plugin_config,
        workspace_root: workspace_root.to_path_buf(),
        agena_version: agena_version.into(),
        callback_base_url: None,
        host_client: None,
        previous: previous_host,
        previous_plugins,
    })
    .await
}

/// Install a caller-supplied concrete callback adapter into a composed host.
pub async fn install_plugin_host_client(
    host_handle: Arc<agena_plugin_host::host::HostHandle>,
    client: Arc<dyn agena_plugin_host::sdk::host_api::HostClient>,
) {
    host_handle.install_client(client).await;
}

/// Compose and install the process-visible host slot. Concrete callers supply
/// only registrations that bind their process-specific tool implementations.
pub async fn compose_and_install_plugin_host(
    static_plugins: Vec<StaticPluginRegistration>,
    plugin_config: PluginsConfig,
    workspace_root: &Path,
    previous_host: Option<Arc<PluginHost>>,
    previous_config: Option<&PluginsConfig>,
    agena_version: impl Into<String>,
) -> Result<Arc<PluginHost>, agena_plugin_host::HostError> {
    let host = compose_plugin_host(
        static_plugins,
        plugin_config,
        workspace_root,
        previous_host,
        previous_config,
        agena_version,
    )
    .await?;
    crate::install_plugin_host(Arc::clone(&host));
    Ok(host)
}
