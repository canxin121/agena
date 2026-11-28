use crate::{
    App, ChoiceItem, ChoiceOverlayAction, I18n, JsonValue, PermissionStudioAction,
    PermissionStudioModeTarget, PermissionStudioNetworkAction, PermissionStudioOverlay,
    PermissionStudioProjection, SettingsFieldProjection, SettingsFieldSpec, SettingsPickerAction,
    SettingsStudioOverlay, permission_studio_source_config, permission_studio_source_is_local,
    settings_field_edit_title, settings_layer_value_label, settings_path_cleared_message,
    settings_path_updated_message,
};

pub(super) fn source_items(
    i18n: &I18n,
    can_inherit: bool,
    inherited: String,
    local: String,
) -> Vec<ChoiceItem> {
    [
        (
            "inherit",
            crate::settings_layer_reset_label(i18n, can_inherit),
            inherited,
        ),
        ("local", i18n.text("settings-layer-local"), local),
    ]
    .into_iter()
    .filter(|(value, _, _)| can_inherit || *value != "local")
    .map(|(value, label, detail)| {
        let label = format!("{label} · {detail}");
        ChoiceItem {
            search_text: label.clone(),
            label,
            detail: String::new(),
            value: value.into(),
            current: false,
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> (tempfile::TempDir, App) {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let app = App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        (temp, app)
    }

    fn sources(local: JsonValue, inherited: JsonValue) -> crate::ConfigJsonSources {
        crate::ConfigJsonSources {
            config_path: "/test/global.json".into(),
            config_found: true,
            project_config_path: "/test/workspace.json".into(),
            project_config_found: true,
            file: local.clone(),
            project_file: local,
            defaults: inherited.clone(),
            global_resolved: JsonValue::Null,
            workspace_resolved: JsonValue::Null,
            global_inherited: JsonValue::Null,
            workspace_inherited: inherited,
            effective: json!({"sandbox":true,"runtime":{"limit":99},"ui":{"tui":{"prompt":"later"}}}),
            applied_layers: vec![],
        }
    }

    fn field(path: &str) -> SettingsFieldSpec {
        SettingsFieldSpec {
            explicit_local: false,
            section: crate::SettingsStudioSectionId::RuntimeSession,
            workspace: true,
            path: path.into(),
            kind: crate::SettingsFieldKind::String,
            label_key: "settings-layer-source-title",
            description_key: "settings-layer-source-description",
            label_override: None,
            description_override: None,
        }
    }

    #[test]
    fn inherited_field_rejects_value_entry_until_source_explicitly_selects_local() {
        let (_temp, mut app) = app();
        let sources = sources(json!({}), json!({"custom":{"label":"parent label"}}));
        let mut field = field("custom.label");
        let projection = crate::SettingsFieldProjection::new(&sources, &field);
        assert_eq!(projection.value, Some(json!("parent label")));
        assert!(projection.source_label(&app.i18n).contains("Global"));
        assert!(matches!(
            projection.control(&app.i18n),
            agena_tui_settings::SettingsStudioControl::Inherited(_)
        ));
        app.open_settings_field_editor_with_sources(field.clone(), &sources);
        assert!(app.overlay.is_none());
        field.explicit_local = true;
        app.open_settings_field_editor_with_sources(field, &sources);
        assert!(matches!(
            app.overlay,
            Some(crate::Overlay::SettingsValueEdit(_))
        ));
        assert_eq!(sources.project_file, json!({}));
    }

    #[test]
    fn direct_editor_edits_atomic_object_children_without_resetting_the_model_group() {
        let (_temp, mut app) = app();
        let raw = json!({"providers":{"default_selection":{"provider":"local","model":"chosen","parallel_tool_calls":false}}});
        let mut sources = sources(
            raw.clone(),
            json!({"providers":{"default_selection":{"provider":"parent","model":"base","verbosity":"high"}}}),
        );
        sources.workspace_resolved = raw.clone();
        let field = field("providers.default_selection.verbosity");
        app.open_settings_field_editor_with_sources(field, &sources);
        let Some(crate::Overlay::SettingsValueEdit(editor)) = &app.overlay else {
            panic!("atomic object children must open the direct value editor");
        };
        assert_eq!(editor.action.path, "providers.default_selection.verbosity");
        assert_eq!(editor.input.text(), "");
        assert_eq!(sources.project_file, raw);
    }

    #[test]
    fn source_picker_pins_typed_falsy_values_and_distinguishes_identical_local_values() {
        let (_temp, mut app) = app();
        let inherited =
            json!({"sandbox":false,"runtime":{"limit":0},"ui":{"tui":{"prompt":"","clear":null}}});
        for local in [json!({}), inherited.clone()] {
            let sources = sources(local.clone(), inherited.clone());
            for (path, expected) in [
                ("sandbox", json!(false)),
                ("runtime.limit", json!(0)),
                ("ui.tui.prompt", json!("")),
                ("ui.tui.clear", JsonValue::Null),
            ] {
                app.open_settings_layer_source(field(path), None, &sources);
                let Some(crate::Overlay::Choice(picker)) = &app.overlay else {
                    panic!("expected source picker");
                };
                let ChoiceOverlayAction::SettingsFieldSource {
                    field, local_value, ..
                } = &picker.action
                else {
                    panic!("expected settings source action");
                };
                assert_eq!(field.path, path);
                assert_eq!(local_value, &Some(expected));
                assert_eq!(
                    agena_tui::choice::selection(&picker.presentation),
                    Some(agena_tui::choice::ChoiceSelection::Item {
                        value: if local == json!({}) {
                            "inherit"
                        } else {
                            "local"
                        }
                        .into(),
                    })
                );
            }
        }
    }

    #[test]
    fn source_picker_restores_actual_ancestor_group_and_keeps_leaf_null_a_leaf() {
        let (_temp, mut app) = app();
        for (raw, expected_path) in [
            (json!({"ui":{"tui":null}}), "ui.tui"),
            (json!({"ui":{"tui":{"theme":null}}}), "ui.tui.theme"),
        ] {
            let sources = sources(
                raw.clone(),
                json!({"ui":{"tui":{"theme":"dark","size":12}}}),
            );
            app.open_settings_layer_source(field("ui.tui.theme"), None, &sources);
            let Some(crate::Overlay::Choice(picker)) = &app.overlay else {
                panic!("expected source picker");
            };
            let ChoiceOverlayAction::SettingsFieldSource {
                field, local_value, ..
            } = &picker.action
            else {
                panic!("expected settings source action");
            };
            assert_eq!(field.path, expected_path);
            assert_eq!(local_value, &Some(JsonValue::Null));
            if expected_path == "ui.tui" {
                assert!(picker.presentation.prompt.contains("ui.tui"));
                assert!(picker.presentation.prompt.contains("whole"));
            }
            app.handle_key_event(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ));
            assert!(app.overlay.is_none());
            assert_eq!(sources.file, raw);
        }
    }

    #[test]
    fn global_uses_a_reset_command_while_workspace_keeps_source_choices_in_every_locale() {
        let (_temp, mut app) = app();
        for (locale, _) in agena_tui::i18n::SUPPORTED_LOCALES {
            app.i18n = I18n::resolve(Some(locale), None);
            for workspace in [false, true] {
                for configured in [false, true] {
                    app.overlay = None;
                    let mut field = field("sandbox");
                    field.workspace = workspace;
                    let sources = sources(
                        if configured {
                            json!({"sandbox":false})
                        } else {
                            json!({})
                        },
                        json!({"sandbox":true}),
                    );
                    let projection = SettingsFieldProjection::new(&sources, &field);
                    assert_eq!(projection.value, Some(json!(!configured)));
                    match projection.control(&app.i18n) {
                        agena_tui_settings::SettingsStudioControl::Source(_)
                        | agena_tui_settings::SettingsStudioControl::Inherited(_) => {
                            assert!(workspace)
                        }
                        agena_tui_settings::SettingsStudioControl::RestoreDefault { enabled } => {
                            assert!(!workspace);
                            assert_eq!(enabled, configured);
                        }
                    }
                    app.open_settings_layer_source(field, None, &sources);
                    if workspace {
                        let Some(crate::Overlay::Choice(picker)) = &app.overlay else {
                            panic!("expected workspace source picker")
                        };
                        assert_eq!(picker.presentation.items.len(), 2);
                        assert!(
                            picker
                                .presentation
                                .items
                                .iter()
                                .any(|item| item.value == "local")
                        );
                    } else if configured {
                        let Some(crate::Overlay::Confirm(confirm)) = &app.overlay else {
                            panic!("expected reset confirmation")
                        };
                        assert!(
                            matches!(&confirm.action, crate::ConfirmAction::SettingsRestoreDefault { field } if !field.workspace && field.path=="sandbox")
                        );
                        assert!(confirm.body_lines.iter().any(|line| line.contains("true")));
                    } else {
                        assert!(
                            app.overlay.is_none(),
                            "missing globals are already default; no pin action is offered"
                        );
                    }
                }
            }
        }
        assert!(
            source_items(&app.i18n, false, "true".into(), "true".into())
                .iter()
                .all(|item| item.value != "local")
        );
    }

    #[test]
    fn global_reset_uses_the_actual_ancestor_boundary_and_cancel_preserves_configuration() {
        let (_temp, mut app) = app();
        for (raw, expected_path) in [
            (json!({"ui":{"tui":null}}), "ui.tui"),
            (json!({"ui":{"tui":{"theme":null}}}), "ui.tui.theme"),
        ] {
            let sources = sources(
                raw.clone(),
                json!({"ui":{"tui":{"theme":"dark","size":12}}}),
            );
            let mut field = field("ui.tui.theme");
            field.workspace = false;
            app.open_settings_layer_source(field, None, &sources);
            let Some(crate::Overlay::Confirm(confirm)) = &app.overlay else {
                panic!("expected reset confirmation")
            };
            assert!(
                matches!(&confirm.action, crate::ConfirmAction::SettingsRestoreDefault { field } if field.path==expected_path)
            );
            if expected_path == "ui.tui" {
                assert!(
                    confirm
                        .body_lines
                        .iter()
                        .any(|line| line.contains("whole") && line.contains("ui.tui"))
                );
            }
            app.handle_key_event(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ));
            assert!(app.overlay.is_none());
            assert_eq!(sources.file, raw);
        }
    }

    #[test]
    fn global_model_reset_is_an_explicit_group_action_without_a_pin_choice() {
        let (_temp, mut app) = app();
        let model = json!({"provider":"p","model":"m","parallel_tool_calls":false});
        let sources = sources(json!({"providers":{"default_selection":model}}), json!({}));
        let mut field = field("providers.default_selection");
        field.workspace = false;
        app.open_settings_layer_source(
            field,
            Some(agena_tui::model_chooser::SessionModelChooserPurpose::GlobalDefault),
            &sources,
        );
        let Some(crate::Overlay::Confirm(confirm)) = &app.overlay else {
            panic!("expected model reset confirmation")
        };
        assert!(
            matches!(&confirm.action, crate::ConfirmAction::SettingsRestoreDefault { field } if field.path=="providers.default_selection")
        );
        assert!(
            confirm
                .body_lines
                .iter()
                .any(|line| line.contains("model") && line.contains("group"))
        );
    }

    #[test]
    fn global_value_picker_selects_the_filled_value_without_a_default_option() {
        let (_temp, mut app) = app();
        let mut field = field("sandbox");
        field.workspace = false;
        app.open_settings_field_editor_with_sources(
            field,
            &sources(json!({}), json!({"sandbox":true})),
        );
        let Some(crate::Overlay::Choice(picker)) = &app.overlay else {
            panic!("expected value picker")
        };
        assert!(picker.presentation.clear_action.is_none());
        assert!(
            matches!(agena_tui::choice::selection(&picker.presentation), Some(agena_tui::choice::ChoiceSelection::Item { ref value }) if value=="true")
        );
        assert_eq!(
            picker
                .presentation
                .items
                .iter()
                .map(|item| item.value.as_str())
                .collect::<Vec<_>>(),
            vec!["true", "false"]
        );
        assert!(!picker.presentation.prompt.contains("Setting source"));
    }

    #[test]
    fn source_picker_model_group_uses_parent_group_and_keeps_model_chooser_for_unset_values() {
        let (_temp, mut app) = app();
        let purpose = agena_tui::model_chooser::SessionModelChooserPurpose::WorkspaceDefault;
        let expected = json!({"provider":"p","model":"m","parallel_tool_calls":false});
        let sources = sources(
            json!({}),
            json!({"providers":{"default_selection":expected.clone()}}),
        );
        app.open_settings_layer_source(
            field("providers.default_selection"),
            Some(purpose),
            &sources,
        );
        let Some(crate::Overlay::Choice(picker)) = &app.overlay else {
            panic!("expected source picker");
        };
        let ChoiceOverlayAction::SettingsFieldSource {
            local_value,
            model_editor,
            ..
        } = &picker.action
        else {
            panic!("expected model source action");
        };
        assert_eq!(local_value, &Some(expected));
        assert_eq!(*model_editor, Some(purpose));
        app.open_settings_layer_source(field("runtime.auxiliary_model"), Some(purpose), &sources);
        let Some(crate::Overlay::Choice(picker)) = &app.overlay else {
            panic!("expected source picker");
        };
        assert!(matches!(
            &picker.action,
            ChoiceOverlayAction::SettingsFieldSource {
                local_value: None,
                model_editor: Some(_),
                ..
            }
        ));
    }
}

impl App {
    fn open_global_default_reset(
        &mut self,
        label: String,
        description: String,
        value: String,
        action: crate::ConfirmAction,
    ) {
        let title = self.i18n.text_args(
            "settings-restore-default-title",
            &agena_tui::fl_args!("setting" => label),
        );
        let body = vec![
            description,
            self.i18n.text_args(
                "settings-restore-default-preview",
                &agena_tui::fl_args!("value" => value),
            ),
        ];
        self.overlay = Some(crate::Overlay::Confirm(
            self.build_confirm_overlay(title, body, action),
        ));
    }

    pub(crate) fn open_settings_studio_source(&mut self, dialog: &SettingsStudioOverlay) {
        let Some(item) = dialog.state.selected_item() else {
            return;
        };
        use agena_tui::model_chooser::SessionModelChooserPurpose as Purpose;
        let (workspace, model_editor) = match &item.action {
            SettingsPickerAction::EditField(field) => (field.workspace, None),
            SettingsPickerAction::OpenGlobalDefaultModelChooser => {
                (false, Some(Purpose::GlobalDefault))
            }
            SettingsPickerAction::OpenWorkspaceDefaultModelChooser => {
                (true, Some(Purpose::WorkspaceDefault))
            }
            SettingsPickerAction::OpenAuxiliaryModelChooser { workspace } => (
                *workspace,
                Some(if *workspace {
                    Purpose::WorkspaceAuxiliary
                } else {
                    Purpose::Auxiliary
                }),
            ),
            SettingsPickerAction::OpenPermissionApprovalModelChooser => {
                (false, Some(Purpose::PermissionApproval))
            }
            SettingsPickerAction::OpenWorkspacePermissionApprovalModelChooser => {
                (true, Some(Purpose::WorkspacePermissionApproval))
            }
            _ => return,
        };
        let field = if let SettingsPickerAction::EditField(field) = &item.action {
            field.clone()
        } else {
            let Some(path) = item.path.clone() else {
                return;
            };
            SettingsFieldSpec {
                explicit_local: false,
                section: crate::SettingsStudioSectionId::ModelsProviders,
                workspace,
                path,
                label_key: "settings-layer-source-title",
                description_key: "settings-layer-group-model",
                kind: crate::SettingsFieldKind::String,
                label_override: Some(item.label.clone()),
                description_override: Some(item.detail.clone()),
            }
        };
        if field.path == crate::MCP_SERVER_URL_SETTINGS_PATH {
            return;
        }
        let sources = match crate::app_backend::config::config_json_sources(&self.application) {
            Ok(sources) => sources,
            Err(error) => {
                self.flash_error(crate::UiFailure::internal(error));
                return;
            }
        };
        self.open_settings_layer_source(field, model_editor, &sources);
    }

    pub(crate) fn open_settings_layer_source(
        &mut self,
        mut field: SettingsFieldSpec,
        model_editor: Option<agena_tui::model_chooser::SessionModelChooserPurpose>,
        sources: &crate::ConfigJsonSources,
    ) {
        let mut projection = SettingsFieldProjection::new(sources, &field);
        let mut description = crate::settings_layer_source_description(&self.i18n, field.workspace);
        if let Some(path) = projection
            .override_path
            .clone()
            .filter(|path| path != &field.path)
        {
            description.push('\n');
            description.push_str(&self.i18n.text_args(
                if field.workspace {
                    "settings-layer-ancestor-override"
                } else {
                    "settings-layer-ancestor-default"
                },
                &agena_tui::fl_args!("path" => path.clone()),
            ));
            field.path = path;
            projection = SettingsFieldProjection::new(sources, &field);
        }
        if model_editor.is_some() {
            description.push('\n');
            description.push_str(&self.i18n.text("settings-layer-group-model"));
        }
        if !field.workspace {
            if projection.local.is_none() {
                self.flash_warning(self.i18n.text("settings-already-default"));
                return;
            }
            self.open_global_default_reset(
                settings_field_edit_title(&self.i18n, &field),
                description,
                settings_layer_value_label(&self.i18n, projection.fallback_value()),
                crate::ConfirmAction::SettingsRestoreDefault { field },
            );
            return;
        }
        // Known-unset optional fields still need a concrete local value.
        // Explicit opaque UI null clears may be copied as real local values.
        let explicit_clear = projection.local.as_ref().is_some_and(JsonValue::is_null)
            || (projection
                .inherited
                .as_ref()
                .is_some_and(JsonValue::is_null)
                && agena_domain::config_setting_has_override(
                    &sources.workspace_inherited,
                    Some(&field.path),
                )
                .unwrap_or(false));
        let local_value = projection
            .value
            .clone()
            .filter(|value| !value.is_null() || explicit_clear);
        let picker = self.build_choice_overlay(
            format!(
                "{} · {}",
                settings_field_edit_title(&self.i18n, &field),
                self.i18n.text("settings-layer-source-title")
            ),
            description,
            Some(
                if projection.local.is_some() {
                    "local"
                } else {
                    "inherit"
                }
                .into(),
            ),
            source_items(
                &self.i18n,
                field.workspace,
                settings_layer_value_label(&self.i18n, projection.fallback_value()),
                settings_layer_value_label(&self.i18n, projection.value.as_ref()),
            ),
            ChoiceOverlayAction::SettingsFieldSource {
                field,
                local_value,
                model_editor,
            },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        );
        self.open_choice_overlay(picker);
    }

    pub(crate) fn open_settings_source_model_editor(
        &mut self,
        purpose: agena_tui::model_chooser::SessionModelChooserPurpose,
    ) {
        self.route_stack.push(self.current_route.clone());
        self.open_model_chooser_with_source(purpose, true, agena_domain::ModelSwitchSource::Picker);
    }

    /// Keep the value typed: false, zero and an empty string are all explicit
    /// overrides, including when they equal the inherited value.
    pub(crate) fn require_settings_field_editable(&mut self, field: &SettingsFieldSpec) -> bool {
        if !field.workspace || field.explicit_local {
            return true;
        }
        let editable = crate::app_backend::config::config_json_sources(&self.application)
            .map(|sources| crate::settings_field_value_editable(&sources, field))
            .unwrap_or(false);
        if !editable {
            self.flash_warning(crate::settings_inherited_locked_message(&self.i18n, false));
        }
        editable
    }

    pub(crate) fn persist_settings_field_value(
        &mut self,
        field: SettingsFieldSpec,
        value: Option<JsonValue>,
    ) {
        let path = field.path.clone();
        let workspace = field.workspace;
        let cleared = value.is_none();
        self.dispatch_backend_operation(
            move |application| async move {
                match (workspace, value) {
                    (true, Some(value)) => {
                        application
                            .set_workspace_config_setting(path.as_str(), value)
                            .await
                    }
                    (false, Some(value)) => {
                        application.set_config_setting(path.as_str(), value).await
                    }
                    (true, None) => {
                        application
                            .delete_workspace_config_setting(path.as_str())
                            .await
                    }
                    (false, None) => application.delete_config_setting(path.as_str()).await,
                }
            },
            move |app, result| match result {
                Ok(_) => {
                    app.flash_success(if cleared {
                        settings_path_cleared_message(&app.i18n, &field.path)
                    } else {
                        settings_path_updated_message(&app.i18n, &field.path)
                    });
                    app.refresh_current_route_after_local_edit();
                }
                Err(error) => app.flash_error(error),
            },
        );
    }

    pub(crate) fn open_permission_studio_source(&mut self, dialog: &PermissionStudioOverlay) {
        if !dialog.editable {
            self.flash_warning(crate::permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        if crate::permission_studio_has_draft(dialog) {
            self.flash_warning(self.i18n.text("permission-studio-network-save-or-cancel"));
            return;
        }
        let Some(item) = dialog.state.selected_item() else {
            return;
        };
        let target = match &item.action {
            PermissionStudioAction::EditMode(target) => target.clone(),
            PermissionStudioAction::AddToolCommandPattern => PermissionStudioModeTarget::ToolRule {
                tool_name: agena_domain::SHELL_COMMAND_RULE_GROUP.to_owned(),
            },
            PermissionStudioAction::Network(
                PermissionStudioNetworkAction::ListSource { deny }
                | PermissionStudioNetworkAction::AddRule { deny }
                | PermissionStudioNetworkAction::EditRule { deny, .. },
            ) => {
                if matches!(dialog.source, crate::PermissionStudioSource::GlobalConfig) {
                    self.open_permission_studio_network_list_reset(dialog, *deny);
                } else {
                    self.open_permission_studio_network_list_source(dialog, *deny);
                }
                return;
            }
            PermissionStudioAction::EditNetworkPolicy
            | PermissionStudioAction::Network(
                PermissionStudioNetworkAction::Source
                | PermissionStudioNetworkAction::Offline
                | PermissionStudioNetworkAction::Outbound,
            ) => {
                self.open_permission_studio_network_source(dialog);
                return;
            }
            _ => return,
        };
        let projection = PermissionStudioProjection::new(dialog);
        let mut inherited_dialog = dialog.clone();
        inherited_dialog.permission =
            match permission_studio_source_config(&self.i18n, dialog, &target, false) {
                Ok(permission) => permission,
                Err(error) => {
                    self.flash_warning(error);
                    return;
                }
            };
        let inherited = PermissionStudioProjection::new(&inherited_dialog);
        let value = |projection: &PermissionStudioProjection| {
            if let PermissionStudioModeTarget::ToolRule { tool_name }
            | PermissionStudioModeTarget::ToolCommandPattern { tool_name, .. } = &target
            {
                match projection
                    .effective
                    .tools
                    .as_ref()
                    .and_then(|tools| tools.rules.get(tool_name))
                {
                    Some(crate::ToolPermissionRules::Ordered(entries)) => self.i18n.text_args(
                        "value-rule-count",
                        &agena_tui::fl_args!("count" => entries.len() as i64),
                    ),
                    Some(crate::ToolPermissionRules::Mode(mode)) => {
                        crate::permission_mode_label(&self.i18n, *mode)
                    }
                    None => {
                        projection
                            .mode_item(&self.i18n, String::new(), target.clone())
                            .value
                    }
                }
            } else {
                projection
                    .mode_item(&self.i18n, String::new(), target.clone())
                    .value
            }
        };
        let can_inherit = !matches!(dialog.source, crate::PermissionStudioSource::GlobalConfig);
        let mut description = crate::settings_layer_source_description(&self.i18n, can_inherit);
        let title = if let PermissionStudioModeTarget::ToolRule { tool_name }
        | PermissionStudioModeTarget::ToolCommandPattern { tool_name, .. } = &target
        {
            description.push('\n');
            if tool_name == agena_domain::SHELL_COMMAND_RULE_GROUP {
                description.push_str(&self.i18n.text("settings-layer-group-commands"));
                self.i18n.text("permission-studio-nav-command-rules")
            } else {
                description.push_str(&self.i18n.text("settings-layer-group-tool"));
                tool_name.clone()
            }
        } else {
            item.label.clone()
        };
        if !can_inherit {
            if !permission_studio_source_is_local(dialog, &target) {
                self.flash_warning(self.i18n.text("settings-already-default"));
                return;
            }
            self.open_global_default_reset(
                title,
                description,
                value(&inherited),
                crate::ConfirmAction::PermissionStudioRestoreDefault { target },
            );
            return;
        }
        let picker = self.build_choice_overlay(
            format!(
                "{title} · {}",
                self.i18n.text("settings-layer-source-title")
            ),
            description,
            Some(
                if permission_studio_source_is_local(dialog, &target) {
                    "local"
                } else {
                    "inherit"
                }
                .into(),
            ),
            source_items(
                &self.i18n,
                can_inherit,
                value(&inherited),
                value(&projection),
            ),
            ChoiceOverlayAction::PermissionStudioSource(target),
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        );
        self.open_choice_overlay(picker);
    }

    fn open_permission_studio_network_list_reset(
        &mut self,
        dialog: &PermissionStudioOverlay,
        deny: bool,
    ) {
        let projection = PermissionStudioProjection::new(dialog);
        if projection.network_list_origin(deny) != crate::PermissionStudioValueSource::Current {
            self.flash_warning(self.i18n.text("settings-already-default"));
            return;
        }
        let mut reset = dialog.clone();
        let Ok(permission) = crate::permission_studio_network_list_source_config(
            &self.i18n, dialog, deny, "inherit",
        ) else {
            return;
        };
        reset.permission = permission;
        let network = PermissionStudioProjection::new(&reset).network();
        let count = network
            .egress
            .as_ref()
            .and_then(|egress| {
                if deny {
                    egress.deny.as_ref()
                } else {
                    egress.allow.as_ref()
                }
            })
            .map_or(0, Vec::len);
        self.open_global_default_reset(
            crate::permission_studio_network_list_label(&self.i18n, deny),
            self.i18n.text("settings-layer-default-description"),
            self.i18n.text_args(
                "value-rule-count",
                &agena_tui::fl_args!("count" => count as i64),
            ),
            crate::ConfirmAction::PermissionStudioRestoreNetworkList { deny },
        );
    }

    pub(crate) fn open_permission_studio_network_list_source(
        &mut self,
        dialog: &PermissionStudioOverlay,
        deny: bool,
    ) {
        if !dialog.editable {
            self.flash_warning(crate::permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        if crate::permission_studio_has_draft(dialog) {
            self.flash_warning(self.i18n.text("permission-studio-network-save-or-cancel"));
            return;
        }
        let projection = PermissionStudioProjection::new(dialog);
        let mut fallback = dialog.clone();
        fallback.permission = match crate::permission_studio_network_list_source_config(
            &self.i18n, dialog, deny, "inherit",
        ) {
            Ok(permission) => permission,
            Err(error) => {
                self.flash_error(error);
                return;
            }
        };
        let count = |projection: &PermissionStudioProjection| {
            let network = projection.network();
            let count = network
                .egress
                .as_ref()
                .and_then(|egress| {
                    if deny {
                        egress.deny.as_ref()
                    } else {
                        egress.allow.as_ref()
                    }
                })
                .map_or(0, Vec::len);
            self.i18n.text_args(
                "value-rule-count",
                &agena_tui::fl_args!("count" => count as i64),
            )
        };
        let can_inherit = !matches!(dialog.source, crate::PermissionStudioSource::GlobalConfig);
        let mut items = source_items(
            &self.i18n,
            can_inherit,
            count(&PermissionStudioProjection::new(&fallback)),
            count(&projection),
        );
        if !can_inherit {
            items.retain(|item| {
                item.value != "inherit"
                    || projection.network_list_origin(deny)
                        == crate::PermissionStudioValueSource::Current
            });
            for item in &mut items {
                item.label = self.i18n.text("settings-key-restore-default");
                item.search_text = item.label.clone();
            }
        }
        let empty_label = self.i18n.text(if can_inherit {
            "permission-studio-network-list-empty"
        } else {
            "permission-studio-network-list-clear"
        });
        items.push(ChoiceItem {
            label: empty_label.clone(),
            search_text: empty_label,
            detail: String::new(),
            value: "empty".into(),
            current: false,
        });
        let picker = self.build_choice_overlay(
            format!(
                "{} · {}",
                crate::permission_studio_network_list_label(&self.i18n, deny),
                self.i18n.text(if can_inherit {
                    "settings-layer-source-title"
                } else {
                    "settings-list-actions"
                })
            ),
            if can_inherit {
                format!(
                    "{}\n{}",
                    crate::settings_layer_source_description(&self.i18n, true),
                    self.i18n.text("permission-studio-network-list-source-help")
                )
            } else {
                self.i18n.text("settings-list-actions-description")
            },
            can_inherit.then(|| {
                if projection.network_list_origin(deny)
                    == crate::PermissionStudioValueSource::Current
                {
                    "local".into()
                } else {
                    "inherit".into()
                }
            }),
            items,
            ChoiceOverlayAction::PermissionStudioNetworkListSource { deny },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        );
        self.open_choice_overlay(picker);
    }

    pub(crate) fn open_permission_studio_network_source(
        &mut self,
        dialog: &PermissionStudioOverlay,
    ) {
        if !dialog.editable || crate::permission_studio_has_draft(dialog) {
            return;
        }
        if matches!(dialog.source, crate::PermissionStudioSource::GlobalConfig) {
            self.activate_permission_studio_network(
                &mut dialog.clone(),
                PermissionStudioNetworkAction::Source,
            );
            return;
        }
        let parent = crate::permission_override_summary(
            &self.i18n,
            &crate::PermissionConfig {
                network: dialog.inherited_permission.network.clone(),
                ..Default::default()
            },
        );
        let current = crate::permission_override_summary(
            &self.i18n,
            &crate::PermissionConfig {
                network: Some(PermissionStudioProjection::new(dialog).network()),
                ..Default::default()
            },
        );
        let picker = self.build_choice_overlay(
            self.i18n.text("permission-studio-network-json"),
            self.i18n.text("settings-network-source-group"),
            Some(
                if crate::permission_studio_network_editable(dialog) {
                    "local"
                } else {
                    "inherit"
                }
                .into(),
            ),
            source_items(&self.i18n, true, parent, current),
            ChoiceOverlayAction::PermissionStudioNetworkSource,
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        );
        self.open_choice_overlay(picker);
    }

    pub(crate) fn commit_permission_studio_network_source(&mut self, value: &str) -> bool {
        let Some((host, mut dialog)) = self.take_permission_studio_dialog() else {
            return false;
        };
        if !dialog.editable
            || matches!(dialog.source, crate::PermissionStudioSource::GlobalConfig)
            || crate::permission_studio_has_draft(&dialog)
        {
            self.restore_permission_studio_dialog(host, dialog);
            return false;
        }
        let mut permission = dialog.permission.clone();
        let result = match value {
            "inherit" => {
                permission.network = None;
                self.persist_permission_studio_source(&mut dialog, permission)
            }
            "local"
                if dialog.inherited_permission.network.is_some()
                    || crate::permission_studio_network_editable(&dialog) =>
            {
                permission.network = Some(PermissionStudioProjection::new(&dialog).network());
                self.persist_permission_studio_source(&mut dialog, permission)
            }
            _ => Err(crate::UiFailure::message(
                self.i18n.text("settings-inherited-unavailable"),
            )),
        };
        self.restore_permission_studio_dialog(host, dialog);
        match result {
            Ok(()) => true,
            Err(error) => {
                self.flash_warning(error);
                false
            }
        }
    }

    pub(crate) fn commit_permission_studio_network_list_source(
        &mut self,
        deny: bool,
        value: &str,
    ) -> bool {
        let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
            self.flash_error(self.i18n.text("flash-permission-studio-context-lost"));
            return true;
        };
        if value == "local" && matches!(parent.source, crate::PermissionStudioSource::GlobalConfig)
        {
            self.restore_permission_studio_dialog(host, parent);
            return false;
        }
        let result = if !parent.editable {
            Err(crate::UiFailure::message(
                crate::permission_studio_read_only_message(&self.i18n, &parent.source),
            ))
        } else if crate::permission_studio_has_draft(&parent) {
            Err(crate::UiFailure::message(
                self.i18n.text("permission-studio-network-save-or-cancel"),
            ))
        } else {
            crate::permission_studio_network_list_source_config(&self.i18n, &parent, deny, value)
                .and_then(|permission| {
                    self.persist_permission_studio_source(&mut parent, permission)
                })
        };
        self.restore_permission_studio_dialog(host, parent);
        match result {
            Ok(()) => true,
            Err(error) => {
                self.flash_warning(error);
                false
            }
        }
    }

    pub(crate) fn commit_permission_studio_source(
        &mut self,
        target: PermissionStudioModeTarget,
        value: &str,
    ) -> bool {
        let local = match value {
            "inherit" => false,
            "local" => true,
            _ => return false,
        };
        let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
            self.flash_error(self.i18n.text("flash-permission-studio-context-lost"));
            return true;
        };
        if local && matches!(parent.source, crate::PermissionStudioSource::GlobalConfig) {
            self.restore_permission_studio_dialog(host, parent);
            return false;
        }
        let result = if !parent.editable {
            Err(crate::UiFailure::message(
                crate::permission_studio_read_only_message(&self.i18n, &parent.source),
            ))
        } else if crate::permission_studio_has_draft(&parent) {
            Err(crate::UiFailure::message(
                self.i18n.text("permission-studio-network-save-or-cancel"),
            ))
        } else {
            permission_studio_source_config(&self.i18n, &parent, &target, local)
                .map_err(crate::UiFailure::message)
                .and_then(|permission| {
                    self.persist_permission_studio_source(&mut parent, permission)
                })
        };
        self.restore_permission_studio_dialog(host, parent);
        match result {
            Ok(()) => true,
            Err(error) => {
                self.flash_warning(error);
                false
            }
        }
    }
}
