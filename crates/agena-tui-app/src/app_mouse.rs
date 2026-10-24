use crate::Route;
use std::time::{Duration, Instant};

const TRANSCRIPT_WHEEL_LINES: isize = 3;
const TRANSCRIPT_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

impl App {
    pub(crate) fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        self.clear_transcript_pending_command();
        if self.handle_surface_pointer(mouse) {
            return;
        }
        if matches!(self.current_route, Route::Hub(_)) {
            self.handle_hub_mouse(mouse);
            return;
        }

        let scrollbar = self.layout.transcript_scrollbar;
        let transcript = self.layout.transcript_body;
        match mouse.kind {
            // A wheel gesture anywhere on the main chat surface moves the
            // transcript cursor. Some multiplexers cannot preserve the original
            // pointer coordinate when forwarding a wheel event, and requiring
            // a body hit made otherwise valid events silently disappear.
            MouseEventKind::ScrollUp => {
                self.cancel_active_pointer_gesture();
                self.cancel_surface_selection();
                self.move_transcript_cursor_by_wheel(-TRANSCRIPT_WHEEL_LINES);
                self.request_older_transcript_parts_if_needed();
            }
            MouseEventKind::ScrollDown => {
                self.cancel_active_pointer_gesture();
                self.cancel_surface_selection();
                self.move_transcript_cursor_by_wheel(TRANSCRIPT_WHEEL_LINES);
            }
            // Dragging a selection that already started on a chat surface
            // (header rows, composer status row, or composer editor) keeps
            // extending that selection instead of starting a transcript
            // gesture.
            MouseEventKind::Drag(MouseButton::Left) if self.surface_selection.is_some() => {
                self.update_surface_selection(mouse.column, mouse.row);
            }
            MouseEventKind::Up(MouseButton::Left) if self.surface_selection.is_some() => {
                let editor = self.surface_layout.composer_editor;
                let click = self.surface_selection.is_some_and(|selection| {
                    selection.kind == crate::SurfaceSelectionKind::ComposerEditor
                        && selection.anchor == (mouse.column, mouse.row)
                        && selection.head == selection.anchor
                });
                if click {
                    self.composer.move_to_visual_cell(
                        editor.width,
                        editor.height,
                        mouse.column.saturating_sub(editor.x),
                        mouse.row.saturating_sub(editor.y),
                    );
                    self.cancel_surface_selection();
                } else {
                    self.update_surface_selection(mouse.column, mouse.row);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if mouse.row == transcript.y.saturating_sub(1)
                    && mouse.column >= transcript.x
                    && mouse.column < transcript.right()
                    && self.transcript.viewport_top() <= 3
                {
                    self.request_older_transcript_parts_if_needed();
                    return;
                }
                if rect_contains(self.surface_layout.composer_editor, mouse.column, mouse.row) {
                    self.focus = agena_tui::main_focus::Focus::Composer;
                }
                // Any new left-click cancels the previous surface selection.
                self.cancel_surface_selection();
                if let Some(kind) = self.surface_layout.kind_at(mouse.column, mouse.row) {
                    self.surface_selection = Some(SurfaceSelection {
                        kind,
                        anchor: (mouse.column, mouse.row),
                        head: (mouse.column, mouse.row),
                    });
                    return;
                }
                if rect_contains(scrollbar, mouse.column, mouse.row) {
                    self.transcript_pointer_gesture = None;
                    self.last_transcript_click = None;
                    self.transcript.cancel_text_selection(
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                    self.begin_transcript_scrollbar_drag(mouse.row);
                } else if rect_contains(transcript, mouse.column, mouse.row) {
                    self.transcript_scrollbar_drag = None;
                    self.begin_transcript_pointer_gesture(mouse.column, mouse.row);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.transcript_scrollbar_drag.is_some() {
                    self.drag_transcript_scrollbar(mouse.row);
                } else if self.transcript_pointer_gesture.is_some() {
                    self.update_transcript_pointer_gesture(mouse.column, mouse.row);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.transcript_scrollbar_drag.take().is_some() {
                    return;
                }
                if self.transcript_pointer_gesture.is_some() {
                    self.finish_transcript_pointer_gesture(mouse.column, mouse.row);
                }
            }
            MouseEventKind::Down(_) => {
                self.cancel_active_pointer_gesture();
                self.cancel_surface_selection();
            }
            MouseEventKind::Up(_) => {
                self.cancel_active_pointer_gesture();
                self.cancel_surface_selection();
            }
            _ => {}
        }
    }

    pub(crate) fn cancel_surface_selection(&mut self) {
        self.surface_selection = None;
    }

    fn update_surface_selection(&mut self, column: u16, row: u16) {
        if let Some(mut selection) = self.surface_selection.take() {
            selection.head = (column, row);
            self.surface_selection = Some(selection);
        }
    }

    /// Copy the active surface selection (session header rows, composer status
    /// row, or composer editor) through the OSC 52 clipboard path. Returns
    /// whether a non-empty selection was copied.
    pub(crate) fn copy_active_surface_selection(&mut self) -> bool {
        let Some(selection) = self.surface_selection else {
            return false;
        };
        let lines = self.surface_display_lines(selection.kind);
        let text = surface_selection_text(&lines, &selection);
        if text.trim().is_empty() {
            return false;
        }
        self.request_clipboard_copy(text, ui_text::t(&self.i18n, "flash-copied-mouse-selection"));
        true
    }

    pub(crate) fn cancel_active_pointer_gesture(&mut self) {
        self.transcript_scrollbar_drag = None;
        self.last_transcript_click = None;
        if self
            .transcript_pointer_gesture
            .take()
            .is_some_and(|gesture| gesture.dragged)
        {
            self.transcript.cancel_text_selection(
                self.layout.transcript_body.width,
                self.layout.transcript_body.height,
            );
        }
    }

    fn move_transcript_cursor_by_wheel(&mut self, delta: isize) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        if width == 0 || height == 0 {
            return;
        }
        let previous = self.transcript.viewport_top();
        self.transcript.move_cursor_by_wheel(width, height, delta);
        self.transcript_motion_prefix = None;
        if self.transcript.viewport_top() < previous && self.transcript_pointer_gesture.is_none() {}
    }

    fn begin_transcript_scrollbar_drag(&mut self, pointer_row: u16) {
        let Some(metrics) = self.current_transcript_scrollbar_metrics() else {
            self.transcript_scrollbar_drag = None;
            return;
        };
        let pointer_line =
            usize::from(pointer_row.saturating_sub(self.layout.transcript_scrollbar.y));
        let thumb_end = metrics.thumb_start.saturating_add(metrics.thumb_len);
        let grab_offset = if (metrics.thumb_start..thumb_end).contains(&pointer_line) {
            pointer_line.saturating_sub(metrics.thumb_start)
        } else {
            metrics.thumb_len / 2
        };
        self.transcript_scrollbar_drag = Some(TranscriptScrollbarDrag { grab_offset });
        self.transcript_motion_prefix = None;
        self.drag_transcript_scrollbar(pointer_row);
    }

    fn drag_transcript_scrollbar(&mut self, pointer_row: u16) {
        let Some(drag) = self.transcript_scrollbar_drag else {
            return;
        };
        let Some(metrics) = self.current_transcript_scrollbar_metrics() else {
            self.transcript_scrollbar_drag = None;
            return;
        };
        let pointer_line =
            usize::from(pointer_row.saturating_sub(self.layout.transcript_scrollbar.y));
        let target =
            agena_tui_transcript::scroll_for_thumb(metrics, pointer_line, drag.grab_offset);
        let previous = self.transcript.viewport_top();
        self.transcript.relocate_cursor_from_scrollbar(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            target,
        );
        if self.transcript.viewport_top() < previous {}
    }

    fn current_transcript_scrollbar_metrics(&mut self) -> Option<TranscriptScrollbarMetrics> {
        let body = self.layout.transcript_body;
        let scrollbar = self.layout.transcript_scrollbar;
        if body.width == 0 || body.height == 0 || scrollbar.height == 0 {
            return None;
        }
        let total_lines = self.transcript.rendered(body.width).lines.len();
        agena_tui_transcript::scrollbar_metrics(
            total_lines,
            usize::from(body.height),
            usize::from(scrollbar.height),
            self.transcript.viewport_top(),
        )
    }

    fn begin_transcript_pointer_gesture(&mut self, pointer_column: u16, pointer_row: u16) {
        let Some(position) = self.transcript_text_position(pointer_column, pointer_row, false)
        else {
            self.transcript_pointer_gesture = None;
            return;
        };
        self.focus = agena_tui::main_focus::Focus::Transcript;
        self.transcript_pointer_gesture = Some(TranscriptPointerGesture::new(position));
        self.transcript_motion_prefix = None;
    }

    fn update_transcript_pointer_gesture(&mut self, pointer_column: u16, pointer_row: u16) {
        let Some(mut gesture) = self.transcript_pointer_gesture.take() else {
            return;
        };
        let Some(position) = self.transcript_text_position(pointer_column, pointer_row, true)
        else {
            self.transcript_pointer_gesture = Some(gesture);
            return;
        };
        gesture.update(position, true);
        if gesture.dragged {
            self.last_transcript_click = None;
            self.transcript
                .set_text_selection(self.layout.transcript_body.width, gesture.selection());
        }
        self.transcript_pointer_gesture = Some(gesture);
    }

    fn finish_transcript_pointer_gesture(&mut self, pointer_column: u16, pointer_row: u16) {
        let Some(mut gesture) = self.transcript_pointer_gesture.take() else {
            return;
        };
        let Some(position) = self.transcript_text_position(pointer_column, pointer_row, true)
        else {
            self.last_transcript_click = None;
            self.transcript.cancel_text_selection(
                self.layout.transcript_body.width,
                self.layout.transcript_body.height,
            );
            return;
        };
        gesture.update(position, false);
        if gesture.dragged {
            self.last_transcript_click = None;
            self.transcript
                .set_text_selection(self.layout.transcript_body.width, gesture.selection());
        } else if self.transcript.focus_pointer_control(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            position,
        ) {
            self.last_transcript_click = None;
            let option = self
                .transcript
                .control_at_cursor(self.layout.transcript_body.width)
                .is_some_and(|control| {
                    matches!(
                        control.action,
                        agena_tui_transcript::TranscriptControlAction::UserInput {
                            kind: agena_tui_transcript::InteractionLineKind::AskOption { .. }
                                | agena_tui_transcript::InteractionLineKind::AskCustomRow { .. },
                            ..
                        }
                    )
                });
            self.handle_active_interaction_action(crossterm::event::KeyEvent::new(
                if option {
                    crossterm::event::KeyCode::Char(' ')
                } else {
                    crossterm::event::KeyCode::Enter
                },
                crossterm::event::KeyModifiers::NONE,
            ));
        } else {
            self.select_completed_transcript_click(position);
        }
    }

    /// Translate a pointer coordinate into the stable logical transcript
    /// coordinate system. During a drag, crossing the top or bottom edge also
    /// scrolls the viewport, allowing selections longer than one screen.
    fn transcript_text_position(
        &mut self,
        pointer_column: u16,
        pointer_row: u16,
        auto_scroll: bool,
    ) -> Option<TranscriptTextPosition> {
        let body = self.layout.transcript_body;
        if body.width == 0 || body.height == 0 {
            return None;
        }

        if auto_scroll {
            let bottom = body.y.saturating_add(body.height);
            let delta = if pointer_row < body.y {
                -isize::try_from(body.y.saturating_sub(pointer_row)).unwrap_or(isize::MAX)
            } else if pointer_row >= bottom {
                isize::try_from(pointer_row.saturating_sub(bottom).saturating_add(1))
                    .unwrap_or(isize::MAX)
            } else {
                0
            };
            if delta != 0 {
                self.transcript
                    .scroll_text_selection_viewport_by(body.width, body.height, delta);
            }
        } else if !rect_contains(body, pointer_column, pointer_row) {
            return None;
        }

        let clamped_row =
            pointer_row.clamp(body.y, body.y.saturating_add(body.height).saturating_sub(1));
        let clamped_column =
            pointer_column.clamp(body.x, body.x.saturating_add(body.width).saturating_sub(1));
        let line = self
            .transcript
            .viewport_top()
            .saturating_add(usize::from(clamped_row.saturating_sub(body.y)));
        let line_count = self.transcript.rendered(body.width).lines.len();
        if line >= line_count {
            return None;
        }
        Some(TranscriptTextPosition {
            line,
            column: usize::from(clamped_column.saturating_sub(body.x)),
        })
    }

    fn select_completed_transcript_click(&mut self, position: TranscriptTextPosition) {
        let now = Instant::now();
        let select_block =
            transcript_click_selects_block(self.last_transcript_click, position.line, now);
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        if select_block {
            self.last_transcript_click = None;
            self.transcript
                .select_pointer_block(width, height, position);
        } else {
            self.last_transcript_click = Some(TranscriptClick {
                line: position.line,
                at: now,
            });
            self.transcript.select_pointer_line(width, height, position);
        }
    }

    pub(crate) fn copy_active_transcript_text_selection(&mut self) -> bool {
        let width = self.layout.transcript_body.width;
        let Some(text) = self.transcript.selected_text(width) else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        self.request_clipboard_copy(text, ui_text::t(&self.i18n, "flash-copied-mouse-selection"));
        true
    }
}

fn transcript_click_selects_block(
    previous: Option<TranscriptClick>,
    line: usize,
    now: Instant,
) -> bool {
    previous.is_some_and(|click| {
        click.line == line
            && now.saturating_duration_since(click.at) <= TRANSCRIPT_DOUBLE_CLICK_WINDOW
    })
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

use crate::{
    App, MouseButton, MouseEvent, MouseEventKind, Rect, SurfaceSelection, TranscriptClick,
    TranscriptPointerGesture, TranscriptScrollbarDrag, TranscriptTextPosition,
    surface_selection_text, ui_text,
};

use agena_tui_transcript::TranscriptScrollbarMetrics;
