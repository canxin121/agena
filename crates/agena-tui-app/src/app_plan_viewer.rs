//! Application adapter for the plan viewer route.

use crossterm::event::KeyCode;
use std::time::{Duration, Instant};

use super::{App, AppMessage, KeyEvent, PlanViewerData, PlanViewerState, Route};
use crate::ui_text;
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;

impl App {
    pub(crate) fn open_external_agent_activity(&mut self, session_id: i64) {
        if self.current_or_selected_session_id() != Some(session_id)
            || self.external_agent_commands().is_none()
        {
            return;
        }
        let mut state = PlanViewerState::new(session_id);
        state.external_activity = true;
        self.current_route = Route::PlanViewer(state);
        self.focus = Focus::Transcript;
        self.refresh_external_agent_activity();
        self.poll_external_agent_auth(Instant::now());
    }

    pub(crate) fn refresh_external_agent_activity(&mut self) {
        let Route::PlanViewer(viewer) = &mut self.current_route else {
            return;
        };
        if !viewer.external_activity {
            return;
        }
        viewer.markdown = self
            .transcript
            .execution
            .as_ref()
            .filter(|execution| execution.session.id == viewer.session_id)
            .map(|execution| {
                external_agent_activity(
                    &execution.execution.external_agent_details,
                    viewer.external_auth_status.as_ref(),
                    &self.i18n,
                )
            });
    }

