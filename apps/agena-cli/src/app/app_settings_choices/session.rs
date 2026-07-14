impl App {
    pub(in crate::app) fn session_model_variant_choice_items(
        &self,
        step: SessionModelVariantStep,
    ) -> UiResult<Vec<ChoiceItem>> {
        let mut items = match step {
            SessionModelVariantStep::ThinkingMode => inspector_rows_to_mode_choice_items(
                self.backend
                    .runtime_thinking_mode_rows(&self.run_options.to_request())
                    .map_err(|error| error.to_string())?,
                ui_text::thinking_mode_display_value,
            ),
            SessionModelVariantStep::SpeedMode => inspector_rows_to_mode_choice_items(
                self.backend
                    .runtime_speed_mode_rows(&self.run_options.to_request())
                    .map_err(|error| error.to_string())?,
                ui_text::speed_mode_display_value,
            ),
            SessionModelVariantStep::Verbosity => self
                .backend
                .runtime_verbosity_values(&self.run_options.to_request())
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(|value| {
                    choice_item(
                        value,
                        runtime_setting_choice_supported_model_detail(&self.i18n),
                    )
                })
                .collect::<Vec<_>>(),
        };
        if items.len() <= 1 {
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

    pub(in crate::app) fn open_session_model_variant_overlay(
        &mut self,
        step: SessionModelVariantStep,
    ) -> UiResult<bool> {
        let items = self.session_model_variant_choice_items(step)?;
        if items.is_empty() {
            return Ok(false);
        }
        let current_summary = self.run_options.model_variant_summary(&self.i18n, step);
        let current_value = self.run_options.model_variant_input(step);
        self.open_choice_overlay(
            self.build_choice_overlay(
                settings_edit_title(
                    &self.i18n,
                    model_variant_display_label(&self.i18n, step).as_str(),
                ),
                [
                    model_variant_display_description(&self.i18n, step),
                    self.i18n.text_args(
                        "overlay-runtime-setting-current-value",
                        &crate::fl_args!("value" => current_summary),
                    ),
                ]
                .join("\n"),
                Some(current_value),
                items,
                ChoiceOverlayAction::SessionModelVariant(step),
                false,
                ChoiceOverlayStyle::SelectOnly,
            ),
        );
        Ok(true)
    }

    pub(in crate::app) fn open_session_model_thinking_step_or_next(&mut self) {
        match self.open_session_model_variant_overlay(SessionModelVariantStep::ThinkingMode) {
            Ok(true) => {}
            Ok(false) => self.open_session_model_speed_step_or_next(),
            Err(error) => self.flash_warning(error),
        }
    }

    pub(in crate::app) fn open_session_model_speed_step_or_next(&mut self) {
        match self.open_session_model_variant_overlay(SessionModelVariantStep::SpeedMode) {
            Ok(true) => {}
            Ok(false) => self.open_session_model_verbosity_step_or_finish(),
            Err(error) => self.flash_warning(error),
        }
    }

    pub(in crate::app) fn open_session_model_verbosity_step_or_finish(&mut self) {
        if let Err(error) =
            self.open_session_model_variant_overlay(SessionModelVariantStep::Verbosity)
        {
            self.flash_warning(error);
        }
    }

    pub(in crate::app) fn advance_session_model_variant_step(
        &mut self,
        step: SessionModelVariantStep,
    ) {
        match step {
            SessionModelVariantStep::ThinkingMode => self.open_session_model_speed_step_or_next(),
            SessionModelVariantStep::SpeedMode => {
                self.open_session_model_verbosity_step_or_finish()
            }
            SessionModelVariantStep::Verbosity => {}
        }
    }
}
use crate::app::{
    App, ChoiceItem, ChoiceOverlayAction, ChoiceOverlayStyle, SessionModelVariantStep, UiResult,
    choice_item, choice_item_with_value, inspector_rows_to_mode_choice_items,
    model_variant_display_description, model_variant_display_label,
    runtime_setting_choice_supported_model_detail, settings_edit_title, ui_text,
};
