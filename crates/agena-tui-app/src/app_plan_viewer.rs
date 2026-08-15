//! Application adapter for the plan viewer route.

use std::time::{Duration, Instant};

use crossterm::event::KeyCode;

use super::{
    App, AppMessage, KeyEvent, PlanDisplayRefreshState, PlanViewerData, PlanViewerState, Route,
};
use crate::ui_text;
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;

impl App {
    /// Open the plan viewer for the current session and fetch the full plan.
    /// Requires an active session; without one the user gets a warning flash.
    pub(crate) fn open_plan_viewer(&mut self) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-plan-viewer-requires-session"));
            return;
        };
        let mut state = PlanViewerState::new();
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
        let Some(session_id) = self.current_or_selected_session_id() else {
            if let Route::PlanViewer(state) = &mut self.current_route {
                state.loading = false;
                state.error = Some(ui_text::t(&self.i18n, "flash-plan-viewer-requires-session"));
            }
            return None;
        };
        let summary = self.plan_viewer_summary(session_id);
        let request_id = self.next_usage_request_id.saturating_add(1);
        self.next_usage_request_id = request_id;
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .invoke_plugin_ui_tool(
                    "agena.plan",
                    "get",
                    serde_json::json!({ "view": "full" }),
                    Some(session_id),
                )
                .await
                .map(|response| PlanViewerData {
                    markdown: response.output_text,
                    autorun: response
                        .payload
                        .as_ref()
                        .and_then(|payload| payload.get("plan"))
                        .and_then(|plan| plan.get("autorun"))
                        .and_then(serde_json::Value::as_bool),
                })
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
                state.error = None;
            }
            Err(error) => state.error = Some(error.to_string()),
        }
    }

    pub(crate) fn handle_plan_autorun_toggled(
        &mut self,
        request_id: u64,
        result: super::UiResult<bool>,
    ) {
        let ok = result.is_ok();
        {
            let Route::PlanViewer(state) = &mut self.current_route else {
                return;
            };
            if request_id != state.toggle_request_id {
                return;
            }
            match result {
                Ok(_) => {}
                Err(error) => state.error = Some(error.to_string()),
            }
        }
        if ok {
            // Re-fetch so the title badge and the display summary reflect
            // the new autorun value.
            self.refresh_plan_viewer();
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
                if let Some(request_id) = self.refresh_plan_viewer() {
                    state.request_id = request_id;
                    state.loading = true;
                    state.error = None;
                }
                false
            }
            Some(KeyAction::PlanToggleAutorun) => {
                self.toggle_plan_autorun(state);
                false
            }
            Some(_) | None => false,
        }
    }

    /// Toggle autorun for the active plan through the application-side
    /// session tool executor (no permission prompt), then refresh the view.
    fn toggle_plan_autorun(&mut self, state: &mut PlanViewerState) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-plan-viewer-requires-session"));
            return;
        };
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
                .invoke_plugin_ui_tool(
                    "agena.plan",
                    "phase",
                    serde_json::json!({ "autorun": target }),
                    Some(session_id),
                )
                .await
                .map(|_| true)
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::PlanAutorunToggled { request_id, result })
                .await;
        });
        state.toggle_request_id = request_id;
    }

    /// Compact display text contributed by the planning plugin for
    /// `session_id` (for example `▶ 2/5 ↻`).
    fn plan_viewer_summary(&self, session_id: i64) -> Option<String> {
        let expected_id = format!("plan:{session_id}");
        crate::app_backend::plugin_effects::plugin_display_contributions(&self.application)
            .into_iter()
            .find(|contribution| {
                contribution.contribution.id == expected_id
                    && matches!(
                        (
                            contribution.contribution.kind,
                            &contribution.contribution.content,
                        ),
                        (
                            agena_plugin_host::ContributionKind::StatusLineText,
                            agena_plugin_host::PluginDisplayContent::Text { .. },
                        )
                    )
            })
            .and_then(|contribution| match contribution.contribution.content {
                agena_plugin_host::PluginDisplayContent::Text { text } => Some(text),
                _ => None,
            })
            .map(|text| text.trim().to_string())
            .filter(|content| !content.is_empty())
    }

    fn plan_viewer_page_size(&self) -> u16 {
        self.layout.overlay_area.height.saturating_sub(2).max(1)
    }

    /// Periodically re-request the plan display contribution for the attached
    /// session while the composer's bottom-right chip is missing.
    ///
    /// The chip is backed by an in-memory display contribution that starts
    /// empty after a process restart or runtime reload, so an existing plan
    /// can silently lose its progress indicator until the next plan mutation
    /// or agent stop. A read-only `agena.plan.get` re-publishes it from
    /// durable storage, healing the chip without waiting for the next run.
    pub(crate) fn heal_plan_display_refresh(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.plan_display_refresh = None;
            return;
        };
        if self.composer_plan_progress_part().is_some() {
            // The chip is already visible; stop polling for it.
            self.plan_display_refresh = None;
            return;
        }
        if plan_display_refresh_due(session_id, self.plan_display_refresh, Instant::now()) {
            self.request_plan_display_refresh(session_id);
        }
    }

    /// Fire a read-only plan display refresh for `session_id` and record the
    /// cooldown state so the periodic heal does not re-request every tick.
    pub(crate) fn request_plan_display_refresh(&mut self, session_id: i64) {
        self.plan_display_refresh = Some(PlanDisplayRefreshState {
            session_id,
            requested_at: Instant::now(),
            result: None,
        });
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result =
                crate::app_backend::plugin_effects::refresh_plan_display(&application, session_id)
                    .await;
            let _ = tx
                .send(AppMessage::PlanDisplayRefreshed {
                    session_id,
                    result: result.map_err(crate::UiFailure::internal),
                })
                .await;
        });
    }

    /// Record the outcome of a plan display refresh so the periodic heal
    /// backs off for sessions that have no plan.
    pub(crate) fn handle_plan_display_refreshed(
        &mut self,
        session_id: i64,
        result: super::UiResult<bool>,
    ) {
        if let Some(state) = self.plan_display_refresh.as_mut()
            && state.session_id == session_id
        {
            state.result = result.ok();
        }
    }
}

