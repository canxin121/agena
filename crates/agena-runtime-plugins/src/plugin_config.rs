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

        let merged =
            merge_bundled_plugin_config(configured, bundled).expect("merge bundled plugin config");

        assert_eq!(
            merged.list["example"].settings(),
            &serde_json::json!({"origin": "user"})
        );
    }

    #[test]
    fn active_profiles_apply_after_bundled_and_user_entries() {
        use agena_plugin_host::{PluginProfile, PluginProfileEntry};

        let bundled = BTreeMap::from([(
            "example.plugin".to_owned(),
            ConfiguredPlugin::static_settings(serde_json::json!({
                "origin": "bundled",
                "nested": {"keep": true}
            })),
        )]);
        let mut configured = PluginsConfig::default();
        configured.list.insert(
            "example.plugin".to_owned(),
            ConfiguredPlugin::static_settings(serde_json::json!({
                "origin": "user",
                "nested": {"user": true}
            })),
        );
        configured.profiles.insert(
            "coding".to_owned(),
            PluginProfile {
                plugins: BTreeMap::from([(
                    "example.plugin".to_owned(),
                    PluginProfileEntry::Patch {
                        enabled: None,
                        package: None,
                        settings_patch: Some(serde_json::json!({
                            "origin": "profile",
                            "nested": {"profile": true}
                        })),
                        timeouts: None,
                        activation: None,
                    },
                )]),
                ..PluginProfile::default()
            },
        );
        configured.active_profiles = vec!["coding".to_owned()];

        let merged = merge_bundled_plugin_config(configured, bundled)
            .expect("resolve profiles over bundled config");

        assert_eq!(
            merged.list["example.plugin"].settings(),
            &serde_json::json!({
                "origin": "profile",
                "nested": {"user": true, "profile": true}
            })
        );
        assert_eq!(merged.profile_resolution.applied_profiles, ["coding"]);
        assert!(merged.profiles.is_empty());
        assert!(merged.active_profiles.is_empty());
    }
}
