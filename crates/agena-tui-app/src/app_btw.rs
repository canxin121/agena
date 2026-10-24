use std::time::Instant;

use agena_api::resource::{BtwAnswer, BtwRequest};
use agena_tui::main_focus::Focus;
use agena_tui_components::{
    Editor, EditorPanelSpec, FramedSurfaceSpec, SurfaceMode, pointer, render_editor_panel,
    render_framed_surface,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Borders, Paragraph, Wrap},
};

use crate::{App, AppMessage, Route};

const HISTORY_LIMIT: usize = 20;
const SESSION_LIMIT: usize = 32;

#[derive(Debug)]
struct BtwTask(tokio::task::JoinHandle<()>);
impl Drop for BtwTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug, Default)]
struct BtwExchange {
    id: u64,
    question: String,
    answer: String,
    error: Option<String>,
    stopped: bool,
    revision: u64,
}

#[derive(Debug)]
pub(crate) struct BtwState {
    pub input: Editor,
    exchanges: Vec<BtwExchange>,
    selected: usize,
    request: Option<BtwTask>,
    request_id: u64,
    pending_question: Option<String>,
    error: Option<String>,
    scroll: u16,
    max_scroll: u16,
    render_key: Option<(u64, u64, u16, ratatui::style::Color)>,
    rendered: Vec<Line<'static>>,
    last_used: Instant,
}

impl Default for BtwState {
    fn default() -> Self {
        Self {
            input: Editor::default(),
            exchanges: Vec::new(),
            selected: 0,
            request: None,
            request_id: 0,
            pending_question: None,
            error: None,
            scroll: 0,
            max_scroll: 0,
            render_key: None,
            rendered: Vec::new(),
            last_used: Instant::now(),
        }
    }
}

impl BtwState {
    fn restore_pending_question(&mut self) {
        if let Some(question) = self.pending_question.take() {
            let current = self.input.text();
            let draft = if current.is_empty() || current == question {
                question
            } else {
                format!("{question}\n\n{current}")
            };
            self.input = Editor::default();
            self.input.insert_str(&draft);
        }
    }
}

