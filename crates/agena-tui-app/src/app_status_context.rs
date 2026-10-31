use super::permission_resource_override_summary;

/// One composer/session status row. `kind` names what the row is and `state`
/// carries its semantics, so the rendered text is never the only witness of
/// what a chip reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusRowState {
    Normal,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusRow {
    pub(crate) kind: &'static str,
    pub(crate) state: StatusRowState,
    pub(crate) text: String,
}

impl StatusRow {
    pub(crate) fn normal(kind: &'static str, text: impl Into<String>) -> Self {
        Self {
            kind,
            state: StatusRowState::Normal,
            text: text.into(),
        }
    }

    pub(crate) fn pending(kind: &'static str, text: impl Into<String>) -> Self {
        Self {
            kind,
            state: StatusRowState::Pending,
            text: text.into(),
        }
    }
}

impl App {
    pub(crate) fn current_session_activity_indicator(&self) -> Option<String> {
        match self.current_session_activity() {
            SessionActivity::Idle => None,
            SessionActivity::Running => Some(spinner_frame(current_spinner_millis()).to_string()),
            SessionActivity::AwaitingPermission => {
                Some(ui_text::t(&self.i18n, "session-awaiting-approval"))
            }
            SessionActivity::AwaitingInteraction => {
                Some(ui_text::t(&self.i18n, "session-awaiting-user-input"))
            }
            SessionActivity::NeedsRecovery => {
                Some(ui_text::t(&self.i18n, "session-state-interrupted"))
            }
        }
    }

    pub(crate) fn start_command_actor(&mut self) {
        if self.command_actor.is_some() {
            return;
        }
        let Some(command_rx) = self.command_rx.take() else {
            return;
        };
        let tx = self.tx.clone();
        self.command_actor = Some(tokio::spawn(run_command_actor(command_rx, tx)));
    }

