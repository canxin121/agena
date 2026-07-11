pub(in crate::app) fn build_plugin_policy_sections<F>(
    sources: &crate::backend::ConfigJsonSources,
    _locale: &str,
    statuses: Vec<agena::plugin::status::PluginStatus>,
    mut inspect_for: F,
) -> Vec<PluginPolicySection>
where
    F: FnMut(&agena::plugin::sdk::PluginKey) -> Option<agena::plugin::PluginInspect>,
{
    let file_plugins = plugins_config_from_root(&sources.file);
    let effective_plugins = plugins_config_from_root(&sources.effective);
    let mut status_by_id = statuses
        .into_iter()
        .map(|status| (status.plugin_id.clone(), status))
        .collect::<BTreeMap<_, _>>();
    let mut plugin_ids = configured_plugin_ids(&sources.file);
    plugin_ids.extend(configured_plugin_ids(&sources.effective));
    plugin_ids.extend(status_by_id.keys().cloned());

    let mut sections = plugin_ids
        .into_iter()
        .map(|plugin_id| {
            let status = status_by_id.remove(&plugin_id).unwrap_or_else(|| {
                let kind = configured_plugin_kind(&sources.effective, &plugin_id)
                    .or_else(|| configured_plugin_kind(&sources.file, &plugin_id))
                    .unwrap_or_else(|| "configured".to_owned());
                let static_kind = match kind.as_str() {
                    "static" => "static",
                    "stdio" => "stdio",
                    "cdylib" => "cdylib",
                    "http" => "http",
                    "wasm" => "wasm",
                    _ => "configured",
                };
                agena::plugin::status::PluginStatus::initial(&plugin_id, static_kind)
            });
            let inspect = inspect_for(&plugin_id);
            build_plugin_policy_section(sources, &file_plugins, &effective_plugins, status, inspect)
        })
        .collect::<Vec<_>>();
    sections.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    sections
}

