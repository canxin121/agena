impl App {
    pub(crate) fn open_session_additional_system_prompt_editor(&mut self, session_id: i64) {
        self.dispatch_backend_operation(
            move |application| async move { application.get_session_state(session_id).await },
            move |app, result| match result {
                Ok(execution) => {
                    let context = execution.execution;
                    let state = match context.additional_system_prompt.as_deref() {
                        None => "Currently inheriting additional instructions.",
                        Some("") => "Inherited additional instructions are disabled.",
                        Some(_) => "This session has its own additional instructions.",
                    };
                    app.overlay = Some(crate::Overlay::SessionPromptEdit(
                        crate::SessionPromptEditOverlay::new(
                            format!("Additional system prompt — session {session_id}"),
                            format!(
                                "{state} Save an empty value to disable inherited instructions. Use Restore session prompt inheritance in settings to inherit again."
                            ),
                            "Ctrl+S save · Enter newline · Esc cancel".to_owned(),
                            true,
                            crate::Editor::from_text(
                                context.additional_system_prompt.unwrap_or_default(),
                            ),
                            session_id,
                        ),
                    ));
                }
                Err(error) => app.flash_error(error),
            },
        );
    }

    pub(crate) fn save_session_additional_system_prompt(
        &mut self,
        session_id: i64,
        value: Option<String>,
    ) {
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .set_session_additional_system_prompt(session_id, value)
                    .await
            },
            |app, result| match result {
                Ok(_) => {
                    app.flash_success("Session additional system prompt updated.");
                    app.refresh_current_route_after_local_edit();
                }
                Err(error) => app.flash_error(error),
            },
        );
    }

    pub(crate) fn handle_session_prompt_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut crate::SessionPromptEditOverlay,
    ) -> bool {
        match crate::drive_editor_dialog_key(dialog, key) {
            crate::EditorDialogKeyResult::Close => true,
            crate::EditorDialogKeyResult::Submit(session_id, input) => {
                self.save_session_additional_system_prompt(session_id, Some(input));
                true
            }
            crate::EditorDialogKeyResult::Continue => false,
        }
    }

    pub(crate) fn handle_settings_value_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SettingsValueEditOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(action, input) => {
                if action.path == MCP_SERVER_URL_SETTINGS_PATH {
                    let public_url = (!input.trim().is_empty()).then(|| input.trim().to_owned());
                    self.dispatch_backend_operation(
                        move |application| async move {
                            application.set_mcp_server_url(public_url).await
                        },
                        |app, result| match result {
                            Ok(_) => {
                                app.flash_success(app.i18n.text("settings-mcp-public-url-updated"));
                                app.refresh_current_route_after_local_edit();
                            }
                            Err(error) => app.flash_error(error),
                        },
                    );
                    return true;
                }
                match parse_settings_field_input(&self.i18n, &action, input.as_str()) {
                    Ok(Some(value)) => {
                        let path = action.path.clone();
                        self.dispatch_backend_operation(
                            move |application| async move {
                                application.set_config_setting(path.as_str(), value).await
                            },
                            move |app, result| match result {
                                Ok(_) => {
                                    app.flash_success(settings_path_updated_message(
                                        &app.i18n,
                                        action.path.as_str(),
                                    ));
                                    app.refresh_current_route_after_local_edit();
                                }
                                Err(error) => app.flash_error(error),
                            },
                        );
                        true
                    }
                    Ok(None) => {
                        let path = action.path.clone();
                        self.dispatch_backend_operation(
                            move |application| async move {
                                application.delete_config_setting(path.as_str()).await
                            },
                            move |app, result| match result {
                                Ok(_) => {
                                    app.flash_success(settings_path_cleared_message(
                                        &app.i18n,
                                        action.path.as_str(),
                                    ));
                                    app.refresh_current_route_after_local_edit();
                                }
                                Err(error) => app.flash_error(error),
                            },
                        );
                        true
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            InputDialogKeyResult::Continue => false,
        }
    }
}
use crate::{
    App, InputDialogKeyResult, KeyEvent, MCP_SERVER_URL_SETTINGS_PATH, SettingsValueEditOverlay,
    drive_input_dialog_key, parse_settings_field_input, settings_path_cleared_message,
    settings_path_updated_message,
};
