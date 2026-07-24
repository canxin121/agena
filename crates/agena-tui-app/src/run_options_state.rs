use super::{I18n, ModelRef, RunOptions, RunOptionsState, SessionModelModeStep};
use crate::ui_text;

impl RunOptionsState {
    pub(crate) fn clear_model_stack(&mut self) {
        self.model = None;
        self.thinking_mode = None;
        self.speed_mode = None;
        self.verbosity = None;
        self.parallel_tool_calls = None;
    }

    pub(crate) fn replace_model_stack(
        &mut self,
        model: Option<ModelRef>,
        thinking_mode: Option<String>,
        speed_mode: Option<String>,
        verbosity: Option<String>,
        parallel_tool_calls: Option<bool>,
    ) {
        self.model = model;
        self.thinking_mode = thinking_mode;
        self.speed_mode = speed_mode;
        self.verbosity = verbosity;
        self.parallel_tool_calls = parallel_tool_calls;
    }

    pub(crate) fn model_stack_request(&self) -> RunOptions {
        RunOptions {
            model: self.model.as_ref().map(model_ref_to_wire),
            thinking_mode: self.thinking_mode.clone(),
            speed_mode: self.speed_mode.clone(),
            verbosity: self.verbosity.clone(),
            parallel_tool_calls: self.parallel_tool_calls,
            ..RunOptions::default()
        }
    }

    pub(crate) fn model_mode_summary(&self, i18n: &I18n, step: SessionModelModeStep) -> String {
        match step {
            SessionModelModeStep::ThinkingMode => self
                .thinking_mode
                .as_deref()
                .map(ui_text::thinking_mode_display_value)
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            SessionModelModeStep::SpeedMode => self
                .speed_mode
                .as_deref()
                .map(ui_text::speed_mode_display_value)
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            SessionModelModeStep::Verbosity => self
                .verbosity
                .clone()
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
        }
    }

    pub(crate) fn model_mode_input(&self, step: SessionModelModeStep) -> String {
        match step {
            SessionModelModeStep::ThinkingMode => self.thinking_mode.clone().unwrap_or_default(),
            SessionModelModeStep::SpeedMode => self.speed_mode.clone().unwrap_or_default(),
            SessionModelModeStep::Verbosity => self.verbosity.clone().unwrap_or_default(),
        }
    }

    pub(crate) fn apply_model_mode_input(&mut self, step: SessionModelModeStep, input: &str) {
        let trimmed = input.trim();
        let value = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        match step {
            SessionModelModeStep::ThinkingMode => self.thinking_mode = value,
            SessionModelModeStep::SpeedMode => self.speed_mode = value,
            SessionModelModeStep::Verbosity => {
                self.verbosity = value.map(|value| value.to_ascii_lowercase())
            }
        }
    }

    pub(crate) fn to_request(&self) -> RunOptions {
        RunOptions {
            model: self.model.as_ref().map(model_ref_to_wire),
            thinking_mode: self.thinking_mode.clone(),
            speed_mode: self.speed_mode.clone(),
            verbosity: self.verbosity.clone(),
            parallel_tool_calls: self.parallel_tool_calls,
            agent_profile: None,
            ..RunOptions::default()
        }
    }

    pub(crate) fn summary(&self, i18n: &I18n) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(model) = self.model.as_ref() {
            parts.push(format!("{}/{}", model.provider_id, model.model_id));
        }
        if let Some(thinking_mode) = self.thinking_mode.as_ref() {
            parts.push(i18n.text_args(
                "run-options-summary-thinking",
                &agena_tui::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
            ));
        }
        if let Some(speed_mode) = self.speed_mode.as_ref() {
            parts.push(i18n.text_args(
                "run-options-summary-speed",
                &agena_tui::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
            ));
        }
        if let Some(verbosity) = self.verbosity.as_ref() {
            parts.push(i18n.text_args(
                "run-options-summary-verbosity",
                &agena_tui::fl_args!("value" => verbosity),
            ));
        }
        if let Some(parallel_tool_calls) = self.parallel_tool_calls {
            parts.push(i18n.text_args(
                "run-options-summary-parallel-tools",
                &agena_tui::fl_args!(
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
        (!parts.is_empty()).then(|| parts.join(" | "))
    }
}

fn model_ref_to_wire(model: &agena_domain::ModelRef) -> agena_api::resource::ModelRef {
    match model.adapter_id.as_ref() {
        Some(adapter_id) => agena_api::resource::ModelRef::new_with_adapter(
            model.provider_id.as_ref(),
            adapter_id.as_ref(),
            model.model_id.as_ref(),
        ),
        None => {
            agena_api::resource::ModelRef::new(model.provider_id.as_ref(), model.model_id.as_ref())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ModelRef, RunOptionsState};

    #[test]
    fn replacing_model_stack_discards_the_previous_sessions_selection() {
        let mut state = RunOptionsState {
            model: Some(ModelRef::new("old-provider", "old-model")),
            thinking_mode: Some("high".to_owned()),
            speed_mode: Some("fast".to_owned()),
            verbosity: Some("high".to_owned()),
            parallel_tool_calls: Some(true),
        };
        let next = ModelRef::new_with_adapter("next-provider", "next-adapter", "next-model");

        state.replace_model_stack(Some(next.clone()), None, None, None, None);

        assert_eq!(state.model, Some(next));
        assert_eq!(state.thinking_mode, None);
        assert_eq!(state.speed_mode, None);
        assert_eq!(state.verbosity, None);
        assert_eq!(state.parallel_tool_calls, None);
    }

    #[test]
    fn session_selection_request_excludes_request_only_overrides() {
        let state = RunOptionsState {
            model: Some(ModelRef::new("provider", "model")),
            thinking_mode: Some("low".to_owned()),
            speed_mode: Some("fast".to_owned()),
            verbosity: Some("medium".to_owned()),
            parallel_tool_calls: Some(false),
        };

        let request = state.model_stack_request();

        assert_eq!(
            request.model,
            state.model.as_ref().map(super::model_ref_to_wire)
        );
        assert_eq!(request.thinking_mode.as_deref(), Some("low"));
        assert_eq!(request.speed_mode, state.speed_mode);
        assert_eq!(request.verbosity, state.verbosity);
        assert_eq!(request.parallel_tool_calls, state.parallel_tool_calls);
    }
}
