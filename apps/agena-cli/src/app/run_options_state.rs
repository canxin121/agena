use super::{runtime_setting_display_label, runtime_setting_override_summary};

impl RunOptionsState {
    pub(in crate::app) fn clear_model_stack(&mut self) {
        self.model = None;
        self.thinking_mode = None;
        self.speed_mode = None;
        self.verbosity = None;
        self.parallel_tool_calls = None;
    }

    pub(in crate::app) fn runtime_setting_summary(
        &self,
        i18n: &I18n,
        field: RuntimeSettingSpec,
    ) -> String {
        match field.id {
            RuntimeSettingId::ThinkingMode => self
                .thinking_mode
                .as_deref()
                .map(|value| {
                    runtime_setting_override_summary(
                        i18n,
                        ui_text::thinking_mode_display_value(value).as_str(),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::SpeedMode => self
                .speed_mode
                .as_deref()
                .map(|value| {
                    runtime_setting_override_summary(
                        i18n,
                        ui_text::speed_mode_display_value(value).as_str(),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::Verbosity => self
                .verbosity
                .as_deref()
                .map(|value| runtime_setting_override_summary(i18n, value))
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::ParallelToolCalls => self
                .parallel_tool_calls
                .map(|value| {
                    runtime_setting_override_summary(
                        i18n,
                        ui_text::t(i18n, if value { "value-on" } else { "value-off" }).as_str(),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::Temperature => self
                .temperature
                .map(|value| runtime_setting_override_summary(i18n, format!("{value:.2}").as_str()))
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::MaxOutput => self
                .max_output_tokens
                .map(|value| runtime_setting_override_summary(i18n, value.to_string().as_str()))
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::System => self
                .system
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    runtime_setting_override_summary(
                        i18n,
                        format_setting_value_inline(&JsonValue::String(value.clone())).as_str(),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
        }
    }

    pub(in crate::app) fn runtime_setting_input_text(&self, field: RuntimeSettingSpec) -> String {
        match field.id {
            RuntimeSettingId::ThinkingMode => self.thinking_mode.clone().unwrap_or_default(),
            RuntimeSettingId::SpeedMode => self.speed_mode.clone().unwrap_or_default(),
            RuntimeSettingId::Verbosity => self.verbosity.clone().unwrap_or_default(),
            RuntimeSettingId::ParallelToolCalls => self
                .parallel_tool_calls
                .map(|value| {
                    if value {
                        "on".to_string()
                    } else {
                        "off".to_string()
                    }
                })
                .unwrap_or_default(),
            RuntimeSettingId::Temperature => self
                .temperature
                .map(|value| format!("{value:.2}"))
                .unwrap_or_default(),
            RuntimeSettingId::MaxOutput => self
                .max_output_tokens
                .map(|value| value.to_string())
                .unwrap_or_default(),
            RuntimeSettingId::System => self.system.clone().unwrap_or_default(),
        }
    }

    pub(in crate::app) fn apply_runtime_setting_input(
        &mut self,
        i18n: &I18n,
        field: RuntimeSettingSpec,
        input: &str,
    ) -> std::result::Result<String, String> {
        let trimmed = input.trim();
        let field_label = runtime_setting_display_label(i18n, field);
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("clear") {
            match field.id {
                RuntimeSettingId::ThinkingMode => self.thinking_mode = None,
                RuntimeSettingId::SpeedMode => self.speed_mode = None,
                RuntimeSettingId::Verbosity => self.verbosity = None,
                RuntimeSettingId::ParallelToolCalls => self.parallel_tool_calls = None,
                RuntimeSettingId::Temperature => self.temperature = None,
                RuntimeSettingId::MaxOutput => self.max_output_tokens = None,
                RuntimeSettingId::System => self.system = None,
            }
            return Ok(i18n.text_args(
                "runtime-setting-apply-cleared",
                &crate::fl_args!("field" => field_label),
            ));
        }

        match field.id {
            RuntimeSettingId::ThinkingMode => self.thinking_mode = Some(trimmed.to_string()),
            RuntimeSettingId::SpeedMode => self.speed_mode = Some(trimmed.to_string()),
            RuntimeSettingId::Verbosity => self.verbosity = Some(trimmed.to_ascii_lowercase()),
            RuntimeSettingId::ParallelToolCalls => {
                let value = match trimmed.to_ascii_lowercase().as_str() {
                    "true" | "on" | "yes" | "1" | "enabled" => true,
                    "false" | "off" | "no" | "0" | "disabled" => false,
                    _ => {
                        return Err(i18n.text_args(
                            "runtime-setting-error-bool",
                            &crate::fl_args!("field" => field_label.clone()),
                        ));
                    }
                };
                self.parallel_tool_calls = Some(value);
            }
            RuntimeSettingId::Temperature => {
                let value = trimmed.parse::<f32>().map_err(|_| {
                    i18n.text_args(
                        "runtime-setting-error-number",
                        &crate::fl_args!("field" => field_label.clone()),
                    )
                })?;
                if !value.is_finite() {
                    return Err(i18n.text_args(
                        "runtime-setting-error-finite",
                        &crate::fl_args!("field" => field_label.clone()),
                    ));
                }
                self.temperature = Some(value);
            }
            RuntimeSettingId::MaxOutput => {
                let value = trimmed.parse::<u32>().map_err(|_| {
                    i18n.text_args(
                        "runtime-setting-error-positive-int",
                        &crate::fl_args!("field" => field_label.clone()),
                    )
                })?;
                if value == 0 {
                    return Err(i18n.text_args(
                        "runtime-setting-error-positive-int",
                        &crate::fl_args!("field" => field_label.clone()),
                    ));
                }
                self.max_output_tokens = Some(value);
            }
            RuntimeSettingId::System => self.system = Some(trimmed.to_string()),
        }

        Ok(i18n.text_args(
            "runtime-setting-apply-updated",
            &crate::fl_args!("field" => field_label),
        ))
    }

    pub(in crate::app) fn to_request(&self) -> RunOptions {
        RunOptions {
            model: self.model.clone(),
            thinking_mode: self.thinking_mode.clone(),
            speed_mode: self.speed_mode.clone(),
            verbosity: self.verbosity.clone(),
            parallel_tool_calls: self.parallel_tool_calls,
            agent_profile: None,
            system: self.system.clone(),
            temperature: self.temperature,
            max_output_tokens: self.max_output_tokens,
        }
    }

    pub(in crate::app) fn summary(&self, i18n: &I18n) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(model) = self.model.as_ref() {
            parts.push(format!("{}/{}", model.provider_id, model.model_id));
        }
        if let Some(thinking_mode) = self.thinking_mode.as_ref() {
            parts.push(i18n.text_args(
                "run-options-summary-thinking",
                &crate::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
            ));
        }
        if let Some(speed_mode) = self.speed_mode.as_ref() {
            parts.push(i18n.text_args(
                "run-options-summary-speed",
                &crate::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
            ));
        }
        if let Some(verbosity) = self.verbosity.as_ref() {
            parts.push(i18n.text_args(
                "run-options-summary-verbosity",
                &crate::fl_args!("value" => verbosity),
            ));
        }
        if let Some(parallel_tool_calls) = self.parallel_tool_calls {
            parts.push(i18n.text_args(
                "run-options-summary-parallel-tools",
                &crate::fl_args!(
                    "value" => ui_text::t(
                        i18n,
                        if parallel_tool_calls {
                            "value-on"
                        } else {
                            "value-off"
                        },
                    )
                ),
            ));
        }
        if let Some(temperature) = self.temperature {
            parts.push(i18n.text_args(
                "run-options-summary-temperature",
                &crate::fl_args!("value" => format!("{temperature:.2}")),
            ));
        }
        if let Some(max_output_tokens) = self.max_output_tokens {
            parts.push(i18n.text_args(
                "run-options-summary-max-output",
                &crate::fl_args!("value" => max_output_tokens as i64),
            ));
        }
        if self
            .system
            .as_ref()
            .is_some_and(|system| !system.trim().is_empty())
        {
            parts.push(ui_text::t(i18n, "run-options-summary-system"));
        }
        (!parts.is_empty()).then(|| parts.join(" | "))
    }
}
use crate::app::{
    I18n, JsonValue, RunOptions, RunOptionsState, RuntimeSettingId, RuntimeSettingSpec,
    format_setting_value_inline, ui_text,
};
