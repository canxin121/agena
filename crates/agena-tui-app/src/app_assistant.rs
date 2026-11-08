//! Optional operator workbench. Natural conversation remains the primary way
//! to set persona, remember/forget and schedule proactive contact.
use crate::{App, Route};
use agena_api::assistant::*;
use agena_tui::main_focus::Focus;
use agena_tui_components::Editor;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone)]
enum EditKind {
    Field(usize),
    Memory(AssistantMemory),
    DeleteMemory(AssistantMemory),
    Reminder(Reminder, ReminderControlAction),
}
#[derive(Debug, Clone)]
struct AssistantEdit {
    kind: EditKind,
    editor: Editor,
}
#[derive(Debug, Clone)]
pub(crate) struct AssistantState {
    source: i64,
    identity: Uuid,
    request: Uuid,
    snapshot: Option<AssistantSnapshot>,
    draft: Option<AssistantPreferences>,
    draft_base: Option<AssistantPreferences>,
    draft_revision: i64,
    reconfirm_on_load: bool,
    tab: usize,
    index: usize,
    loading: bool,
    busy: bool,
    last_read: Instant,
    error: Option<String>,
    notice: Option<String>,
    edit: Option<AssistantEdit>,
    profile_request: Option<AssistantUpdate>,
}
impl AssistantState {
    fn new(source: i64) -> Self {
        Self {
            source,
            identity: Uuid::new_v4(),
            request: Uuid::new_v4(),
            snapshot: None,
            draft: None,
            draft_base: None,
            draft_revision: 0,
            reconfirm_on_load: false,
            tab: 0,
            index: 0,
            loading: false,
            busy: false,
            last_read: Instant::now(),
            error: None,
            notice: None,
            edit: None,
            profile_request: None,
        }
    }
    fn dirty(&self) -> bool {
        self.draft_base
            .as_ref()
            .zip(self.draft.as_ref())
            .is_some_and(|(base, draft)| base != draft)
    }
    fn count(&self) -> usize {
        match self.tab {
            0 => 14,
            1 => self.snapshot.as_ref().map_or(0, |s| s.memories.len()),
            _ => self.snapshot.as_ref().map_or(0, |s| s.reminders.len()),
        }
    }
    pub(crate) fn accepts_text(&self) -> bool {
        !self.busy
            && self
                .edit
                .as_ref()
                .is_some_and(|edit| matches!(edit.kind, EditKind::Field(_) | EditKind::Memory(_)))
    }
    pub(crate) fn paste(&mut self, text: &str) {
        if self.accepts_text()
            && let Some(edit) = &mut self.edit
            && edit.editor.text().len() + text.len() <= 16384
        {
            edit.editor.insert_str(text);
        }
    }
}
const FIELDS: [&str; 14] = [
    "name",
    "address",
    "tone",
    "language",
    "instructions",
    "memoryRecall",
    "proactive",
    "timezone",
    "quiet",
    "quietStart",
    "quietEnd",
    "daily",
    "cooldown",
    "scope",
];
fn time_text(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}
fn field_value(p: &AssistantPreferences, index: usize) -> String {
    match index {
        0 => p.name.clone(),
        1 => p.address_user.clone(),
        2 => format!("{:?}", p.tone),
        3 => p.language.clone(),
        4 => p.instructions.clone(),
        5 => p.memory_enabled.to_string(),
        6 => p.proactive_enabled.to_string(),
        7 => p.timezone.clone(),
        8 => p.quiet_enabled.to_string(),
        9 => time_text(p.quiet_start_minute),
        10 => time_text(p.quiet_end_minute),
        11 => p.daily_limit.to_string(),
        12 => p.cooldown_seconds.to_string(),
        _ => "workspace".into(),
    }
}
fn parse_time(text: &str) -> Option<u16> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m) = (h.parse::<u16>().ok()?, m.parse::<u16>().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}
fn set_field(p: &mut AssistantPreferences, index: usize, text: &str) -> Result<(), String> {
    let mut next = p.clone();
    match index {
        0 => next.name = text.trim().into(),
        1 => next.address_user = text.trim().into(),
        3 => next.language = text.trim().into(),
        4 => next.instructions = text.into(),
        7 => next.timezone = text.trim().into(),
        9 => next.quiet_start_minute = parse_time(text).ok_or("HH:MM")?,
        10 => next.quiet_end_minute = parse_time(text).ok_or("HH:MM")?,
        11 => next.daily_limit = text.trim().parse().map_err(|_| "1–24")?,
        12 => next.cooldown_seconds = text.trim().parse().map_err(|_| "60–86400")?,
        _ => return Err("not an editable text field".into()),
    }
    next.validate().map_err(|e| e.to_string())?;
    *p = next;
    Ok(())
}
fn patch(base: &AssistantPreferences, p: &AssistantPreferences) -> AssistantPatch {
    let mut delta = AssistantPatch::default();
    macro_rules! strings{($($field:ident),*)=>{$(if base.$field!=p.$field{delta.$field=Some(p.$field.clone());})*};}
    macro_rules! values{($($field:ident),*)=>{$(if base.$field!=p.$field{delta.$field=Some(p.$field);})*};}
    strings!(name, address_user, language, instructions, timezone);
    values!(
        tone,
        memory_enabled,
        proactive_enabled,
        quiet_enabled,
        quiet_start_minute,
        quiet_end_minute,
        daily_limit,
        cooldown_seconds
    );
    delta
}

