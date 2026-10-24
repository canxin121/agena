//! Runtime-owned plugin configuration policy and best-effort notification.

use std::collections::BTreeMap;
use std::sync::Arc;

use agena_plugin_host::{ConfigInput, ConfiguredPlugin, PluginHost, PluginPackage, PluginsConfig};

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
    // A retired builtin may still be patched/disabled by an existing profile.
    // Resolve those layers against an inert entry, then drop it before host
    // activation. Explicit external replacements keep the usual precedence.
    const RETIRED_SNAPSHOT_PLUGIN: &str = "agena.snapshot";
    list.entry(RETIRED_SNAPSHOT_PLUGIN.to_owned())
        .or_insert_with(ConfiguredPlugin::static_default);
    let mut resolved = PluginsConfig {
        host,
        policy,
        list,
        profiles,
        active_profiles,
        profile_resolution,
    };
    resolved.resolve_profiles_in_place()?;
    if resolved
        .list
        .get(RETIRED_SNAPSHOT_PLUGIN)
        .is_some_and(|plugin| matches!(plugin.package, PluginPackage::Static {}))
    {
        resolved.list.remove(RETIRED_SNAPSHOT_PLUGIN);
    }
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
