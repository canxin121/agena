impl App {
    pub(crate) fn finish_model_selection(
        &mut self,
        purpose: agena_tui::model_chooser::SessionModelChooserPurpose,
        model: ModelRef,
        thinking_mode: Option<String>,
        speed_mode: Option<String>,
        verbosity: Option<String>,
    ) -> bool {
        match purpose {
            agena_tui::model_chooser::SessionModelChooserPurpose::PermissionApproval => {
                let mut approval = serde_json::Map::new();
                approval.insert(
                    "provider_id".to_owned(),
                    JsonValue::String(model.provider_id.to_string()),
                );
                if let Some(adapter_id) = model.adapter_id.as_ref() {
                    approval.insert(
                        "adapter_id".to_owned(),
                        JsonValue::String(adapter_id.to_string()),
                    );
                }
                approval.insert(
                    "model_id".to_owned(),
                    JsonValue::String(model.model_id.to_string()),
                );
                insert_optional_selection_value(&mut approval, "thinking_mode", thinking_mode);
                insert_optional_selection_value(&mut approval, "speed_mode", speed_mode);
                insert_optional_selection_value(&mut approval, "verbosity", verbosity);
                self.dispatch_backend_operation(
                    move |application| async move {
                        application
                            .set_config_setting(
                                "permission.approval_model",
                                JsonValue::Object(approval),
                            )
                            .await
                    },
                    move |app, result| {
                        app.finish_model_selection_persisted(purpose, model, result.map(|_| ()))
                    },
                );
                false
            }
            agena_tui::model_chooser::SessionModelChooserPurpose::RuntimeOverride => {
                self.finish_model_selection_persisted(purpose, model, Ok(()));
                true
            }
        }
    }

    fn finish_model_selection_persisted(
        &mut self,
        purpose: agena_tui::model_chooser::SessionModelChooserPurpose,
        model: ModelRef,
        result: UiResult<()>,
    ) {
        match result {
            Ok(()) => {
                self.flash_success(self.i18n.text_args(
                    if purpose
                        == agena_tui::model_chooser::SessionModelChooserPurpose::PermissionApproval
                    {
                        "flash-permission-approval-model-updated"
                    } else {
                        "flash-session-model-updated"
                    },
                    &agena_tui::fl_args!(
                        "provider" => model.provider_id.to_string(),
                        "model" => model.model_id.to_string(),
                    ),
                ));
                self.refresh_tui_palette_from_runtime();
                self.current_route = self
                    .route_stack
                    .pop()
                    .map(|route| self.refresh_restored_route(route))
                    .unwrap_or(crate::Route::Main);
            }
            Err(error) => {
                self.flash_error(error);
            }
        }
    }

    pub(crate) fn current_permission_approval_model_ref(&self) -> Option<ModelRef> {
        let sources = match crate::app_backend::config::config_json_sources(&self.application) {
            Ok(sources) => sources,
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "load effective configuration for the permission approval model",
                        error.as_ref(),
                    ),
                    "permission approval model is unavailable"
                );
                return None;
            }
        };
        let permission = match get_json_path(&sources.effective, Some("permission")) {
            Ok(permission) => permission,
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "read the permission configuration for the approval model",
                        &error,
                    ),
                    "permission approval model is unavailable"
                );
                return None;
            }
        };
        let permission = match serde_json::from_value::<PermissionConfig>(permission) {
            Ok(permission) => permission,
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "decode the permission configuration for the approval model",
                        &error,
                    ),
                    "permission approval model is unavailable"
                );
                return None;
            }
        };
        let approval_model = permission.approval_model?;
        match approval_model.model_ref() {
            Ok(model_ref) => Some(model_ref),
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "decode the configured permission approval model reference",
                        &error,
                    ),
                    "permission approval model is unavailable"
                );
                None
            }
        }
    }
}

fn insert_optional_selection_value(
    selection: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        selection.insert(key.to_owned(), JsonValue::String(value));
    }
}

#[cfg(test)]
mod tests;

use crate::{App, JsonMap, JsonValue, ModelRef, PermissionConfig, UiResult, get_json_path};