    pub(crate) fn dispatch_backend_operation<T, E, Op, Fut, Complete>(
        &mut self,
        operation: Op,
        complete: Complete,
    ) where
        T: Send + 'static,
        E: Into<anyhow::Error> + Send + 'static,
        Op: FnOnce(crate::TuiBackend) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = std::result::Result<T, E>> + Send + 'static,
        Complete: FnOnce(&mut App, UiResult<T>) + Send + 'static,
    {
        let application = self.application.clone();
        let command: crate::UiCommand = Box::pin(async move {
            let result = operation(application)
                .await
                .map_err(|error| crate::UiFailure::from_backend(error.into()));
            let completion: crate::UiCompletion =
                Box::new(move |app: &mut App| complete(app, result));
            completion
        });
        match self.command_tx.try_send(command) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.flash_error(ui_text::t(&self.i18n, "flash-ui-backend-queue-full"))
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.flash_error(ui_text::t(&self.i18n, "flash-ui-backend-unavailable"))
            }
        }
    }

    pub(crate) fn current_runtime_status_summary(&self) -> String {
        let mut parts = vec![
            self.run_options
                .summary(&self.i18n)
                .unwrap_or_else(|| ui_text::t(&self.i18n, "runtime-status-default")),
        ];
        parts.extend(
            self.current_execution_context_parts(false)
                .into_iter()
                .map(|row| row.text),
        );
        parts.push(self.workspace_context_label());
        parts.push(self.i18n.text_args(
            "runtime-status-keys",
            &agena_tui::fl_args!(
                "queue" => self.keybindings.queue.len() as i64,
                "send" => self.keybindings.submit.len() as i64,
            ),
        ));
        parts.push(self.i18n.text_args(
            "runtime-status-display",
            &agena_tui::fl_args!(
                "value" => ui_text::t(
                    &self.i18n,
                                        if crate::app_backend::plugin_effects::plugin_display_contributions(
                                            &self.application,
                                        )
                                        .is_empty() {
                        "runtime-status-display-default"
                    } else {
                        "runtime-status-display-plugin"
                    },
                )
            ),
        ));
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
            &agena_tui::fl_args!(
                "value" => crate::app_backend::plugin_effects::workspace_name(&self.application)
            ),
        )
    }

    pub(crate) fn current_execution_context_parts(
        &self,
        include_workspace_root: bool,
    ) -> Vec<StatusRow> {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return Vec::new();
        };

        let mut parts = Vec::new();
        parts.push(StatusRow::normal(
            "session-state",
            self.i18n.text_args(
                "status-part-state",
                &agena_tui::fl_args!(
                    "value" => ui_text::session_workflow_state_label(&self.i18n, execution)
                ),
            ),
        ));
        parts.push(StatusRow::normal(
            "session-agent",
            self.i18n.text_args(
                "status-part-agent",
                &agena_tui::fl_args!("value" => execution.execution.agent_id.as_str()),
            ),
        ));
        if let Some(agent_name) = execution.execution.agent_name.as_deref()
            && !agent_name.trim().is_empty()
        {
            parts.push(StatusRow::normal(
                "session-agent-name",
                self.i18n.text_args(
                    "status-part-agent-name",
                    &agena_tui::fl_args!("value" => agent_name),
                ),
            ));
        }
        if let Some(model_label) = execution_model_status_label(&execution.execution) {
            parts.push(StatusRow::normal(
                "session-model",
                self.i18n.text_args(
                    "status-part-model",
                    &agena_tui::fl_args!("value" => model_label),
                ),
            ));
        }
        let execution_model = execution_model_ref(&execution.execution);
        if let Some(thinking_mode) = execution.execution.model_thinking_mode.as_deref()
            && !thinking_mode.trim().is_empty()
        {
            parts.push(StatusRow::normal("session-thinking", self.i18n.text_args(
                "status-part-thinking",
                &agena_tui::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
            )));
        }
        if let Some(speed_mode) = execution.execution.model_speed_mode.as_deref()
            && !speed_mode.trim().is_empty()
        {
            parts.push(StatusRow::normal(
                "session-speed",
                self.i18n.text_args(
                    "status-part-speed",
                    &agena_tui::fl_args!(
                        "value" => speed_mode_display_value_for_model(
                            &self.application,
                            execution_model.as_ref(),
                            speed_mode,
                        )
                    ),
                ),
            ));
        } else if model_has_speed_modes(&self.application, execution_model.as_ref()) {
            parts.push(StatusRow::normal(
                "session-speed",
                self.i18n.text_args(
                    "status-part-speed",
                    &agena_tui::fl_args!("value" => ui_text::t(&self.i18n, "value-default")),
                ),
            ));
        }
        if let Some(verbosity) = execution.execution.model_verbosity.as_deref()
            && !verbosity.trim().is_empty()
        {
            parts.push(StatusRow::normal(
                "session-verbosity",
                self.i18n.text_args(
                    "status-part-verbosity",
                    &agena_tui::fl_args!("value" => verbosity),
                ),
            ));
        }
        if let Some(parallel_tool_calls) = execution.execution.model_parallel_tool_calls {
            parts.push(StatusRow::normal(
                "session-parallel-tools",
                self.i18n.text_args(
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
                ),
            ));
        }
        if let Some(workspace_root) = execution.execution.effective_workspace_root.as_deref()
            && !workspace_root.trim().is_empty()
            && include_workspace_root
        {
            parts.push(StatusRow::normal(
                "session-cwd",
                self.i18n.text_args(
                    "status-part-cwd",
                    &agena_tui::fl_args!("value" => workspace_root),
                ),
            ));
        }
        if !execution.execution.effective_permission.is_empty() {
            parts.push(StatusRow::normal(
                "session-permission",
                self.i18n.text_args(
                    "status-part-permission",
                    &agena_tui::fl_args!(
                        "value" => permission_resource_override_summary(
                            &self.i18n,
                            &execution.execution.effective_permission,
                        )
                    ),
                ),
            ));
        }
        let (permission_count, user_input_count) =
            pending_interactive_counts_for_execution(execution);
        if permission_count > 0 {
            parts.push(StatusRow::pending(
                "session-permissions",
                self.i18n.text_args(
                    "status-part-permissions",
                    &agena_tui::fl_args!("count" => permission_count as i64),
                ),
            ));
        }
        if user_input_count > 0 {
            parts.push(StatusRow::pending(
                "session-user-input",
                self.i18n.text_args(
                    "status-part-user-input",
                    &agena_tui::fl_args!("count" => user_input_count as i64),
                ),
            ));
        }
        parts
    }

    pub(crate) fn current_session_status_parts(&self) -> Vec<StatusRow> {
        let model_label = |model: &crate::ModelRef| {
            crate::app_backend::provider_mappings::model_display_name(&self.application, model)
                .unwrap_or_else(|| model_name_status_label(model))
        };
        let resolved_run_model = match self
            .application
            .resolved_model_for_run_options(&self.run_options.to_request())
        {
            Ok(model) => Some(model),
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "resolve the selected model for the TUI session status",
                        error.as_ref(),
                    ),
                    "TUI session status omitted an unresolved model"
                );
                None
            }
        };
        let fallback_model = || resolved_run_model.as_ref().map(model_label);

        // Model label: prefer the active session's model (run-options
        // override or the persisted execution context), then the execution
        // label, then the explicitly selected model so the status stays
        // populated even before a session exists.
        let model_part =
            self.current_session_model_ref()
                .map(|model| model_label(&model))
                .or_else(|| {
                    self.transcript.execution.as_ref().and_then(|execution| {
                        execution_model_name_status_label(&execution.execution)
                    })
                })
                .or_else(fallback_model);

        // Think/speed: prefer the execution context (the modes a run actually
        // used), then run-options overrides, then the selected model's native
        // mode metadata so the modes are visible before the first message is
        // sent. Speed has a meaningful empty value: when the selected model
        // exposes speed modes but none is explicitly selected, the
        // provider/model native default is used and shown as `default`.
        let status_model = self
            .current_session_model_ref()
            .or_else(|| resolved_run_model.clone());
        let (default_thinking, default_speed) = self
            .application
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
            agena_tui::session_status::session_summary_status_parts(model_part, None, None)
                .into_iter()
                .map(|text| StatusRow::normal("session-summary", text))
                .collect::<Vec<_>>();
        if let Some(thinking_mode) = thinking_mode {
            parts.push(StatusRow::normal("session-thinking", self.i18n.text_args(
                "session-status-thinking",
                &agena_tui::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
            )));
        }
        if let Some(speed_mode) = speed_mode {
            parts.push(StatusRow::normal(
                "session-speed",
                self.i18n.text_args(
                    "session-status-speed",
                    &agena_tui::fl_args!(
                        "value" => speed_mode_display_value_for_model(
                            &self.application,
                            status_model.as_ref(),
                            speed_mode,
                        )
                    ),
                ),
            ));
        } else if model_has_speed_modes(&self.application, status_model.as_ref()) {
            parts.push(StatusRow::normal(
                "session-speed",
                self.i18n.text_args(
                    "session-status-speed",
                    &agena_tui::fl_args!("value" => ui_text::t(&self.i18n, "value-default")),
                ),
            ));
        }
        if let Some(execution) = self.transcript.execution.as_ref() {
            let token_usage = agena_tui::session_status::token_usage_status(
                execution.usage.current_tokens,
                execution.usage.input_limit_tokens,
            );
            parts.push(StatusRow::normal(
                "session-token-usage",
                token_usage.label(),
            ));
        }
        parts
    }
}

