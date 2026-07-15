//! Plugin source resolution.
//!
//! Every plugin reaches the host through `plugins.list.<id>`. Bundled plugins
//! are just one source that contributes ordinary static package entries before
//! user configuration is applied.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::plugin::{ConfiguredPlugin, PluginsConfig, StaticPluginRegistration};

fn plugin_key(value: &str) -> crate::plugin::PluginKey {
    value.parse().expect("static plugin key")
}

fn static_entry(config: serde_json::Value) -> ConfiguredPlugin {
    ConfiguredPlugin::static_config(config)
}

pub(crate) fn bundled_plugin_entries() -> BTreeMap<String, ConfiguredPlugin> {
    let mut entries = BTreeMap::from([
        (
            crate::tool::skills_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::lsp_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::cron_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::code_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::fs_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::settings_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::shell_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::catalog_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::agent_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::session_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::interaction_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::plan_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::tasks_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::snapshot_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::web::web_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::memory::memory_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::mcp_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
    ]);
    if crate::tool::schema_lab_builtin_enabled() {
        entries.insert(
            crate::tool::schema_lab_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        );
    }
    entries
}

pub(crate) fn resolve_plugin_config(configured: PluginsConfig) -> PluginsConfig {
    let PluginsConfig {
        host,
        policy,
        list: configured_list,
    } = configured;
    let mut list = bundled_plugin_entries();
    list.extend(configured_list);
    PluginsConfig { host, policy, list }
}

pub(crate) fn static_plugin_registrations(
    mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
) -> Vec<StaticPluginRegistration> {
    let mut registrations = vec![
        StaticPluginRegistration::new(
            plugin_key(crate::tool::skills_plugin_id()),
            crate::tool::new_skills_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::lsp_plugin_id()),
            crate::tool::new_lsp_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::cron_plugin_id()),
            crate::tool::new_cron_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::code_plugin_id()),
            crate::tool::new_code_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::fs_plugin_id()),
            crate::tool::new_fs_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::settings_plugin_id()),
            crate::tool::new_settings_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::shell_plugin_id()),
            crate::tool::new_shell_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::catalog_plugin_id()),
            crate::tool::new_catalog_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::agent_plugin_id()),
            crate::tool::new_agent_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::session_plugin_id()),
            crate::tool::new_session_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::interaction_plugin_id()),
            crate::tool::new_interaction_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::plan_plugin_id()),
            crate::tool::new_plan_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::tasks_plugin_id()),
            crate::tool::new_tasks_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::snapshot_plugin_id()),
            crate::tool::new_snapshot_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::web::web_plugin_id()),
            crate::web::new_web_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::memory::memory_plugin_id()),
            crate::memory::new_memory_plugin(),
        ),
    ];
    if crate::tool::schema_lab_builtin_enabled() {
        registrations.push(StaticPluginRegistration::new(
            plugin_key(crate::tool::schema_lab_plugin_id()),
            crate::tool::new_schema_lab_plugin(),
        ));
    }
    if let Some(manager) = mcp_manager {
        registrations.push(StaticPluginRegistration::new(
            plugin_key(crate::tool::mcp_plugin_id()),
            crate::tool::new_mcp_plugin(manager),
        ));
    }
    registrations
}

#[cfg(test)]
mod tests {
    use super::bundled_plugin_entries;

    #[test]
    fn bundled_entries_register_agent_session_and_interaction_instead_of_runtime() {
        let entries = bundled_plugin_entries();

        assert!(entries.contains_key("agena.agent"));
        assert!(entries.contains_key("agena.session"));
        assert!(entries.contains_key("agena.interaction"));
        assert!(!entries.contains_key("agena.runtime"));
    }
}
