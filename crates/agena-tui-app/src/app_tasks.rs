//! Durable-task workbench. HTTP-only, bounded visible-page refresh, and retained
//! exact mutation payloads. No model execution or SQLite access lives here.
use std::time::{Duration, Instant};

use agena_api::tasks::{CreateTask, ResumeTask, Task, TaskQuery, TaskState};
use agena_tui::i18n::I18n;
use agena_tui_components::Editor;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use uuid::Uuid;

use crate::{App, Route};
use agena_tui::main_focus::Focus;

const PAGE_SIZE: usize = 20;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub(crate) enum TaskCommand {
    Create(CreateTask),
    Resume(Uuid, ResumeTask),
    Cancel(Uuid, i64),
    Accept(Uuid, i64),
}

impl TaskCommand {
    async fn run(
        &self,
        client: &agena_client::AgenaClient,
    ) -> Result<Task, agena_client::ClientError> {
        match self {
            Self::Create(request) => client.create_task(request).await,
            Self::Resume(id, request) => client.resume_task(*id, request).await,
            Self::Cancel(id, revision) => client.cancel_task(*id, *revision).await,
            Self::Accept(id, revision) => client.accept_task(*id, *revision).await,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormKind {
    Create,
    Resume,
    Cancel,
    Accept,
}

#[derive(Debug, Clone)]
pub(crate) struct TaskForm {
    kind: FormKind,
    target: Option<Task>,
    editor: Editor,
    key: String,
    submitted: Option<TaskCommand>,
    conflict: bool,
}

impl TaskForm {
    fn new(kind: FormKind, target: Option<Task>, text: String) -> Self {
        let mut editor = Editor::default();
        editor.set_text(text);
        Self {
            kind,
            target,
            editor,
            key: Uuid::new_v4().to_string(),
            submitted: None,
            conflict: false,
        }
    }

    fn command(&mut self, source: i64) -> Option<TaskCommand> {
        if self.conflict {
            return None;
        }
        if let Some(command) = &self.submitted {
            return Some(command.clone());
        }
        let text = self.editor.text().to_owned();
        let command = match self.kind {
            FormKind::Create if source > 0 && valid_text(&text) => {
                TaskCommand::Create(CreateTask {
                    source_session_id: source,
                    goal: text,
                    request_key: self.key.clone(),
                })
            }
            FormKind::Resume if valid_text(&text) => {
                let task = self.target.as_ref()?;
                if !task.state.can_resume() {
                    return None;
                }
                TaskCommand::Resume(
                    task.id,
                    ResumeTask {
                        expected_revision: task.revision,
                        request_key: self.key.clone(),
                        instruction: text,
                    },
                )
            }
            FormKind::Cancel => {
                let task = self.target.as_ref()?;
                if !can_cancel(task) {
                    return None;
                }
                TaskCommand::Cancel(task.id, task.revision)
            }
            FormKind::Accept => {
                let task = self.target.as_ref()?;
                if task.state != TaskState::Review {
                    return None;
                }
                TaskCommand::Accept(task.id, task.revision)
            }
            _ => return None,
        };
        self.submitted = Some(command.clone());
        Some(command)
    }
}

fn valid_text(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= 32768
}
fn can_cancel(task: &Task) -> bool {
    !matches!(
        task.state,
        TaskState::Completed | TaskState::Cancelled | TaskState::Cancelling
    )
}

#[derive(Debug, Clone)]
pub(crate) struct TaskBoardState {
    source: i64,
    instance: Uuid,
    request: Uuid,
    pub(crate) items: Vec<Task>,
    selected: usize,
    page: u32,
    has_more: bool,
    only_source: bool,
    filter: Option<TaskState>,
    loading: bool,
    busy: bool,
    last_read: Instant,
    pub(crate) form: Option<TaskForm>,
    error: Option<String>,
    detail_scroll: u16,
}

impl TaskBoardState {
    fn new(source: i64) -> Self {
        Self {
            source,
            instance: Uuid::new_v4(),
            request: Uuid::new_v4(),
            items: vec![],
            selected: 0,
            page: 0,
            has_more: false,
            only_source: source > 0,
            filter: None,
            loading: false,
            busy: false,
            last_read: Instant::now(),
            form: None,
            error: None,
            detail_scroll: 0,
        }
    }
    fn selected(&self) -> Option<&Task> {
        self.items.get(self.selected)
    }
    pub(crate) fn accepts_text(&self) -> bool {
        self.form.as_ref().is_some_and(|form| {
            matches!(form.kind, FormKind::Create | FormKind::Resume)
                && form.submitted.is_none()
                && !self.busy
        })
    }
    pub(crate) fn paste(&mut self, text: &str) {
        if self.accepts_text()
            && let Some(form) = &mut self.form
            && form.editor.text().len().saturating_add(text.len()) <= 32768
        {
            form.editor.insert_str(text);
        }
    }
    fn apply_rows(&mut self, rows: Vec<Task>) {
        let selected = self.selected().map(|task| task.id);
        self.has_more = rows.len() > PAGE_SIZE;
        self.items = rows
            .into_iter()
            .take(PAGE_SIZE)
            .map(|row| {
                self.items
                    .iter()
                    .find(|saved| saved.id == row.id && saved.revision > row.revision)
                    .cloned()
                    .unwrap_or(row)
            })
            .collect();
        self.selected = selected
            .and_then(|id| self.items.iter().position(|row| row.id == id))
            .unwrap_or(self.selected)
            .min(self.items.len().saturating_sub(1));
    }
}

impl App {
    pub(crate) fn open_tasks(&mut self, compose: bool, goal: &str) {
        let active = self.current_or_selected_session_id().unwrap_or(0);
        // Returning from a worker should reopen its owning task conversation,
        // rather than an empty board scoped to the worker's own session ID.
        let source = self
            .task_boards
            .values()
            .flat_map(|board| board.items.iter())
            .find(|task| task.worker_session_id == Some(active))
            .map(|task| task.source_session_id)
            .unwrap_or(active);
        let mut state = self
            .task_boards
            .remove(&source)
            .unwrap_or_else(|| TaskBoardState::new(source));
        if compose {
            if source <= 0 {
                self.flash_warning(self.i18n.text("task-board-noSession"));
            } else if state.form.is_none() {
                state.form = Some(TaskForm::new(FormKind::Create, None, goal.to_owned()));
            }
        }
        self.route_stack.clear();
        self.current_route = Route::Tasks(source);
        self.focus = Focus::Transcript;
        self.refresh_tasks(&mut state);
        self.task_boards.insert(source, state);
    }

    fn refresh_tasks(&mut self, state: &mut TaskBoardState) {
        if state.loading || state.busy {
            return;
        }
        let source = state.source;
        let instance = state.instance;
        let request = Uuid::new_v4();
        state.request = request;
        state.loading = true;
        state.last_read = Instant::now();
        let query = TaskQuery {
            source_session_id: (state.only_source && source > 0).then_some(source),
            offset: state.page * PAGE_SIZE as u32,
            limit: Some(PAGE_SIZE + 1),
            state: state.filter,
            ..Default::default()
        };
        self.dispatch_backend_operation(
            move |backend| async move {
                Ok::<_, anyhow::Error>(
                    tokio::time::timeout(REQUEST_TIMEOUT, backend.client().list_tasks(&query))
                        .await??,
                )
            },
            move |app, result| {
                let Some(state) = app.task_boards.get_mut(&source) else {
                    return;
                };
                if state.instance != instance || state.request != request {
                    return;
                }
                state.loading = false;
                state.last_read = Instant::now();
                match result {
                    Ok(rows) => {
                        state.apply_rows(rows);
                        if state.form.is_none() {
                            state.error = None;
                        }
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            },
        );
    }

    pub(crate) fn tick_tasks(&mut self) {
        let Route::Tasks(source) = self.current_route else {
            return;
        };
        let Some(mut state) = self.task_boards.remove(&source) else {
            return;
        };
        let delay = if state.error.is_some() {
            Duration::from_secs(8)
        } else {
            Duration::from_secs(2)
        };
        if !state.loading && !state.busy && state.last_read.elapsed() >= delay {
            self.refresh_tasks(&mut state);
        }
        self.task_boards.insert(source, state);
    }

    fn submit_task_form(&mut self, state: &mut TaskBoardState) {
        if state.busy {
            return;
        }
        let Some(command) = state
            .form
            .as_mut()
            .and_then(|form| form.command(state.source))
        else {
            state.error = Some(self.i18n.text("task-board-invalid"));
            return;
        };
        let source = state.source;
        let instance = state.instance;
        state.busy = true;
        state.error = None;
        // Fence an older page read; it may not replace the mutation result.
        state.request = Uuid::new_v4();
        state.loading = false;
        self.dispatch_backend_operation(
            move |backend| async move {
                let outcome = match tokio::time::timeout(REQUEST_TIMEOUT, command.run(backend.client())).await {
                    Ok(Ok(task)) => Ok(task),
                    Ok(Err(error)) => {
                        let conflict = error.problem().is_some_and(|problem| problem.category == agena_failure::FailureCategory::Conflict);
                        Err((error.user_message(), conflict))
                    }
                    Err(_) => Err(("Request timed out; the server may have accepted it. Retry the same command or inspect the task before changing it.".into(), false)),
                };
                Ok::<_, anyhow::Error>(outcome)
            },
            move |app, result| {
                let Some(state) = app.task_boards.get_mut(&source) else { return; };
                if state.instance != instance { return; }
                state.busy = false;
                match result {
                    Ok(Ok(task)) => {
                        state.form = None;
                        state.error = None;
                        if let Some(index) = state.items.iter().position(|old| old.id == task.id) {
                            state.items[index] = task; state.selected = index;
                        } else {
                            state.items.insert(0, task); state.items.truncate(PAGE_SIZE); state.selected = 0;
                        }
                        state.last_read = Instant::now() - Duration::from_secs(10);
                    }
                    Ok(Err((error, conflict))) => {
                        state.error = Some(error);
                        if let Some(form) = &mut state.form { form.conflict = conflict; }
                        state.last_read = Instant::now() - Duration::from_secs(10);
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            },
        );
    }

    pub(crate) fn handle_tasks_key(&mut self, key: KeyEvent, source: i64) -> bool {
        let Some(mut state) = self.task_boards.remove(&source) else {
            return true;
        };
        let mut close = false;
        if state.form.is_some() {
            if key.code == KeyCode::Esc && !state.busy {
                state.form = None;
                state.error = None;
            } else if key.code == KeyCode::Enter
                && key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                self.submit_task_form(&mut state);
            } else if key.code == KeyCode::Char('r')
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && !state.busy
            {
                let latest = state.selected().cloned();
                if let (Some(form), Some(task)) = (&mut state.form, latest)
                    && form.conflict
                    && form.target.as_ref().is_some_and(|old| old.id == task.id)
                {
                    *form = TaskForm::new(form.kind, Some(task), form.editor.text().to_owned());
                    state.error = None;
                }
            } else if state.accepts_text() {
                if let Some(form) = &mut state.form {
                    form.editor.handle_multiline_input_key(key);
                }
            } else if key.code == KeyCode::Enter && !state.busy {
                self.submit_task_form(&mut state);
            }
        } else {
            match key.code {
                KeyCode::Esc => close = true,
                KeyCode::Up | KeyCode::Char('k') => {
                    state.selected = state.selected.saturating_sub(1);
                    state.detail_scroll = 0;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    state.selected = (state.selected + 1).min(state.items.len().saturating_sub(1));
                    state.detail_scroll = 0;
                }
                KeyCode::PageDown => state.detail_scroll = state.detail_scroll.saturating_add(8),
                KeyCode::PageUp => state.detail_scroll = state.detail_scroll.saturating_sub(8),
                KeyCode::Home => {
                    state.selected = 0;
                    state.detail_scroll = 0;
                }
                KeyCode::End => {
                    state.selected = state.items.len().saturating_sub(1);
                    state.detail_scroll = 0;
                }
                KeyCode::Char('r') => self.refresh_tasks(&mut state),
                KeyCode::Char('n') if source > 0 && !state.busy => {
                    state.form = Some(TaskForm::new(FormKind::Create, None, String::new()))
                }
                KeyCode::Char('c' | 'u' | 'a') if !state.busy => {
                    if let Some(task) = state.selected().cloned() {
                        let kind = match key.code {
                            KeyCode::Char('c') if can_cancel(&task) => Some(FormKind::Cancel),
                            KeyCode::Char('u') if task.state.can_resume() => Some(FormKind::Resume),
                            KeyCode::Char('a') if task.state == TaskState::Review => {
                                Some(FormKind::Accept)
                            }
                            _ => None,
                        };
                        if let Some(kind) = kind {
                            state.form = Some(TaskForm::new(kind, Some(task), String::new()));
                            state.error = None;
                        }
                    }
                }
                KeyCode::Enter | KeyCode::Char('o') => {
                    if let Some(task) = state.selected() {
                        let id = if key.code == KeyCode::Enter {
                            task.worker_session_id
                        } else {
                            Some(task.source_session_id)
                        };
                        if let Some(id) = id {
                            self.open_session(id, task.goal.chars().take(70).collect());
                            self.focus = Focus::Transcript;
                            close = true;
                        }
                    }
                }
                KeyCode::Char('g') if source > 0 && !state.busy => {
                    state.only_source = !state.only_source;
                    state.page = 0;
                    state.selected = 0;
                    state.items.clear();
                    state.loading = false;
                    self.refresh_tasks(&mut state);
                }
                KeyCode::Char('f') if !state.busy => {
                    state.filter = match state.filter {
                        None => Some(TaskState::Review),
                        Some(TaskState::Review) => Some(TaskState::Waiting),
                        Some(TaskState::Waiting) => Some(TaskState::Running),
                        Some(TaskState::Running) => Some(TaskState::Failed),
                        Some(TaskState::Failed) => Some(TaskState::Interrupted),
                        Some(TaskState::Interrupted) => Some(TaskState::Completed),
                        _ => None,
                    };
                    state.page = 0;
                    state.items.clear();
                    state.selected = 0;
                    state.loading = false;
                    self.refresh_tasks(&mut state);
                }
                KeyCode::Char('[') if state.page > 0 && !state.loading && !state.busy => {
                    state.page -= 1;
                    state.selected = 0;
                    self.refresh_tasks(&mut state);
                }
                KeyCode::Char(']')
                    if state.has_more && state.page < 500 && !state.loading && !state.busy =>
                {
                    state.page += 1;
                    state.selected = 0;
                    self.refresh_tasks(&mut state);
                }
                _ => {}
            }
        }
        self.task_boards.insert(source, state);
        close
    }

    pub(crate) fn render_tasks(&self, frame: &mut Frame, area: Rect, source: i64) {
        if let Some(state) = self.task_boards.get(&source) {
            render_board(frame, area, state, &self.i18n);
        }
    }
}

fn label(i18n: &I18n, key: &str) -> String {
    i18n.text(&format!("task-board-{key}"))
}
fn state_label(i18n: &I18n, state: TaskState) -> String {
    i18n.text(&format!("task-state-{}", state.as_str()))
}

fn render_board(frame: &mut Frame, area: Rect, state: &TaskBoardState, i18n: &I18n) {
    if area.is_empty() {
        return;
    }
    let scope = if state.only_source {
        format!("{} #{}", label(i18n, "currentSession"), state.source)
    } else {
        label(i18n, "allTasks")
    };
    let title = format!(
        " {} · {} · {} {} ",
        label(i18n, "title"),
        scope,
        state.page + 1,
        state
            .filter
            .map(|s| state_label(i18n, s))
            .unwrap_or_else(|| label(i18n, "allStates"))
    );
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let sections = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(inner);
    let status = state.error.clone().unwrap_or_else(|| {
        if state.busy {
            label(i18n, "saving")
        } else if state.loading {
            label(i18n, "loading")
        } else {
            label(i18n, "serial")
        }
    });
    frame.render_widget(
        Paragraph::new(status)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(if state.error.is_some() {
                Color::Red
            } else {
                Color::DarkGray
            })),
        sections[0],
    );
    let help = if state.form.is_some() {
        label(i18n, "formKeys")
    } else if inner.width < 85 {
        label(i18n, "compactKeys")
    } else {
        label(i18n, "keys")
    };
    frame.render_widget(Paragraph::new(help).wrap(Wrap { trim: false }), sections[2]);
    if let Some(form) = &state.form {
        let title = match form.kind {
            FormKind::Create => "create",
            FormKind::Resume => "resume",
            FormKind::Cancel => "cancel",
            FormKind::Accept => "accept",
        };
        let areas = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(sections[1]);
        let prompt = form
            .target
            .as_ref()
            .map(|task| task.goal.as_str())
            .unwrap_or("");
        frame.render_widget(
            Paragraph::new(prompt)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(label(i18n, title)),
                )
                .wrap(Wrap { trim: false }),
            areas[0],
        );
        if matches!(form.kind, FormKind::Create | FormKind::Resume) {
            let spec = agena_tui_components::EditorPanelSpec {
                title: Some(
                    label(
                        i18n,
                        if form.kind == FormKind::Create {
                            "goal"
                        } else {
                            "feedback"
                        },
                    )
                    .into(),
                ),
                borders: Borders::ALL,
            };
            let output = agena_tui_components::render_wrapped_editor_panel(
                frame,
                areas[1],
                &spec,
                &form.editor,
            );
            if !state.busy && form.submitted.is_none() {
                frame.set_cursor_position(output.cursor);
            }
        } else {
            frame.render_widget(
                Paragraph::new(label(
                    i18n,
                    if form.kind == FormKind::Cancel {
                        "confirmCancel"
                    } else {
                        "confirmAccept"
                    },
                ))
                .wrap(Wrap { trim: false }),
                areas[1],
            );
        }
        let hint = if form.conflict {
            "conflict"
        } else if form.submitted.is_some() && state.error.is_some() {
            "uncertain"
        } else {
            "formHint"
        };
        frame.render_widget(
            Paragraph::new(label(i18n, hint)).wrap(Wrap { trim: false }),
            areas[2],
        );
        return;
    }
    let panes = Layout::default()
        .direction(if inner.width >= 85 {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(sections[1]);
    let rows: Vec<ListItem> = state
        .items
        .iter()
        .map(|task| {
            // A narrow terminal can leave only two content rows for the list.
            // Multi-line ListItems would disappear completely in that space.
            if inner.width < 85 {
                return ListItem::new(Line::from(format!(
                    "{} · {}",
                    state_label(i18n, task.state),
                    task.goal.lines().next().unwrap_or("")
                )));
            }
            ListItem::new(vec![
                Line::from(Span::styled(
                    state_label(i18n, task.state),
                    Style::default().fg(
                        if matches!(
                            task.state,
                            TaskState::Waiting
                                | TaskState::Review
                                | TaskState::Failed
                                | TaskState::Interrupted
                        ) {
                            Color::Yellow
                        } else {
                            Color::Cyan
                        },
                    ),
                )),
                Line::from(task.goal.lines().next().unwrap_or("").to_owned()),
                Line::from(""),
            ])
        })
        .collect();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(label(i18n, if state.loading { "loading" } else { "empty" }))
                .wrap(Wrap { trim: false }),
            panes[0],
        );
    } else {
        let list = List::new(rows)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(label(i18n, "list")),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("› ");
        let mut selection = ListState::default().with_selected(Some(state.selected));
        frame.render_stateful_widget(list, panes[0], &mut selection);
    }
    let detail = match state.selected() {
        Some(task) => format!(
            "{}\n{}\n\n{}\n\n{}: {}\n{}: #{}\n{}: {}\n\n{}\n\n{}",
            state_label(i18n, task.state),
            task.id,
            task.goal,
            label(i18n, "attempt"),
            task.attempt,
            label(i18n, "source"),
            task.source_session_id,
            label(i18n, "worker"),
            task.worker_session_id
                .map(|id| format!("#{id}"))
                .unwrap_or_else(|| "—".into()),
            task.note.as_deref().unwrap_or(""),
            if task.state == TaskState::Review {
                label(i18n, "reviewHint")
            } else if task.state == TaskState::Waiting {
                label(i18n, "waitingHint")
            } else {
                task.instruction.clone()
            }
        ),
        None => label(i18n, "select"),
    };
    frame.render_widget(
        Paragraph::new(detail)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(label(i18n, "details")),
            )
            .wrap(Wrap { trim: false })
            .scroll((state.detail_scroll, 0)),
        panes[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(key: &str) -> Task {
        Task::new(
            CreateTask {
                source_session_id: 7,
                goal: "验证中文任务".into(),
                request_key: key.into(),
            },
            1,
            0,
        )
        .unwrap()
    }

    #[test]
    fn retries_keep_exact_payload_and_unicode_drafts() {
        let mut form = TaskForm::new(FormKind::Create, None, "测试 👩‍💻\n保留输入".into());
        let first = form.command(7).unwrap();
        form.editor
            .set_text("edited after uncertain receipt".into());
        let second = form.command(7).unwrap();
        let (TaskCommand::Create(a), TaskCommand::Create(b)) = (first, second) else {
            panic!("create command");
        };
        assert_eq!(a.request_key, b.request_key);
        assert_eq!(a.goal, b.goal);
        assert_eq!(a.goal, "测试 👩‍💻\n保留输入");
    }

    #[test]
    fn refresh_preserves_selection_and_newer_revisions() {
        let a = task("a");
        let mut b = task("b");
        b.revision = 9;
        let mut state = TaskBoardState::new(7);
        state.items = vec![a.clone(), b.clone()];
        state.selected = 1;
        let mut stale = b.clone();
        stale.revision = 2;
        state.apply_rows(vec![stale, a]);
        assert_eq!(state.selected, 0);
        assert_eq!(state.selected().unwrap().revision, 9);
    }

    #[test]
    fn conflict_and_illegal_actions_never_submit() {
        let mut target = task("a");
        target.state = TaskState::Running;
        assert!(
            TaskForm::new(FormKind::Accept, Some(target.clone()), String::new())
                .command(7)
                .is_none()
        );
        let mut form = TaskForm::new(FormKind::Resume, Some(target), "continue".into());
        assert!(form.command(7).is_none());
        form.conflict = true;
        assert!(form.command(7).is_none());
        assert!(
            TaskForm::new(FormKind::Create, None, " ".into())
                .command(7)
                .is_none()
        );
    }

    #[test]
    fn renders_readable_board_and_editor_at_narrow_and_wide_sizes() {
        for (width, height) in [(120, 32), (60, 22), (30, 10)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            let mut state = TaskBoardState::new(7);
            state.items.push(task("render"));
            terminal
                .draw(|frame| render_board(frame, frame.area(), &state, &I18n::english()))
                .unwrap();
            state.form = Some(TaskForm::new(
                FormKind::Resume,
                state.selected().cloned(),
                "反馈\n补充测试".into(),
            ));
            terminal
                .draw(|frame| render_board(frame, frame.area(), &state, &I18n::english()))
                .unwrap();
        }
    }

    #[test]
    fn narrow_terminal_still_renders_the_selected_task_row() {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(42, 18)).unwrap();
        let mut state = TaskBoardState::new(7);
        state.items.push(task("narrow"));
        terminal
            .draw(|frame| render_board(frame, frame.area(), &state, &I18n::english()))
            .unwrap();
        let content = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            content.replace(' ', "").contains("Queued·验证中文任务"),
            "selected list item disappeared: {content}"
        );
    }
}
