impl App {
    pub(crate) fn current_provider_default_model_ref(&self) -> Option<ModelRef> {
        let provider_id = self
            .backend
            .config_json_sources()
            .ok()
            .and_then(|sources| get_json_path(&sources.effective, Some("providers.default")).ok())
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

    pub(crate) fn apply_provider_default_model(&mut self, model: ModelRef) -> bool {
        match self.persist_provider_default_model(&model) {
            Ok(()) => {
                self.flash_success(self.i18n.text_args(
                    "flash-provider-default-updated",
                    &agena_tui::fl_args!(
                        "provider" => model.provider_id.to_string(),
                        "model" => model.model_id.to_string(),
                    ),
                ));
                self.open_provider_default_model_mode_step_or_next(
                    model,
                    SessionModelModeStep::ThinkingMode,
                );
                true
            }
            Err(error) => {
                self.flash_error(error);
                false
            }
        }
    }

    pub(crate) fn persist_provider_default_model(&self, model: &ModelRef) -> UiResult<()> {
        let provider_id = model.provider_id.to_string();
        let sources = self
            .backend
            .config_json_sources()
            .map_err(crate::UiFailure::internal)?;
        let defaults_path = provider_defaults_settings_path(provider_id.as_str());
        let defaults = get_json_path(&sources.file, Some(defaults_path.as_str()))
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let defaults = provider_defaults_with_model(defaults, model);

        self.block_on_async(
            self.backend
                .set_config_setting(defaults_path.as_str(), JsonValue::Object(defaults)),
        )?;
        self.block_on_async(
            self.backend
                .set_config_setting("providers.default", JsonValue::String(provider_id)),
        )?;
        Ok(())
    }

    pub(crate) fn provider_default_model_mode_choice_items(
        &self,
        model: &ModelRef,
        step: SessionModelModeStep,
    ) -> UiResult<Vec<ChoiceItem>> {
        let mut items = match step {
            SessionModelModeStep::ThinkingMode => inspector_rows_to_mode_choice_items(
                self.backend
                    .model_thinking_mode_rows(model)
                    .map_err(crate::UiFailure::internal)?,
                ui_text::thinking_mode_display_value,
            ),
            SessionModelModeStep::SpeedMode => inspector_rows_to_mode_choice_items(
                self.backend
                    .model_speed_mode_rows(model)
                    .map_err(crate::UiFailure::internal)?,
                ui_text::speed_mode_display_value,
            ),
            SessionModelModeStep::Verbosity => self
                .backend
                .model_verbosity_values(model)
                .map_err(crate::UiFailure::internal)?
                .into_iter()
                .map(|value| {
                    choice_item(
                        value,
                        runtime_setting_choice_supported_model_detail(&self.i18n),
                    )
                })
                .collect(),
        };
        if items.is_empty() {
            return Ok(items);
        }
        items.insert(
            0,
            choice_item_with_value(
                ui_text::t(&self.i18n, "value-default"),
                "",
                ui_text::t(&self.i18n, "settings-provider-default-mode-inherit-detail"),
            ),
        );
        Ok(items)
    }

    pub(crate) fn open_provider_default_model_mode_step_or_next(
        &mut self,
        model: ModelRef,
        step: SessionModelModeStep,
    ) {
        let items = match self.provider_default_model_mode_choice_items(&model, step) {
            Ok(items) => items,
            Err(error) => {
                self.flash_warning(error);
                return;
            }
        };
        if items.is_empty() {
            self.advance_provider_default_model_mode_step(model, step);
            return;
        }
        let current_value = self.provider_default_model_mode_value(&model, step);
        let current_summary = current_value
            .as_deref()
            .map(|value| match step {
                SessionModelModeStep::ThinkingMode => ui_text::thinking_mode_display_value(value),
                SessionModelModeStep::SpeedMode => ui_text::speed_mode_display_value(value),
                SessionModelModeStep::Verbosity => value.to_owned(),
            })
            .unwrap_or_else(|| ui_text::t(&self.i18n, "value-default"));
        self.open_choice_overlay(
            self.build_choice_overlay(
                settings_edit_title(
                    &self.i18n,
                    model_mode_display_label(&self.i18n, step).as_str(),
                ),
                [
                    model_mode_display_description(&self.i18n, step),
                    self.i18n.text_args(
                        "overlay-runtime-setting-current-value",
                        &agena_tui::fl_args!("value" => current_summary),
                    ),
                ]
                .join("\n"),
                Some(current_value.unwrap_or_default()),
                items,
                ChoiceOverlayAction::ProviderDefaultModelMode { model, step },
                false,
                agena_tui::choice::ChoicePresentationStyle::SelectOnly,
            ),
        );
    }

    pub(crate) fn advance_provider_default_model_mode_step(
        &mut self,
        model: ModelRef,
        step: SessionModelModeStep,
    ) {
        match step {
            SessionModelModeStep::ThinkingMode => self
                .open_provider_default_model_mode_step_or_next(
                    model,
                    SessionModelModeStep::SpeedMode,
                ),
            SessionModelModeStep::SpeedMode => self.open_provider_default_model_mode_step_or_next(
                model,
                SessionModelModeStep::Verbosity,
            ),
            SessionModelModeStep::Verbosity => {
                self.refresh_current_route_after_local_edit();
            }
        }
    }

    pub(crate) fn persist_provider_default_model_mode(
        &self,
        model: &ModelRef,
        step: SessionModelModeStep,
        value: &str,
    ) -> UiResult<()> {
        let provider_id = model.provider_id.to_string();
        let defaults_path = provider_defaults_settings_path(provider_id.as_str());
        let sources = self
            .backend
            .config_json_sources()
            .map_err(crate::UiFailure::internal)?;
        let mut defaults = get_json_path(&sources.file, Some(defaults_path.as_str()))
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        defaults = provider_defaults_with_mode(defaults, step, value);
        self.block_on_async(
            self.backend
                .set_config_setting(defaults_path.as_str(), JsonValue::Object(defaults)),
        )?;
        Ok(())
    }

    fn provider_default_model_mode_value(
        &self,
        model: &ModelRef,
        step: SessionModelModeStep,
    ) -> Option<String> {
        let provider = self
            .backend
            .list_configured_providers()
            .into_iter()
            .find(|provider| provider.provider_id == model.provider_id.as_ref())?;
        match step {
            SessionModelModeStep::ThinkingMode => provider.defaults.thinking_mode,
            SessionModelModeStep::SpeedMode => provider.defaults.speed_mode,
            SessionModelModeStep::Verbosity => provider.defaults.verbosity,
        }
    }
}

fn provider_defaults_with_model(
    mut defaults: JsonMap<String, JsonValue>,
    model: &ModelRef,
) -> JsonMap<String, JsonValue> {
    if let Some(adapter_id) = model.adapter_id.as_ref().map(ToString::to_string) {
        defaults.insert("adapter".to_string(), JsonValue::String(adapter_id));
    } else {
        defaults.remove("adapter");
    }
    defaults.insert(
        "model".to_string(),
        JsonValue::String(model.model_id.to_string()),
    );
    defaults.remove("thinking_mode");
    defaults.remove("speed_mode");
    defaults.remove("verbosity");
    defaults
}

fn provider_defaults_with_mode(
    mut defaults: JsonMap<String, JsonValue>,
    step: SessionModelModeStep,
    value: &str,
) -> JsonMap<String, JsonValue> {
    let key = match step {
        SessionModelModeStep::ThinkingMode => "thinking_mode",
        SessionModelModeStep::SpeedMode => "speed_mode",
        SessionModelModeStep::Verbosity => "verbosity",
    };
    let value = value.trim();
    if value.is_empty() {
        defaults.remove(key);
    } else {
        defaults.insert(key.to_owned(), JsonValue::String(value.to_owned()));
    }
    defaults
}

#[cfg(test)]
mod tests;

use crate::{
    App, ChoiceItem, ChoiceOverlayAction, JsonMap, JsonValue, ModelRef, SessionModelModeStep,
    UiResult, choice_item, choice_item_with_value, get_json_path,
    inspector_rows_to_mode_choice_items, model_mode_display_description, model_mode_display_label,
    provider_defaults_settings_path, runtime_setting_choice_supported_model_detail,
    settings_edit_title, ui_text,
};