async fn run_command_actor(
    mut command_rx: tokio::sync::mpsc::Receiver<crate::UiCommand>,
    tx: tokio::sync::mpsc::Sender<crate::AppMessage>,
) {
    let mut active = tokio::task::JoinSet::new();
    let mut ingress_open = true;
    while ingress_open || !active.is_empty() {
        if active.len() >= crate::UI_COMMAND_MAX_CONCURRENCY || !ingress_open {
            if let Some(result) = active.join_next().await {
                send_command_completion(&tx, result).await;
            }
            continue;
        }
        tokio::select! {
            biased;
            result = active.join_next(), if !active.is_empty() => {
                if let Some(result) = result {
                    send_command_completion(&tx, result).await;
                }
            }
            command = command_rx.recv() => match command {
                Some(command) => {
                    active.spawn(command);
                }
                None => ingress_open = false,
            }
        }
    }
}

async fn send_command_completion(
    tx: &tokio::sync::mpsc::Sender<crate::AppMessage>,
    result: Result<crate::UiCompletion, tokio::task::JoinError>,
) {
    let completion = match result {
        Ok(completion) => completion,
        Err(error) => Box::new(move |app: &mut App| {
            app.flash_error(app.i18n.text_args(
                "flash-ui-backend-task-failed",
                &agena_tui::fl_args!("error" => error.to_string()),
            ));
        }) as crate::UiCompletion,
    };
    let _ = tx
        .send(crate::AppMessage::AsyncOperationCompleted(completion))
        .await;
}

/// Picks the first non-empty status mode value from the execution context,
/// run-options overrides, and the selected model's native mode metadata. The
/// composer status shows the modes a run actually used once a message has
/// been sent, but keeps the pending/effective modes visible before the first
/// message.
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

fn execution_model_ref(
    execution: &agena_api::resource::SessionExecutionContextResource,
) -> Option<crate::ModelRef> {
    let provider_id = execution
        .model_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let model_id = execution
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(
        execution
            .model_adapter_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|adapter_id| crate::ModelRef::new_with_adapter(provider_id, adapter_id, model_id))
            .unwrap_or_else(|| crate::ModelRef::new(provider_id, model_id)),
    )
}

fn model_has_speed_modes(application: &crate::TuiBackend, model: Option<&crate::ModelRef>) -> bool {
    model
        .and_then(|model| application.configured_model(model))
        .is_some_and(|model| !model.speed_modes.is_empty())
}

fn speed_mode_display_value_for_model(
    application: &crate::TuiBackend,
    model: Option<&crate::ModelRef>,
    value: &str,
) -> String {
    let display_name = model
        .and_then(|model| application.configured_model(model))
        .and_then(|model| {
            let key = value.trim();
            model
                .speed_modes
                .get(key)
                .or_else(|| {
                    key.strip_prefix("speed-")
                        .and_then(|key| model.speed_modes.get(key))
                })
                .and_then(|mode| mode.display_name.as_deref())
                .map(str::to_owned)
        });
    ui_text::speed_mode_display_value_with_name(value, display_name.as_deref())
}
use crate::{
    App, SessionActivity, UiResult, current_spinner_millis, execution_model_name_status_label,
    execution_model_status_label, model_name_status_label,
    pending_interactive_counts_for_execution, spinner_frame, ui_text,
};
