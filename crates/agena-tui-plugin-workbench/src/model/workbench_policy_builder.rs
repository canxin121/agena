pub fn build_plugin_workbench_plugin(
    sources: &agena_application::dto::ConfigJsonSources,
    _locale: &str,
    status: agena_plugin_host::status::PluginStatus,
    inspect: Option<agena_plugin_host::PluginInspect>,
    logs: Vec<agena_plugin_host::PluginLogRecord>,
) -> PluginWorkbenchPlugin {
    let manifest = inspect
        .as_ref()
        .and_then(|inspect| inspect.manifest.as_ref());
    let tools = manifest
        .map(|manifest| manifest.tools.clone())
        .unwrap_or_default();
    let commands = manifest
        .map(|manifest| manifest.commands.clone())
        .unwrap_or_default();
    let version = manifest
        .map(|manifest| manifest.version.clone())
        .unwrap_or_else(|| "n/a".to_owned());
    let visible_tool = tools
        .first()
        .map(|tool| tool.name.clone())
        .unwrap_or_else(|| status.plugin_id.name().to_owned());
    let plugin_id = status.plugin_id.to_string();
    let configured_plugin_value = inspect
        .as_ref()
        .and_then(|inspect| inspect.configured_plugin.as_ref())
        .and_then(|configured_plugin| serde_json::to_value(configured_plugin).ok())
        .or_else(|| {
            plugin_get_json_path(
                &sources.effective,
                Some(
                    format!(
                        "plugins.list.{}",
                        quote_settings_segment(plugin_id.as_str())
                    )
                    .as_str(),
                ),
            )
            .ok()
            .filter(|value| !value.is_null())
        })
        .filter(|value| !value.is_null());
    let raw_config = configured_plugin_value
        .as_ref()
        .and_then(|configured_plugin| configured_plugin.get("settings"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let schema = manifest.and_then(|manifest| plugin_settings_schema(manifest));
    let schema_missing = schema.is_none();
    let default_config = materialized_config_value(schema.as_ref(), &JsonValue::Null);
    let saved_config = materialized_config_value(schema.as_ref(), &raw_config);
    let saved_override = derive_override_value(&default_config, &saved_config);
    let mut plugin = PluginWorkbenchPlugin {
        plugin_id,
        visible_tool,
        version,
        transport: status.kind.clone(),
        tools,
        commands,
        config_status: PluginConfigStatus {
            kind: PluginConfigStatusKind::Valid,
            label: "Valid".to_owned(),
        },
        status,
        inspect,
        configured_plugin_value,
        saved_override: saved_override.clone(),
        draft_override: saved_override,
        default_config,
        saved_config: saved_config.clone(),
        draft_config: saved_config,
        schema,
        schema_missing,
        diagnostics: Vec::new(),
        runtime_diagnostics: Vec::new(),
        diff: Vec::new(),
        sections: Vec::new(),
        logs,
        dirty: false,
        branch_drafts: BTreeMap::new(),
    };
    recompute_plugin_config_state(&mut plugin);
    plugin
}
use super::{
    BTreeMap, JsonValue, PluginConfigStatus, PluginConfigStatusKind, PluginWorkbenchPlugin,
    derive_override_value, materialized_config_value, plugin_get_json_path, plugin_settings_schema,
    quote_settings_segment, recompute_plugin_config_state,
};
