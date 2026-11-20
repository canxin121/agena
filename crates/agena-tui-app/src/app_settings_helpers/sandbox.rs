use super::{
    ConfigJsonSources, I18n, JsonValue, SettingsFieldKind, SettingsFieldSpec, SettingsPickerAction,
    SettingsStudioItem, SettingsStudioSectionId, settings_source_rows_for_config_path,
    settings_source_rows_for_workspace_config_path,
};
use crate::ui_text;

fn execution_boolean(source: &JsonValue, key: &str) -> Option<bool> {
    source.get(key)?.as_bool()
}

pub(crate) fn settings_studio_sandbox_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let configured = |key: &str, workspace: bool| {
        super::SettingsFieldProjection::at_path(sources, key, workspace)
            .value
            .as_ref()
            .and_then(JsonValue::as_bool)
    };
    let effective =
        |key: &str| execution_boolean(&sources.effective, key).or_else(|| configured(key, true));
    let value_label = |value: Option<bool>| {
        value
            .map(|value| value.to_string())
            .unwrap_or_else(|| ui_text::t(i18n, "settings-source-unset"))
    };
    let isolated = effective("sandbox");
    let resource_guard = match isolated {
        Some(true) => Some(false),
        Some(false) => effective("resource_guard"),
        None => None,
    };
    let environment_disabled = isolated == Some(false) && configured("sandbox", true) == Some(true);
    let fields = [
        (
            "sandbox",
            "settings-sandbox-enabled-label",
            "settings-sandbox-enabled-description",
            isolated,
        ),
        (
            "resource_guard",
            "settings-resource-guard-label",
            "settings-resource-guard-description",
            resource_guard,
        ),
    ];
    let mut items = Vec::with_capacity(4);
    for workspace in [false, true] {
        for (key, label_key, description_key, active) in fields {
            let path = key.to_string();
            let label = i18n.text_args(
                if workspace {
                    "settings-sandbox-workspace-label"
                } else {
                    "settings-sandbox-global-label"
                },
                &agena_tui::fl_args!("label" => ui_text::t(i18n, label_key)),
            );
            let mut description = ui_text::t(i18n, description_key);
            let inactive_reason = if key == "sandbox" && environment_disabled {
                Some("settings-sandbox-environment-disabled")
            } else if key == "resource_guard"
                && isolated == Some(true)
                && effective("resource_guard") == Some(true)
            {
                Some("settings-sandbox-inactive-isolated")
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
            let current_summary = execution_boolean(file, key)
                .map(|value| value.to_string())
                .unwrap_or_else(|| ui_text::t(i18n, "settings-source-unset"));
            let display_value = value_label(configured(key, workspace));
            let display_value = match active {
                Some(active) => format!(
                    "{} · {}",
                    display_value,
                    ui_text::t(
                        i18n,
                        if active {
                            "settings-sandbox-active"
                        } else {
                            "settings-sandbox-inactive"
                        }
                    )
                ),
                None => display_value,
            };
            let source_rows = if workspace {
                settings_source_rows_for_workspace_config_path(
                    i18n,
                    sources,
                    &path,
                    &current_summary,
                    value_label(effective(key)),
                )
            } else {
                settings_source_rows_for_config_path(
                    i18n,
                    sources,
                    &path,
                    &current_summary,
                    value_label(effective(key)),
                )
            };
            let field = SettingsFieldSpec {
                explicit_local: false,
                section: SettingsStudioSectionId::Permissions,
                workspace,
                path: path.clone(),
                label_key,
                description_key,
                kind: SettingsFieldKind::Bool,
                label_override: Some(label.clone()),
                description_override: Some(description.clone()),
            };
            items.push(
                SettingsStudioItem::from_parts(
                    label,
                    display_value,
                    description,
                    Some(path),
                    Some(current_summary),
                    Some(value_label(effective(key))),
                    source_rows,
                    SettingsPickerAction::EditField(field.clone()),
                )
                .with_control(super::SettingsFieldProjection::new(sources, &field).control(i18n)),
            );
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
            defaults: json!({"sandbox":true,"resource_guard":true}),
            global_resolved: JsonValue::Null,
            workspace_resolved: JsonValue::Null,
            global_inherited: JsonValue::Null,
            workspace_inherited: json!({
                "sandbox": global.get("sandbox").and_then(JsonValue::as_bool).unwrap_or(true),
                "resource_guard": global.get("resource_guard").and_then(JsonValue::as_bool).unwrap_or(true),
            }),
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
        assert_eq!(items.len(), 4);
        for (index, item) in items.iter().enumerate() {
            let SettingsPickerAction::EditField(field) = &item.action else {
                panic!("expected sandbox editor")
            };
            assert_eq!(field.workspace, index >= 2);
            assert_eq!(
                field.path,
                if index % 2 == 0 {
                    "sandbox"
                } else {
                    "resource_guard"
                }
            );
            assert!(matches!(field.kind, SettingsFieldKind::Bool));
            assert_eq!(
                crate::App::settings_field_choice_overlay_style(field),
                agena_tui::choice::ChoicePresentationStyle::SelectOnly,
            );
        }
        assert_eq!(items[0].effective_value.as_deref(), Some("true"));
        assert_eq!(items[1].effective_value.as_deref(), Some("true"));
        assert!(items[1].detail.contains("On standby while"));
    }

    #[test]
    fn disabled_resource_guard_is_not_presented_as_waiting_for_mxc() {
        let items = settings_studio_sandbox_items(
            &I18n::english(),
            &sources(
                json!({"resource_guard":false}),
                json!({}),
                json!({"sandbox":true,"resource_guard":false}),
            ),
        );
        assert!(items[1].value.starts_with("false"));
        assert!(!items[1].detail.contains("On standby"));
        assert_eq!(items[1].effective_value.as_deref(), Some("false"));
    }

    #[test]
    fn sandbox_fields_keep_each_layer_editable_when_overridden() {
        let state = sources(
            json!({"sandbox":false,"resource_guard":false}),
            json!({"resource_guard":true}),
            json!({"sandbox":false,"resource_guard":true}),
        );
        let items = settings_studio_sandbox_items(&I18n::english(), &state);
        assert!(items[1].value.starts_with("false"));
        assert!(items[3].value.starts_with("true"));
        assert_eq!(items[3].effective_value.as_deref(), Some("true"));
        assert_eq!(items[2].effective_value.as_deref(), Some("false"));
    }

    #[test]
    fn sandbox_fields_explain_explicit_environment_override_without_changing_guard() {
        let state = sources(
            json!({"sandbox":true,"resource_guard":false}),
            json!({}),
            json!({"sandbox":false,"resource_guard":false}),
        );
        let items = settings_studio_sandbox_items(&I18n::english(), &state);
        assert!(items[0].value.starts_with("true"));
        assert!(items[0].detail.contains("runtime environment"));
        assert_eq!(items[0].effective_value.as_deref(), Some("false"));
        assert_eq!(items[1].effective_value.as_deref(), Some("false"));
        assert!(items[1].value.starts_with("false"));
    }
    #[test]
    fn sandbox_fields_take_authoritative_layer_defaults_without_guessing_missing_values() {
        let mut state = sources(json!({}), json!({}), JsonValue::Null);
        state.defaults = json!({"sandbox":false,"resource_guard":false});
        state.global_resolved = state.defaults.clone();
        state.workspace_inherited = state.defaults.clone();
        state.workspace_resolved = state.defaults.clone();
        let items = settings_studio_sandbox_items(&I18n::english(), &state);
        assert!(items.iter().all(|item| item.value.starts_with("false")));

        state.defaults = JsonValue::Null;
        state.global_resolved = JsonValue::Null;
        state.workspace_inherited = JsonValue::Null;
        state.workspace_resolved = JsonValue::Null;
        let items = settings_studio_sandbox_items(&I18n::english(), &state);
        let unset = I18n::english().text("settings-source-unset");
        assert!(items.iter().all(|item| item.value == unset));
        assert!(
            items
                .iter()
                .all(|item| item.effective_value.as_deref() == Some(unset.as_str()))
        );
    }
}
