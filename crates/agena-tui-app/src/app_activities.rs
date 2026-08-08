//! Application adapter for the unified background-activities panel.

use agena_api::resource::BackgroundActivityResource;
use crossterm::event::KeyCode;

use super::{ActivitiesState, App, AppMessage, KeyEvent, Route};
use agena_tui::activities::{
    ActivitiesControl, ActivitiesEffect, ActivitiesLogTail, ActivitiesRow,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;

impl App {
    /// Slow-cadence background poll for the footer activity pill. Fires an
    /// active-only list request and lets the response handler record the
    /// refreshed timestamp, so a failed backend does not hammer every tick.
    pub(crate) fn refresh_background_activity_summary_if_due(&mut self, now: super::Instant) {
        const INTERVAL_MS: u64 = 10_000;
        if let Some((_, refreshed)) = self.background_activity_summary
            && now.duration_since(refreshed) < super::Duration::from_millis(INTERVAL_MS)
        {
            return;
        }
        let filter = agena_domain::BackgroundActivityFilter {
            active_only: true,
            ..Default::default()
        };
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let count = backend
                .list_activities(filter)
                .await
                .map(|items| items.len())
                .unwrap_or(0);
            let _ = tx.send(AppMessage::BackgroundActivitySummaryLoaded { count });
        });
    }

    pub(crate) fn open_activities_panel(&mut self) {
        let state = ActivitiesState::new();
        self.route_stack.clear();
        self.current_route = Route::Activities(state);
        self.refresh_activities_panel();
        self.focus = Focus::Transcript;
    }

    /// Reload the activities list from the backend and track the request id on
    /// the active panel state (if any). Safe to call while a route borrow of
    /// `self.current_route` is not active.
    pub(crate) fn refresh_activities_panel(&mut self) {
        let request_id = self.next_usage_request_id.saturating_add(1);
        self.next_usage_request_id = request_id;
        let filter = agena_domain::BackgroundActivityFilter::default();
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_activities(filter)
                .await
                .map(|items| {
                    items
                        .iter()
                        .map(activities_row_from_resource)
                        .collect::<Vec<_>>()
                })
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::ActivitiesLoaded { request_id, result });
        });
        if let Route::Activities(state) = &mut self.current_route {
            state.request_id = request_id;
            state.loading = true;
            state.error = None;
        }
    }

    /// Fetch the log tail for one activity and track the request id on the
    /// active panel state.
    pub(crate) fn spawn_activity_log_tail(&mut self, activity_id: String) {
        let request_id = self.next_usage_request_id.saturating_add(1);
        self.next_usage_request_id = request_id;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .activity_logs(&activity_id, 0, Some(500), 0)
                .await
                .map(|read| ActivitiesLogTail {
                    lines: read
                        .lines
                        .into_iter()
                        .map(|line| {
                            if line.stream == "stderr" {
                                format!("e> {}", line.text)
                            } else {
                                line.text
                            }
                        })
                        .collect(),
                    last_seq: read.last_seq,
                    has_more: read.has_more,
                    dropped_lines: read.dropped_lines,
                })
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::ActivitiesLogLoaded {
                activity_id,
                request_id,
                result,
            });
        });
        if let Route::Activities(state) = &mut self.current_route {
            state.log_request_id = request_id;
        }
    }

    pub(crate) fn handle_activities_loaded(
        &mut self,
        request_id: u64,
        result: super::UiResult<Vec<ActivitiesRow>>,
    ) {
        let Route::Activities(state) = &mut self.current_route else {
            return;
        };
        if request_id != state.request_id {
            return;
        }
        state.loading = false;
        let wants_detail = state.presentation.detail;
        let mut selected_id: Option<String> = None;
        match result {
            Ok(mut rows) => {
                agena_tui::activities::sort_rows(&mut rows);
                state.rows = rows;
                let visible =
                    agena_tui::activities::visible_rows(state.rows.as_slice(), &state.presentation)
                        .len();
                state.presentation.clamp_selection(visible);
                state.error = None;
                selected_id = state
                    .rows
                    .get(state.presentation.selected)
                    .or_else(|| state.rows.first())
                    .map(|row| row.id.clone());
            }
            Err(error) => state.error = Some(error.to_string()),
        }
        if wants_detail && let Some(selected_id) = selected_id {
            self.spawn_activity_log_tail(selected_id);
        }
    }

    pub(crate) fn handle_activity_log_loaded(
        &mut self,
        activity_id: String,
        request_id: u64,
        result: super::UiResult<ActivitiesLogTail>,
    ) {
        let Route::Activities(state) = &mut self.current_route else {
            return;
        };
        if request_id != state.log_request_id {
            return;
        }
        let Some(selected) = state
            .rows
            .get(state.presentation.selected)
            .or_else(|| state.rows.first())
        else {
            return;
        };
        if selected.id != activity_id {
            return;
        }
        match result {
            Ok(tail) => state.log_tail = Some(tail),
            Err(error) => state.error = Some(error.to_string()),
        }
    }

    pub(crate) fn handle_activities_stopped(&mut self, activity_id: String, ok: bool) {
        let Route::Activities(state) = &mut self.current_route else {
            return;
        };
        state.loading = false;
        if ok {
            if let Some(row) = state.rows.iter_mut().find(|row| row.id == activity_id) {
                row.status = "stopped".to_owned();
            }
            state.log_tail = None;
        }
    }

    pub(crate) fn handle_activities_dismissed(&mut self, activity_id: String, ok: bool) {
        let Route::Activities(state) = &mut self.current_route else {
            return;
        };
        state.loading = false;
        if ok {
            state.rows.retain(|row| row.id != activity_id);
            let visible =
                agena_tui::activities::visible_rows(state.rows.as_slice(), &state.presentation)
                    .len();
            state.presentation.clamp_selection(visible);
            state.log_tail = None;
        }
    }

    pub(crate) fn handle_activities_cleared(&mut self, ok: bool) {
        let Route::Activities(state) = &mut self.current_route else {
            return;
        };
        state.loading = false;
        if ok {
            state.rows.retain(|row| row.is_active());
            state.log_tail = None;
        }
    }

    pub(crate) fn handle_activities_key(
        &mut self,
        key: KeyEvent,
        state: &mut ActivitiesState,
    ) -> bool {
        match resolve_tui_key(KeyContext::Activities, key) {
            Some(KeyAction::Close) => return true,
            Some(KeyAction::MoveUp) => {
                let (count, offsets, viewport_lines) = self.activities_navigation(state);
                state.presentation.move_selection(count, -1);
                state.presentation.reveal_selected(&offsets, viewport_lines);
            }
            Some(KeyAction::MoveDown) => {
                let (count, offsets, viewport_lines) = self.activities_navigation(state);
                state.presentation.move_selection(count, 1);
                state.presentation.reveal_selected(&offsets, viewport_lines);
            }
            Some(KeyAction::PageUp) | Some(KeyAction::PageDown) => {
                let (count, offsets, viewport_lines) = self.activities_navigation(state);
                let delta = if key.code == KeyCode::PageUp { -1 } else { 1 };
                let page = viewport_lines.max(1) as isize;
                state.presentation.move_selection(count, delta * page);
                state
                    .presentation
                    .reveal_selected(&offsets, viewport_lines.max(1));
            }
            Some(KeyAction::Open) => {
                self.activate_activities_control(state, ActivitiesControl::ToggleDetail);
            }
            Some(KeyAction::Refresh) => {
                self.activate_activities_control(state, ActivitiesControl::Refresh);
            }
            Some(KeyAction::ActivitiesToggleFinished) => {
                self.activate_activities_control(state, ActivitiesControl::ToggleFinished);
            }
            Some(KeyAction::ActivitiesCycleKind) => {
                self.activate_activities_control(state, ActivitiesControl::CycleKindFilter);
            }
            Some(KeyAction::ActivitiesCycleStatus) => {
                self.activate_activities_control(state, ActivitiesControl::CycleStatusFilter);
            }
            Some(KeyAction::ActivitiesStop) => {
                self.activate_activities_control(state, ActivitiesControl::Stop);
            }
            Some(KeyAction::ActivitiesDismiss) => {
                self.activate_activities_control(state, ActivitiesControl::Dismiss);
            }
            Some(KeyAction::ActivitiesClearFinished) => {
                self.activate_activities_control(state, ActivitiesControl::ClearFinished);
            }
            Some(_) | None => {}
        }
        false
    }

    /// Row count, per-row rendered line offsets, and list viewport height for
    /// the active activities panel. Used by selection movement and paging.
    fn activities_navigation(&self, state: &ActivitiesState) -> (usize, Vec<usize>, usize) {
        let visible =
            agena_tui::activities::visible_rows(state.rows.as_slice(), &state.presentation);
        let viewport_lines = self.layout.overlay_area.height.saturating_sub(2) as usize;
        (
            visible.len(),
            agena_tui::activities::row_line_offsets(&visible),
            viewport_lines,
        )
    }

    fn activate_activities_control(
        &mut self,
        state: &mut ActivitiesState,
        control: ActivitiesControl,
    ) {
        let effect = match control {
            ActivitiesControl::ToggleDetail => ActivitiesEffect::ToggleDetail,
            ActivitiesControl::Refresh => ActivitiesEffect::Reload,
            ActivitiesControl::ToggleFinished => {
                state.presentation.show_finished = !state.presentation.show_finished;
                state.presentation.selected = 0;
                state.presentation.scroll = 0;
                ActivitiesEffect::None
            }
            ActivitiesControl::CycleKindFilter => {
                state.presentation.cycle_kind_filter();
                ActivitiesEffect::None
            }
            ActivitiesControl::CycleStatusFilter => {
                state.presentation.cycle_status_filter();
                ActivitiesEffect::None
            }
            ActivitiesControl::Stop => match state.rows.get(state.presentation.selected) {
                Some(row) if row.is_active() && row.cancellable => {
                    ActivitiesEffect::Stop(row.id.clone())
                }
                _ => ActivitiesEffect::None,
            },
            ActivitiesControl::Dismiss => match state.rows.get(state.presentation.selected) {
                Some(row) if row.dismissible => ActivitiesEffect::Dismiss(row.id.clone()),
                _ => ActivitiesEffect::None,
            },
            ActivitiesControl::ClearFinished => ActivitiesEffect::ClearFinished,
            ActivitiesControl::Close => ActivitiesEffect::None,
            ActivitiesControl::MoveUp | ActivitiesControl::MoveDown => ActivitiesEffect::None,
        };
        match effect {
            ActivitiesEffect::Reload => {
                self.refresh_activities_panel();
            }
            ActivitiesEffect::ToggleDetail => {
                state.presentation.detail = !state.presentation.detail;
                if state.presentation.detail {
                    if let Some(selected) = state.rows.get(state.presentation.selected) {
                        let activity_id = selected.id.clone();
                        self.spawn_activity_log_tail(activity_id);
                    }
                } else {
                    state.log_tail = None;
                }
            }
            ActivitiesEffect::Stop(activity_id) => {
                state.loading = true;
                let backend = self.backend.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = backend
                        .stop_activity(&activity_id)
                        .await
                        .map(|_| true)
                        .map_err(crate::UiFailure::internal);
                    let _ = tx.send(AppMessage::ActivitiesStopped {
                        activity_id,
                        result,
                    });
                });
            }
            ActivitiesEffect::Dismiss(activity_id) => {
                state.loading = true;
                let backend = self.backend.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = backend
                        .dismiss_activity(&activity_id)
                        .await
                        .map(|_| true)
                        .map_err(crate::UiFailure::internal);
                    let _ = tx.send(AppMessage::ActivitiesDismissed {
                        activity_id,
                        result,
                    });
                });
            }
            ActivitiesEffect::ClearFinished => {
                state.loading = true;
                let backend = self.backend.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = backend
                        .clear_finished_activities()
                        .await
                        .map(|_| true)
                        .map_err(crate::UiFailure::internal);
                    let _ = tx.send(AppMessage::ActivitiesCleared { result });
                });
            }
            ActivitiesEffect::None => {}
        }
    }
}

pub(super) fn activities_row_from_resource(activity: &BackgroundActivityResource) -> ActivitiesRow {
    ActivitiesRow {
        id: activity.id.clone(),
        kind: activity.kind.clone(),
        status: activity.status.clone(),
        title: activity.title.clone(),
        description: activity.description.clone(),
        command: activity.command.clone(),
        session_id: activity.session_id,
        started_at_ms: activity.started_at_ms,
        finished_at_ms: activity.finished_at_ms,
        exit_code: activity.exit_code,
        message: activity.message.clone(),
        cancellable: activity.cancellable,
        dismissible: activity.dismissible,
    }
}