    pub(crate) fn poll_external_agent_auth(&mut self, now: Instant) {
        let Route::PlanViewer(viewer) = &mut self.current_route else {
            return;
        };
        if !viewer.external_activity
            || viewer.request_id != 0
            || viewer
                .external_auth_refresh_at
                .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(2))
        {
            return;
        }
        let session_id = viewer.session_id;
        let request_id = self.next_usage_request_id.saturating_add(1);
        self.next_usage_request_id = request_id;
        viewer.request_id = request_id;
        viewer.external_auth_refresh_at = Some(now);
        self.dispatch_backend_operation(
            move |backend| async move { backend.client().external_agent_live_auth(session_id).await },
            move |app, result| {
                let Route::PlanViewer(viewer) = &mut app.current_route else { return; };
                if !viewer.external_activity || viewer.session_id != session_id || viewer.request_id != request_id {
                    return;
                }
                viewer.request_id = 0;
                let status = result.ok().flatten();
                if viewer.external_auth_status != status {
                    viewer.external_auth_status = status;
                    app.refresh_external_agent_activity();
                }
            },
        );
    }

    /// Open the plan viewer for the current session and fetch the full plan.
    /// Requires an active session; without one the user gets a warning flash.
    pub(crate) fn open_plan_viewer_fullscreen(&mut self) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-plan-viewer-requires-session"));
            return;
        };
        let mut state = PlanViewerState::new(session_id);
        state.summary = self.plan_viewer_summary(session_id);
        self.route_stack.clear();
        self.current_route = Route::PlanViewer(state);
        self.focus = Focus::Transcript;
        self.refresh_plan_viewer();
    }

    /// Reload the full plan markdown and the compact display summary,
    /// tracking the request id on the active plan-viewer state. Returns the
    /// request id so callers that hold a detached route state (during route
    /// key handling) can record it themselves.
    pub(crate) fn refresh_plan_viewer(&mut self) -> Option<u64> {
        let Route::PlanViewer(state) = &self.current_route else {
            return None;
        };
        if state.external_activity || state.loading || state.toggle_request_id != 0 {
            return None;
        }
        self.refresh_plan_viewer_for(state.session_id)
    }

    fn refresh_plan_viewer_for(&mut self, session_id: i64) -> Option<u64> {
        let summary = self.plan_viewer_summary(session_id);
        let request_id = self.next_usage_request_id.saturating_add(1);
        self.next_usage_request_id = request_id;
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .invoke_plugin_tool(
                    "agena.plan",
                    "get",
                    serde_json::json!({ "view": "full" }),
                    Some(session_id),
                )
                .await
                .map(crate::app_plan_document::plan_viewer_data)
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::PlanViewerLoaded { request_id, result })
                .await;
        });
        if let Route::PlanViewer(state) = &mut self.current_route {
            state.request_id = request_id;
            state.loading = true;
            state.error = None;
            state.summary = summary;
        }
        Some(request_id)
    }

    pub(crate) fn handle_plan_viewer_loaded(
        &mut self,
        request_id: u64,
        result: super::UiResult<PlanViewerData>,
    ) {
        let Route::PlanViewer(state) = &mut self.current_route else {
            return;
        };
        if request_id != state.request_id {
            return;
        }
        state.loading = false;
        match result {
            Ok(data) => {
                state.markdown = Some(data.markdown);
                state.autorun = data.autorun;
                state.summary = data.summary;
                state.error = None;
            }
            Err(error) => state.error = Some(error.to_string()),
        }
    }

    pub(crate) fn handle_plan_autorun_toggled(
        &mut self,
        request_id: u64,
        result: super::UiResult<PlanViewerData>,
    ) {
        let ok = result.is_ok();
        {
            let Route::PlanViewer(state) = &mut self.current_route else {
                return;
            };
            if request_id != state.toggle_request_id {
                return;
            }
            state.toggle_request_id = 0;
            match result {
                Ok(data) => {
                    state.markdown = Some(data.markdown);
                    state.autorun = data.autorun;
                    state.summary = data.summary;
                    state.error = None;
                }
                Err(error) => state.error = Some(error.to_string()),
            }
        }
        if ok {
            let session_id = match &self.current_route {
                Route::PlanViewer(state) => state.session_id,
                _ => return,
            };
            self.request_plan_display_refresh(session_id);
        }
    }

    pub(crate) fn handle_plan_viewer_key(
        &mut self,
        key: KeyEvent,
        state: &mut PlanViewerState,
    ) -> bool {
        match resolve_tui_key(KeyContext::PlanViewer, key) {
            Some(KeyAction::Close) => true,
            Some(KeyAction::MoveUp) => {
                state.presentation.scroll_by(-1);
                false
            }
            Some(KeyAction::MoveDown) => {
                state.presentation.scroll_by(1);
                false
            }
            Some(KeyAction::PageUp) | Some(KeyAction::PageDown) => {
                let page = i64::from(self.plan_viewer_page_size());
                let delta = if key.code == KeyCode::PageUp {
                    -page
                } else {
                    page
                };
                state.presentation.scroll_by(delta);
                false
            }
            Some(KeyAction::Refresh) => {
                if state.external_activity {
                    state.external_auth_refresh_at = None;
                    return false;
                }
                if !state.loading
                    && state.toggle_request_id == 0
                    && let Some(request_id) = self.refresh_plan_viewer_for(state.session_id)
                {
                    state.request_id = request_id;
                    state.loading = true;
                    state.error = None;
                }
                false
            }
            Some(KeyAction::PlanToggleAutorun) => {
                if !state.external_activity {
                    self.toggle_plan_autorun(state);
                }
                false
            }
            Some(_) | None => false,
        }
    }

    /// Toggle autorun for the active plan through the application-side
    /// session tool executor, applying the returned plan immediately.
    fn toggle_plan_autorun(&mut self, state: &mut PlanViewerState) {
        if state.loading || state.toggle_request_id != 0 {
            return;
        }
        let session_id = state.session_id;
        let Some(current) = state.autorun else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-plan-viewer-no-plan"));
            return;
        };
        let target = !current;
        let request_id = self.next_usage_request_id.saturating_add(1);
        self.next_usage_request_id = request_id;
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .invoke_plugin_tool(
                    "agena.plan",
                    "phase",
                    serde_json::json!({ "autorun": target }),
                    Some(session_id),
                )
                .await
                .map(crate::app_plan_document::plan_viewer_data)
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::PlanAutorunToggled { request_id, result })
                .await;
        });
        state.toggle_request_id = request_id;
    }

    /// Last canonical plan progress for `session_id` (for example `▶ 2/5 ↻`).
    fn plan_viewer_summary(&self, session_id: i64) -> Option<String> {
        self.session_plan_progress(session_id).map(str::to_owned)
    }

    fn plan_viewer_page_size(&self) -> u16 {
        self.layout.overlay_area.height.saturating_sub(4).max(1)
    }
}