/// Pure decision for the periodic plan-chip heal: returns `true` when a
/// read-only `agena.plan.get` refresh should be fired for `session_id` given
/// the last refresh state. Sessions known to have no plan back off far longer
/// than sessions whose plan chip is missing for another reason.
fn plan_display_refresh_due(
    session_id: i64,
    state: Option<PlanDisplayRefreshState>,
    now: Instant,
) -> bool {
    let Some(state) = state else {
        return true;
    };
    if state.session_id != session_id {
        return true;
    }
    let interval = match state.result {
        Some(true) | None => Duration::from_millis(crate::PLAN_DISPLAY_REFRESH_RETRY_MS),
        Some(false) => Duration::from_millis(crate::PLAN_DISPLAY_REFRESH_NO_PLAN_BACKOFF_MS),
    };
    now.saturating_duration_since(state.requested_at) >= interval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_display_refresh_fires_when_no_state_exists() {
        let now = Instant::now();
        assert!(plan_display_refresh_due(7, None, now));
    }

    #[test]
    fn plan_display_refresh_fires_after_switching_sessions() {
        let now = Instant::now();
        let state = Some(PlanDisplayRefreshState {
            session_id: 6,
            requested_at: now,
            result: Some(false),
        });
        assert!(
            plan_display_refresh_due(7, state, now),
            "a different attached session must refresh immediately"
        );
    }

    #[test]
    fn plan_display_refresh_backs_off_within_the_retry_window() {
        let now = Instant::now();
        let state = Some(PlanDisplayRefreshState {
            session_id: 7,
            requested_at: now,
            result: None,
        });
        assert!(
            !plan_display_refresh_due(7, state, now),
            "an in-flight refresh must not re-fire immediately"
        );
    }

    #[test]
    fn plan_display_refresh_retries_after_the_retry_window() {
        let requested_at = Instant::now();
        let later = requested_at
            .checked_add(Duration::from_millis(
                crate::PLAN_DISPLAY_REFRESH_RETRY_MS + 1,
            ))
            .expect("clock must not overflow");
        let state = Some(PlanDisplayRefreshState {
            session_id: 7,
            requested_at,
            result: None,
        });
        assert!(plan_display_refresh_due(7, state, later));
    }

    #[test]
    fn plan_display_refresh_known_no_plan_backs_off_longer() {
        let requested_at = Instant::now();
        let retry_later = requested_at
            .checked_add(Duration::from_millis(
                crate::PLAN_DISPLAY_REFRESH_RETRY_MS + 1,
            ))
            .expect("clock must not overflow");
        let state = Some(PlanDisplayRefreshState {
            session_id: 7,
            requested_at,
            result: Some(false),
        });
        assert!(
            !plan_display_refresh_due(7, state, retry_later),
            "a session known to have no plan must not re-poll at the short retry interval"
        );
        let backoff_later = requested_at
            .checked_add(Duration::from_millis(
                crate::PLAN_DISPLAY_REFRESH_NO_PLAN_BACKOFF_MS + 1,
            ))
            .expect("clock must not overflow");
        assert!(plan_display_refresh_due(7, state, backoff_later));
    }
}
