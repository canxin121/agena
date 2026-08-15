//! Plugin source resolution.
//!
//! Every plugin reaches the host through `plugins.list.<id>`. Bundled plugins
//! are just one source that contributes ordinary static package entries before
//! user configuration is applied.

use std::collections::BTreeMap;
use std::sync::Arc;

use agena_plugin_host::{ConfiguredPlugin, StaticPluginRegistration};

fn plugin_key(value: &str) -> agena_plugin_host::PluginKey {
    value.parse().expect("static plugin key")
}

fn static_entry(config: serde_json::Value) -> ConfiguredPlugin {
    ConfiguredPlugin::static_config(config)
}

pub fn bundled_plugin_entries() -> BTreeMap<String, ConfiguredPlugin> {
    let mut entries = BTreeMap::from([
        (
            crate::tool::chatgpt_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::gemini_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::claude_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
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
            crate::tool::monitor_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::tool_api_plugin_id().to_string(),
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
            crate::tool::terminal_plugin_id().to_string(),
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
            crate::tool::report_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::notebook_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::snapshot_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::web_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::memory_plugin_id().to_string(),
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

pub fn static_plugin_registrations(
    mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
) -> Vec<StaticPluginRegistration> {
    let mut registrations = vec![
        StaticPluginRegistration::new(
            plugin_key(crate::tool::chatgpt_plugin_id()),
            crate::tool::new_chatgpt_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::gemini_plugin_id()),
            crate::tool::new_gemini_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::claude_plugin_id()),
            crate::tool::new_claude_plugin(),
        ),
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
            plugin_key(crate::tool::monitor_plugin_id()),
            crate::tool::new_monitor_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::tool_api_plugin_id()),
            crate::tool::new_tool_api_plugin(),
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
            plugin_key(crate::tool::terminal_plugin_id()),
            crate::tool::new_terminal_plugin(),
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
            plugin_key(crate::tool::report_plugin_id()),
            crate::tool::new_report_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::notebook_plugin_id()),
            crate::tool::new_notebook_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::snapshot_plugin_id()),
            crate::tool::new_snapshot_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::web_plugin_id()),
            crate::tool::new_web_plugin(),
        ),
        StaticPluginRegistration::new(
            plugin_key(crate::tool::memory_plugin_id()),
            crate::tool::new_memory_plugin(),
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
    #[test]
    fn default_bundled_entries_enable_monitor() {
        let entries = super::bundled_plugin_entries();
        assert!(entries.contains_key(crate::tool::monitor_plugin_id()));
    }

    #[test]
    fn default_bundled_entries_serve_runtime_facts_from_session_only() {
        let entries = super::bundled_plugin_entries();
        assert!(entries.contains_key(crate::tool::session_plugin_id()));
        assert!(!entries.contains_key("agena.context"));
    }
}