fn external_agent_activity(
    details: &agena_domain::ExternalAgentDetails,
    auth: Option<&agena_domain::ExternalAgentAuthStatus>,
    i18n: &agena_tui::i18n::I18n,
) -> String {
    use std::fmt::Write as _;

    let mut text = String::new();
    if let Some(auth) = auth {
        let state = i18n.text(match auth.state.as_str() {
            "authenticated" => "external-agent-auth-signed-in",
            "unauthenticated" => "external-agent-auth-required",
            "not_required" => "external-agent-auth-not-required",
            _ => "external-agent-auth-unknown",
        });
        let _ = writeln!(
            text,
            "{} · {state}",
            i18n.text("external-agent-auth-connection")
        );
        for account in &auth.accounts {
            let details = [
                Some(account.label.as_str()),
                account.name.as_deref(),
                account.email.as_deref(),
                account.organization.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let _ = writeln!(text, "{details}");
            if let Some(description) = &account.description {
                let _ = writeln!(text, "{description}");
            }
        }
        text.push('\n');
    }
    if !details.suggestions.is_empty() {
        let _ = writeln!(text, "{}\n", i18n.text("external-agent-suggestions"));
        for suggestion in &details.suggestions {
            let _ = writeln!(text, "{}\n", suggestion.display_text().trim_end());
        }
    }
    if let Some(cost) = &details.cost {
        let _ = writeln!(
            text,
            "{}: {} {}\n",
            i18n.text("external-agent-reported-total"),
            cost.amount,
            cost.currency
        );
    }
    for notice in &details.notices {
        if let Some(session_id) = &notice.session_id {
            let agent = details
                .subagents
                .get(session_id)
                .and_then(|child| child.title.as_deref())
                .unwrap_or(session_id);
            let _ = writeln!(text, "{agent} · {} · {}", notice.severity, notice.title);
        } else {
            let _ = writeln!(text, "{} · {}", notice.severity, notice.title);
        }
        if let Some(description) = &notice.description {
            let _ = writeln!(text, "{description}");
        }
        text.push('\n');
    }
    for compaction in &details.compactions {
        let _ = writeln!(
            text,
            "{} · {}",
            i18n.text("external-agent-compaction"),
            compaction.status
        );
        if !compaction.summary.is_empty() {
            let _ = writeln!(text, "{}", compaction.summary);
        }
        if let Some(error) = &compaction.error {
            let _ = writeln!(text, "{error}");
        }
        text.push('\n');
    }
    let append_messages = |text: &mut String, messages: &[agena_domain::ExternalAgentMessage]| {
        for message in messages {
            if message.text.is_empty() {
                continue;
            }
            let label = if let Some(sender) = message.sender_id.as_deref() {
                details
                    .subagents
                    .get(sender)
                    .and_then(|child| child.title.as_deref())
                    .unwrap_or(sender)
                    .to_owned()
            } else if let Some(recipient) = message.recipient_id.as_deref() {
                let name = details
                    .subagents
                    .get(recipient)
                    .and_then(|child| child.title.as_deref())
                    .unwrap_or(recipient);
                i18n.text_args(
                    "external-agent-message-to",
                    &agena_tui::fl_args!("name" => name.to_owned()),
                )
            } else {
                message.role.clone()
            };
            let _ = writeln!(text, "{label}\n{}\n", message.text);
        }
    };
    append_messages(&mut text, &details.messages);
    for terminal in &details.terminals {
        let heading = terminal.command.clone().unwrap_or_else(|| {
            i18n.text_args(
                "external-agent-terminal",
                &agena_tui::fl_args!("id" => terminal.id.clone()),
            )
        });
        let status = match terminal.exit_code {
            Some(code) => i18n.text_args(
                "external-agent-terminal-exit",
                &agena_tui::fl_args!("code" => code),
            ),
            None if terminal.exited => i18n.text("external-agent-terminal-exited"),
            None => i18n.text("external-agent-terminal-running"),
        };
        if let Some(child) = details.subagents.get(&terminal.session_id) {
            let _ = write!(
                text,
                "{} · ",
                child.title.as_deref().unwrap_or(&terminal.session_id)
            );
        }
        let _ = write!(text, "{heading} · {status}");
        if let Some(signal) = &terminal.signal {
            let _ = write!(text, " · {signal}");
        }
        text.push('\n');
        if let Some(cwd) = &terminal.cwd {
            let _ = writeln!(text, "{cwd}");
        }
        if !terminal.output.is_empty() {
            let _ = writeln!(text, "{}", terminal.output);
        }
        text.push('\n');
    }
    for (id, child) in &details.subagents {
        let _ = writeln!(
            text,
            "{} · {}",
            child.title.as_deref().unwrap_or(id),
            child.state.as_deref().unwrap_or("unknown")
        );
        if let Some(description) = &child.description {
            let _ = writeln!(text, "{description}");
        }
        if let Some(error) = child.error.as_ref().or(child.stop_reason.as_ref()) {
            let _ = writeln!(text, "{error}");
        }
        if let Some(cwd) = &child.cwd {
            let _ = writeln!(text, "{cwd}");
        }
        for path in &child.additional_directories {
            let _ = writeln!(text, "{path}");
        }
        text.push('\n');
        append_messages(&mut text, &child.messages);
    }
    if text.is_empty() {
        i18n.text("external-agent-activity-empty")
    } else {
        text
    }
}
