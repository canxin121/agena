//! Runtime-owned plugin configuration policy and best-effort notification.

use std::collections::BTreeMap;
use std::sync::Arc;

use agena_plugin_host::{ConfigInput, ConfiguredPlugin, PluginHost, PluginsConfig};

/// Merge the runtime's bundled plugin entries with user configuration.
///
/// Runtime owns the precedence rule; its concrete composition supplies only
/// the static entries it registers. User entries intentionally win when an
/// identifier is repeated.
pub fn merge_bundled_plugin_config(
    configured: PluginsConfig,
    bundled: BTreeMap<String, ConfiguredPlugin>,
) -> Result<PluginsConfig, String> {
    let PluginsConfig {
        host,
        policy,
        list: configured_list,
        profiles,
        active_profiles,
        profile_resolution,
    } = configured;
    let mut list = bundled;
    list.extend(configured_list);
    let mut resolved = PluginsConfig {
        host,
        policy,
        list,
        profiles,
        active_profiles,
        profile_resolution,
    };
    resolved.resolve_profiles_in_place()?;
    Ok(resolved)
}

/// Notify non-empty plugin hosts of a resolved configuration value.
pub async fn dispatch_config_if_nonempty(plugins: Arc<PluginHost>, value: serde_json::Value) {
    if !plugins.is_empty() {
        let _ = plugins
            .dispatch_config(ConfigInput { current: value })
            .await;
    }
}

#[cfg(test)]
mod tests;
