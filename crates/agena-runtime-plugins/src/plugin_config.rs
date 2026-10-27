//! Runtime-owned plugin configuration policy and best-effort notification.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use agena_plugin_host::sdk::PluginKey;
use agena_plugin_host::{
    ConfigInput, ConfiguredPlugin, PluginActivationConfig, PluginHost, PluginPackage,
    PluginProfileEntry, PluginsConfig,
};

const LEGACY_NOTIFICATIONS_PLUGIN: &str = "agena.terminal";
const NOTIFICATIONS_PLUGIN: &str = "agena.notifications";

fn rename_notifications_entry<T>(entries: &mut BTreeMap<String, T>) {
    if let Some(legacy) = entries.remove(LEGACY_NOTIFICATIONS_PLUGIN) {
        entries
            .entry(NOTIFICATIONS_PLUGIN.to_owned())
            .or_insert(legacy);
    }
}

fn rename_notifications_dependencies(
    activation: &mut PluginActivationConfig,
    legacy: &PluginKey,
    canonical: &PluginKey,
) {
    for dependencies in [&mut activation.requires, &mut activation.after] {
        if !dependencies.contains(legacy) {
            continue;
        }
        let mut seen = BTreeSet::new();
        for dependency in dependencies.iter_mut() {
            if dependency == legacy {
                *dependency = canonical.clone();
            }
        }
        dependencies.retain(|dependency| seen.insert(dependency.clone()));
    }
}

/// Normalize aliases before injecting defaults or applying profile layers, so
/// an old disabled entry cannot leave a second enabled builtin behind. Within
/// a map the canonical entry wins; parent/child precedence stays profile-owned.
fn rename_notifications_config(configured: &mut PluginsConfig) {
    let legacy: PluginKey = LEGACY_NOTIFICATIONS_PLUGIN
        .parse()
        .expect("static plugin ID");
    let canonical: PluginKey = NOTIFICATIONS_PLUGIN.parse().expect("static plugin ID");
    rename_notifications_entry(&mut configured.list);
    for plugin in configured.list.values_mut() {
        rename_notifications_dependencies(&mut plugin.activation, &legacy, &canonical);
    }
    for profile in configured.profiles.values_mut() {
        rename_notifications_entry(&mut profile.plugins);
        for entry in profile.plugins.values_mut() {
            match entry {
                PluginProfileEntry::Replace { plugin } => {
                    rename_notifications_dependencies(&mut plugin.activation, &legacy, &canonical);
                }
                PluginProfileEntry::Patch {
                    activation: Some(activation),
                    ..
                } => rename_notifications_dependencies(activation, &legacy, &canonical),
                _ => {}
            }
        }
    }
    if let Some(quota) = configured.host.quotas.remove(&legacy) {
        configured.host.quotas.entry(canonical).or_insert(quota);
    }
    for change in &mut configured.profile_resolution.changes {
        if change.plugin_id == LEGACY_NOTIFICATIONS_PLUGIN {
            change.plugin_id = NOTIFICATIONS_PLUGIN.to_owned();
        }
    }
}

/// Merge the runtime's bundled plugin entries with user configuration.
///
/// Runtime owns the precedence rule; its concrete composition supplies only
/// the static entries it registers. User entries intentionally win when an
/// identifier is repeated.
pub fn merge_bundled_plugin_config(
    mut configured: PluginsConfig,
    bundled: BTreeMap<String, ConfiguredPlugin>,
) -> Result<PluginsConfig, String> {
    if bundled.contains_key(NOTIFICATIONS_PLUGIN) {
        rename_notifications_config(&mut configured);
    }
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

#[cfg(test)]
mod tests;
