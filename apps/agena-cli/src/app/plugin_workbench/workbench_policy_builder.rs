pub(in crate::app) fn build_plugin_workbench_plugin(
    sources: &crate::backend::ConfigJsonSources,
    locale: &str,
    status: agena::plugin::status::PluginStatus,
    inspect: Option<agena::plugin::PluginInspect>,
    logs: Vec<agena::plugin::PluginLogRecord>,
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
    let plugin_ui_default = plugin_text_display_mode_from_declared(
        manifest.and_then(|manifest| manifest.ui_display_mode),
    );
    let effective_plugins = sources
        .effective
        .get("plugins")
        .cloned()
        .and_then(|value| serde_json::from_value::<agena::plugin::PluginsConfig>(value).ok())
        .unwrap_or_default();
    let ui_policy = &effective_plugins.policy.ui_presentation;
    let plugin_ui_override = ui_policy.plugins.get(&status.plugin_id).copied();
    let visible_tool = tools
        .first()
        .map(|tool| tool.name.clone())
        .unwrap_or_else(|| status.plugin_id.name().to_owned());
    let plugin_id = status.plugin_id.to_string();
    let ui_display_mode = resolve_plugin_ui_display_mode(
        plugin_ui_override,
        plugin_ui_default,
        ui_policy.default_mode,
    );
    let ui_display_source = match plugin_ui_override {
        Some(
            agena::plugin::UiPresentationOverride::Detailed
            | agena::plugin::UiPresentationOverride::Summary,
        ) => PluginTextDisplaySource::PluginPolicy,
        _ if plugin_ui_default.is_some() => PluginTextDisplaySource::PluginManifest,
        _ => PluginTextDisplaySource::GlobalPolicy,
    };
    let tool_ui_display_modes = tools
        .iter()
        .map(|tool| {
            let tool_key = format!("{}.{}", status.plugin_id, tool.name)
                .parse()
                .expect("manifest tool key should be valid");
            (
                tool.name.clone(),
                plugin_text_display_mode_from_declared(Some(
                    ui_policy.mode_for(
                        &status.plugin_id,
                        &tool_key,
                        tool.display
                            .ui_display_mode
                            .or(manifest.and_then(|manifest| manifest.ui_display_mode)),
                    ),
                ))
                .unwrap_or(PluginTextDisplayMode::Detailed),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let tool_ui_display_defaults = tools
        .iter()
        .filter_map(|tool| {
            plugin_text_display_mode_from_declared(tool.display.ui_display_mode)
                .or(plugin_ui_default)
                .map(|mode| (tool.name.clone(), mode))
        })
        .collect::<BTreeMap<_, _>>();
    let tool_ui_display_sources = tools
        .iter()
        .map(|tool| {
            let tool_default = plugin_text_display_mode_from_declared(tool.display.ui_display_mode);
            let tool_key = format!("{}.{}", status.plugin_id, tool.name)
                .parse()
                .expect("manifest tool key should be valid");
            let source = if ui_policy.tools.contains_key(&tool_key) {
                PluginTextDisplaySource::ToolPolicy
            } else if plugin_ui_override
                .is_some_and(|mode| !matches!(mode, agena::plugin::UiPresentationOverride::Default))
            {
                PluginTextDisplaySource::PluginPolicy
            } else if tool_default.is_some() {
                PluginTextDisplaySource::ToolManifest
            } else if plugin_ui_default.is_some() {
                PluginTextDisplaySource::PluginManifest
            } else {
                PluginTextDisplaySource::GlobalPolicy
            };
            (tool.name.clone(), source)
        })
        .collect::<BTreeMap<_, _>>();
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
        .and_then(|configured_plugin| configured_plugin.get("config"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let schema = manifest.and_then(|manifest| localized_config_schema(manifest, locale));
    let schema_missing = schema.is_none();
    let default_config = materialized_config_value(schema.as_ref(), &JsonValue::Null);
    let saved_config = materialized_config_value(schema.as_ref(), &raw_config);
    let saved_override = derive_override_value(&default_config, &saved_config);
    let mut plugin = PluginWorkbenchPlugin {
        plugin_id,
        visible_tool,
        version,
        transport: status.kind.to_owned(),
        ui_display_mode,
        ui_display_source,
        tool_ui_display_modes,
        tool_ui_display_defaults,
        tool_ui_display_sources,
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

fn resolve_plugin_ui_display_mode(
    override_mode: Option<agena::plugin::UiPresentationOverride>,
    declared: Option<PluginTextDisplayMode>,
    global: agena::plugin::UiTextDisplayMode,
) -> PluginTextDisplayMode {
    match override_mode {
        Some(agena::plugin::UiPresentationOverride::Detailed) => PluginTextDisplayMode::Detailed,
        Some(agena::plugin::UiPresentationOverride::Summary) => PluginTextDisplayMode::Summary,
        Some(agena::plugin::UiPresentationOverride::Default) | None => declared
            .or_else(|| plugin_text_display_mode_from_declared(Some(global)))
            .unwrap_or(PluginTextDisplayMode::Detailed),
    }
}
use super::{
    BTreeMap, JsonValue, PluginConfigStatus, PluginConfigStatusKind, PluginTextDisplayMode,
    PluginTextDisplaySource, PluginWorkbenchPlugin, derive_override_value, localized_config_schema,
    materialized_config_value, plugin_get_json_path, plugin_text_display_mode_from_declared,
    quote_settings_segment, recompute_plugin_config_state,
};
