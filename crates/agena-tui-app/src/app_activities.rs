//! Application adapter for the unified background-activities panel.

use agena_api::resource::BackgroundActivityResource;
use crossterm::event::KeyCode;

use super::{ActivitiesState, App, AppMessage, KeyEvent, Route};
use agena_tui::activities::{
    ActivitiesControl, ActivitiesEffect, ActivitiesLogTail, ActivitiesRow, activities_pane_areas,
    selected_row,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;

const ACTIVITIES_REFRESH_INTERVAL_MS: u64 = 1_000;
const ACTIVITY_LOG_REFRESH_INTERVAL_MS: u64 = 500;
const ACTIVITY_REQUEST_TIMEOUT_SECS: u64 = 10;
const ACTIVITY_LOG_LIMIT: usize = 500;

impl App {
    /// Slow-cadence background poll for the footer activity pill. Reserve the
    /// cadence window before spawning so a slow backend cannot create one
    /// request per UI tick.
    pub(crate) fn refresh_background_activity_summary_if_due(&mut self, now: super::Instant) {
        const INTERVAL_MS: u64 = 10_000;
        if let Some((_, refreshed)) = self.background_activity_summary
            && now.duration_since(refreshed) < super::Duration::from_millis(INTERVAL_MS)
        {
            return;
        }
        let previous_count = self
            .background_activity_summary
            .map_or(0, |(count, _)| count);
        // Reserve the polling window before spawning. Otherwise every 50 ms UI
        // tick starts another request until the first response arrives.
        self.background_activity_summary = Some((previous_count, now));
        let filter = agena_domain::BackgroundActivityFilter {
            active_only: true,
            ..Default::default()
        };
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let count = tokio::time::timeout(
                super::Duration::from_secs(ACTIVITY_REQUEST_TIMEOUT_SECS),
                backend.list_activities(filter),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .map_or(previous_count, |items| items.len());
            let _ = tx
                .send(AppMessage::BackgroundActivitySummaryLoaded { count })
                .await;
        });
    }

    /// Keep a visible Activities route live without requiring manual refresh.
    /// List and log requests have independent cadence and in-flight gates.
    pub(crate) fn refresh_activities_panel_if_due(&mut self, now: super::Instant) {
        let (reload_list, reload_log) = match &self.current_route {
            Route::Activities(state) => {
                let reload_list = !state.loading
                    && !state.mutation_loading
                    && now.duration_since(state.last_refresh_at)
                        >= super::Duration::from_millis(ACTIVITIES_REFRESH_INTERVAL_MS);
                let reload_log = state
                    .presentation
                    .detail
                    .then(|| selected_row(&state.rows, &state.presentation))
                    .flatten()
                    .filter(|row| row.is_active())
                    .filter(|_| !state.log_loading)
                    .filter(|_| {
                        now.duration_since(state.last_log_refresh_at)
                            >= super::Duration::from_millis(ACTIVITY_LOG_REFRESH_INTERVAL_MS)
                    })
                    .map(|row| row.id.clone());
                (reload_list, reload_log)
            }
            _ => (false, None),
        };
        if reload_list {
            self.refresh_activities_panel();
        }
        if let Some(activity_id) = reload_log {
            self.spawn_activity_log_tail(activity_id);
        }
    }

    pub(crate) fn open_activities_panel(&mut self) {
        let mut state = ActivitiesState::new();
        self.start_activities_reload(&mut state);
        self.route_stack.clear();
        self.current_route = Route::Activities(state);
        self.focus = Focus::Transcript;
    }

    fn start_activities_reload(&mut self, state: &mut ActivitiesState) {
        let request_id = self.next_usage_request_id.saturating_add(1);
        self.next_usage_request_id = request_id;
        state.request_id = request_id;
        state.loading = true;
        state.error = None;
        state.last_refresh_at = super::Instant::now();
        let filter = agena_domain::BackgroundActivityFilter::default();
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = match tokio::time::timeout(
                super::Duration::from_secs(ACTIVITY_REQUEST_TIMEOUT_SECS),
                backend.list_activities(filter),
            )
            .await
            {
                Ok(result) => result
                    .map(|items| {
                        items
                            .iter()
                            .map(activities_row_from_resource)
                            .collect::<Vec<_>>()
                    })
                    .map_err(crate::UiFailure::internal),
                Err(_) => Err(crate::UiFailure::internal(
                    "background activity list timed out after 10 seconds",
                )),
            };
            let _ = tx
                .send(AppMessage::ActivitiesLoaded { request_id, result })
                .await;
        });
    }

    fn start_activity_log_tail(&mut self, state: &mut ActivitiesState, activity_id: String) {
        let same_activity = state.log_activity_id.as_deref() == Some(activity_id.as_str());
        let since_seq = if same_activity {
            state.log_tail.as_ref().map_or(0, |tail| tail.last_seq)
        } else {
            state.log_tail = None;
            state.log_error = None;
            state.log_activity_id = Some(activity_id.clone());
            0
        };
        let request_id = self.next_usage_request_id.saturating_add(1);
        self.next_usage_request_id = request_id;
        state.log_request_id = request_id;
        state.log_request_since = since_seq;
        state.log_loading = true;
        state.log_error = None;
        state.last_log_refresh_at = super::Instant::now();
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = match tokio::time::timeout(
                super::Duration::from_secs(ACTIVITY_REQUEST_TIMEOUT_SECS),
                backend.activity_logs(&activity_id, since_seq, Some(ACTIVITY_LOG_LIMIT as u32), 0),
            )
            .await
            {
                Ok(result) => result
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
                    .map_err(crate::UiFailure::internal),
                Err(_) => Err(crate::UiFailure::internal(
                    "background activity log read timed out after 10 seconds",
                )),
            };
            let _ = tx
                .send(AppMessage::ActivitiesLogLoaded {
                    activity_id,
                    request_id,
                    result,
                })
                .await;
        });
    }

    /// Reload an installed Activities route. Key handling uses the `start_*`
    /// helpers directly because the route is temporarily moved out of `App`.
    fn refresh_activities_panel(&mut self) {
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        self.current_route = match route {
            Route::Activities(mut state) => {
                self.start_activities_reload(&mut state);
                Route::Activities(state)
            }
            route => route,
        };
    }

    fn spawn_activity_log_tail(&mut self, activity_id: String) {
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        self.current_route = match route {
            Route::Activities(mut state) => {
                self.start_activity_log_tail(&mut state, activity_id);
                Route::Activities(state)
            }
            route => route,
        };
    }

    pub(crate) fn handle_activities_loaded(
        &mut self,
        request_id: u64,
        result: super::UiResult<Vec<ActivitiesRow>>,
    ) {
        let selected_id = {
            let Route::Activities(state) = &mut self.current_route else {
                return;
            };
            if request_id != state.request_id {
                return;
            }
            state.loading = false;
            let selected_before = selected_row(&state.rows, &state.presentation)
                .map(|row| (row.id.clone(), row.is_active()));
            match result {
                Ok(mut rows) => {
                    agena_tui::activities::sort_rows(&mut rows);
                    state.rows = rows;
                    let visible = agena_tui::activities::visible_rows(
                        state.rows.as_slice(),
                        &state.presentation,
                    );
                    let visible_len = visible.len();
                    let selected_position = selected_before
                        .as_ref()
                        .map(|(id, _)| id.as_str())
                        .and_then(|id| visible.iter().position(|row| row.id == id));
                    state.presentation.selected = selected_position.unwrap_or_else(|| {
                        state
                            .presentation
                            .selected
                            .min(visible_len.saturating_sub(1))
                    });
                    state.presentation.clamp_selection(visible_len);
                    state.error = None;
                }
                Err(error) => state.error = Some(error.to_string()),
            }
            let selected_was_active = selected_before.is_some_and(|(_, active)| active);
            state
                .presentation
                .detail
                .then(|| selected_row(&state.rows, &state.presentation))
                .flatten()
                .filter(|row| {
                    state.log_activity_id.as_deref() != Some(row.id.as_str())
                        || state.log_tail.is_none()
                        || (selected_was_active && !row.is_active())
                })
                .filter(|_| !state.log_loading)
                .map(|row| row.id.clone())
        };
        if let Some(selected_id) = selected_id {
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
        state.log_loading = false;
        let Some(selected) = selected_row(&state.rows, &state.presentation) else {
            return;
        };
        if selected.id != activity_id
            || state.log_activity_id.as_deref() != Some(activity_id.as_str())
        {
            return;
        }
        match result {
            Ok(tail) => {
                state.log_error = None;
                merge_activity_log_tail(&mut state.log_tail, tail, state.log_request_since);
            }
            Err(error) => state.log_error = Some(error.to_string()),
        }
    }

    pub(crate) fn handle_activities_stopped(
        &mut self,
        activity_id: String,
        request_id: u64,
        ok: bool,
    ) {
        let refresh_log = {
            let Route::Activities(state) = &mut self.current_route else {
                return;
            };
            if request_id != state.mutation_request_id {
                return;
            }
            state.mutation_loading = false;
            if ok && let Some(row) = state.rows.iter_mut().find(|row| row.id == activity_id) {
                row.status = "stopped".to_owned();
            }
            (ok && state.presentation.detail)
                .then(|| selected_row(&state.rows, &state.presentation))
                .flatten()
                .filter(|row| row.id == activity_id)
                .map(|row| row.id.clone())
        };
        if let Some(activity_id) = refresh_log {
            self.spawn_activity_log_tail(activity_id);
        }
    }

    pub(crate) fn handle_activities_dismissed(
        &mut self,
        activity_id: String,
        request_id: u64,
        ok: bool,
    ) {
        let next_detail = {
            let Route::Activities(state) = &mut self.current_route else {
                return;
            };
            if request_id != state.mutation_request_id {
                return;
            }
            state.mutation_loading = false;
            if ok {
                state.rows.retain(|row| row.id != activity_id);
                let visible =
                    agena_tui::activities::visible_rows(state.rows.as_slice(), &state.presentation)
                        .len();
                state.presentation.clamp_selection(visible);
                state.log_tail = None;
                state.log_error = None;
                state.log_activity_id = None;
                state.log_loading = false;
            }
            (ok && state.presentation.detail)
                .then(|| selected_row(&state.rows, &state.presentation))
                .flatten()
                .map(|row| row.id.clone())
        };
        if let Some(activity_id) = next_detail {
            self.spawn_activity_log_tail(activity_id);
        }
    }

    pub(crate) fn handle_activities_cleared(&mut self, request_id: u64, ok: bool) {
        let next_detail = {
            let Route::Activities(state) = &mut self.current_route else {
                return;
            };
            if request_id != state.mutation_request_id {
                return;
            }
            state.mutation_loading = false;
            if ok {
                state.rows.retain(|row| row.is_active());
                let visible =
                    agena_tui::activities::visible_rows(state.rows.as_slice(), &state.presentation)
                        .len();
                state.presentation.clamp_selection(visible);
                state.log_tail = None;
                state.log_error = None;
                state.log_activity_id = None;
                state.log_loading = false;
            }
            (ok && state.presentation.detail)
                .then(|| selected_row(&state.rows, &state.presentation))
                .flatten()
                .map(|row| row.id.clone())
        };
        if let Some(activity_id) = next_detail {
            self.spawn_activity_log_tail(activity_id);
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
                self.ensure_activity_detail_matches_selection(state);
            }
            Some(KeyAction::MoveDown) => {
                let (count, offsets, viewport_lines) = self.activities_navigation(state);
                state.presentation.move_selection(count, 1);
                state.presentation.reveal_selected(&offsets, viewport_lines);
                self.ensure_activity_detail_matches_selection(state);
            }
            Some(KeyAction::PageUp) | Some(KeyAction::PageDown) => {
                let (count, offsets, viewport_lines) = self.activities_navigation(state);
                let delta = if key.code == KeyCode::PageUp { -1 } else { 1 };
                let page = viewport_lines.max(1) as isize;
                state.presentation.move_selection(count, delta * page);
                state
                    .presentation
                    .reveal_selected(&offsets, viewport_lines.max(1));
                self.ensure_activity_detail_matches_selection(state);
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
        let (list_area, _) =
            activities_pane_areas(self.layout.overlay_area, state.presentation.detail);
        let viewport_lines = list_area.height.saturating_sub(2) as usize;
        (
            visible.len(),
            agena_tui::activities::row_line_offsets(&visible),
            viewport_lines,
        )
    }

    fn ensure_activity_detail_matches_selection(&mut self, state: &mut ActivitiesState) {
        if !state.presentation.detail {
            return;
        }
        let selected_id = selected_row(&state.rows, &state.presentation).map(|row| row.id.clone());
        match selected_id {
            Some(activity_id)
                if state.log_activity_id.as_deref() != Some(activity_id.as_str())
                    || (state.log_tail.is_none() && !state.log_loading) =>
            {
                self.start_activity_log_tail(state, activity_id);
            }
            None => {
                state.log_tail = None;
                state.log_error = None;
                state.log_activity_id = None;
                state.log_loading = false;
            }
            Some(_) => {}
        }
    }

    fn activate_activities_control(
        &mut self,
        state: &mut ActivitiesState,
        control: ActivitiesControl,
    ) {
        if state.mutation_loading
            && matches!(
                control,
                ActivitiesControl::Refresh
                    | ActivitiesControl::Stop
                    | ActivitiesControl::Dismiss
                    | ActivitiesControl::ClearFinished
            )
        {
            return;
        }
        let effect = match control {
            ActivitiesControl::ToggleDetail => ActivitiesEffect::ToggleDetail,
            ActivitiesControl::Refresh => ActivitiesEffect::Reload,
            ActivitiesControl::ToggleFinished => {
                state.presentation.show_finished = !state.presentation.show_finished;
                state.presentation.selected = 0;
                state.presentation.scroll = 0;
                self.ensure_activity_detail_matches_selection(state);
                ActivitiesEffect::None
            }
            ActivitiesControl::CycleKindFilter => {
                state.presentation.cycle_kind_filter();
                self.ensure_activity_detail_matches_selection(state);
                ActivitiesEffect::None
            }
            ActivitiesControl::CycleStatusFilter => {
                state.presentation.cycle_status_filter();
                self.ensure_activity_detail_matches_selection(state);
                ActivitiesEffect::None
            }
            ActivitiesControl::Stop => match selected_row(&state.rows, &state.presentation) {
                Some(row) if row.is_active() && row.cancellable => {
                    ActivitiesEffect::Stop(row.id.clone())
                }
                _ => ActivitiesEffect::None,
            },
            ActivitiesControl::Dismiss => match selected_row(&state.rows, &state.presentation) {
                Some(row) if row.dismissible => ActivitiesEffect::Dismiss(row.id.clone()),
                _ => ActivitiesEffect::None,
            },
            ActivitiesControl::ClearFinished => ActivitiesEffect::ClearFinished,
            ActivitiesControl::Close => ActivitiesEffect::None,
            ActivitiesControl::MoveUp | ActivitiesControl::MoveDown => ActivitiesEffect::None,
        };
        match effect {
            ActivitiesEffect::Reload => {
                self.start_activities_reload(state);
            }
            ActivitiesEffect::ToggleDetail => {
                state.presentation.detail = !state.presentation.detail;
                if state.presentation.detail {
                    self.ensure_activity_detail_matches_selection(state);
                } else {
                    state.log_tail = None;
                    state.log_error = None;
                    state.log_activity_id = None;
                    state.log_loading = false;
                }
            }
            ActivitiesEffect::Stop(activity_id) => {
                let request_id = self.next_usage_request_id.saturating_add(1);
                self.next_usage_request_id = request_id;
                state.mutation_request_id = request_id;
                state.mutation_loading = true;
                let backend = self.backend.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = match tokio::time::timeout(
                        super::Duration::from_secs(ACTIVITY_REQUEST_TIMEOUT_SECS),
                        backend.stop_activity(&activity_id),
                    )
                    .await
                    {
                        Ok(result) => result.map(|_| true).map_err(crate::UiFailure::internal),
                        Err(_) => Err(crate::UiFailure::internal(
                            "stopping background activity timed out after 10 seconds",
                        )),
                    };
                    let _ = tx
                        .send(AppMessage::ActivitiesStopped {
                            activity_id,
                            request_id,
                            result,
                        })
                        .await;
                });
            }
            ActivitiesEffect::Dismiss(activity_id) => {
                let request_id = self.next_usage_request_id.saturating_add(1);
                self.next_usage_request_id = request_id;
                state.mutation_request_id = request_id;
                state.mutation_loading = true;
                let backend = self.backend.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = match tokio::time::timeout(
                        super::Duration::from_secs(ACTIVITY_REQUEST_TIMEOUT_SECS),
                        backend.dismiss_activity(&activity_id),
                    )
                    .await
                    {
                        Ok(result) => result.map(|_| true).map_err(crate::UiFailure::internal),
                        Err(_) => Err(crate::UiFailure::internal(
                            "dismissing background activity timed out after 10 seconds",
                        )),
                    };
                    let _ = tx
                        .send(AppMessage::ActivitiesDismissed {
                            activity_id,
                            request_id,
                            result,
                        })
                        .await;
                });
            }
            ActivitiesEffect::ClearFinished => {
                let request_id = self.next_usage_request_id.saturating_add(1);
                self.next_usage_request_id = request_id;
                state.mutation_request_id = request_id;
                state.mutation_loading = true;
                let backend = self.backend.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = match tokio::time::timeout(
                        super::Duration::from_secs(ACTIVITY_REQUEST_TIMEOUT_SECS),
                        backend.clear_finished_activities(),
                    )
                    .await
                    {
                        Ok(result) => result.map(|_| true).map_err(crate::UiFailure::internal),
                        Err(_) => Err(crate::UiFailure::internal(
                            "clearing background activities timed out after 10 seconds",
                        )),
                    };
                    let _ = tx
                        .send(AppMessage::ActivitiesCleared { request_id, result })
                        .await;
                });
            }
            ActivitiesEffect::None => {}
        }
    }
}

