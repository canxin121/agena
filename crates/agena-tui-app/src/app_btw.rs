use std::{cell::Cell, sync::Arc};

use agena_api::resource::{BtwAnswer, BtwRequest};
use agena_tui_components::{
    Editor, EditorPanelSpec, FramedSurfaceSpec, SurfaceMode, pointer, render_editor_panel,
    render_framed_surface,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Borders, Paragraph, Wrap},
};

use crate::{App, AppMessage, Route};

#[derive(Debug)]
struct BtwTask(tokio::task::JoinHandle<()>);
impl Drop for BtwTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BtwState {
    pub session_id: i64,
    pub input: Editor,
    request_id: u64,
    request: Option<Arc<BtwTask>>,
    answer: String,
    rendered_answer: Vec<ratatui::text::Line<'static>>,
    error: Option<String>,
    scroll: Cell<u16>,
    max_scroll: Cell<u16>,
}

impl BtwState {
    pub(crate) fn accepts_input(&self) -> bool {
        self.request.is_none()
    }
}

impl App {
    pub(crate) fn open_btw(&mut self, question: &str) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(self.i18n.text("flash-side-requires-session"));
            return;
        };
        let mut input = Editor::default();
        input.insert_str(question);
        let mut state = BtwState {
            session_id,
            input,
            request_id: 0,
            request: None,
            answer: String::new(),
            rendered_answer: Vec::new(),
            error: None,
            scroll: Cell::new(0),
            max_scroll: Cell::new(0),
        };
        if !question.trim().is_empty() {
            self.submit_btw(&mut state);
        }
        self.current_route = Route::Btw(state);
    }

    fn submit_btw(&mut self, state: &mut BtwState) {
        let question = state.input.text().trim().to_owned();
        if question.is_empty() || state.request.is_some() {
            return;
        }
        self.next_usage_request_id = self.next_usage_request_id.saturating_add(1);
        state.request_id = self.next_usage_request_id;
        state.answer.clear();
        state.rendered_answer.clear();
        state.error = None;
        state.scroll.set(0);
        let request_id = state.request_id;
        let session_id = state.session_id;
        let backend = self.application.clone();
        let tx = self.tx.clone();
        state.request = Some(Arc::new(BtwTask(tokio::spawn(async move {
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
                        .send(AppMessage::BtwUpdated { request_id, answer })
                        .await
                        .is_err()
                        || done
                    {
                        break;
                    }
                }
                Ok::<(), agena_client::ClientError>(())
            }
            .await;
            if let Err(error) = result {
                let _ = tx
                    .send(AppMessage::BtwUpdated {
                        request_id,
                        answer: BtwAnswer {
                            done: true,
                            error: Some(error.to_string()),
                            ..Default::default()
                        },
                    })
                    .await;
            }
        }))));
    }

    pub(crate) fn handle_btw_updated(&mut self, request_id: u64, answer: BtwAnswer) {
        let Route::Btw(state) = &mut self.current_route else {
            return;
        };
        if state.request_id != request_id || state.request.is_none() {
            return;
        }
        if (!answer.text.is_empty() || answer.error.is_none()) && state.answer != answer.text {
            state.rendered_answer = agena_tui::user_input::markdown_lines(&answer.text);
            state.answer = answer.text;
        }
        state.error = answer.error;
        if answer.done {
            state.request = None;
        }
    }

    pub(crate) fn handle_btw_key(&mut self, key: KeyEvent, state: &mut BtwState) -> bool {
        match key.code {
            KeyCode::Esc => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(task) = state.request.take() {
                    task.0.abort();
                } else {
                    return true;
                }
            }
            KeyCode::Enter => self.submit_btw(state),
            KeyCode::PageUp | KeyCode::Up => state.scroll.set(
                state
                    .scroll
                    .get()
                    .saturating_sub(if key.code == KeyCode::Up { 1 } else { 10 }),
            ),
            KeyCode::PageDown | KeyCode::Down => state.scroll.set(
                state
                    .scroll
                    .get()
                    .saturating_add(if key.code == KeyCode::Down { 1 } else { 10 })
                    .min(state.max_scroll.get()),
            ),
            _ if state.request.is_none() => {
                state.input.handle_line_input_key(key);
            }
            _ => {}
        }
        false
    }

    pub(crate) fn render_btw(&self, frame: &mut Frame, area: Rect, state: &BtwState) {
        let surface = render_framed_surface(
            frame,
            area,
            SurfaceMode::Overlay,
            &FramedSurfaceSpec {
                title: self.i18n.text("btw-title").into(),
                target_width: 110,
                target_height: area.height.saturating_sub(2),
            },
        );
        let inner = surface.inner;
        if inner.height < 4 {
            return;
        }
        let description = Rect { height: 1, ..inner };
        frame.render_widget(
            Paragraph::new(self.i18n.text("btw-description"))
                .style(agena_tui_components::theme::muted_style()),
            description,
        );
        let input = Rect::new(inner.x, inner.y + 1, inner.width, 3.min(inner.height - 1));
        let input_result = render_editor_panel(
            frame,
            input,
            &EditorPanelSpec {
                title: None,
                borders: Borders::ALL,
            },
            &state.input,
        );
        if state.request.is_none() {
            frame.set_cursor_position(input_result.cursor);
        }
        let buttons = Rect::new(
            inner.x,
            input.bottom(),
            inner.width,
            1.min(inner.bottom().saturating_sub(input.bottom())),
        );
        let send = self.i18n.text(if state.request.is_some() {
            "btw-stop"
        } else {
            "btw-send"
        });
        pointer::render_buttons(
            frame,
            buttons,
            &[(
                send.as_str(),
                if state.request.is_some() {
                    pointer::PointerAction::Key(KeyEvent::new(
                        KeyCode::Char('c'),
                        KeyModifiers::CONTROL,
                    ))
                } else {
                    pointer::key(KeyCode::Enter)
                },
            )],
        );
        let body = Rect::new(
            inner.x,
            buttons.bottom(),
            inner.width,
            inner
                .bottom()
                .saturating_sub(buttons.bottom())
                .saturating_sub(1),
        );
        let mut lines = state.rendered_answer.clone();
        if let Some(error) = &state.error {
            lines.push(ratatui::text::Line::from(error.clone()).style(
                ratatui::style::Style::default().fg(agena_tui_components::theme::danger_color()),
            ));
        }
        if state.request.is_some() {
            lines.push(
                ratatui::text::Line::from(self.i18n.text("btw-loading"))
                    .style(agena_tui_components::theme::muted_style()),
            );
        }
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let max_scroll = paragraph
            .line_count(body.width)
            .saturating_sub(usize::from(body.height))
            .min(usize::from(u16::MAX)) as u16;
        state.max_scroll.set(max_scroll);
        state.scroll.set(state.scroll.get().min(max_scroll));
        frame.render_widget(paragraph.scroll((state.scroll.get(), 0)), body);
        agena_tui_components::render_shortcut_footer(
            frame,
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            &self.i18n.text("btw-footer"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{I18n, LaunchOptions, TuiBackend};
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

    #[tokio::test]
    async fn btw_is_a_separate_input_and_ctrl_c_cannot_reach_the_parent() {
        let mut app = app();
        app.composer.insert_str("main draft");
        app.open_btw("");
        app.handle_paste("question".into());
        assert_eq!(app.composer.text(), "main draft");
        let task = tokio::spawn(std::future::pending());
        let abort = task.abort_handle();
        let Route::Btw(state) = &mut app.current_route else {
            panic!()
        };
        assert_eq!(state.input.text(), "question");
        state.request_id = 1;
        state.request = Some(Arc::new(BtwTask(task)));
        app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
        assert!(app.last_ctrl_c_at.is_none());
        assert!(!app.should_quit);
        assert_eq!(app.transcript.session_id, Some(7));
        assert_eq!(app.composer.text(), "main draft");
        app.handle_btw_updated(
            1,
            BtwAnswer {
                text: "stale".into(),
                done: true,
                error: None,
            },
        );
        let Route::Btw(state) = &app.current_route else {
            panic!()
        };
        assert!(state.answer.is_empty());
        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(app.current_route, Route::Main));
    }

    #[tokio::test]
    async fn btw_scroll_and_close_targets_belong_to_the_window() {
        let mut app = app();
        app.open_btw("");
        let Route::Btw(state) = &mut app.current_route else {
            panic!()
        };
        state.answer = (0..100).map(|n| format!("- **item {n}**\n")).collect();
        state.rendered_answer = agena_tui::user_input::markdown_lines(&state.answer);
        let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.handle_key_event(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        let Route::Btw(state) = &app.current_route else {
            panic!()
        };
        assert!(state.scroll.get() > 0);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let mut close = None;
        for y in 0..24 {
            for x in 0..90 {
                let event = crossterm::event::MouseEvent {
                    kind: crossterm::event::MouseEventKind::Down(
                        crossterm::event::MouseButton::Left,
                    ),
                    column: x,
                    row: y,
                    modifiers: KeyModifiers::NONE,
                };
                if app.pointer_targets.action(event) == Some(pointer::key(KeyCode::Esc)) {
                    close = Some(event);
                }
            }
        }
        app.handle_mouse_event(close.expect("clickable close control"));
        assert!(matches!(app.current_route, Route::Main));
        assert_eq!(app.transcript.session_id, Some(7));
    }

    #[tokio::test]
    async fn late_answers_cannot_populate_a_reopened_window() {
        let mut app = app();
        app.open_btw("");
        let Route::Btw(state) = &mut app.current_route else {
            panic!()
        };
        state.request_id = 2;
        state.request = Some(Arc::new(BtwTask(tokio::spawn(std::future::pending()))));
        app.handle_btw_updated(
            1,
            BtwAnswer {
                text: "old".into(),
                done: true,
                error: None,
            },
        );
        let Route::Btw(state) = &app.current_route else {
            panic!()
        };
        assert!(state.answer.is_empty());
        assert!(state.request.is_some());
        app.handle_btw_updated(
            2,
            BtwAnswer {
                text: "**current**".into(),
                done: true,
                error: None,
            },
        );
        let Route::Btw(state) = &app.current_route else {
            panic!()
        };
        assert_eq!(state.answer, "**current**");
        assert!(state.request.is_none());
    }
}
