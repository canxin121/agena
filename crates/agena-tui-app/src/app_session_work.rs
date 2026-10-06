//! Compact session accessories. One expanded body, bounded reads, and the
//! canonical execution snapshot for task/request/retry state.
use crate::{App, AppMessage, Route};
use agena_api::resource::{BackgroundActivityLogResource, BackgroundActivityResource};
use agena_tui::main_focus::Focus;
use agena_tui_components::{
    pointer::{self, PointerAction},
    theme,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Wrap},
};
use std::time::{Duration, Instant};

mod requests;
mod inline;
#[cfg(test)]
mod tests;
mod view;

#[derive(Debug)]
struct ReadTask(tokio::task::JoinHandle<()>);
impl Drop for ReadTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    Tasks,
    #[default]
    Files,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Detail {
    Task(String),
    File(String),
}

pub(crate) type FilePage = agena_api::resource::SessionFileChangesResource;

#[derive(Debug)]
pub(crate) enum WorkResult {
    Files(FilePage),
    Logs(BackgroundActivityLogResource),
    Diff(String, bool),
    Control(Box<BackgroundActivityResource>),
}

#[derive(Debug, Default)]
pub(crate) struct SessionWorkState {
    expanded: bool,
    tab: Tab,
    files: Option<FilePage>,
    page: usize,
    selected: usize,
    detail: Option<Detail>,
    detail_activity: Option<BackgroundActivityResource>,
    logs: Option<BackgroundActivityLogResource>,
    inline_logs: std::collections::BTreeMap<String, inline::InlineLogState>,
    diff: String,
    diff_limit: usize,
    diff_truncated: bool,
    file_task: Option<ReadTask>,
    detail_task: Option<ReadTask>,
    file_request: u64,
    detail_request: u64,
    control_request: u64,
    control_observation: Option<(String, String, Option<i64>)>,
    file_at: Option<Instant>,
    detail_at: Option<Instant>,
    file_dirty: bool,
    detail_dirty: bool,
    activity_times: std::collections::BTreeMap<String, i64>,
    file_error: Option<String>,
    detail_error: Option<String>,
    control_error: Option<String>,
    scroll: u16,
    max_scroll: u16,
    render_key: Option<(String, u16, ratatui::style::Color)>,
    rendered: Vec<Line<'static>>,
}

impl SessionWorkState {
    fn cancel_detail(&mut self) {
        self.detail_task = None;
        self.detail_request = 0;
        self.detail_at = None;
        self.detail_dirty = true;
    }
    fn select_detail(&mut self, detail: Option<Detail>) {
        self.cancel_detail();
        self.detail = detail;
        self.detail_activity = None;
        self.logs = None;
        self.diff.clear();
        self.diff_limit = 256 * 1024;
        self.diff_truncated = false;
        self.detail_error = None;
        self.render_key = None;
        self.scroll = 0;
    }
}

impl App {
    fn session_work_has_files(&self) -> bool {
        self.transcript
            .session_id
            .and_then(|id| self.session_work.get(&id))
            .and_then(|state| state.files.as_ref())
            .is_some_and(|files| files.total_files > 0 || files.recording_incomplete)
    }

    fn session_work_has_tasks(&self) -> bool {
        self.transcript
            .execution
            .as_ref()
            .is_some_and(|e| !e.background_activities.is_empty())
    }

    pub(crate) fn session_work_visible(&self) -> bool {
        self.transcript.session_id.is_some()
            && (self.session_work_has_files()
                || self.session_work_has_tasks()
                || self.session_work_has_status())
    }

    fn heal_session_work_selection(&mut self, id: i64) {
        if self.transcript.session_id != Some(id) {
            return;
        }
        let has_files = self.session_work_has_files();
        let has_tasks = self.session_work_has_tasks();
        let has_status = self.session_work_has_status();
        let Some(state) = self.session_work.get_mut(&id) else {
            return;
        };
        let tab_available = match state.tab {
            Tab::Files => has_files,
            Tab::Tasks => has_tasks,
            Tab::Status => has_status,
        };
        if !tab_available {
            state.tab = if has_files {
                Tab::Files
            } else if has_tasks {
                Tab::Tasks
            } else if has_status {
                Tab::Status
            } else {
                Tab::Files
            };
            state.expanded = false;
            state.select_detail(None);
            if self.work_focus == Some(id) {
                self.work_focus = None;
                self.focus = Focus::Composer;
            }
        }
    }