fn merge_activity_log_tail(
    current: &mut Option<ActivitiesLogTail>,
    mut incoming: ActivitiesLogTail,
    since_seq: u64,
) {
    if since_seq == 0 || current.is_none() {
        trim_activity_log_tail(&mut incoming);
        *current = Some(incoming);
        return;
    }

    let existing = current.as_mut().expect("checked above");
    existing.lines.append(&mut incoming.lines);
    existing.last_seq = existing.last_seq.max(incoming.last_seq);
    // Once older output is known to be omitted, an incremental read cannot
    // restore it. Preserve that signal while accepting a larger backend count.
    existing.has_more |= incoming.has_more;
    existing.dropped_lines = existing.dropped_lines.max(incoming.dropped_lines);
    trim_activity_log_tail(existing);
}

fn trim_activity_log_tail(tail: &mut ActivitiesLogTail) {
    if tail.lines.len() <= ACTIVITY_LOG_LIMIT {
        return;
    }
    let overflow = tail.lines.len() - ACTIVITY_LOG_LIMIT;
    tail.lines.drain(..overflow);
    tail.dropped_lines = tail.dropped_lines.saturating_add(overflow as u64);
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

#[cfg(test)]
mod tests {
    use super::{
        ACTIVITY_LOG_LIMIT, ActivitiesLogTail, merge_activity_log_tail, trim_activity_log_tail,
    };

    fn tail(lines: impl IntoIterator<Item = &'static str>, last_seq: u64) -> ActivitiesLogTail {
        ActivitiesLogTail {
            lines: lines.into_iter().map(str::to_owned).collect(),
            last_seq,
            has_more: false,
            dropped_lines: 0,
        }
    }

    #[test]
    fn incremental_log_reads_append_without_reordering() {
        let mut current = Some(tail(["one", "two"], 2));
        merge_activity_log_tail(&mut current, tail(["three", "four"], 4), 2);
        let current = current.expect("merged tail");
        assert_eq!(current.lines, ["one", "two", "three", "four"]);
        assert_eq!(current.last_seq, 4);
    }

    #[test]
    fn log_tail_is_bounded_and_accounts_for_local_drops() {
        let mut oversized = ActivitiesLogTail {
            lines: (0..ACTIVITY_LOG_LIMIT + 7)
                .map(|index| format!("line-{index}"))
                .collect(),
            last_seq: (ACTIVITY_LOG_LIMIT + 7) as u64,
            has_more: false,
            dropped_lines: 3,
        };
        trim_activity_log_tail(&mut oversized);
        assert_eq!(oversized.lines.len(), ACTIVITY_LOG_LIMIT);
        assert_eq!(oversized.lines.first().map(String::as_str), Some("line-7"));
        assert_eq!(oversized.dropped_lines, 10);
    }
}
