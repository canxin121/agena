impl App {
    pub(crate) fn model_selection_mode_choice_items(
        &self,
        model: &ModelRef,
        step: SessionModelModeStep,
    ) -> UiResult<Vec<ChoiceItem>> {
        let mut items = match step {
            SessionModelModeStep::ThinkingMode => inspector_rows_to_mode_choice_items(
                crate::app_backend::provider_mappings::model_thinking_mode_rows(
                    &self.application,
                    model,
                )
                .map_err(crate::UiFailure::internal)?,
                ui_text::thinking_mode_display_value,
            ),
            SessionModelModeStep::SpeedMode => inspector_rows_to_mode_choice_items(
                crate::app_backend::provider_mappings::model_speed_mode_rows(
                    &self.application,
                    model,
                )
                .map_err(crate::UiFailure::internal)?,
                ui_text::speed_mode_display_value,
            ),
            SessionModelModeStep::Verbosity => {
                crate::app_backend::provider_mappings::model_verbosity_values(
                    &self.application,
                    model,
                )
                .map_err(crate::UiFailure::internal)?
                .into_iter()
                .map(|value| {
                    choice_item(
                        value,
                        runtime_setting_choice_supported_model_detail(&self.i18n),
                    )
                })
                .collect::<Vec<_>>()
            }
        };
        if !items.is_empty() {
            items.insert(
                0,
                choice_item_with_value(
                    ui_text::t(&self.i18n, "value-default"),
                    "",
                    ui_text::t(&self.i18n, "settings-provider-default-mode-inherit-detail"),
                ),
            );
        }
        Ok(items)
    }

    pub(crate) fn open_model_selection_mode_step_or_finish(
        &mut self,
        purpose: agena_tui::model_chooser::SessionModelChooserPurpose,
        model: ModelRef,
        thinking_mode: Option<String>,
        speed_mode: Option<String>,
        verbosity: Option<String>,
        step: SessionModelModeStep,
    ) {
        let items = match self.model_selection_mode_choice_items(&model, step) {
            Ok(items) => items,
            Err(error) => {
                self.flash_warning(error);
                return;
            }
        };
        if items.is_empty() {
            match step {
                SessionModelModeStep::ThinkingMode => self
                    .open_model_selection_mode_step_or_finish(
                        purpose,
                        model,
                        thinking_mode,
                        speed_mode,
                        verbosity,
                        SessionModelModeStep::SpeedMode,
                    ),
                SessionModelModeStep::SpeedMode => self.open_model_selection_mode_step_or_finish(
                    purpose,
                    model,
                    thinking_mode,
                    speed_mode,
                    verbosity,
                    SessionModelModeStep::Verbosity,
                ),
                SessionModelModeStep::Verbosity => {
                    self.finish_model_selection(
                        purpose,
                        model,
                        thinking_mode,
                        speed_mode,
                        verbosity,
                    );
                }
            }
            return;
        }

        let current_value = match step {
            SessionModelModeStep::ThinkingMode => thinking_mode.clone().unwrap_or_default(),
            SessionModelModeStep::SpeedMode => speed_mode.clone().unwrap_or_default(),
            SessionModelModeStep::Verbosity => verbosity.clone().unwrap_or_default(),
        };
        self.open_choice_overlay(self.build_choice_overlay(
            settings_edit_title(
                &self.i18n,
                model_mode_display_label(&self.i18n, step).as_str(),
            ),
            model_mode_display_description(&self.i18n, step),
            Some(current_value),
            items,
            ChoiceOverlayAction::ModelSelectionMode {
                purpose,
                model,
                step,
                thinking_mode,
                speed_mode,
                verbosity,
            },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        ));
    }

    pub(crate) fn session_model_mode_choice_items(
        &self,
        step: SessionModelModeStep,
    ) -> UiResult<Vec<ChoiceItem>> {
        let mut items = match step {
            SessionModelModeStep::ThinkingMode => inspector_rows_to_mode_choice_items(
                crate::app_backend::provider_mappings::runtime_thinking_mode_rows(
                    &self.application,
                    &self.run_options.to_request(),
                )
                .map_err(crate::UiFailure::internal)?,
                ui_text::thinking_mode_display_value,
            ),
            SessionModelModeStep::SpeedMode => inspector_rows_to_mode_choice_items(
                crate::app_backend::provider_mappings::runtime_speed_mode_rows(
                    &self.application,
                    &self.run_options.to_request(),
                )
                .map_err(crate::UiFailure::internal)?,
                ui_text::speed_mode_display_value,
            ),
            SessionModelModeStep::Verbosity => {
                crate::app_backend::provider_mappings::runtime_verbosity_values(
                    &self.application,
                    &self.run_options.to_request(),
                )
                .map_err(crate::UiFailure::internal)?
                .into_iter()
                .map(|value| {
                    choice_item(
                        value,
                        runtime_setting_choice_supported_model_detail(&self.i18n),
                    )
                })
                .collect::<Vec<_>>()
            }
        };
        if items.is_empty() {
            return Ok(Vec::new());
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

    pub(crate) fn open_session_model_mode_overlay(
        &mut self,
        step: SessionModelModeStep,
    ) -> UiResult<bool> {
        let items = self.session_model_mode_choice_items(step)?;
        if items.is_empty() {
            return Ok(false);
        }
        let current_summary = self.run_options.model_mode_summary(&self.i18n, step);
        let current_value = self.run_options.model_mode_input(step);
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
                Some(current_value),
                items,
                ChoiceOverlayAction::SessionModelMode(step),
                false,
                agena_tui::choice::ChoicePresentationStyle::SelectOnly,
            ),
        );
        Ok(true)
    }

    pub(crate) fn open_session_model_thinking_step_or_next(&mut self) {
        match self.open_session_model_mode_overlay(SessionModelModeStep::ThinkingMode) {
            Ok(true) => {}
            Ok(false) => self.open_session_model_speed_step_or_next(),
            Err(error) => self.flash_warning(error),
        }
    }

    pub(crate) fn open_session_model_speed_step_or_next(&mut self) {
        match self.open_session_model_mode_overlay(SessionModelModeStep::SpeedMode) {
            Ok(true) => {}
            Ok(false) => self.open_session_model_verbosity_step_or_finish(),
            Err(error) => self.flash_warning(error),
        }
    }

    pub(crate) fn open_session_model_verbosity_step_or_finish(&mut self) {
        if let Err(error) = self.open_session_model_mode_overlay(SessionModelModeStep::Verbosity) {
            self.flash_warning(error);
        }
    }

    pub(crate) fn advance_session_model_mode_step(&mut self, step: SessionModelModeStep) {
        match step {
            SessionModelModeStep::ThinkingMode => self.open_session_model_speed_step_or_next(),
            SessionModelModeStep::SpeedMode => self.open_session_model_verbosity_step_or_finish(),
            SessionModelModeStep::Verbosity => {}
        }
    }
}
use crate::{
    App, ChoiceItem, ChoiceOverlayAction, ModelRef, SessionModelModeStep, UiResult, choice_item,
    choice_item_with_value, inspector_rows_to_mode_choice_items, model_mode_display_description,
    model_mode_display_label, runtime_setting_choice_supported_model_detail, settings_edit_title,
    ui_text,
};
