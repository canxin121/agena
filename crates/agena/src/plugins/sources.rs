//! Plugin source resolution.
//!
//! Every plugin reaches the host through `plugins.list.<id>`. Bundled plugins
//! are just one source that contributes ordinary static package entries before
//! user configuration is applied.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::plugin::{ConfiguredPlugin, PluginHostBuilder, PluginPackage, PluginsConfig};

const LEGACY_WORKFLOW_PLUGIN_ID: &str = "agena.workflow";

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
            crate::tool::process_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::catalog_plugin_id().to_string(),
            static_entry(serde_json::Value::Null),
        ),
        (
            crate::tool::runtime_plugin_id().to_string(),
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
        list: mut configured_list,
    } = configured;
    migrate_legacy_workflow_plugin(&mut configured_list);
    normalize_legacy_plan_plugin_config(&mut configured_list);
    let mut list = bundled_plugin_entries();
    list.extend(configured_list);
    PluginsConfig { host, policy, list }
}

fn migrate_legacy_workflow_plugin(configured_list: &mut BTreeMap<String, ConfiguredPlugin>) {
    let Some(legacy) = configured_list.remove(LEGACY_WORKFLOW_PLUGIN_ID) else {
        return;
    };

    if !legacy.enabled || !matches!(legacy.package, PluginPackage::Static {}) {
        configured_list.insert(LEGACY_WORKFLOW_PLUGIN_ID.to_owned(), legacy);
        return;
    }

    let legacy_config = legacy.config.as_object();
    let Some(legacy_config) = legacy_config else {
        return;
    };

    migrate_legacy_workflow_child_config(
        configured_list,
        crate::tool::catalog_plugin_id(),
        legacy_config.get("tool_catalog"),
        &legacy,
    );
    migrate_legacy_workflow_child_config(
        configured_list,
        crate::tool::plan_plugin_id(),
        legacy_config.get("plan"),
        &legacy,
    );
}

fn normalize_legacy_plan_plugin_config(configured_list: &mut BTreeMap<String, ConfiguredPlugin>) {
    let Some(plugin) = configured_list.get_mut(crate::tool::plan_plugin_id()) else {
        return;
    };
    let Some(config) = plugin.config.as_object_mut() else {
        return;
    };

    let legacy_value = config.remove("default_auto_continue");
    if !config.contains_key("default_autorun")
        && let Some(legacy_value) = legacy_value
    {
        config.insert("default_autorun".to_owned(), legacy_value);
    }
}

fn migrate_legacy_workflow_child_config(
    configured_list: &mut BTreeMap<String, ConfiguredPlugin>,
    target_plugin_id: &str,
    child_config: Option<&serde_json::Value>,
    legacy: &ConfiguredPlugin,
) {
    if configured_list.contains_key(target_plugin_id) {
        return;
    }
    let Some(child_config) = child_config else {
        return;
    };
    configured_list.insert(
        target_plugin_id.to_owned(),
        ConfiguredPlugin {
            enabled: legacy.enabled,
            package: PluginPackage::Static {},
            config: child_config.clone(),
            timeouts: legacy.timeouts.clone(),
        },
    );
}