impl App {
    pub(crate) fn open_assistant(&mut self) {
        let active = self.current_or_selected_session_id().unwrap_or(0);
        let source = self
            .task_boards
            .values()
            .flat_map(|b| b.items.iter())
            .find(|t| t.worker_session_id == Some(active))
            .map_or(active, |t| t.source_session_id);
        if source <= 0 {
            self.flash_warning(self.i18n.text("assistant-noSession"));
            return;
        }
        let mut state = self
            .assistant_panels
            .remove(&source)
            .unwrap_or_else(|| AssistantState::new(source));
        self.route_stack.clear();
        self.current_route = Route::Assistant(source);
        self.focus = Focus::Transcript;
        self.refresh_assistant(&mut state);
        self.assistant_panels.insert(source, state);
    }
    fn refresh_assistant(&mut self, state: &mut AssistantState) {
        if state.loading || state.busy {
            return;
        }
        let source = state.source;
        let identity = state.identity;
        let request = Uuid::new_v4();
        state.request = request;
        state.loading = true;
        state.last_read = Instant::now();
        self.dispatch_backend_operation(
            move |backend| async move {
                Ok::<_, anyhow::Error>(
                    tokio::time::timeout(
                        Duration::from_secs(20),
                        backend.client().assistant_snapshot(source),
                    )
                    .await??,
                )
            },
            move |app, result| {
                let Some(state) = app.assistant_panels.get_mut(&source) else {
                    return;
                };
                if state.identity != identity || state.request != request {
                    return;
                }
                state.loading = false;
                state.last_read = Instant::now();
                match result {
                    Ok(snapshot) => {
                        if state.reconfirm_on_load {
                            let mut next = snapshot.profile.preferences.clone();
                            let reconciled = if let (Some(base), Some(draft)) =
                                (&state.draft_base, &state.draft)
                            {
                                patch(base, draft).apply(&mut next)
                            } else {
                                Ok(())
                            };
                            match reconciled {
                                Ok(()) => {
                                    state.draft = Some(next);
                                    state.draft_base = Some(snapshot.profile.preferences.clone());
                                    state.draft_revision = snapshot.profile.revision;
                                    state.error = None;
                                }
                                Err(error) => state.error = Some(error.to_string()),
                            }
                            state.reconfirm_on_load = false;
                        } else if !state.dirty() && state.profile_request.is_none() {
                            state.draft = Some(snapshot.profile.preferences.clone());
                            state.draft_base = Some(snapshot.profile.preferences.clone());
                            state.draft_revision = snapshot.profile.revision;
                        }
                        state.snapshot = Some(snapshot);
                        state.index = state.index.min(state.count().saturating_sub(1));
                    }
                    Err(e) => state.error = Some(e.to_string()),
                }
            },
        );
    }
    pub(crate) fn tick_assistant(&mut self) {
        let Route::Assistant(source) = self.current_route else {
            return;
        };
        let Some(mut state) = self.assistant_panels.remove(&source) else {
            return;
        };
        if !state.dirty()
            && state.edit.is_none()
            && state.profile_request.is_none()
            && state.last_read.elapsed() >= Duration::from_secs(5)
        {
            self.refresh_assistant(&mut state);
        }
        self.assistant_panels.insert(source, state);
    }
    fn save_assistant_profile(&mut self, state: &mut AssistantState) {
        if state.busy {
            return;
        }
        let Some(base) = &state.draft_base else {
            return;
        };
        let Some(draft) = &state.draft else {
            return;
        };
        if let Err(e) = draft.validate() {
            state.error = Some(e.to_string());
            return;
        }
        let request = state
            .profile_request
            .get_or_insert_with(|| AssistantUpdate {
                expected_revision: state.draft_revision,
                request_key: Uuid::new_v4().to_string(),
                patch: patch(base, draft),
            })
            .clone();
        let (source, identity) = (state.source, state.identity);
        state.request = Uuid::new_v4();
        state.loading = false;
        state.busy = true;
        state.error = None;
        self.dispatch_backend_operation(
            move |backend| async move {
                Ok::<_, anyhow::Error>(
                    tokio::time::timeout(
                        Duration::from_secs(20),
                        backend.client().update_assistant(source, &request),
                    )
                    .await??,
                )
            },
            move |app, result| {
                let Some(state) = app.assistant_panels.get_mut(&source) else {
                    return;
                };
                if state.identity != identity {
                    return;
                }
                state.busy = false;
                match result {
                    Ok(profile) => {
                        state.draft = Some(profile.preferences.clone());
                        state.draft_base = Some(profile.preferences.clone());
                        state.draft_revision = profile.revision;
                        if let Some(snapshot) = &mut state.snapshot {
                            snapshot.profile = profile;
                        }
                        state.profile_request = None;
                        state.notice = Some(app.i18n.text("assistant-saved"));
                    }
                    Err(e) => state.error = Some(e.to_string()),
                }
            },
        );
    }
    fn commit_assistant_edit(&mut self, state: &mut AssistantState) {
        if state.busy {
            return;
        }
        let Some(edit) = state.edit.clone() else {
            return;
        };
        if let EditKind::Field(index) = edit.kind {
            if let Some(draft) = &mut state.draft {
                match set_field(draft, index, edit.editor.text()) {
                    Ok(()) => {
                        state.edit = None;
                        state.profile_request = None;
                        state.error = None;
                    }
                    Err(e) => state.error = Some(e),
                }
            }
            return;
        }
        let (source, identity) = (state.source, state.identity);
        state.request = Uuid::new_v4();
        state.loading = false;
        state.busy = true;
        state.error = None;
        self.dispatch_backend_operation(
            move |backend| async move {
                let result = async {
                    match edit.kind {
                        EditKind::Memory(memory) => {
                            if memory.truncated || edit.editor.text().len() > 16384 {
                                return Err(anyhow::anyhow!(
                                    "memory preview is too large to edit safely"
                                ));
                            }
                            backend
                                .client()
                                .edit_assistant_memory(
                                    source,
                                    &AssistantMemoryUpdate {
                                        name: memory.name,
                                        body: Some(edit.editor.text().into()),
                                        description: Some(memory.description),
                                        memory_type: memory.memory_type,
                                        expected_sha256: Some(memory.sha256),
                                    },
                                )
                                .await?;
                        }
                        EditKind::DeleteMemory(memory) => {
                            backend
                                .client()
                                .edit_assistant_memory(
                                    source,
                                    &AssistantMemoryUpdate {
                                        name: memory.name,
                                        body: None,
                                        description: None,
                                        memory_type: None,
                                        expected_sha256: Some(memory.sha256),
                                    },
                                )
                                .await?;
                        }
                        EditKind::Reminder(reminder, action) => {
                            backend
                                .client()
                                .control_assistant_reminder(
                                    source,
                                    reminder.id,
                                    &ReminderControl {
                                        expected_revision: reminder.revision,
                                        action,
                                    },
                                )
                                .await?;
                        }
                        EditKind::Field(_) => {}
                    }
                    Ok::<(), anyhow::Error>(())
                };
                tokio::time::timeout(Duration::from_secs(20), result).await?
            },
            move |app, result| {
                let Some(mut state) = app.assistant_panels.remove(&source) else {
                    return;
                };
                if state.identity != identity {
                    app.assistant_panels.insert(source, state);
                    return;
                }
                state.busy = false;
                match result {
                    Ok(()) => {
                        state.edit = None;
                        state.notice = Some(app.i18n.text("assistant-saved"));
                        app.refresh_assistant(&mut state);
                    }
                    Err(e) => state.error = Some(e.to_string()),
                }
                app.assistant_panels.insert(source, state);
            },
        );
    }
    pub(crate) fn handle_assistant_key(&mut self, key: KeyEvent, source: i64) -> bool {
        let Some(mut state) = self.assistant_panels.remove(&source) else {
            return true;
        };
        let mut close = false;
        if state.edit.is_some() {
            if key.code == KeyCode::Esc && !state.busy {
                state.edit = None;
                state.error = None;
            } else if key.code == KeyCode::Enter
                && (key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
                    || !state.accepts_text())
            {
                self.commit_assistant_edit(&mut state);
            } else if state.accepts_text()
                && let Some(edit) = &mut state.edit
            {
                edit.editor.handle_multiline_input_key(key);
            }
        } else {
            match key.code {
                KeyCode::Esc => close = true,
                KeyCode::Char('1' | '2' | '3') => {
                    if let KeyCode::Char(c) = key.code {
                        state.tab = (c as usize) - ('1' as usize);
                        state.index = 0;
                    }
                }
                KeyCode::Tab => {
                    state.tab = (state.tab + 1) % 3;
                    state.index = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => state.index = state.index.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    state.index = (state.index + 1).min(state.count().saturating_sub(1))
                }
                KeyCode::Char('r') if !state.busy => {
                    state.profile_request = None;
                    state.reconfirm_on_load = true;
                    self.refresh_assistant(&mut state);
                }
                KeyCode::Char('s') if state.tab == 0 => self.save_assistant_profile(&mut state),
                KeyCode::Enter | KeyCode::Char('e' | ' ') if !state.busy => {
                    if state.tab == 0
                        && state.profile_request.is_none()
                        && let Some(draft) = &mut state.draft
                    {
                        match state.index {
                            2 => {
                                draft.tone = match draft.tone {
                                    AssistantTone::Balanced => AssistantTone::Warm,
                                    AssistantTone::Warm => AssistantTone::Direct,
                                    AssistantTone::Direct => AssistantTone::Playful,
                                    AssistantTone::Playful => AssistantTone::Balanced,
                                }
                            }
                            5 => draft.memory_enabled = !draft.memory_enabled,
                            6 => draft.proactive_enabled = !draft.proactive_enabled,
                            8 => draft.quiet_enabled = !draft.quiet_enabled,
                            13 => {}
                            index => {
                                let mut editor = Editor::default();
                                editor.set_text(field_value(draft, index));
                                state.edit = Some(AssistantEdit {
                                    kind: EditKind::Field(index),
                                    editor,
                                });
                            }
                        }
                    } else if state.tab == 1
                        && let Some(memory) = state
                            .snapshot
                            .as_ref()
                            .and_then(|s| s.memories.get(state.index))
                            .cloned()
                    {
                        if memory.truncated {
                            state.error = Some(self.i18n.text("assistant-truncated"));
                        } else {
                            let mut editor = Editor::default();
                            editor.set_text(memory.body.clone());
                            state.edit = Some(AssistantEdit {
                                kind: EditKind::Memory(memory),
                                editor,
                            });
                        }
                    }
                }
                KeyCode::Char('d') if state.tab == 1 && !state.busy => {
                    if let Some(memory) = state
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.memories.get(state.index))
                        .cloned()
                    {
                        state.edit = Some(AssistantEdit {
                            kind: EditKind::DeleteMemory(memory),
                            editor: Editor::default(),
                        });
                    }
                }
                KeyCode::Char('p' | 'c') if state.tab == 2 && !state.busy => {
                    if let Some(reminder) = state
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.reminders.get(state.index))
                        .cloned()
                        && !matches!(
                            reminder.state,
                            ReminderState::Completed | ReminderState::Cancelled
                        )
                    {
                        let action = if key.code == KeyCode::Char('c') {
                            ReminderControlAction::Cancel
                        } else if reminder.state == ReminderState::Paused {
                            ReminderControlAction::Resume
                        } else {
                            ReminderControlAction::Pause
                        };
                        state.edit = Some(AssistantEdit {
                            kind: EditKind::Reminder(reminder, action),
                            editor: Editor::default(),
                        });
                    }
                }
                _ => {}
            }
        }
        self.assistant_panels.insert(source, state);
        close
    }
    pub(crate) fn render_assistant(&self, frame: &mut Frame, area: Rect, source: i64) {
        if let Some(state) = self.assistant_panels.get(&source) {
            render(frame, area, state, &self.i18n);
        }
    }
}
fn label(i18n: &agena_tui::i18n::I18n, key: &str) -> String {
    i18n.text(&format!("assistant-{key}"))
}
fn render(frame: &mut Frame, area: Rect, state: &AssistantState, i18n: &agena_tui::i18n::I18n) {
    if area.is_empty() {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", label(i18n, "title")));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let regions = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(inner);
    let tabs = ["profile", "memory", "reminders"]
        .iter()
        .enumerate()
        .map(|(i, key)| {
            format!(
                "{}{}:{}{}",
                if state.tab == i { "[" } else { " " },
                i + 1,
                label(i18n, key),
                if state.tab == i { "]" } else { " " }
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let status = state
        .error
        .clone()
        .or_else(|| state.notice.clone())
        .unwrap_or_else(|| {
            if state.busy {
                label(i18n, "saving")
            } else if state.loading {
                label(i18n, "loading")
            } else {
                String::new()
            }
        });
    frame.render_widget(
        Paragraph::new(vec![Line::from(tabs), Line::from(status)]).style(Style::default().fg(
            if state.error.is_some() {
                Color::Red
            } else {
                Color::Reset
            },
        )),
        regions[0],
    );
    frame.render_widget(
        Paragraph::new(label(
            i18n,
            if state.edit.is_some() {
                "formKeys"
            } else {
                "keys"
            },
        ))
        .wrap(Wrap { trim: false }),
        regions[2],
    );
    if let Some(edit) = &state.edit {
        match &edit.kind {
            EditKind::Field(_) | EditKind::Memory(_) => {
                let spec = agena_tui_components::EditorPanelSpec {
                    title: Some(label(i18n, "edit").into()),
                    borders: Borders::ALL,
                };
                let output = agena_tui_components::render_wrapped_editor_panel(
                    frame,
                    regions[1],
                    &spec,
                    &edit.editor,
                );
                if !state.busy {
                    frame.set_cursor_position(output.cursor);
                }
            }
            EditKind::DeleteMemory(m) => {
                frame.render_widget(
                    Paragraph::new(format!(
                        "{}\n\n{}\n\n{}",
                        label(i18n, "confirmDelete"),
                        m.name,
                        m.body
                    ))
                    .wrap(Wrap { trim: false }),
                    regions[1],
                );
            }
            EditKind::Reminder(r, action) => {
                frame.render_widget(
                    Paragraph::new(format!(
                        "{}\n\n{:?}\n{}",
                        label(i18n, "confirmSchedule"),
                        action,
                        r.spec.text
                    ))
                    .wrap(Wrap { trim: false }),
                    regions[1],
                );
            }
        }
        return;
    }
    let rows: Vec<ListItem> = match state.tab {
        0 => state
            .draft
            .as_ref()
            .map(|p| {
                FIELDS
                    .iter()
                    .enumerate()
                    .map(|(i, key)| {
                        ListItem::new(Line::from(format!(
                            "{}: {}",
                            label(i18n, key),
                            field_value(p, i).lines().next().unwrap_or("")
                        )))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        1 => state
            .snapshot
            .as_ref()
            .map(|s| {
                s.memories
                    .iter()
                    .map(|m| {
                        ListItem::new(vec![
                            Line::from(format!(
                                "{} · {}",
                                m.name,
                                m.memory_type.as_deref().unwrap_or("")
                            )),
                            Line::from(m.body.lines().next().unwrap_or("")),
                        ])
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => state
            .snapshot
            .as_ref()
            .map(|s| {
                s.reminders
                    .iter()
                    .map(|r| {
                        ListItem::new(vec![
                            Line::from(format!(
                                "{} · {}/{}",
                                label(i18n, &format!("state_{}", r.state.as_str())),
                                r.delivered,
                                r.spec.occurrences
                            )),
                            Line::from(r.spec.text.lines().next().unwrap_or("")),
                        ])
                    })
                    .collect()
            })
            .unwrap_or_default(),
    };
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(label(
                i18n,
                if state.loading {
                    "loading"
                } else if state.tab == 1 {
                    "noMemories"
                } else {
                    "noReminders"
                },
            ))
            .wrap(Wrap { trim: false }),
            regions[1],
        );
    } else {
        let mut selected = ListState::default().with_selected(Some(state.index));
        frame.render_stateful_widget(
            List::new(rows)
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› "),
            regions[1],
            &mut selected,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_patch_preserves_unrelated_concurrent_settings() {
        let original = AssistantPreferences::default();
        let mut draft = original.clone();
        draft.name = "小鱼".into();
        let mut latest = original.clone();
        latest.timezone = "Asia/Shanghai".into();
        latest.daily_limit = 2;
        let change = patch(&original, &draft);
        assert!(change.timezone.is_none());
        change.apply(&mut latest).unwrap();
        assert_eq!(latest.name, "小鱼");
        assert_eq!(latest.timezone, "Asia/Shanghai");
        assert_eq!(latest.daily_limit, 2);
    }

    #[test]
    fn profile_fields_validate_without_overwriting_on_error() {
        let mut p = AssistantPreferences::default();
        set_field(&mut p, 0, "小鱼").unwrap();
        assert_eq!(p.name, "小鱼");
        assert!(set_field(&mut p, 9, "25:99").is_err());
        assert_eq!(p.quiet_start_minute, 1320);
        assert!(set_field(&mut p, 11, "0").is_err());
    }
    #[test]
    fn native_panel_and_unicode_editor_render_at_small_widths() {
        for (w, h) in [(110, 30), (42, 18), (30, 10)] {
            let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            let mut s = AssistantState::new(1);
            s.draft = Some(AssistantPreferences::default());
            t.draw(|f| render(f, f.area(), &s, &agena_tui::i18n::I18n::english()))
                .unwrap();
            let mut editor = Editor::default();
            editor.set_text("保持中文、自然、简洁\n第二行".into());
            s.edit = Some(AssistantEdit {
                kind: EditKind::Field(4),
                editor,
            });
            t.draw(|f| render(f, f.area(), &s, &agena_tui::i18n::I18n::english()))
                .unwrap();
        }
    }
}
