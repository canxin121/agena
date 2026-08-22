impl App {
    pub(crate) fn current_global_default_model_ref(&self) -> Option<ModelRef> {
        if let Some(selection) = self
            .application
            .runtime_status()
            .and_then(|status| status.default_selection)
            && let (Some(provider_id), Some(model_id)) =
                (selection.provider.as_deref(), selection.model.as_deref())
        {
            return match selection.adapter.as_deref() {
                Some(adapter_id) => {
                    ModelRef::try_new_with_adapter(provider_id, adapter_id, model_id).ok()
                }
                None => ModelRef::try_new(provider_id, model_id).ok(),
            };
        }

        let sources = crate::app_backend::config::config_json_sources(&self.application).ok()?;
        let selection = get_json_path(&sources.effective, Some("providers.default_selection"))
            .ok()
            .and_then(|value| {
                serde_json::from_value::<agena_domain::ModelSelectionConfig>(value).ok()
            })?;
        let (provider_id, model_id) = (selection.provider?, selection.model?);
        match selection.adapter.as_deref() {
            Some(adapter_id) => {
                ModelRef::try_new_with_adapter(provider_id, adapter_id, model_id).ok()
            }
            None => ModelRef::try_new(provider_id, model_id).ok(),
        }
    }

    pub(crate) fn finish_model_selection(
        &mut self,
        purpose: agena_tui::model_chooser::SessionModelChooserPurpose,
        model: ModelRef,
        thinking_mode: Option<String>,
        speed_mode: Option<String>,
        verbosity: Option<String>,
    ) -> bool {
        let selection = model_selection_value(
            &model,
            thinking_mode.clone(),
            speed_mode.clone(),
            verbosity.clone(),
        );
        match purpose {
            agena_tui::model_chooser::SessionModelChooserPurpose::GlobalDefault => {
                self.dispatch_backend_operation(
                    move |application| async move {
                        application
                            .set_config_setting("providers.default_selection", selection)
                            .await
                    },
                    move |app, result| {
                        app.finish_model_selection_persisted(purpose, model, result.map(|_| ()))
                    },
                );
                false
            }
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
                    match purpose {
                        agena_tui::model_chooser::SessionModelChooserPurpose::PermissionApproval => {
                            "flash-permission-approval-model-updated"
                        }
                        agena_tui::model_chooser::SessionModelChooserPurpose::GlobalDefault => {
                            "flash-global-default-model-updated"
                        }
                        agena_tui::model_chooser::SessionModelChooserPurpose::RuntimeOverride => {
                            "flash-model-selected"
                        }
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

fn model_selection_value(
    model: &ModelRef,
    thinking_mode: Option<String>,
    speed_mode: Option<String>,
    verbosity: Option<String>,
) -> JsonValue {
    let mut selection = serde_json::Map::new();
    selection.insert(
        "provider".to_owned(),
        JsonValue::String(model.provider_id.to_string()),
    );
    if let Some(adapter_id) = model.adapter_id.as_ref() {
        selection.insert(
            "adapter".to_owned(),
            JsonValue::String(adapter_id.to_string()),
        );
    }
    selection.insert(
        "model".to_owned(),
        JsonValue::String(model.model_id.to_string()),
    );
    insert_optional_selection_value(&mut selection, "thinking_mode", thinking_mode);
    insert_optional_selection_value(&mut selection, "speed_mode", speed_mode);
    insert_optional_selection_value(&mut selection, "verbosity", verbosity);
    JsonValue::Object(selection)
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