pub(in crate::app) fn build_plugin_policy_section(
    sources: &crate::backend::ConfigJsonSources,
    file_plugins: &agena::plugin::PluginsConfig,
    effective_plugins: &agena::plugin::PluginsConfig,
    status: agena::plugin::status::PluginStatus,
    inspect: Option<agena::plugin::PluginInspect>,
) -> PluginPolicySection {
    let manifest = inspect
        .as_ref()
        .and_then(|inspect| inspect.manifest.as_ref());
    let label = manifest
        .map(|manifest| manifest.name.clone())
        .unwrap_or_else(|| status.plugin_id.to_string());
    let description = manifest
        .and_then(|manifest| manifest.summary.clone().or_else(|| manifest.help.clone()))
        .unwrap_or_else(|| {
            "No plugin metadata available until this plugin can be inspected at runtime.".to_owned()
        });
    let plugin_prompt_default = manifest.and_then(|manifest| manifest.tool_description_mode);
    let plugin_ui_default = manifest
        .and_then(|manifest| plugin_text_display_mode_from_declared(manifest.ui_display_mode));
    let effective_plugin_prompt_override = effective_plugins
        .policy
        .tool_presentation
        .plugins
        .get(&status.plugin_id)
        .copied();
    let effective_plugin_ui_override = effective_plugins
        .policy
        .ui_presentation
        .plugins
        .get(&status.plugin_id)
        .copied();

    let mut items = vec![PluginPolicyItem {
        key: "plugin".to_owned(),
        label: "Plugin".to_owned(),
        scope_label: format!("plugin {}", status.plugin_id),
        description: description.clone(),
        prompt_tool_default: None,
        prompt_plugin_declared_default: plugin_prompt_default,
        prompt_file_override: file_plugins
            .policy
            .tool_presentation
            .plugins
            .get(&status.plugin_id)
            .copied(),
        prompt_effective_mode: resolve_plugin_prompt_mode(
            &effective_plugins.policy.tool_presentation,
            &status.plugin_id,
            plugin_prompt_default,
        ),
        prompt_source: prompt_plugin_source(
            effective_plugin_prompt_override,
            plugin_prompt_default,
        ),
        prompt_path: plugin_policy_prompt_path(&status.plugin_id.to_string(), None),
        ui_tool_default: None,
        ui_plugin_declared_default: plugin_ui_default,
        ui_file_override: file_plugins
            .policy
            .ui_presentation
            .plugins
            .get(&status.plugin_id)
            .copied(),
        ui_effective_mode: resolve_plugin_ui_mode(
            &effective_plugins.policy.ui_presentation,
            &status.plugin_id,
            plugin_ui_default,
        ),
        ui_source: ui_plugin_source(effective_plugin_ui_override, plugin_ui_default),
        ui_path: plugin_policy_ui_path(&status.plugin_id.to_string(), None),
    }];

    if let Some(manifest) = manifest {
        for tool in &manifest.tools {
            let tool_prompt_default = tool.display.description_mode;
            let nearest_prompt_default = tool_prompt_default.or(plugin_prompt_default);
            let tool_ui_default =
                plugin_text_display_mode_from_declared(tool.display.ui_display_mode);
            let nearest_ui_default = tool_ui_default.or(plugin_ui_default);
            let effective_tool_prompt_override = tool_prompt_override_from_policy(
                &effective_plugins.policy.tool_presentation,
                &status.plugin_id,
                tool.name.as_str(),
            );
            let effective_tool_ui_override = tool_ui_override_from_policy(
                &effective_plugins.policy.ui_presentation,
                &status.plugin_id,
                tool.name.as_str(),
            );
            items.push(PluginPolicyItem {
                key: format!("tool:{}", tool.name),
                label: tool.name.clone(),
                scope_label: format!("tool {}/{}", status.plugin_id, tool.name),
                description: tool
                    .docs
                    .summary
                    .clone()
                    .or_else(|| tool.docs.help.clone())
                    .unwrap_or_else(|| "No tool metadata available.".to_owned()),
                prompt_tool_default: tool_prompt_default,
                prompt_plugin_declared_default: plugin_prompt_default,
                prompt_file_override: tool_prompt_override_from_policy(
                    &file_plugins.policy.tool_presentation,
                    &status.plugin_id,
                    tool.name.as_str(),
                ),
                prompt_effective_mode: effective_plugins.policy.tool_presentation.mode_for(
                    &status.plugin_id,
                    &format!("{}.{}", status.plugin_id, tool.name)
                        .parse()
                        .expect("tool key should parse"),
                    nearest_prompt_default,
                ),
                prompt_source: prompt_tool_source(
                    effective_tool_prompt_override,
                    effective_plugin_prompt_override,
                    tool_prompt_default,
                    plugin_prompt_default,
                ),
                prompt_path: plugin_policy_prompt_path(
                    &status.plugin_id.to_string(),
                    Some(tool.name.as_str()),
                ),
                ui_tool_default: tool_ui_default,
                ui_plugin_declared_default: plugin_ui_default,
                ui_file_override: tool_ui_override_from_policy(
                    &file_plugins.policy.ui_presentation,
                    &status.plugin_id,
                    tool.name.as_str(),
                ),
                ui_effective_mode: resolve_tool_text_display_mode(
                    sources,
                    &status.plugin_id.to_string(),
                    tool.name.as_str(),
                    nearest_ui_default,
                    plugin_ui_default,
                ),
                ui_source: ui_tool_source(
                    effective_tool_ui_override,
                    effective_plugin_ui_override,
                    tool_ui_default,
                    plugin_ui_default,
                ),
                ui_path: plugin_policy_ui_path(
                    &status.plugin_id.to_string(),
                    Some(tool.name.as_str()),
                ),
            });
        }
    }

    let override_count = items
        .iter()
        .filter(|item| plugin_policy_item_has_override(item))
        .count();
    let tool_count = items.len().saturating_sub(1);
    let summary = format!(
        "{} tools        {} changed        {}",
        tool_count,
        override_count,
        clean(status.kind)
    );

    PluginPolicySection {
        plugin_id: status.plugin_id.to_string(),
        label,
        summary,
        description,
        items,
    }
}

pub(in crate::app) fn config_plugin_text_display_mode_override(
    sources: &crate::backend::ConfigJsonSources,
    plugin_id: &str,
) -> Option<PluginTextDisplayMode> {
    let plugin_path = format!(
        "{PLUGIN_UI_PRESENTATION_PATH}.plugins.{}",
        quote_settings_segment(plugin_id)
    );
    plugin_get_json_path(&sources.effective, Some(plugin_path.as_str()))
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .and_then(|value| plugin_text_display_mode_from_str(value.as_str()))
}

pub(in crate::app) fn config_tool_text_display_mode_override(
    sources: &crate::backend::ConfigJsonSources,
    plugin_id: &str,
    tool_name: &str,
) -> Option<PluginTextDisplayMode> {
    for key in [
        format!("{plugin_id}/{tool_name}"),
        format!("{plugin_id}.{tool_name}"),
        tool_name.to_owned(),
    ] {
        let path = format!(
            "{PLUGIN_UI_PRESENTATION_PATH}.tools.{}",
            quote_settings_segment(key.as_str())
        );
        if let Some(mode) = plugin_get_json_path(&sources.effective, Some(path.as_str()))
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .and_then(|value| plugin_text_display_mode_from_str(value.as_str()))
        {
            return Some(mode);
        }
    }
    None
}

