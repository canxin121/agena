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
    inputs: crate::PluginCompositionInputs<
        PluginsConfig,
        &Path,
        Option<Arc<PluginHost>>,
        Option<PluginsConfig>,
        Option<Arc<agena_mcp_client::McpConnectionManager>>,
    >,
    agena_version: impl Into<String>,
) -> Result<Arc<PluginHost>, agena_plugin_host::HostError> {
    let previous_plugins = inputs
        .previous_config
        .as_ref()
        .map(PluginHostBuildConfig::previous_plugins)
        .unwrap_or_default();
    PluginHost::new_with_callback_dispatcher(
        PluginHostBuildConfig {
            static_plugins,
            config: inputs.plugin_config,
            workspace_root: inputs.workspace_root.to_path_buf(),
            agena_version: agena_version.into(),
            callback_base_url: Some(inputs.callback_base_url),
            host_client: Some(inputs.host_client),
            previous: inputs.previous_host,
            previous_plugins,
        },
        inputs.callback_dispatcher,
    )
    .await
}
