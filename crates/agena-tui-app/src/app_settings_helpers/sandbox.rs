use super::{
    ConfigJsonSources, I18n, JsonValue, SettingsFieldKind, SettingsFieldSpec, SettingsPickerAction,
    SettingsStudioItem, SettingsStudioSectionId, settings_source_rows_for_config_path,
    settings_source_rows_for_workspace_config_path,
};
use crate::ui_text;

fn sandbox_boolean(source: &JsonValue, key: &str) -> Option<bool> {
    source.get("sandbox")?.get(key)?.as_bool()
}

pub(crate) fn settings_studio_sandbox_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let configured = |key: &str, workspace: bool| {
        workspace
            .then(|| sandbox_boolean(&sources.project_file, key))
            .flatten()
            .or_else(|| sandbox_boolean(&sources.file, key))
            .unwrap_or(true)
    };
    let effective = |key: &str| {
        sandbox_boolean(&sources.effective, key).unwrap_or_else(|| configured(key, true))
    };
    let isolated = effective("enabled");
    let resource_checks = !isolated && effective("resource_checks");
    let environment_disabled = !isolated && configured("enabled", true);
    let fields = [
        (
            "enabled",
            "settings-sandbox-enabled-label",
            "settings-sandbox-enabled-description",
            isolated,
        ),
        (
            "resource_checks",
            "settings-sandbox-resource-checks-label",
            "settings-sandbox-resource-checks-description",
            resource_checks,
        ),
        (
            "command_hints",
            "settings-sandbox-command-hints-label",
            "settings-sandbox-command-hints-description",
            resource_checks && effective("command_hints"),
        ),
    ];
    let mut items = Vec::with_capacity(6);
    for workspace in [false, true] {
        for (key, label_key, description_key, active) in fields {
            let path = format!("sandbox.{key}");
            let label = i18n.text_args(
                if workspace {
                    "settings-sandbox-workspace-label"
                } else {
                    "settings-sandbox-global-label"
                },
                &agena_tui::fl_args!("label" => ui_text::t(i18n, label_key)),
            );
            let mut description = ui_text::t(i18n, description_key);
            let inactive_reason = if key == "enabled" && environment_disabled {
                Some("settings-sandbox-environment-disabled")
            } else if key != "enabled" && isolated {
                Some("settings-sandbox-inactive-isolated")
            } else if key == "command_hints" && !resource_checks {
                Some("settings-sandbox-inactive-checks")
            } else {
                None
            };
            if let Some(reason) = inactive_reason {
                description.push(' ');
                description.push_str(&ui_text::t(i18n, reason));
            }
            let file = if workspace {
                &sources.project_file
            } else {
                &sources.file
            };
            let current_summary = sandbox_boolean(file, key)
                .map(|value| value.to_string())
                .unwrap_or_else(|| ui_text::t(i18n, "settings-source-unset"));
            let status = ui_text::t(
                i18n,
                if active {
                    "settings-sandbox-active"
                } else {
                    "settings-sandbox-inactive"
                },
            );
            let source_rows = if workspace {
                settings_source_rows_for_workspace_config_path(
                    i18n,
                    sources,
                    &path,
                    &current_summary,
                    active.to_string(),
                )
            } else {
                settings_source_rows_for_config_path(
                    i18n,
                    sources,
                    &path,
                    &current_summary,
                    active.to_string(),
                )
            };
            let field = SettingsFieldSpec {
                section: SettingsStudioSectionId::Permissions,
                workspace,
                path: path.clone(),
                label_key,
                description_key,
                kind: SettingsFieldKind::Bool,
                label_override: Some(label.clone()),
                description_override: Some(description.clone()),
            };
            items.push(SettingsStudioItem::from_parts(
                label,
                format!("{} · {status}", configured(key, workspace)),
                description,
                Some(path),
                Some(current_summary),
                Some(active.to_string()),
                source_rows,
                SettingsPickerAction::EditField(field),
            ));
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sources(global: JsonValue, workspace: JsonValue, effective: JsonValue) -> ConfigJsonSources {
        ConfigJsonSources {
            config_path: "/test/global.json".into(),
            config_found: true,
            project_config_path: "/test/workspace.json".into(),
            project_config_found: true,
            applied_layers: vec!["global".into(), "workspace".into()],
            file: global,
            project_file: workspace,
            effective,
        }
    }

    #[test]
    fn sandbox_fields_offer_both_layers_and_boolean_choices() {
        let items = settings_studio_sandbox_items(
            &I18n::english(),
            &sources(json!({}), json!({}), json!({})),
        );
        assert_eq!(items.len(), 6);
        for (index, item) in items.iter().enumerate() {
            let SettingsPickerAction::EditField(field) = &item.action else {
                panic!("expected sandbox editor")
            };
            assert_eq!(field.workspace, index >= 3);
            assert!(matches!(field.kind, SettingsFieldKind::Bool));
            assert_eq!(
                crate::App::settings_field_choice_overlay_style(field),
                agena_tui::choice::ChoicePresentationStyle::SelectOnly,
            );
        }
        assert_eq!(items[0].effective_value.as_deref(), Some("true"));
        assert_eq!(items[1].effective_value.as_deref(), Some("false"));
        assert!(items[1].detail.contains("Inactive while"));
    }

    #[test]
    fn sandbox_fields_keep_each_layer_editable_when_overridden() {
        let state = sources(
            json!({"sandbox":{"enabled":false,"resource_checks":false}}),
            json!({"sandbox":{"resource_checks":true,"command_hints":false}}),
            json!({"sandbox":{"enabled":false,"resource_checks":true,"command_hints":false}}),
        );
        let items = settings_studio_sandbox_items(&I18n::english(), &state);
        assert!(items[1].value.starts_with("false"));
        assert!(items[4].value.starts_with("true"));
        assert_eq!(items[4].effective_value.as_deref(), Some("true"));
        assert_eq!(items[5].effective_value.as_deref(), Some("false"));
    }

    #[test]
    fn sandbox_fields_explain_environment_and_dependency_overrides() {
        let state = sources(
            json!({"sandbox":{"enabled":true,"resource_checks":false,"command_hints":true}}),
            json!({}),
            json!({"sandbox":{"enabled":false,"resource_checks":false,"command_hints":true}}),
        );
        let items = settings_studio_sandbox_items(&I18n::english(), &state);
        assert!(items[0].value.starts_with("true"));
        assert!(items[0].detail.contains("runtime environment"));
        assert_eq!(items[0].effective_value.as_deref(), Some("false"));
        assert_eq!(items[2].effective_value.as_deref(), Some("false"));
        assert!(items[2].detail.contains("declaration checks are disabled"));
    }
}
