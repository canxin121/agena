use super::permission_resource_override_summary;

impl App {
    pub(crate) fn current_session_activity_indicator(&self) -> Option<String> {
        match self.current_session_activity() {
            SessionActivity::Idle => None,
            SessionActivity::Running => Some(spinner_frame(current_spinner_millis()).to_string()),
            SessionActivity::AwaitingPermission => {
                Some(ui_text::t(&self.i18n, "session-awaiting-approval"))
            }
            SessionActivity::AwaitingUserInput => {
                Some(ui_text::t(&self.i18n, "session-awaiting-user-input"))
            }
            SessionActivity::Blocked => Some(ui_text::t(&self.i18n, "session-blocked")),
        }
    }

    pub(crate) fn block_on_async<F, T, E>(&self, fut: F) -> UiResult<T>
    where
        F: std::future::Future<Output = std::result::Result<T, E>>,
        E: Into<anyhow::Error>,
    {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => match handle.runtime_flavor() {
                tokio::runtime::RuntimeFlavor::MultiThread => {
                    tokio::task::block_in_place(|| handle.block_on(fut))
                        .map_err(|error| crate::UiFailure::from_backend(error.into()))
                }
                _ => Err(crate::UiFailure::internal(
                    "cannot synchronously wait for async work inside the current-thread runtime",
                )),
            },
            Err(_) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(crate::UiFailure::internal)?;
                runtime
                    .block_on(fut)
                    .map_err(|error| crate::UiFailure::from_backend(error.into()))
            }
        }
    }

    pub(crate) fn current_runtime_status_summary(&self) -> String {
        let mut parts = vec![
            self.run_options
                .summary(&self.i18n)
                .unwrap_or_else(|| ui_text::t(&self.i18n, "runtime-status-default")),
        ];
        parts.extend(self.current_execution_context_parts(false));
        parts.push(self.workspace_context_label());
        parts.push(self.i18n.text_args(
            "runtime-status-keys",
            &agena_tui::fl_args!(
                "queue" => self.keybindings.queue.len() as i64,
                "send" => self.keybindings.submit.len() as i64,
            ),
        ));
        parts.push(self.i18n.text_args(
            "runtime-status-statusline",
            &agena_tui::fl_args!(
                "value" => ui_text::t(
                    &self.i18n,
                    if self.backend.plugin_statusline_segments().is_empty() {
                        "runtime-status-statusline-default"
                    } else {
                        "runtime-status-statusline-plugin"
                    },
                )
            ),
        ));
        let tui_blocks = self.backend.plugin_tui_content_blocks().len();
        if tui_blocks > 0 {
            parts.push(self.i18n.text_args(
                "runtime-status-tui-blocks",
                &agena_tui::fl_args!("count" => tui_blocks as i64),
            ));
        }
        if let Some(theme) = self.plugin_theme.as_ref() {
            parts.push(self.i18n.text_args(
                "runtime-status-theme",
                &agena_tui::fl_args!("value" => theme.id.clone()),
            ));
        }
        self.i18n.text_args(
            "flash-runtime-status",
            &agena_tui::fl_args!("summary" => parts.join(" | ")),
        )
    }

    pub(crate) fn current_session_view_summary(&self) -> String {
        self.sessions
            .view_mode()
            .label(&self.i18n, self.sessions.subtree_root_id())
    }

    pub(crate) fn workspace_context_label(&self) -> String {
        self.i18n.text_args(
            "status-part-workspace",
            &agena_tui::fl_args!("value" => self.backend.workspace_name()),
        )
    }

    pub(crate) fn current_execution_context_parts(
        &self,
        include_workspace_root: bool,
    ) -> Vec<String> {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return Vec::new();
        };

        let mut parts = Vec::new();
        parts.push(self.i18n.text_args(
            "status-part-state",
            &agena_tui::fl_args!(
                "value" => ui_text::session_workflow_state_label(&self.i18n, execution)
            ),
        ));
        parts.push(self.i18n.text_args(
            "status-part-agent",
            &agena_tui::fl_args!("value" => execution.execution.agent_id.as_str()),
        ));
        if let Some(task_id) = execution.execution.task_id.as_deref()
            && !task_id.trim().is_empty()
        {
            parts.push(
                self.i18n
                    .text_args("status-part-task", &agena_tui::fl_args!("value" => task_id)),
            );
        }
        if let Some(model_label) = execution_model_status_label(&execution.execution) {
            parts.push(self.i18n.text_args(
                "status-part-model",
                &agena_tui::fl_args!("value" => model_label),
            ));
        }
        if let Some(thinking_mode) = execution.execution.model_thinking_mode.as_deref()
            && !thinking_mode.trim().is_empty()
        {
            parts.push(self.i18n.text_args(
                "status-part-thinking",
                &agena_tui::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
            ));
        }
        if let Some(speed_mode) = execution.execution.model_speed_mode.as_deref()
            && !speed_mode.trim().is_empty()
        {
            parts.push(self.i18n.text_args(
                "status-part-speed",
                &agena_tui::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
            ));
        }
        if let Some(verbosity) = execution.execution.model_verbosity.as_deref()
            && !verbosity.trim().is_empty()
        {
            parts.push(self.i18n.text_args(
                "status-part-verbosity",
                &agena_tui::fl_args!("value" => verbosity),
            ));
        }
        if let Some(parallel_tool_calls) = execution.execution.model_parallel_tool_calls {
            parts.push(self.i18n.text_args(
                "status-part-parallel-tools",
                &agena_tui::fl_args!(
                    "value" => ui_text::t(
                        &self.i18n,
                        if parallel_tool_calls {
                            "value-on"
                        } else {
                            "value-off"
                        },
                    )
                ),
            ));
        }
        if let Some(workspace_root) = execution.execution.effective_workspace_root.as_deref()
            && !workspace_root.trim().is_empty()
            && include_workspace_root
        {
            parts.push(self.i18n.text_args(
                "status-part-cwd",
                &agena_tui::fl_args!("value" => workspace_root),
            ));
        }
        if !execution.execution.effective_permission.is_empty() {
            parts.push(self.i18n.text_args(
                "status-part-permission",
                &agena_tui::fl_args!(
                    "value" => permission_resource_override_summary(
                        &self.i18n,
                        &execution.execution.effective_permission,
                    )
                ),
            ));
        }
        let (permission_count, user_input_count) =
            pending_interactive_counts_for_execution(execution);
        if permission_count > 0 {
            parts.push(self.i18n.text_args(
                "status-part-permissions",
                &agena_tui::fl_args!("count" => permission_count as i64),
            ));
        }
        if user_input_count > 0 {
            parts.push(self.i18n.text_args(
                "status-part-user-input",
                &agena_tui::fl_args!("count" => user_input_count as i64),
            ));
        }
        parts
    }

    pub(crate) fn current_session_status_parts(&self) -> Vec<String> {
        let model_label = |model: &crate::ModelRef| {
            self.backend
                .model_display_name(model)
                .unwrap_or_else(|| model_name_status_label(model))
        };
        let fallback_model = || {
            self.backend
                .resolved_model_for_run_options(&self.run_options.to_request())
                .ok()
                .map(|model| model_label(&model))
        };

        // Model label: prefer the active session's model (run-options
        // override or the persisted execution context), then the execution
        // label, then the resolved default model so the status stays
        // populated even before a session exists.
        let model_part = self
            .current_session_model_ref()
            .map(|model| model_label(&model))
            .or_else(|| {
                self.transcript
                    .execution
                    .as_ref()
                    .and_then(|execution| execution_model_name_status_label(&execution.execution))
            })
            .or_else(fallback_model);

        // Think/speed: prefer the execution context (the modes a run actually
        // used), then run-options overrides, then the resolved model's
        // default modes so the modes are always visible before the first
        // message is sent.
        let (default_thinking, default_speed) = self
            .backend
            .resolved_model_default_modes(&self.run_options.to_request());
        let thinking_mode = status_mode_value(
            self.transcript
                .execution
                .as_ref()
                .and_then(|execution| execution.execution.model_thinking_mode.as_deref()),
            self.run_options.thinking_mode.as_deref(),
            default_thinking.as_deref(),
        );
        let speed_mode = status_mode_value(
            self.transcript
                .execution
                .as_ref()
                .and_then(|execution| execution.execution.model_speed_mode.as_deref()),
            self.run_options.speed_mode.as_deref(),
            default_speed.as_deref(),
        );

        // Order: model, think, speed, then the context percentage at the
        // far right — think/speed stay directly adjacent to the model name
        // instead of having the usage percentage between them.
        let mut parts =
            agena_tui::session_status::session_summary_status_parts(model_part, None, None);
        if let Some(thinking_mode) = thinking_mode {
            parts.push(self.i18n.text_args(
                "session-status-thinking",
                &agena_tui::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
            ));
        }
        if let Some(speed_mode) = speed_mode {
            parts.push(self.i18n.text_args(
                "session-status-speed",
                &agena_tui::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
            ));
        }
        if let Some(execution) = self.transcript.execution.as_ref() {
            let token_usage = agena_tui::session_status::token_usage_status(
                execution.usage.current_tokens,
                execution.usage.projected_tokens,
                execution.usage.model_context_window_tokens,
            );
            parts.push(token_usage.label());
        }
        parts
    }
}

