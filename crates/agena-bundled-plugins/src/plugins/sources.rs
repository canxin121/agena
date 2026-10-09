//! Plugin source resolution.
//!
//! Every plugin reaches the host through `plugins.list.<id>`. Bundled plugins
//! are just one source that contributes ordinary static package entries before
//! user configuration is applied.

use std::collections::BTreeMap;
use std::sync::Arc;

use agena_plugin_host::{ConfiguredPlugin, StaticPluginRegistration, sdk::Plugin};

fn plugin_key(value: &str) -> agena_plugin_host::PluginKey {
    value.parse().expect("static plugin key")
}

fn static_entry(config: serde_json::Value) -> ConfiguredPlugin {
    ConfiguredPlugin::static_settings(config)
}

pub fn bundled_plugin_entries() -> BTreeMap<String, ConfiguredPlugin> {
    BTreeMap::from([
        (
            crate::tool::content_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
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
            crate::tool::commands_plugin_id().to_string(),
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
    ])
}

fn localized_static_plugin<P: Plugin>(
    key: agena_plugin_host::PluginKey,
    plugin: P,
) -> StaticPluginRegistration {
    StaticPluginRegistration::new_with_manifest_transform(
        key,
        plugin,
        crate::plugin_tool_docs::localize_bundled_plugin_manifest,
    )
}

pub fn static_plugin_registrations(
    mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
) -> Vec<StaticPluginRegistration> {
    let mut registrations = vec![
        localized_static_plugin(
            plugin_key(crate::tool::content_plugin_id()),
            crate::tool::new_content_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::chatgpt_plugin_id()),
            crate::tool::new_chatgpt_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::gemini_plugin_id()),
            crate::tool::new_gemini_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::claude_plugin_id()),
            crate::tool::new_claude_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::commands_plugin_id()),
            crate::tool::new_commands_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::lsp_plugin_id()),
            crate::tool::new_lsp_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::cron_plugin_id()),
            crate::tool::new_cron_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::code_plugin_id()),
            crate::tool::new_code_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::fs_plugin_id()),
            crate::tool::new_fs_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::settings_plugin_id()),
            crate::tool::new_settings_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::shell_plugin_id()),
            crate::tool::new_shell_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::monitor_plugin_id()),
            crate::tool::new_monitor_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::tool_api_plugin_id()),
            crate::tool::new_tool_api_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::session_plugin_id()),
            crate::tool::new_session_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::interaction_plugin_id()),
            crate::tool::new_interaction_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::terminal_plugin_id()),
            crate::tool::new_terminal_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::plan_plugin_id()),
            crate::tool::new_plan_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::tasks_plugin_id()),
            crate::tool::new_tasks_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::report_plugin_id()),
            crate::tool::new_report_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::notebook_plugin_id()),
            crate::tool::new_notebook_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::web_plugin_id()),
            crate::tool::new_web_plugin(),
        ),
        localized_static_plugin(
            plugin_key(crate::tool::memory_plugin_id()),
            crate::tool::new_memory_plugin(),
        ),
    ];
    if let Some(manager) = mcp_manager {
        registrations.push(localized_static_plugin(
            plugin_key(crate::tool::mcp_plugin_id()),
            crate::tool::new_mcp_plugin(manager),
        ));
    }
    registrations
}

#[cfg(test)]
mod tests {
    use super::Arc;
    use agena_plugin_host::sdk::{Plugin, PluginManifest};

    fn manifest(plugin: impl Plugin) -> PluginManifest {
        plugin.manifest()
    }

    #[test]
    fn every_bundled_plugin_has_summaries_for_supported_ui_locales() {
        let plugins = vec![
            manifest(crate::tool::new_content_plugin()),
            manifest(crate::tool::new_chatgpt_plugin()),
            manifest(crate::tool::new_gemini_plugin()),
            manifest(crate::tool::new_claude_plugin()),
            manifest(crate::tool::new_commands_plugin()),
            manifest(crate::tool::new_lsp_plugin()),
            manifest(crate::tool::new_cron_plugin()),
            manifest(crate::tool::new_code_plugin()),
            manifest(crate::tool::new_fs_plugin()),
            manifest(crate::tool::new_settings_plugin()),
            manifest(crate::tool::new_shell_plugin()),
            manifest(crate::tool::new_monitor_plugin()),
            manifest(crate::tool::new_tool_api_plugin()),
            manifest(crate::tool::new_session_plugin()),
            manifest(crate::tool::new_interaction_plugin()),
            manifest(crate::tool::new_terminal_plugin()),
            manifest(crate::tool::new_plan_plugin()),
            manifest(crate::tool::new_tasks_plugin()),
            manifest(crate::tool::new_report_plugin()),
            manifest(crate::tool::new_notebook_plugin()),
            manifest(crate::tool::new_web_plugin()),
            manifest(crate::tool::new_memory_plugin()),
            manifest(crate::tool::new_mcp_plugin(Arc::new(
                agena_mcp_client::McpConnectionManager::new("translation-test", "1"),
            ))),
        ];
        let locales = [
            "zh-CN", "zh-TW", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "hi-IN", "ar-SA",
            "pt-BR",
        ];

        for plugin in plugins {
            let summary = plugin.summary.as_deref().unwrap_or_else(|| {
                panic!("{}.{} has no plugin summary", plugin.namespace, plugin.name)
            });
            for locale in locales {
                let translated = plugin
                    .translations
                    .get(locale)
                    .and_then(|translation| translation.summary.as_deref());
                assert!(
                    translated.is_some_and(|value| !value.trim().is_empty() && value != summary),
                    "{}.{} needs a native {locale} summary",
                    plugin.namespace,
                    plugin.name,
                );
            }

            if ["content", "fs", "session", "shell", "tasks"].contains(&plugin.name.as_str()) {
                for tool in &plugin.tools {
                    let summary = tool.docs.summary.as_deref().unwrap_or_else(|| {
                        panic!("{}.{} has no tool summary", plugin.name, tool.name)
                    });
                    for locale in locales {
                        let translated = tool
                            .docs
                            .translations
                            .get(locale)
                            .and_then(|translation| translation.summary.as_deref());
                        assert!(
                            translated
                                .is_some_and(|value| !value.trim().is_empty() && value != summary),
                            "{}.{} needs a native {locale} summary",
                            plugin.name,
                            tool.name,
                        );
                    }
                }
            }
        }
    }

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
