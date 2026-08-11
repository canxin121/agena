impl App {
    pub(crate) fn handle_settings_value_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SettingsValueEditOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(action, input) => {
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
    App, InputDialogKeyResult, KeyEvent, SettingsValueEditOverlay, drive_input_dialog_key,
    parse_settings_field_input, settings_path_cleared_message, settings_path_updated_message,
};