/// Picks the first non-empty status mode value from the execution context,
/// run-options overrides, and the resolved model defaults. The composer
/// status shows the modes a run actually used once a message has been sent,
/// but keeps the pending/effective modes visible before the first message.
fn status_mode_value<'a>(
    execution_mode: Option<&'a str>,
    run_options_mode: Option<&'a str>,
    default_mode: Option<&'a str>,
) -> Option<&'a str> {
    execution_mode
        .filter(|value| !value.trim().is_empty())
        .or_else(|| run_options_mode.filter(|value| !value.trim().is_empty()))
        .or_else(|| default_mode.filter(|value| !value.trim().is_empty()))
}
use crate::{
    App, SessionActivity, UiResult, current_spinner_millis, execution_model_name_status_label,
    execution_model_status_label, model_name_status_label,
    pending_interactive_counts_for_execution, spinner_frame, ui_text,
};

#[cfg(test)]
mod status_mode_value_tests {
    use super::status_mode_value;

    #[test]
    fn prefers_the_execution_context_mode() {
        assert_eq!(
            status_mode_value(Some("high"), Some("low"), Some("medium")),
            Some("high"),
        );
    }

    #[test]
    fn falls_back_to_run_options_when_execution_mode_is_empty() {
        assert_eq!(
            status_mode_value(Some("   "), Some("low"), Some("medium")),
            Some("low"),
        );
        assert_eq!(
            status_mode_value(None, Some("fast"), Some("balanced")),
            Some("fast"),
        );
    }

    #[test]
    fn falls_back_to_defaults_before_any_message_is_sent() {
        assert_eq!(
            status_mode_value(None, None, Some("medium")),
            Some("medium"),
        );
    }

    #[test]
    fn returns_none_when_no_mode_is_available() {
        assert_eq!(status_mode_value(None, None, None), None);
        assert_eq!(status_mode_value(Some(""), Some("  "), Some("")), None);
    }
}
