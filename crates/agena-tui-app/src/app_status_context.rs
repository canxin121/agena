use super::permission_resource_override_summary;

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
                self.flash_error("UI backend command queue is full; wait for current operations")
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.flash_error("UI backend command actor is unavailable")
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
            crate::app_backend::provider_mappings::model_display_name(&self.application, model)
                .unwrap_or_else(|| model_name_status_label(model))
        };
        let fallback_model = || {
            self.application
                .resolved_model_for_run_options(&self.run_options.to_request())
                .ok()
                .map(|model| model_label(&model))
        };

        // Model label: prefer the active session's model (run-options
        // override or the persisted execution context), then the execution
        // label, then the resolved default model so the status stays
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
        // used), then run-options overrides, then the resolved model's
        // default modes so the modes are always visible before the first
        // message is sent.
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
            app.flash_error(format!("UI backend command task failed: {error}"));
        }) as crate::UiCompletion,
    };
    let _ = tx
        .send(crate::AppMessage::AsyncOperationCompleted(completion))
        .await;
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

#[cfg(test)]
mod command_actor_tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use tokio::sync::{Semaphore, mpsc};

    use super::run_command_actor;
    use crate::{
        AppMessage, UI_COMMAND_MAX_CONCURRENCY, UI_COMMAND_QUEUE_CAPACITY, UiCommand, UiCompletion,
    };

    fn empty_completion() -> UiCompletion {
        Box::new(|_| {})
    }

    fn pending_command() -> UiCommand {
        Box::pin(std::future::pending())
    }

    #[test]
    fn command_ingress_is_bounded() {
        let (tx, _rx) = mpsc::channel::<UiCommand>(UI_COMMAND_QUEUE_CAPACITY);
        for _ in 0..UI_COMMAND_QUEUE_CAPACITY {
            tx.try_send(pending_command()).expect("queue has capacity");
        }

        assert!(matches!(
            tx.try_send(pending_command()),
            Err(mpsc::error::TrySendError::Full(_))
        ));
    }

    #[tokio::test]
    async fn command_actor_enforces_the_concurrency_limit_and_drains_on_close() {
        let command_count = UI_COMMAND_MAX_CONCURRENCY * 3;
        let (command_tx, command_rx) = mpsc::channel::<UiCommand>(command_count);
        let (message_tx, mut message_rx) = mpsc::channel(command_count);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Semaphore::new(0));

        for _ in 0..command_count {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let started = Arc::clone(&started);
            let gate = Arc::clone(&gate);
            command_tx
                .send(Box::pin(async move {
                    let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now_active, Ordering::SeqCst);
                    started.fetch_add(1, Ordering::SeqCst);
                    let permit = gate.acquire().await.expect("test gate remains open");
                    permit.forget();
                    active.fetch_sub(1, Ordering::SeqCst);
                    empty_completion()
                }))
                .await
                .expect("actor ingress remains open");
        }
        drop(command_tx);

        let actor = tokio::spawn(run_command_actor(command_rx, message_tx));
        tokio::time::timeout(Duration::from_secs(1), async {
            while started.load(Ordering::SeqCst) < UI_COMMAND_MAX_CONCURRENCY {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first command batch starts");

        assert_eq!(started.load(Ordering::SeqCst), UI_COMMAND_MAX_CONCURRENCY);
        assert_eq!(peak.load(Ordering::SeqCst), UI_COMMAND_MAX_CONCURRENCY);

        gate.add_permits(command_count);
        tokio::time::timeout(Duration::from_secs(1), actor)
            .await
            .expect("actor drains accepted commands")
            .expect("actor does not panic");

        let mut completions = 0;
        while let Ok(message) = message_rx.try_recv() {
            if matches!(message, AppMessage::AsyncOperationCompleted(_)) {
                completions += 1;
            }
        }
        assert_eq!(completions, command_count);
        assert_eq!(started.load(Ordering::SeqCst), command_count);
        assert!(peak.load(Ordering::SeqCst) <= UI_COMMAND_MAX_CONCURRENCY);
    }
}