pub(crate) fn register_static_transports(
    mut builder: PluginHostBuilder,
    mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
) -> PluginHostBuilder {
    builder = builder
        .register_static(
            crate::tool::skills_plugin_id(),
            crate::tool::new_skills_plugin(),
        )
        .register_static(crate::tool::lsp_plugin_id(), crate::tool::new_lsp_plugin())
        .register_static(
            crate::tool::cron_plugin_id(),
            crate::tool::new_cron_plugin(),
        )
        .register_static(
            crate::tool::code_plugin_id(),
            crate::tool::new_code_plugin(),
        )
        .register_static(crate::tool::fs_plugin_id(), crate::tool::new_fs_plugin())
        .register_static(
            crate::tool::settings_plugin_id(),
            crate::tool::new_settings_plugin(),
        )
        .register_static(
            crate::tool::process_plugin_id(),
            crate::tool::new_process_plugin(),
        )
        .register_static(
            crate::tool::catalog_plugin_id(),
            crate::tool::new_catalog_plugin(),
        )
        .register_static(
            crate::tool::runtime_plugin_id(),
            crate::tool::new_runtime_plugin(),
        )
        .register_static(
            crate::tool::plan_plugin_id(),
            crate::tool::new_plan_plugin(),
        )
        .register_static(
            crate::tool::tasks_plugin_id(),
            crate::tool::new_tasks_plugin(),
        )
        .register_static(
            crate::tool::snapshot_plugin_id(),
            crate::tool::new_snapshot_plugin(),
        );
    if crate::tool::schema_lab_builtin_enabled() {
        builder = builder.register_static(
            crate::tool::schema_lab_plugin_id(),
            crate::tool::new_schema_lab_plugin(),
        );
    }
    builder = builder
        .register_static(crate::web::web_plugin_id(), crate::web::new_web_plugin())
        .register_static(
            crate::memory::memory_plugin_id(),
            crate::memory::new_memory_plugin(),
        );
    if let Some(manager) = mcp_manager {
        builder = builder.register_static(
            crate::tool::mcp_plugin_id(),
            crate::tool::new_mcp_plugin(manager),
        );
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_workflow_config_is_migrated_to_split_builtin_plugins() {
        let mut configured = PluginsConfig::default();
        configured.list.insert(
            LEGACY_WORKFLOW_PLUGIN_ID.to_owned(),
            ConfiguredPlugin::static_config(serde_json::json!({
                "tool_catalog": {
                    "search": {
                        "default_limit": 3
                    }
                },
                "plan": {
                    "default_auto_continue": false
                }
            })),
        );

        let resolved = resolve_plugin_config(configured);

        assert!(!resolved.list.contains_key(LEGACY_WORKFLOW_PLUGIN_ID));
        assert_eq!(
            resolved
                .list
                .get(crate::tool::catalog_plugin_id())
                .and_then(|plugin| plugin.config.pointer("/search/default_limit"))
                .and_then(serde_json::Value::as_u64),
            Some(3)
        );
        assert_eq!(
            resolved
                .list
                .get(crate::tool::plan_plugin_id())
                .and_then(|plugin| plugin.config.get("default_autorun"))
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn explicit_split_plugin_config_wins_over_legacy_workflow_config() {
        let mut configured = PluginsConfig::default();
        configured.list.insert(
            LEGACY_WORKFLOW_PLUGIN_ID.to_owned(),
            ConfiguredPlugin::static_config(serde_json::json!({
                "plan": {
                    "default_auto_continue": false
                }
            })),
        );
        configured.list.insert(
            crate::tool::plan_plugin_id().to_owned(),
            ConfiguredPlugin::static_config(serde_json::json!({
                "default_autorun": true
            })),
        );

        let resolved = resolve_plugin_config(configured);

        assert_eq!(
            resolved
                .list
                .get(crate::tool::plan_plugin_id())
                .and_then(|plugin| plugin.config.get("default_autorun"))
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(
            resolved
                .list
                .get(crate::tool::plan_plugin_id())
                .and_then(|plugin| plugin.config.get("default_auto_continue"))
                .is_none()
        );
    }

    #[test]
    fn direct_plan_config_accepts_legacy_auto_continue_name() {
        let mut configured = PluginsConfig::default();
        configured.list.insert(
            crate::tool::plan_plugin_id().to_owned(),
            ConfiguredPlugin::static_config(serde_json::json!({
                "default_auto_continue": false
            })),
        );

        let resolved = resolve_plugin_config(configured);

        assert_eq!(
            resolved
                .list
                .get(crate::tool::plan_plugin_id())
                .and_then(|plugin| plugin.config.get("default_autorun"))
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert!(
            resolved
                .list
                .get(crate::tool::plan_plugin_id())
                .and_then(|plugin| plugin.config.get("default_auto_continue"))
                .is_none()
        );
    }
}
