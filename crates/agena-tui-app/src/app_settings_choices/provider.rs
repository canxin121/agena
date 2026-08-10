impl App {
    pub(crate) fn current_provider_default_model_ref(&self) -> Option<ModelRef> {
        let sources = self.backend.config_json_sources().ok()?;
        if let Some(selection) =
            get_json_path(&sources.effective, Some("providers.default_selection"))
                .ok()
                .and_then(|value| {
                    serde_json::from_value::<agena_domain::ModelSelectionConfig>(value).ok()
                })
            && let (Some(provider_id), Some(model_id)) =
                (selection.provider.as_deref(), selection.model.as_deref())
        {
            return Some(match selection.adapter.as_deref() {
                Some(adapter_id) => ModelRef::new_with_adapter(provider_id, adapter_id, model_id),
                None => ModelRef::new(provider_id, model_id),
            });
        }
        let provider_id = get_json_path(&sources.effective, Some("providers.default"))
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))?;
        let provider = self
            .backend
            .list_configured_providers()
            .into_iter()
            .find(|provider| provider.provider_id == provider_id)?;
        let model_id = provider.defaults.model.trim();
        if model_id.is_empty() {
            return None;
        }
        Some(
            provider
                .defaults
                .adapter
                .as_deref()
                .map(str::trim)
                .filter(|adapter_id| !adapter_id.is_empty())
                .map(|adapter_id| {
                    ModelRef::new_with_adapter(provider.provider_id.clone(), adapter_id, model_id)
                })
                .unwrap_or_else(|| ModelRef::new(provider.provider_id, model_id)),
        )
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
            agena_tui::model_chooser::SessionModelChooserPurpose::ProviderDefault => {
                let provider_id = model.provider_id.to_string();
                self.dispatch_backend_operation(
                    move |backend| async move {
                        backend
                            .set_provider_default_selection(provider_id.as_str(), selection)
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
                    move |backend| async move {
                        backend
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
                        "flash-provider-default-updated"
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
        let sources = self.backend.config_json_sources().ok()?;
        let permission = get_json_path(&sources.effective, Some("permission")).ok()?;
        let permission = serde_json::from_value::<PermissionConfig>(permission).ok()?;
        permission.approval_model?.model_ref().ok()
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
