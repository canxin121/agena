impl App {
    pub(crate) fn handle_settings_value_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SettingsValueEditOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(action, input) => {
                if action.path == MCP_PUBLIC_URL_SETTINGS_PATH {
                    let public_url = (!input.trim().is_empty()).then(|| input.trim().to_owned());
                    self.dispatch_backend_operation(
                        move |application| async move {
                            application.set_mcp_public_url(public_url).await
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
                if action.path == MCP_OAUTH_ISSUER_URL_SETTINGS_PATH {
                    let oauth_issuer_url =
                        (!input.trim().is_empty()).then(|| input.trim().to_owned());
                    self.dispatch_backend_operation(
                        move |application| async move {
                            application.set_mcp_oauth_issuer_url(oauth_issuer_url).await
                        },
                        |app, result| match result {
                            Ok(_) => {
                                app.flash_success(
                                    app.i18n.text("settings-mcp-oauth-issuer-updated"),
                                );
                                app.refresh_current_route_after_local_edit();
                            }
                            Err(error) => app.flash_error(error),
                        },
                    );
                    return true;
                }
                if action.path == MCP_OAUTH_PASSWORD_SETTINGS_PATH {
                    self.dispatch_backend_operation(
                        move |application| async move {
                            application.set_mcp_oauth_password(input.as_str()).await
                        },
                        |app, result| match result {
                            Ok(_) => {
                                app.flash_success(
                                    app.i18n.text("settings-mcp-oauth-password-updated"),
                                );
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
    App, InputDialogKeyResult, KeyEvent, MCP_OAUTH_ISSUER_URL_SETTINGS_PATH,
    MCP_OAUTH_PASSWORD_SETTINGS_PATH, MCP_PUBLIC_URL_SETTINGS_PATH, SettingsValueEditOverlay,
    drive_input_dialog_key, parse_settings_field_input, settings_path_cleared_message,
    settings_path_updated_message,
};
