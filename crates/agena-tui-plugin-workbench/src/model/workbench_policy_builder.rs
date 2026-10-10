use super::{
    BTreeMap, JsonValue, PluginConfigStatus, PluginConfigStatusKind, PluginWorkbenchPlugin,
    derive_override_value, materialized_config_value, plugin_get_json_path, plugin_settings_schema,
    quote_settings_segment, recompute_plugin_config_state,
};

pub fn build_plugin_workbench_plugin(
    sources: &agena_application::dto::ConfigJsonSources,
    locale: &str,
    status: agena_plugin_host::status::PluginStatus,
    mut inspect: Option<agena_plugin_host::PluginInspect>,
    logs: Vec<agena_plugin_host::PluginLogRecord>,
) -> PluginWorkbenchPlugin {
    if let Some(manifest) = inspect
        .as_mut()
        .and_then(|inspect| inspect.manifest.as_mut())
    {
        localize_manifest_docs(manifest, locale);
    }
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
        .and_then(
            |configured_plugin| match serde_json::to_value(configured_plugin) {
                Ok(configured_plugin) => Some(configured_plugin),
                Err(error) => {
                    tracing::error!(
                        plugin_id,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "serialize configured plugin data for the TUI workbench",
                            &error,
                        ),
                        "TUI plugin workbench will fall back to the effective configuration source"
                    );
                    None
                }
            },
        )
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
    let schema = manifest.and_then(plugin_settings_schema);
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

/// Resolve presentation copy in the workbench's owned inspection snapshot.
/// The manifest sent to model tool selection keeps its stable base docs.
fn localize_manifest_docs(manifest: &mut agena_plugin_host::sdk::PluginManifest, locale: &str) {
    manifest.localize_settings(locale);
    let summary = manifest.summary_for_locale(locale).map(str::to_owned);
    let help = manifest.help_for_locale(locale).map(str::to_owned);
    manifest.summary = summary;
    manifest.help = help;
    for tool in &mut manifest.tools {
        let docs = &mut tool.docs;
        let before_help = docs.before_help_for_locale(locale).map(str::to_owned);
        let after_help = docs.after_help_for_locale(locale).map(str::to_owned);
        let summary = docs.summary_for_locale(locale).map(str::to_owned);
        let help = docs.help_for_locale(locale).map(str::to_owned);
        docs.before_help = before_help;
        docs.after_help = after_help;
        docs.summary = summary;
        docs.help = help;
    }
}

#[cfg(test)]
mod localization_tests {
    use std::collections::BTreeMap;

    use agena_plugin_host::sdk::{
        PluginManifest, PluginManifestTranslation, ToolContract, ToolDefinition, ToolDocs,
        ToolDocsTranslation, ToolRuntimePolicy,
    };

    use super::localize_manifest_docs;

    #[test]
    fn workbench_uses_localized_docs_without_changing_the_model_base_contract() {
        let mut manifest = PluginManifest::new("test", "localized", "1");
        manifest.summary = Some("Stable plugin summary".to_owned());
        manifest.translations.insert(
            "zh-CN".to_owned(),
            PluginManifestTranslation {
                summary: Some("插件摘要".to_owned()),
                help: None,
                settings: BTreeMap::from([
                    ("Settings".to_owned(), "设置".to_owned()),
                    ("Plugin settings".to_owned(), "插件设置说明".to_owned()),
                    ("Enabled".to_owned(), "已启用".to_owned()),
                ]),
            },
        );
        let settings = serde_json::from_value(serde_json::json!({
            "version": 1,
            "root": {
                "id": "root", "path": "", "title": "Settings", "description": "Plugin settings",
                "kind": "object", "fields": [
                    {"id": "enabled", "path": "/enabled", "title": "Enabled", "kind": "boolean", "default": true}
                ]
            }
        }))
        .unwrap();
        manifest.settings = Some(settings);
        let base_settings = manifest.settings.clone();
        let base_docs = ToolDocs {
            summary: Some("Stable tool summary".to_owned()),
            translations: BTreeMap::from([(
                "zh-CN".to_owned(),
                ToolDocsTranslation {
                    summary: Some("工具摘要".to_owned()),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        manifest.tools.push(ToolDefinition {
            name: "inspect".to_owned(),
            contract: ToolContract::default(),
            docs: base_docs.clone(),
            runtime: ToolRuntimePolicy::default(),
            tags: Vec::new(),
        });

        localize_manifest_docs(&mut manifest, "zh-CN");

        assert_eq!(manifest.summary.as_deref(), Some("插件摘要"));
        assert_eq!(manifest.tools[0].docs.summary.as_deref(), Some("工具摘要"));
        assert_eq!(base_docs.summary.as_deref(), Some("Stable tool summary"));
        let localized = manifest.settings.as_ref().unwrap();
        assert_eq!(localized.root.title, "设置");
        assert_eq!(localized.root.description, "插件设置说明");
        let schema = super::plugin_settings_schema(&manifest).unwrap();
        assert_eq!(schema["properties"]["enabled"]["title"], "已启用");
        assert_eq!(
            localized.default_value().unwrap(),
            serde_json::json!({"enabled": true})
        );
        assert_eq!(base_settings.unwrap().root.title, "Settings");
    }
}
