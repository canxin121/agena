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
) -> PluginsConfig {
    let PluginsConfig {
        host,
        policy,
        list: configured_list,
    } = configured;
    let mut list = bundled;
    list.extend(configured_list);
    PluginsConfig { host, policy, list }
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
mod tests {
    use std::collections::BTreeMap;

    use agena_plugin_host::{ConfiguredPlugin, PluginsConfig};

    use super::merge_bundled_plugin_config;

    #[test]
    fn user_plugin_entries_override_bundled_entries() {
        let mut bundled = BTreeMap::new();
        bundled.insert(
            "example".to_owned(),
            ConfiguredPlugin::static_settings(serde_json::json!({"origin": "bundled"})),
        );
        let mut configured = PluginsConfig::default();
        configured.list.insert(
            "example".to_owned(),
            ConfiguredPlugin::static_settings(serde_json::json!({"origin": "user"})),
        );

        let merged = merge_bundled_plugin_config(configured, bundled);

        assert_eq!(
            merged.list["example"].settings(),
            &serde_json::json!({"origin": "user"})
        );
    }
}
