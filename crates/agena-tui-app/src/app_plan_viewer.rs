//! Application adapter for the plan viewer route.

use crossterm::event::KeyCode;

use super::{App, AppMessage, KeyEvent, PlanViewerData, PlanViewerState, Route};
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

    /// Reload the full plan markdown and the compact statusline summary,
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
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
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
            let _ = tx.send(AppMessage::PlanViewerLoaded { request_id, result });
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
            // Re-fetch so the title badge and the statusline summary reflect
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
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .invoke_plugin_ui_tool(
                    "agena.plan",
                    "update",
                    serde_json::json!({ "autorun": target }),
                    Some(session_id),
                )
                .await
                .map(|_| true)
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::PlanAutorunToggled { request_id, result });
        });
        state.toggle_request_id = request_id;
    }

    /// Compact statusline text contributed by the planning plugin for
    /// `session_id` (for example `▶ 2/5 ↻`).
    fn plan_viewer_summary(&self, session_id: i64) -> Option<String> {
        let expected = format!("plan:{session_id}");
        self.backend
            .plugin_statusline_segments()
            .into_iter()
            .find(|segment| segment.segment_id == expected)
            .map(|segment| segment.content.trim().to_string())
            .filter(|content| !content.is_empty())
    }

    fn plan_viewer_page_size(&self) -> u16 {
        self.layout.overlay_area.height.saturating_sub(2).max(1)
    }
}
