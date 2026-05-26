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
    BTreeMap::from([
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
            crate::tool::workflow_plugin_id().to_string(),
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
    ])
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
            crate::tool::shell_plugin_id(),
            crate::tool::new_shell_plugin(),
        )
        .register_static(
            crate::tool::workflow_plugin_id(),
            crate::tool::new_workflow_plugin(),
        )
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
