//! Plugin source resolution.
//!
//! Every plugin reaches the host through `plugins.list.<id>`. Bundled plugins
//! are just one source that contributes ordinary static package entries before
//! user configuration is applied.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::plugin::{ConfiguredPlugin, PluginHostBuilder, PluginsConfig};

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
        list: configured_list,
    } = configured;
    let mut list = bundled_plugin_entries();
    list.extend(configured_list);
    PluginsConfig { host, policy, list }
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