pub(in crate::app) fn resolve_plugin_text_display_mode(
    sources: &crate::backend::ConfigJsonSources,
    plugin_id: &str,
    plugin_default: Option<PluginTextDisplayMode>,
) -> PluginTextDisplayMode {
    if let Some(mode) = config_plugin_text_display_mode_override(sources, plugin_id) {
        return mode;
    }
    plugin_default.unwrap_or_else(|| {
        plugin_get_json_path(
            &sources.effective,
            Some(PLUGIN_UI_PRESENTATION_DEFAULT_MODE_PATH),
        )
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .and_then(|value| plugin_text_display_mode_from_str(value.as_str()))
        .unwrap_or(PluginTextDisplayMode::Detailed)
    })
}

pub(in crate::app) fn resolve_tool_text_display_mode(
    sources: &crate::backend::ConfigJsonSources,
    plugin_id: &str,
    tool_name: &str,
    tool_default: Option<PluginTextDisplayMode>,
    plugin_default: Option<PluginTextDisplayMode>,
) -> PluginTextDisplayMode {
    for key in [
        format!("{plugin_id}/{tool_name}"),
        format!("{plugin_id}.{tool_name}"),
        tool_name.to_owned(),
    ] {
        let path = format!(
            "{PLUGIN_UI_PRESENTATION_PATH}.tools.{}",
            quote_settings_segment(key.as_str())
        );
        if let Some(mode) = plugin_get_json_path(&sources.effective, Some(path.as_str()))
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .and_then(|value| plugin_text_display_mode_from_str(value.as_str()))
        {
            return mode;
        }
    }
    if let Some(mode) = config_plugin_text_display_mode_override(sources, plugin_id) {
        return mode;
    }
    tool_default
        .unwrap_or_else(|| resolve_plugin_text_display_mode(sources, plugin_id, plugin_default))
}

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
    let visible_tool = tools
        .first()
        .map(|tool| tool.name.clone())
        .unwrap_or_else(|| status.plugin_id.name().to_owned());
    let plugin_id = status.plugin_id.to_string();
    let ui_display_mode =
        resolve_plugin_text_display_mode(sources, plugin_id.as_str(), plugin_ui_default);
    let ui_display_source =
        if config_plugin_text_display_mode_override(sources, plugin_id.as_str()).is_some() {
            PluginTextDisplaySource::PluginOverride
        } else if plugin_ui_default.is_some() {
            PluginTextDisplaySource::PluginDefault
        } else {
            PluginTextDisplaySource::GlobalDefault
        };
    let tool_ui_display_modes = tools
        .iter()
        .map(|tool| {
            let tool_default = plugin_text_display_mode_from_declared(tool.display.ui_display_mode);
            (
                tool.name.clone(),
                resolve_tool_text_display_mode(
                    sources,
                    plugin_id.as_str(),
                    tool.name.as_str(),
                    tool_default,
                    plugin_ui_default,
                ),
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
            let source = if config_tool_text_display_mode_override(
                sources,
                plugin_id.as_str(),
                tool.name.as_str(),
            )
            .is_some()
            {
                PluginTextDisplaySource::ToolOverride
            } else if config_plugin_text_display_mode_override(sources, plugin_id.as_str())
                .is_some()
            {
                PluginTextDisplaySource::PluginOverride
            } else if tool_default.is_some() {
                PluginTextDisplaySource::ToolDefault
            } else if plugin_ui_default.is_some() {
                PluginTextDisplaySource::PluginDefault
            } else {
                PluginTextDisplaySource::GlobalDefault
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
use super::{
    BTreeMap, JsonValue, PLUGIN_UI_PRESENTATION_DEFAULT_MODE_PATH, PLUGIN_UI_PRESENTATION_PATH,
    PluginConfigStatus, PluginConfigStatusKind, PluginPolicyItem, PluginPolicySection,
    PluginTextDisplayMode, PluginTextDisplaySource, PluginWorkbenchPlugin, clean,
    configured_plugin_ids, configured_plugin_kind, derive_override_value, localized_config_schema,
    materialized_config_value, plugin_get_json_path, plugin_policy_item_has_override,
    plugin_policy_prompt_path, plugin_policy_ui_path, plugin_text_display_mode_from_declared,
    plugin_text_display_mode_from_str, plugins_config_from_root, prompt_plugin_source,
    prompt_tool_source, quote_settings_segment, recompute_plugin_config_state,
    resolve_plugin_prompt_mode, resolve_plugin_ui_mode, tool_prompt_override_from_policy,
    tool_ui_override_from_policy, ui_plugin_source, ui_tool_source,
};
