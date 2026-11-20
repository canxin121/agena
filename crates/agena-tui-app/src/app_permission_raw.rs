use agena_tui_components::{EditorDialogKeyResult, EditorDialogState, drive_editor_dialog_key};

use crate::{App, ConfigJsonSources, Editor, I18n, JsonValue, KeyEvent, Overlay, UiResult};

#[derive(Debug, Clone)]
pub(crate) struct PermissionRawEditOverlay {
    pub(crate) editor: EditorDialogState<bool>,
    pub(crate) initial_value: JsonValue,
    pub(crate) save_request: Option<uuid::Uuid>,
}

#[derive(Debug, PartialEq)]
struct PermissionRawMutation {
    workspace: bool,
    value: Option<JsonValue>,
}

impl PermissionRawMutation {
    fn path(&self) -> &'static str {
        "permission"
    }
}

fn build_permission_raw_editor(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    workspace: bool,
) -> UiResult<PermissionRawEditOverlay> {
    let source = if workspace {
        &sources.project_file
    } else {
        &sources.file
    };
    let document = source.as_object().ok_or_else(|| {
        crate::UiFailure::internal("the current configuration layer's raw document is unavailable")
    })?;
    let initial_value = document
        .get("permission")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let scope = i18n.text(if workspace {
        "permission-studio-source-workspace"
    } else {
        "permission-studio-source-global"
    });
    let title = i18n.text_args(
        "permission-studio-raw-title",
        &agena_tui::fl_args!("scope" => scope),
    );
    let prompt = format!(
        "{}\n{}",
        i18n.text("permission-studio-raw-unavailable"),
        i18n.text("permission-studio-raw-help")
    );
    Ok(PermissionRawEditOverlay {
        editor: EditorDialogState::new(
            title,
            prompt,
            crate::editor_save_footer(i18n, true),
            true,
            Editor::from_text(
                serde_json::to_string_pretty(&initial_value).map_err(crate::UiFailure::internal)?,
            ),
            workspace,
        ),
        initial_value,
        save_request: None,
    })
}

fn permission_raw_mutation(
    dialog: &PermissionRawEditOverlay,
    input: &str,
) -> Result<Option<PermissionRawMutation>, String> {
    let value: JsonValue = serde_json::from_str(input).map_err(|error| error.to_string())?;
    if value == dialog.initial_value {
        return Ok(None);
    }
    if !value.is_null() {
        if !value.is_object() {
            return Err("expected a JSON object or null".into());
        }
        serde_json::from_value::<agena_domain::PermissionConfig>(value.clone())
            .map_err(|error| error.to_string())?;
    }
    Ok(Some(PermissionRawMutation {
        workspace: dialog.editor.action,
        value: (!value.is_null()).then_some(value),
    }))
}

impl App {
    pub(crate) fn open_permission_raw_editor(&mut self, workspace: bool) {
        let result = crate::app_backend::config::config_json_sources(&self.application)
            .map_err(crate::UiFailure::internal)
            .and_then(|sources| build_permission_raw_editor(&self.i18n, &sources, workspace));
        match result {
            Ok(dialog) => self.overlay = Some(Overlay::PermissionRawEdit(dialog)),
            Err(error) => self.flash_error(error),
        }
    }