impl App {
    /// Open the full-screen BTW surface for the current session. A non-empty
    /// `question` submits immediately; the empty form only opens the surface.
    pub(crate) fn open_btw(&mut self, question: &str) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(self.i18n.text("flash-side-requires-session"));
            return;
        };
        if self.transcript.session_id != Some(session_id) {
            self.open_session(session_id, String::new());
        }
        if !self.btw_sessions.contains_key(&session_id) && self.btw_sessions.len() >= SESSION_LIMIT
        {
            let oldest = self
                .btw_sessions
                .iter()
                .filter(|(_, state)| {
                    state.request.is_none() && state.input.text().trim().is_empty()
                })
                .min_by_key(|(_, state)| state.last_used)
                .map(|(id, _)| *id);
            if let Some(id) = oldest {
                self.btw_sessions.remove(&id);
            }
        }
        let mut state = self.btw_sessions.remove(&session_id).unwrap_or_default();
        state.last_used = Instant::now();
        if !question.trim().is_empty() {
            state.input = Editor::default();
            state.input.insert_str(question);
            self.submit_btw(session_id, &mut state);
        }
        self.btw_sessions.insert(session_id, state);
        self.route_stack.clear();
        self.current_route = Route::Btw(session_id);
        self.focus = Focus::Transcript;
    }

    /// Leave the BTW surface and hand the keyboard back to the chat composer.
    pub(crate) fn close_btw(&mut self) {
        if matches!(self.current_route, Route::Btw(_)) {
            self.current_route = Route::Main;
        }
        self.focus = Focus::Composer;
    }

    /// Compact label for the composer's bottom-left BTW segment.
    pub(crate) fn btw_chip_label(&self) -> Option<String> {
        let session_id = self.transcript.session_id?;
        let state = self.btw_sessions.get(&session_id)?;
        if state.exchanges.is_empty() {
            return None;
        }
        Some(self.i18n.text_args(
            "composer-status-btw",
            &agena_tui::fl_args!("count" => state.exchanges.len() as i64),
        ))
    }

    fn submit_btw(&mut self, session_id: i64, state: &mut BtwState) {
        let question = state.input.text().trim().to_owned();
        if question.is_empty() || state.request.is_some() {
            return;
        }
        self.next_usage_request_id = self.next_usage_request_id.saturating_add(1);
        let request_id = self.next_usage_request_id;
        state.request_id = request_id;
        state.pending_question = Some(question.clone());
        state.error = None;
        state.input = Editor::default();
        state.scroll = 0;
        let backend = self.application.clone();
        let tx = self.tx.clone();
        state.request = Some(BtwTask(tokio::spawn(async move {
            let result = async {
                let mut stream = backend
                    .client()
                    .ask_btw(
                        session_id,
                        BtwRequest {
                            question,
                            options: Default::default(),
                        },
                    )
                    .await?;
                while let Some(answer) = stream.recv().await {
                    let answer = answer?;
                    let done = answer.done;
                    if tx
                        .send(AppMessage::BtwUpdated {
                            session_id,
                            request_id,
                            answer,
                        })
                        .await
                        .is_err()
                        || done
                    {
                        return Ok::<(), agena_client::ClientError>(());
                    }
                }
                let _ = tx
                    .send(AppMessage::BtwUpdated {
                        session_id,
                        request_id,
                        answer: BtwAnswer {
                            done: true,
                            error: Some("The BTW stream ended before completion".into()),
                            ..Default::default()
                        },
                    })
                    .await;
                Ok(())
            }
            .await;
            if let Err(error) = result {
                let _ = tx
                    .send(AppMessage::BtwUpdated {
                        session_id,
                        request_id,
                        answer: BtwAnswer {
                            done: true,
                            error: Some(error.to_string()),
                            ..Default::default()
                        },
                    })
                    .await;
            }
        })));
    }

    pub(crate) fn handle_btw_updated(
        &mut self,
        session_id: i64,
        request_id: u64,
        answer: BtwAnswer,
    ) {
        let Some(state) = self.btw_sessions.get_mut(&session_id) else {
            return;
        };
        if state.request_id != request_id || state.request.is_none() {
            return;
        }
        if let Some(question) = answer
            .question
            .or_else(|| (!answer.text.is_empty()).then(String::new))
        {
            if state
                .exchanges
                .last()
                .is_none_or(|entry| entry.id != request_id)
            {
                state.exchanges.push(BtwExchange {
                    id: request_id,
                    question,
                    ..Default::default()
                });
                if state.exchanges.len() > HISTORY_LIMIT {
                    state.exchanges.remove(0);
                }
                state.selected = state.exchanges.len() - 1;
                state.pending_question = None;
            } else if !question.is_empty() {
                state
                    .exchanges
                    .last_mut()
                    .expect("current BTW exchange")
                    .question = question;
            }
        }
        let Some(entry) = state
            .exchanges
            .last_mut()
            .filter(|entry| entry.id == request_id)
        else {
            state.error = answer.error;
            if answer.done {
                state.request = None;
                state.restore_pending_question();
            }
            return;
        };
        if (!answer.text.is_empty() || answer.error.is_none()) && entry.answer != answer.text {
            entry.answer = answer.text;
            entry.revision = entry.revision.wrapping_add(1);
        }
        entry.error = answer.error;
        if answer.done {
            state.request = None;
        }
    }

    /// Whether the BTW surface currently owns the keyboard.
    pub(crate) fn btw_has_focus(&self) -> bool {
        matches!(&self.current_route, Route::Btw(id) if Some(*id) == self.transcript.session_id)
    }

    pub(crate) fn handle_btw_input(&mut self, key: KeyEvent) -> bool {
        if !self.btw_has_focus() {
            return false;
        }
        let Some(id) = self.transcript.session_id else {
            return false;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => self.close_btw(),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.handle_btw_action("btw-stop")
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                if let Some(state) = self.btw_sessions.get_mut(&id) {
                    state.input.insert_explicit_newline();
                }
            }
            KeyCode::Enter => self.handle_btw_action("btw-send"),
            KeyCode::PageUp => self.handle_btw_action("btw-scroll-up"),
            KeyCode::PageDown => self.handle_btw_action("btw-scroll-down"),
            _ => {
                if let Some(state) = self.btw_sessions.get_mut(&id) {
                    state.input.handle_multiline_input_key(key);
                }
            }
        }
        true
    }

    /// Keys owned by the full-screen BTW route. Returns `true` when the route
    /// must close so the caller can pop back to the chat surface.
    pub(crate) fn handle_btw_route_key(&mut self, key: KeyEvent) -> bool {
        let Some(id) = self.transcript.session_id else {
            return false;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => {
                self.close_btw();
                return true;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.handle_btw_action("btw-stop");
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                if let Some(state) = self.btw_sessions.get_mut(&id) {
                    state.input.insert_explicit_newline();
                }
            }
            KeyCode::Enter => self.handle_btw_action("btw-send"),
            KeyCode::PageUp => self.handle_btw_action("btw-scroll-up"),
            KeyCode::PageDown => self.handle_btw_action("btw-scroll-down"),
            _ => {
                if let Some(state) = self.btw_sessions.get_mut(&id) {
                    state.input.handle_multiline_input_key(key);
                }
            }
        }
        false
    }

    pub(crate) fn handle_btw_action(&mut self, action: &str) {
        let Some(id) = self.transcript.session_id else {
            return;
        };
        if matches!(action, "btw-clear") {
            // Clearing drops the whole surface (and any in-flight question).
            self.btw_sessions.remove(&id);
            self.close_btw();
            return;
        }
        if matches!(action, "btw-toggle" | "btw-leave") {
            self.close_btw();
            return;
        }
        let Some(mut state) = self.btw_sessions.remove(&id) else {
            return;
        };
        match action {
            "btw-focus" => {
                self.focus = Focus::Transcript;
            }
            "btw-send" => self.submit_btw(id, &mut state),
            "btw-stop" => {
                if state.request.take().is_some()
                    && let Some(entry) = state.exchanges.last_mut()
                    && entry.id == state.request_id
                {
                    entry.stopped = true;
                }
                state.restore_pending_question();
            }
            "btw-older" => {
                state.selected = state.selected.saturating_sub(1);
                state.scroll = 0;
            }
            "btw-newer" => {
                state.selected = (state.selected + 1).min(state.exchanges.len().saturating_sub(1));
                state.scroll = 0;
            }
            "btw-scroll-up" => state.scroll = state.scroll.saturating_sub(3),
            "btw-scroll-down" => {
                state.scroll = state.scroll.saturating_add(3).min(state.max_scroll)
            }
            "btw-copy" => {
                if let Some(entry) = state.exchanges.get(state.selected)
                    && !entry.answer.is_empty()
                {
                    self.request_clipboard_copy(entry.answer.clone(), self.i18n.text("btw-copied"));
                }
            }
            _ => {}
        }
        self.btw_sessions.insert(id, state);
    }

    pub(crate) fn render_btw_panel(&mut self, frame: &mut Frame, area: Rect) {
        let surface = render_framed_surface(
            frame,
            area,
            SurfaceMode::Route,
            &FramedSurfaceSpec {
                title: self.i18n.text("btw-title").into(),
                target_width: area.width,
                target_height: area.height,
            },
        );
        let area = surface.inner;
        if area.is_empty() {
            return;
        }
        let focused = self.btw_has_focus();
        let Some(state) = self
            .transcript
            .session_id
            .and_then(|id| self.btw_sessions.get_mut(&id))
        else {
            return;
        };
        let clear = self.i18n.text("btw-clear");
        let header = Rect::new(area.x, area.y, area.width, 1);
        pointer::render_buttons(
            frame,
            header,
            &[
                ("←", pointer::PointerAction::Named("btw-older")),
                ("→", pointer::PointerAction::Named("btw-newer")),
                (&clear, pointer::PointerAction::Named("btw-clear")),
            ],
        );
        if area.height < 6 {
            return;
        }
        let input_area = Rect::new(area.x, area.bottom().saturating_sub(4), area.width, 3);
        let body = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            input_area.y.saturating_sub(area.y + 1),
        );
        let mut lines = Vec::new();
        if let Some(entry) = state.exchanges.get(state.selected) {
            let key = (
                entry.id,
                entry.revision,
                body.width,
                agena_tui_components::theme::accent_color(),
            );
            if state.render_key != Some(key) {
                state.rendered =
                    agena_tui_transcript::render_markdown_document(&entry.answer, body.width)
                        .into_iter()
                        .map(|line| {
                            line.rich_line
                                .unwrap_or_else(|| Line::from(Span::styled(line.text, line.style)))
                        })
                        .collect();
                state.render_key = Some(key);
            }
            lines.push(
                Line::from(format!("/btw {}", entry.question))
                    .style(agena_tui_components::theme::muted_style()),
            );
            lines.extend(state.rendered.iter().cloned());
            if let Some(error) = &entry.error {
                lines.push(
                    Line::from(error.clone()).style(
                        ratatui::style::Style::default()
                            .fg(agena_tui_components::theme::danger_color()),
                    ),
                );
            }
            if state.request.is_some()
                && entry.id == state.request_id
                && entry.answer.is_empty()
                && entry.error.is_none()
            {
                lines.push(
                    Line::from(self.i18n.text("btw-loading"))
                        .style(agena_tui_components::theme::muted_style()),
                );
            } else if entry.stopped {
                lines.push(
                    Line::from(self.i18n.text("btw-stopped"))
                        .style(agena_tui_components::theme::muted_style()),
                );
            }
        } else {
            lines.push(
                Line::from(self.i18n.text("btw-description"))
                    .style(agena_tui_components::theme::muted_style()),
            );
        }
        if state.request.is_some() && state.pending_question.is_some() {
            lines.push(
                Line::from(self.i18n.text("btw-loading"))
                    .style(agena_tui_components::theme::muted_style()),
            );
        }
        if let Some(error) = &state.error {
            lines.push(Line::from(error.clone()).style(
                ratatui::style::Style::default().fg(agena_tui_components::theme::danger_color()),
            ));
        }
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        state.max_scroll = paragraph
            .line_count(body.width)
            .saturating_sub(body.height as usize)
            .min(u16::MAX as usize) as u16;
        state.scroll = state.scroll.min(state.max_scroll);
        frame.render_widget(paragraph.scroll((state.scroll, 0)), body);
        pointer::register(
            body,
            Some(pointer::PointerAction::Named("btw-focus")),
            Some((
                pointer::PointerAction::Named("btw-scroll-up"),
                pointer::PointerAction::Named("btw-scroll-down"),
            )),
        );
        let input = render_editor_panel(
            frame,
            input_area,
            &EditorPanelSpec {
                title: Some(self.i18n.text("btw-input").into()),
                borders: Borders::ALL,
            },
            &state.input,
        );
        pointer::register(
            input_area,
            Some(pointer::PointerAction::Named("btw-focus")),
            None,
        );
        if focused {
            frame.set_cursor_position(input.cursor);
        }
        let action_label = self.i18n.text(if state.request.is_some() {
            "btw-stop"
        } else {
            "btw-send"
        });
        let copy = self.i18n.text("btw-copy");
        let leave = self.i18n.text("btw-leave");
        pointer::render_buttons(
            frame,
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
            &[
                (
                    &action_label,
                    pointer::PointerAction::Named(if state.request.is_some() {
                        "btw-stop"
                    } else {
                        "btw-send"
                    }),
                ),
                (&copy, pointer::PointerAction::Named("btw-copy")),
                (&leave, pointer::PointerAction::Named("btw-leave")),
            ],
        );
    }
}
