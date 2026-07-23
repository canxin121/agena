impl App {
    pub(in crate::app) fn handle_settings_value_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SettingsValueEditOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(action, input) => {
                match parse_settings_field_input(&self.i18n, action, input.as_str()) {
                    Ok(Some(value)) => match self
                        .block_on_async(self.backend.set_config_setting(action.path, value))
                    {
                        Ok(_) => {
                            self.flash_success(settings_path_updated_message(
                                &self.i18n,
                                action.path,
                            ));
                            self.refresh_current_route_after_local_edit();
                            true
                        }
                        Err(error) => {
                            self.flash_error(error);
                            false
                        }
                    },
                    Ok(None) => {
                        match self.block_on_async(self.backend.delete_config_setting(action.path)) {
                            Ok(_) => {
                                self.flash_success(settings_path_cleared_message(
                                    &self.i18n,
                                    action.path,
                                ));
                                self.refresh_current_route_after_local_edit();
                                true
                            }
                            Err(error) => {
                                self.flash_error(error);
                                false
                            }
                        }
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
use crate::app::{
    App, InputDialogKeyResult, KeyEvent, SettingsValueEditOverlay, drive_input_dialog_key,
    parse_settings_field_input, settings_path_cleared_message, settings_path_updated_message,
};