    fn complete_permission_raw_save(&mut self, request: uuid::Uuid, result: UiResult<()>) {
        let Some(Overlay::PermissionRawEdit(active)) = self.overlay.as_mut() else {
            return;
        };
        if active.save_request != Some(request) {
            return;
        }
        active.save_request = None;
        active.editor.footer = crate::editor_save_footer(&self.i18n, true);
        match result {
            Ok(()) => {
                self.overlay = None;
                self.flash_success(crate::settings_path_updated_message(
                    &self.i18n,
                    "permission",
                ));
                self.refresh_current_route_after_local_edit();
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn handle_permission_raw_editor_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionRawEditOverlay,
    ) -> bool {
        if dialog.save_request.is_some() {
            return false;
        }
        match drive_editor_dialog_key(&mut dialog.editor, key) {
            EditorDialogKeyResult::Close => true,
            EditorDialogKeyResult::Continue => false,
            EditorDialogKeyResult::Submit(_, input) => {
                let mutation = match permission_raw_mutation(dialog, &input) {
                    Ok(Some(mutation)) => mutation,
                    Ok(None) => return true,
                    Err(error) => {
                        self.flash_warning(self.i18n.text_args(
                            "permission-studio-raw-invalid",
                            &agena_tui::fl_args!("error" => error),
                        ));
                        return false;
                    }
                };
                let request = uuid::Uuid::new_v4();
                dialog.save_request = Some(request);
                dialog.editor.footer = self.i18n.text("permission-studio-network-saving");
                self.dispatch_backend_operation(
                    move |application| async move {
                        let path = mutation.path();
                        match (mutation.workspace, mutation.value) {
                            (true, Some(value)) => {
                                application.set_workspace_config_setting(path, value).await
                            }
                            (false, Some(value)) => {
                                application.set_config_setting(path, value).await
                            }
                            (true, None) => application.delete_workspace_config_setting(path).await,
                            (false, None) => application.delete_config_setting(path).await,
                        }
                    },
                    move |app, result| {
                        app.complete_permission_raw_save(request, result.map(|_| ()));
                    },
                );
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use serde_json::json;

    fn sources() -> ConfigJsonSources {
        ConfigJsonSources {
            config_path: Default::default(),
            config_found: true,
            project_config_path: Default::default(),
            project_config_found: true,
            applied_layers: vec![],
            defaults: JsonValue::Null,
            global_resolved: JsonValue::Null,
            workspace_resolved: JsonValue::Null,
            global_inherited: JsonValue::Null,
            workspace_inherited: JsonValue::Null,
            file: json!({"permission":{"network":{"version":2,"allow":[]}},"sandbox":false}),
            project_file: json!({"permission":{"tools":{"default":"ask"}},"session":{"compaction":{"auto":false}}}),
            effective: json!({"permission":{"network":{"version":2,"default":"allow"}}}),
        }
    }

    #[test]
    fn permission_raw_editor_preserves_unknown_base_and_only_edits_the_selected_permission_subtree()
    {
        let sources = sources();
        for workspace in [false, true] {
            let dialog =
                build_permission_raw_editor(&I18n::english(), &sources, workspace).unwrap();
            let original = if workspace {
                &sources.project_file
            } else {
                &sources.file
            };
            assert_eq!(dialog.initial_value, original["permission"]);
            assert_eq!(
                serde_json::from_str::<JsonValue>(&dialog.editor.input.text()).unwrap(),
                original["permission"]
            );
            let mutation = permission_raw_mutation(&dialog, r#"{"path":{"workspace":"readonly"}}"#)
                .unwrap()
                .unwrap();
            assert_eq!(mutation.path(), "permission");
            assert_eq!(mutation.workspace, workspace);
            assert_eq!(
                mutation.value,
                Some(json!({"path":{"workspace":"readonly"}}))
            );
            let reset = permission_raw_mutation(&dialog, "null").unwrap().unwrap();
            assert_eq!(reset.path(), "permission");
            assert_eq!(reset.value, None);
        }
        assert_eq!(sources.file["sandbox"], false);
        assert_eq!(sources.project_file["session"]["compaction"]["auto"], false);
    }

    #[test]
    fn permission_raw_editor_cancel_and_unchanged_save_do_not_start_a_write() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        for key in [
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        ] {
            let mut dialog = build_permission_raw_editor(&app.i18n, &sources(), false).unwrap();
            assert!(app.handle_permission_raw_editor_key(key, &mut dialog));
            assert!(dialog.save_request.is_none());
            assert!(app.deferred_ui_completions.is_empty());
        }
    }

    #[test]
    fn permission_raw_editor_keeps_missing_permissions_sparse_and_blocks_duplicate_pending_saves() {
        let mut sources = sources();
        sources.file = json!({"sandbox":false});
        let mut dialog = build_permission_raw_editor(&I18n::english(), &sources, false).unwrap();
        assert_eq!(dialog.initial_value, json!({}));
        assert_eq!(permission_raw_mutation(&dialog, " { } ").unwrap(), None);
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        let request = uuid::Uuid::new_v4();
        dialog.save_request = Some(request);
        for key in [
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        ] {
            assert!(!app.handle_permission_raw_editor_key(key, &mut dialog));
            assert_eq!(dialog.save_request, Some(request));
            assert!(app.deferred_ui_completions.is_empty());
            assert_eq!(dialog.editor.input.text(), "{}");
        }
        assert_eq!(sources.file, json!({"sandbox":false}));
    }

    #[test]
    fn permission_raw_editor_ignores_late_completions_and_retains_the_draft_after_save_errors() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        let mut dialog = build_permission_raw_editor(&app.i18n, &sources(), true).unwrap();
        let request = uuid::Uuid::new_v4();
        let input = r#"{"tools":{"default":"deny"}}"#;
        dialog.editor.input = Editor::from_text(input.to_owned());
        dialog.save_request = Some(request);
        app.overlay = Some(Overlay::PermissionRawEdit(dialog));
        app.complete_permission_raw_save(uuid::Uuid::new_v4(), Ok(()));
        let Some(Overlay::PermissionRawEdit(active)) = &app.overlay else {
            panic!("late result closed a newer editor")
        };
        assert_eq!(active.save_request, Some(request));
        app.complete_permission_raw_save(request, Err(crate::UiFailure::message("save failed")));
        let Some(Overlay::PermissionRawEdit(active)) = &app.overlay else {
            panic!("save failure discarded the draft")
        };
        assert_eq!(active.save_request, None);
        assert_eq!(active.editor.input.text(), input);
        assert!(active.editor.action);
    }

    #[test]
    fn permission_raw_editor_keeps_invalid_input_and_requires_an_explicit_json_value() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        for input in ["", "{", "[]", "true", r#"{"tools":{"default":false}}"#] {
            let mut dialog = build_permission_raw_editor(&app.i18n, &sources(), false).unwrap();
            dialog.editor.input = Editor::from_text(input.to_owned());
            assert!(!app.handle_permission_raw_editor_key(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                &mut dialog
            ));
            assert_eq!(dialog.editor.input.text(), input);
            assert!(dialog.save_request.is_none());
            assert!(app.deferred_ui_completions.is_empty());
        }
    }
}
