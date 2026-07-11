impl App {
    pub(in crate::app) fn session_model_variant_choice_items(
        &self,
        field: RuntimeSettingSpec,
    ) -> UiResult<Vec<ChoiceItem>> {
        let mut items = match field.id {
            RuntimeSettingId::ThinkingMode => inspector_rows_to_mode_choice_items(
                self.backend
                    .runtime_thinking_mode_rows(&self.run_options.to_request())
                    .map_err(|error| error.to_string())?,
                ui_text::thinking_mode_display_value,
            ),
            RuntimeSettingId::SpeedMode => inspector_rows_to_mode_choice_items(
                self.backend
                    .runtime_speed_mode_rows(&self.run_options.to_request())
                    .map_err(|error| error.to_string())?,
                ui_text::speed_mode_display_value,
            ),
            RuntimeSettingId::Verbosity => self
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
            RuntimeSettingId::ParallelToolCalls
            | RuntimeSettingId::Temperature
            | RuntimeSettingId::MaxOutput
            | RuntimeSettingId::System => Vec::new(),
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
        let field = session_model_variant_field(step);
        let items = self.session_model_variant_choice_items(field)?;
        if items.is_empty() {
            return Ok(false);
        }
        let current_summary = self.run_options.runtime_setting_summary(&self.i18n, field);
        self.open_choice_overlay(
            self.build_choice_overlay(
                settings_edit_title(
                    &self.i18n,
                    runtime_setting_display_label(&self.i18n, field).as_str(),
                ),
                [
                    runtime_setting_display_description(&self.i18n, field),
                    self.i18n.text_args(
                        "overlay-runtime-setting-current-value",
                        &crate::fl_args!("value" => current_summary),
                    ),
                ]
                .join("\n"),
                Editor::default(),
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

    pub(in crate::app) fn open_runtime_setting_editor(
        &mut self,
        field: RuntimeSettingSpec,
        _return_query: &str,
    ) {
        let current_summary = self.run_options.runtime_setting_summary(&self.i18n, field);
        if let Some(all_items) = self.runtime_setting_choice_items(field) {
            self.open_choice_overlay(self.build_choice_overlay(
                settings_edit_title(
                    &self.i18n,
                    runtime_setting_display_label(&self.i18n, field).as_str(),
                ),
                runtime_setting_edit_prompt(&self.i18n, field, current_summary.as_str()),
                Editor::from_text(self.run_options.runtime_setting_input_text(field)),
                all_items,
                ChoiceOverlayAction::RuntimeSetting(field),
                true,
                Self::runtime_setting_choice_overlay_style(field),
            ));
            return;
        }
        self.overlay = Some(Overlay::RuntimeSettingEdit(RuntimeSettingEditOverlay::new(
            settings_edit_title(
                &self.i18n,
                runtime_setting_display_label(&self.i18n, field).as_str(),
            ),
            runtime_setting_edit_prompt(&self.i18n, field, current_summary.as_str()),
            Editor::from_text(self.run_options.runtime_setting_input_text(field)),
            field,
        )));
    }
}
use crate::app::{
    App, ChoiceItem, ChoiceOverlayAction, ChoiceOverlayStyle, Editor, Overlay,
    RuntimeSettingEditOverlay, RuntimeSettingId, RuntimeSettingSpec, SessionModelVariantStep,
    UiResult, choice_item, choice_item_with_value, inspector_rows_to_mode_choice_items,
    runtime_setting_choice_supported_model_detail, runtime_setting_display_description,
    runtime_setting_display_label, runtime_setting_edit_prompt, session_model_variant_field,
    settings_edit_title, ui_text,
};