    fn session_work_has_status(&self) -> bool {
        self.transcript
            .execution
            .as_ref()
            .is_some_and(|e| e.execution.provider_retry.is_some())
    }

    pub(crate) fn collapse_session_work(&mut self, id: i64) {
        if let Some(state) = self.session_work.get_mut(&id) {
            state.expanded = false;
            state.cancel_detail();
        }
        self.work_focus = None;
    }

    pub(crate) fn heal_session_work(&mut self) {
        let selected = self.transcript.session_id;
        for (id, state) in &mut self.session_work {
            if Some(*id) != selected || !matches!(self.current_route, Route::Main) {
                if state.file_task.take().is_some() {
                    state.file_request = 0;
                    state.file_at = None;
                }
                state.cancel_detail();
            }
        }
        if !matches!(self.current_route, Route::Main) {
            return;
        }
        let Some(id) = selected else {
            return;
        };
        if !self.session_work.contains_key(&id) && self.session_work.len() >= 16 {
            let oldest = self
                .session_work
                .iter()
                .filter(|(_, s)| s.control_request == 0)
                .min_by_key(|(_, s)| s.file_at)
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                self.session_work.remove(&oldest);
            }
        }
        self.session_work.entry(id).or_default();
        self.heal_session_work_selection(id);
        let state = self.session_work.get_mut(&id).expect("work state exists");
        let now = Instant::now();
        let interval = if state.file_error.is_some() {
            60
        } else {
            30
        };
        let files_due = state.file_task.is_none()
            && state
                .file_at
                .is_none_or(|at| now.duration_since(at) >= if state.file_dirty && state.file_error.is_none() {
                    Duration::from_millis(750)
                } else { Duration::from_secs(interval) });
        let detail_due = state.expanded
            && state.detail.is_some()
            && state.detail_task.is_none()
            && state.detail_at.is_none_or(|at| {
                now.duration_since(at)
                    >= Duration::from_millis(if state.detail_error.is_some() {
                        30_000
                    } else if state.detail_dirty || state.logs.as_ref().is_some_and(|logs| logs.has_more)
                    {
                        750
                    } else {
                        30_000
                    })
            });
        if files_due {
            self.request_work_files(id);
        }
        if detail_due {
            self.request_work_detail(id);
        }
    }

    pub(crate) fn mark_session_files_changed(&mut self, id: i64) {
        let state = self.session_work.entry(id).or_default();
        state.file_dirty = true;
        if matches!(state.detail, Some(Detail::File(_))) { state.detail_dirty = true; }
    }

    pub(crate) fn apply_session_activity(&mut self, id: i64, time: i64, dismissed: bool, activity: BackgroundActivityResource) {
        let state = self.session_work.entry(id).or_default();
        if state.activity_times.get(&activity.id).is_some_and(|previous| *previous >= time) { return; }
        if state.activity_times.len() >= 512 && !state.activity_times.contains_key(&activity.id) {
            if let Some(oldest) = state.activity_times.iter().min_by_key(|(_, time)| *time).map(|(id, _)| id.clone()) {
                state.activity_times.remove(&oldest);
            }
        }
        state.activity_times.insert(activity.id.clone(), time);
        if let Some(logs) = state.inline_logs.get_mut(&activity.id) { logs.dirty = true; }
        if matches!(&state.detail, Some(Detail::Task(task)) if task == &activity.id) {
            if state.detail_activity.as_ref() != Some(&activity) { state.detail_activity = Some(activity.clone()); }
            state.detail_dirty = true;
        }
        let mut representation_changed = false;
        let execution_absent = self.transcript.execution.is_none();
        if let Some(execution) = self.transcript.execution.as_mut() {
            if dismissed {
                representation_changed = execution.background_activities.iter().any(|row| row.id == activity.id);
                execution.background_activities.retain(|row| row.id != activity.id);
            } else if let Some(row) = execution.background_activities.iter_mut().find(|row| row.id == activity.id) {
                if row != &activity { *row = activity; representation_changed = true; }
            } else {
                execution.background_activities.insert(0, activity);
                representation_changed = true;
            }
        }
        // A snapshot already in flight may precede this event. Leave one
        // trailing refresh so applying that older body cannot erase the event.
        if execution_absent || (representation_changed && (self.transcript.refresh_in_flight_since.is_some() || self.transcript.state_load_in_flight_since.is_some())) {
            self.pending_refresh_for(id);
        }
        self.heal_session_work_selection(id);
    }

    pub(crate) fn handle_work_row(&mut self, index: usize) {
        let Some(id) = self.transcript.session_id else {
            return;
        };
        let Some(state) = self.session_work.get_mut(&id) else {
            return;
        };
        state.selected = index;
        let detail = if state.tab == Tab::Tasks {
            self.transcript
                .execution
                .as_ref()
                .and_then(|e| e.background_activities.get(index))
                .map(|a| Detail::Task(a.id.clone()))
        } else {
            state
                .files
                .as_ref()
                .and_then(|p| p.files.get(index))
                .map(|f| Detail::File(f.path.clone()))
        };
        state.select_detail(detail);
        state.detail_activity = if state.tab == Tab::Tasks {
            self.transcript
                .execution
                .as_ref()
                .and_then(|e| e.background_activities.get(index))
                .cloned()
        } else {
            None
        };
        self.work_focus = Some(id);
        self.request_work_detail(id);
    }

    pub(crate) fn handle_session_work_action(&mut self, action: &str) {
        let Some(id) = self.transcript.session_id else {
            return;
        };
        if !self.session_work_visible() {
            return;
        }
        if action == "work-control-primary" {
            let task = self.session_work.get(&id).and_then(|s| {
                if let Some(Detail::Task(task)) = &s.detail {
                    Some(task)
                } else {
                    None
                }
            });
            let action = self
                .transcript
                .execution
                .as_ref()
                .and_then(|e| e.background_activities.iter().find(|a| Some(&a.id) == task))
                .and_then(|a| a.controls.first())
                .cloned();
            if let Some(action) = action {
                self.control_work_activity(&action);
            }
            return;
        }
        if let Some(action) = action.strip_prefix("work-control-") {
            self.control_work_activity(action);
            return;
        }
        let tabs: Vec<_> = [
            (Tab::Files, self.session_work_has_files()),
            (Tab::Tasks, self.session_work_has_tasks()),
            (Tab::Status, self.session_work_has_status()),
        ]
        .into_iter()
        .filter_map(|(tab, available)| available.then_some(tab))
        .collect();
        let state = self.session_work.entry(id).or_default();
        if matches!(
            action,
            "work-toggle"
                | "work-tasks"
                | "work-files"
                | "work-status"
                | "work-next-tab"
                | "work-prev"
                | "work-next"
        ) {
            state.file_task = None;
            state.file_request = 0;
            state.file_at = None;
        }
        match action {
            "work-toggle" => state.expanded = !state.expanded,
            "work-tasks" | "work-files" | "work-status" => {
                state.tab = match action {
                    "work-tasks" => Tab::Tasks,
                    "work-status" => Tab::Status,
                    _ => Tab::Files,
                };
                state.expanded = true;
                state.selected = 0;
                state.select_detail(None);
                state.file_at = None;
            }
            "work-next-tab" => {
                let next = tabs
                    .iter()
                    .position(|tab| *tab == state.tab)
                    .map_or(0, |i| (i + 1) % tabs.len());
                state.tab = tabs[next];
                state.select_detail(None);
                state.selected = 0;
                state.file_at = None;
            }
            "work-leave" => {
                self.work_focus = None;
                self.focus = Focus::Composer;
                return;
            }
            "work-focus" => {
                self.work_focus = Some(id);
                self.focus = Focus::Transcript;
                return;
            }
            "work-back" => state.select_detail(None),
            "work-refresh" => {
                state.file_at = None;
                state.detail_at = None;
            }
            "work-prev" => {
                state.page = state.page.saturating_sub(1);
                state.file_at = None;
                state.selected = 0;
                if let Some(files) = &mut state.files {
                    files.files.clear();
                }
            }
            "work-next" => {
                if state.files.as_ref().is_some_and(|f| f.has_more) {
                    state.page += 1;
                    state.file_at = None;
                    if let Some(files) = &mut state.files {
                        files.files.clear();
                    }
                    state.selected = 0;
                }
            }
            "work-more-diff" => {
                state.diff_limit = (state.diff_limit + 256 * 1024).min(2 * 1024 * 1024);
                state.detail_at = None;
            }
            "work-up" => {
                if state.detail.is_some() || state.tab == Tab::Status {
                    state.scroll = state.scroll.saturating_sub(3);
                } else {
                    state.selected = state.selected.saturating_sub(1);
                }
            }
            "work-down" => {
                if state.detail.is_some() || state.tab == Tab::Status {
                    state.scroll = state.scroll.saturating_add(3).min(state.max_scroll);
                } else {
                    state.selected = state.selected.saturating_add(1);
                }
            }
            _ => {}
        }
        if state.expanded {
            self.collapse_inline_plan(id);
            self.collapse_btw(id);
            self.work_focus = Some(id);
            self.focus = Focus::Transcript;
        } else {
            self.collapse_session_work(id);
            self.focus = Focus::Composer;
        }
        self.heal_session_work();
    }

    pub(crate) fn handle_session_work_key(&mut self, key: KeyEvent) -> bool {
        if !matches!(self.current_route, Route::Main) {
            return false;
        }
        if agena_tui::keymap::resolve(agena_tui::keymap::KeyContext::Main, key)
            == Some(agena_tui::keymap::KeyAction::ToggleSessionWork)
            && self.interaction_editing.is_none()
            && self.prompt_history_search.is_none()
            && !self.btw_has_focus()
        {
            self.handle_session_work_action("work-toggle");
            return true;
        }
        if self.work_focus.is_none()
            || self.work_focus != self.transcript.session_id
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return false;
        }
        let action = match key.code {
            KeyCode::Esc | KeyCode::Tab => "work-leave",
            KeyCode::Char('q') => "work-toggle",
            KeyCode::Char('r') => "work-refresh",
            KeyCode::Left | KeyCode::Right => "work-next-tab",
            KeyCode::Up | KeyCode::Char('k') => "work-up",
            KeyCode::Down | KeyCode::Char('j') => "work-down",
            KeyCode::Backspace => "work-back",
            KeyCode::Char('m') => "work-more-diff",
            KeyCode::Char('n') => "work-next",
            KeyCode::Char('p') => "work-prev",
            KeyCode::Char('x') => "work-control-primary",
            KeyCode::Enter => {
                let index = self
                    .work_focus
                    .and_then(|id| self.session_work.get(&id))
                    .map_or(0, |s| s.selected);
                self.handle_work_row(index);
                return true;
            }
            _ => return false,
        };
        self.handle_session_work_action(action);
        true
    }
}

fn merge_log_tail(previous: Option<BackgroundActivityLogResource>, mut logs: BackgroundActivityLogResource) -> BackgroundActivityLogResource {
    if let Some(old) = previous
        .filter(|old| old.activity_id == logs.activity_id)
    {
        logs.last_seq = logs.last_seq.max(old.last_seq);
        let mut lines = old
            .lines
            .into_iter()
            .map(|line| (line.seq, line))
            .collect::<std::collections::BTreeMap<_, _>>();
        for line in logs.lines {
            lines.insert(line.seq, line);
        }
        logs.lines = lines.into_values().collect();
    }
    let excess = logs.lines.len().saturating_sub(200);
    logs.lines.drain(..excess);
    let mut budget = 128 * 1024;
    let mut keep = 0;
    for line in logs.lines.iter_mut().rev() {
        if budget < 4 {
            break;
        }
        if line.text.len() > budget {
            let mut start = line.text.len() - budget + 3;
            while !line.text.is_char_boundary(start) {
                start += 1;
            }
            line.text = format!("…{}", &line.text[start..]);
        }
        budget = budget.saturating_sub(line.text.len());
        keep += 1;
    }
    logs.lines.drain(..logs.lines.len() - keep);
    logs
}
