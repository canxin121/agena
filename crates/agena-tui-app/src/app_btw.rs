use std::time::Instant;

use agena_api::resource::{BtwAnswer, BtwRequest};
use agena_tui::main_focus::Focus;
use agena_tui_components::{Editor, EditorPanelSpec, pointer, render_editor_panel};
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
    expanded: bool,
    exchanges: Vec<BtwExchange>,
    selected: usize,
    request: Option<BtwTask>,
    request_id: u64,
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
            expanded: true,
            exchanges: Vec::new(),
            selected: 0,
            request: None,
            request_id: 0,
            scroll: 0,
            max_scroll: 0,
            render_key: None,
            rendered: Vec::new(),
            last_used: Instant::now(),
        }
    }
}

impl App {
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
        state.expanded = true;
        state.last_used = Instant::now();
        if !question.trim().is_empty() {
            state.input = Editor::default();
            state.input.insert_str(question);
            self.submit_btw(session_id, &mut state);
        }
        self.btw_sessions.insert(session_id, state);
        self.collapse_inline_plan(session_id);
        self.collapse_session_work(session_id);
        self.current_route = Route::Main;
        self.focus = Focus::Transcript;
        self.btw_focus = Some(session_id);
    }

    fn submit_btw(&mut self, session_id: i64, state: &mut BtwState) {
        let question = state.input.text().trim().to_owned();
        if question.is_empty() || state.request.is_some() {
            return;
        }
        self.next_usage_request_id = self.next_usage_request_id.saturating_add(1);
        let request_id = self.next_usage_request_id;
        state.request_id = request_id;
        state.exchanges.push(BtwExchange {
            id: request_id,
            question: question.clone(),
            ..Default::default()
        });
        if state.exchanges.len() > HISTORY_LIMIT {
            state.exchanges.remove(0);
        }
        state.selected = state.exchanges.len() - 1;
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
        let Some(entry) = state.exchanges.last_mut() else {
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

    pub(crate) fn btw_has_focus(&self) -> bool {
        self.current_route_is_main()
            && self.overlay.is_none()
            && self.context_help.is_none()
            && self.btw_focus.is_some()
            && self.btw_focus == self.transcript.session_id
            && self
                .btw_focus
                .and_then(|id| self.btw_sessions.get(&id))
                .is_some_and(|state| state.expanded)
    }

    pub(crate) fn handle_btw_input(&mut self, key: KeyEvent) -> bool {
        if !self.btw_has_focus() {
            return false;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => {
                self.btw_focus = None;
                self.focus = if key.code == KeyCode::Esc {
                    Focus::Transcript
                } else {
                    Focus::Composer
                };
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.handle_btw_action("btw-stop")
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                if let Some(state) = self.btw_focus.and_then(|id| self.btw_sessions.get_mut(&id)) {
                    state.input.insert_explicit_newline();
                }
            }
            KeyCode::Enter => self.handle_btw_action("btw-send"),
            KeyCode::PageUp => self.handle_btw_action("btw-scroll-up"),
            KeyCode::PageDown => self.handle_btw_action("btw-scroll-down"),
            _ => {
                if let Some(state) = self.btw_focus.and_then(|id| self.btw_sessions.get_mut(&id)) {
                    state.input.handle_multiline_input_key(key);
                }
            }
        }
        true
    }

    pub(crate) fn handle_btw_action(&mut self, action: &str) {
        let Some(id) = self.transcript.session_id else {
            return;
        };
        let Some(mut state) = self.btw_sessions.remove(&id) else {
            return;
        };
        match action {
            "btw-clear" => {
                self.btw_focus = None;
                return;
            }
            "btw-toggle" => {
                state.expanded = !state.expanded;
                if state.expanded {
                    self.collapse_inline_plan(id);
                    self.collapse_session_work(id);
                }
                if !state.expanded {
                    self.btw_focus = None;
                }
            }
            "btw-focus" => {
                self.btw_focus = Some(id);
                self.focus = Focus::Transcript;
            }
            "btw-leave" => {
                self.btw_focus = None;
                self.focus = Focus::Composer;
            }
            "btw-send" => self.submit_btw(id, &mut state),
            "btw-stop" => {
                if state.request.take().is_some()
                    && let Some(entry) = state.exchanges.last_mut()
                {
                    entry.stopped = true;
                }
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

    pub(crate) fn btw_inline_height(&self, available: u16) -> u16 {
        let Some(state) = self
            .transcript
            .session_id
            .and_then(|id| self.btw_sessions.get(&id))
        else {
            return 0;
        };
        if state.expanded && available >= 7 {
            available.min(13)
        } else {
            available.min(1)
        }
    }

    pub(crate) fn collapse_btw(&mut self, session_id: i64) {
        if let Some(state) = self.btw_sessions.get_mut(&session_id) {
            state.expanded = false;
        }
    }

    pub(crate) fn render_btw_inline(&mut self, frame: &mut Frame, area: Rect) {
        self.btw_area = area;
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
        let toggle = format!(
            "{} BTW {}/{}",
            if state.expanded { "▾" } else { "▸" },
            if state.exchanges.is_empty() {
                0
            } else {
                state.selected + 1
            },
            state.exchanges.len()
        );
        let clear = self.i18n.text("btw-clear");
        let header = Rect::new(area.x, area.y, area.width, 1);
        pointer::render_buttons(
            frame,
            header,
            &[
                (&toggle, pointer::PointerAction::Named("btw-toggle")),
                ("←", pointer::PointerAction::Named("btw-older")),
                ("→", pointer::PointerAction::Named("btw-newer")),
                (&clear, pointer::PointerAction::Named("btw-clear")),
            ],
        );
        if !state.expanded || area.height < 7 {
            return;
        }
        let input_area = Rect::new(area.x, area.bottom() - 4, area.width, 3);
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
            if state.request.is_some() && entry.id == state.request_id {
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
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{I18n, LaunchOptions, TuiBackend};
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    fn app() -> App {
        let mut app = App::new_with_backend(
            TuiBackend::remote_mock(),
            LaunchOptions::default(),
            I18n::english(),
        );
        app.transcript.session_id = Some(7);
        app
    }

    fn running(app: &mut App, session_id: i64, request_id: u64) -> tokio::task::AbortHandle {
        let task = tokio::spawn(std::future::pending());
        let abort = task.abort_handle();
        let state = app.btw_sessions.entry(session_id).or_default();
        state.request_id = request_id;
        state.request = Some(BtwTask(task));
        state.exchanges.push(BtwExchange {
            id: request_id,
            question: "A question".into(),
            ..Default::default()
        });
        state.selected = state.exchanges.len() - 1;
        abort
    }

    fn answer(text: &str, done: bool) -> BtwAnswer {
        BtwAnswer {
            text: text.into(),
            done,
            error: None,
        }
    }

    fn click_action(app: &mut App, terminal: &mut Terminal<TestBackend>, name: &'static str) {
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let mut target = None;
        for y in 0..terminal.backend().buffer().area.height {
            for x in 0..terminal.backend().buffer().area.width {
                let mouse = MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: x,
                    row: y,
                    modifiers: KeyModifiers::NONE,
                };
                if app.pointer_targets.action(mouse) == Some(pointer::PointerAction::Named(name)) {
                    target = Some(mouse);
                }
            }
        }
        app.handle_mouse_event(target.expect("visible clickable BTW action"));
    }

    #[tokio::test]
    async fn inline_question_does_not_replace_chat_or_own_the_main_draft() {
        let mut app = app();
        app.composer.insert_str("main draft");
        app.open_btw("");
        assert!(matches!(app.current_route, Route::Main));
        app.handle_paste("side draft".into());
        assert_eq!(app.btw_sessions[&7].input.text(), "side draft");
        assert_eq!(app.composer.text(), "main draft");
        let abort = running(&mut app, 7, 1);
        app.handle_btw_updated(7, 1, answer("partial", false));
        app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
        assert!(!app.should_quit);
        assert!(app.last_ctrl_c_at.is_none());
        assert_eq!(app.btw_sessions[&7].exchanges[0].answer, "partial");
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(!app.btw_has_focus());
        assert_eq!(app.focus, Focus::Composer);
        app.handle_paste(" continued".into());
        assert_eq!(app.composer.text(), "main draft continued");
    }

    #[tokio::test]
    async fn navigation_keeps_background_requests_and_late_answers_with_their_owner() {
        let mut app = app();
        app.open_btw("");
        let first = running(&mut app, 7, 1);
        app.open_session(9, "Other session".into());
        assert!(!first.is_finished());
        assert!(!app.btw_has_focus());
        app.open_btw("");
        let second = running(&mut app, 9, 2);
        app.handle_btw_updated(7, 1, answer("first answer", true));
        assert_eq!(app.btw_sessions[&7].exchanges[0].answer, "first answer");
        assert!(app.btw_sessions[&9].exchanges[0].answer.is_empty());
        app.handle_btw_action("btw-clear");
        tokio::task::yield_now().await;
        assert!(second.is_finished());
        app.open_btw("");
        app.handle_btw_updated(9, 2, answer("stale", true));
        assert!(app.btw_sessions[&9].exchanges.is_empty());
        app.open_session(7, "Original session".into());
        assert_eq!(app.btw_sessions[&7].exchanges[0].answer, "first answer");
    }

    #[tokio::test]
    async fn inline_layout_and_pointer_controls_preserve_chat_at_different_sizes() {
        let mut app = app();
        app.composer.insert_str("MAIN DRAFT IS VISIBLE");
        app.open_btw("");
        running(&mut app, 7, 1);
        let markdown = "## Heading\n\n**Strong** and `code`\n\n| Key | Value |\n| --- | --- |\n| one | two |\n\n```rust\nlet answer = 42;\n```\n";
        app.handle_btw_updated(7, 1, answer(markdown, true));
        for (width, height) in [(100, 30), (70, 24), (40, 20)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            assert!(app.layout.transcript_body.height > 0);
            assert!(app.layout.transcript_body.bottom() <= app.btw_area.y);
            assert!(app.btw_area.bottom() < height);
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(
                text.contains("MAIN DRAFT IS VISIBLE"),
                "{width}x{height}: main editor is visible"
            );
            assert!(text.contains("BTW"));
            let state = &app.btw_sessions[&7];
            let rich_text = state
                .rendered
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(rich_text.contains("Heading") && !rich_text.contains("## Heading"));
            assert!(rich_text.contains("Key") && rich_text.contains("Value"));
            assert!(
                state
                    .rendered
                    .iter()
                    .flat_map(|line| &line.spans)
                    .any(|span| span.content.contains("Strong")
                        && span
                            .style
                            .add_modifier
                            .contains(ratatui::style::Modifier::BOLD))
            );
            assert!(
                state
                    .rendered
                    .iter()
                    .flat_map(|line| &line.spans)
                    .any(|span| span.content.contains("let") && span.style.fg.is_some())
            );
            click_action(&mut app, &mut terminal, "btw-toggle");
            assert!(!app.btw_sessions[&7].expanded);
            assert!(matches!(app.current_route, Route::Main));
            click_action(&mut app, &mut terminal, "btw-toggle");
        }
    }

    #[tokio::test]
    async fn collapse_and_reopen_keep_the_request_and_scroll_belongs_to_btw() {
        let mut app = app();
        app.open_btw("");
        let task = running(&mut app, 7, 1);
        app.handle_btw_updated(
            7,
            1,
            answer(
                &(0..100)
                    .map(|n| format!("- **item {n}**\n"))
                    .collect::<String>(),
                false,
            ),
        );
        let mut terminal = Terminal::new(TestBackend::new(90, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.handle_key_event(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert!(app.btw_sessions[&7].scroll > 0);
        click_action(&mut app, &mut terminal, "btw-toggle");
        assert!(!task.is_finished());
        app.open_btw("");
        assert_eq!(app.btw_sessions[&7].request_id, 1);
        assert_eq!(app.btw_sessions[&7].exchanges.len(), 1);
        app.handle_btw_updated(7, 1, answer("complete", true));
        assert_eq!(app.btw_sessions[&7].exchanges[0].answer, "complete");
    }
}
