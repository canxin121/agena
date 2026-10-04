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
use serde::Deserialize;
use std::time::{Duration, Instant};

mod requests;
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
    File(String, bool),
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilePage {
    #[serde(default)]
    files: Vec<FileRow>,
    total_files: usize,
    #[serde(default)]
    has_more: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileRow {
    path: String,
    index: String,
    working_dir: String,
    #[serde(default)]
    index_old_path: Option<String>,
    #[serde(default)]
    working_old_path: Option<String>,
}

#[derive(Debug)]
pub(crate) enum WorkResult {
    Files(FilePage),
    Logs(BackgroundActivityLogResource),
    Diff(String, bool),
    Control(Box<BackgroundActivityResource>),
}

#[derive(Debug, Default)]
pub(crate) struct SessionWorkState {
    directory: String,
    expanded: bool,
    tab: Tab,
    files: Option<FilePage>,
    page: usize,
    selected: usize,
    detail: Option<Detail>,
    detail_activity: Option<BackgroundActivityResource>,
    logs: Option<BackgroundActivityLogResource>,
    diff: String,
    diff_limit: usize,
    diff_truncated: bool,
    file_task: Option<ReadTask>,
    detail_task: Option<ReadTask>,
    file_request: u64,
    detail_request: u64,
    control_request: u64,
    file_at: Option<Instant>,
    detail_at: Option<Instant>,
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
        let directory = self
            .transcript
            .execution
            .as_ref()
            .and_then(|e| e.execution.effective_workspace_root.clone())
            .unwrap_or_default();
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
        let busy = self.active_run_session_id() == Some(id);
        let has_status = self.session_work_has_status();
        let state = self.session_work.entry(id).or_default();
        if state.directory != directory {
            *state = SessionWorkState {
                directory,
                ..Default::default()
            };
        }
        if state.tab == Tab::Status && !has_status {
            state.tab = Tab::Files;
            state.expanded = false;
            if self.work_focus == Some(id) {
                self.work_focus = None;
                self.focus = Focus::Composer;
            }
        }
        let now = Instant::now();
        let interval = if state.file_error.is_some() {
            60
        } else if state.expanded || busy {
            5
        } else {
            30
        };
        let files_due = !state.directory.is_empty()
            && state.file_task.is_none()
            && state
                .file_at
                .is_none_or(|at| now.duration_since(at) >= Duration::from_secs(interval));
        let detail_due = state.expanded
            && state.detail.is_some()
            && state.detail_task.is_none()
            && state.detail_at.is_none_or(|at| {
                now.duration_since(at)
                    >= Duration::from_secs(if state.detail_error.is_some() {
                        30
                    } else if matches!(state.detail, Some(Detail::Task(_)))
                        && state.logs.as_ref().is_none_or(|l| {
                            l.has_more
                                || matches!(
                                    l.status.as_str(),
                                    "running" | "pending" | "waiting" | "paused"
                                )
                        })
                    {
                        2
                    } else if busy {
                        5
                    } else {
                        30
                    })
            });
        if files_due {
            self.request_work_files(id);
        }
        if detail_due {
            self.request_work_detail(id);
        }
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
                .map(|f| {
                    Detail::File(
                        f.path.clone(),
                        !f.index.trim().is_empty() && f.working_dir.trim().is_empty(),
                    )
                })
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
        let has_status = self.session_work_has_status();
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
                state.tab = match state.tab {
                    Tab::Tasks => Tab::Files,
                    Tab::Files => {
                        if has_status {
                            Tab::Status
                        } else {
                            Tab::Tasks
                        }
                    }
                    Tab::Status => Tab::Tasks,
                };
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
                state.files = None;
            }
            "work-next" => {
                if state.files.as_ref().is_some_and(|f| f.has_more) {
                    state.page += 1;
                    state.file_at = None;
                    state.files = None;
                    state.selected = 0;
                }
            }
            "work-more-diff" => {
                state.diff_limit += 256 * 1024;
                state.detail_at = None;
            }
            "work-staged" => {
                if let Some(Detail::File(path, staged)) = state.detail.clone() {
                    state.select_detail(Some(Detail::File(path, !staged)));
                }
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
        if key.code == KeyCode::F(6) {
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
            KeyCode::Char('s') => "work-staged",
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
