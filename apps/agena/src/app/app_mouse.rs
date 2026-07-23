use std::time::{Duration, Instant};

const TRANSCRIPT_WHEEL_LINES: isize = 3;
const TRANSCRIPT_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

impl App {
    pub(in crate::app) fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        self.transcript_yank_pending = false;
        if !self.mouse_capture_active() {
            self.transcript_scrollbar_drag = None;
            self.transcript_pointer_gesture = None;
            self.last_transcript_click = None;
            self.transcript.cancel_text_selection(
                self.layout.transcript_body.width,
                self.layout.transcript_body.height,
            );
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
                self.move_transcript_cursor_by_wheel(-TRANSCRIPT_WHEEL_LINES);
            }
            MouseEventKind::ScrollDown => {
                self.cancel_active_pointer_gesture();
                self.move_transcript_cursor_by_wheel(TRANSCRIPT_WHEEL_LINES);
            }
            MouseEventKind::Down(MouseButton::Left)
                if rect_contains(scrollbar, mouse.column, mouse.row) =>
            {
                self.transcript_pointer_gesture = None;
                self.last_transcript_click = None;
                self.transcript.cancel_text_selection(
                    self.layout.transcript_body.width,
                    self.layout.transcript_body.height,
                );
                self.begin_transcript_scrollbar_drag(mouse.row);
            }
            MouseEventKind::Down(MouseButton::Left)
                if rect_contains(transcript, mouse.column, mouse.row) =>
            {
                self.transcript_scrollbar_drag = None;
                self.begin_transcript_pointer_gesture(mouse.column, mouse.row);
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
            }
            MouseEventKind::Up(_) => self.cancel_active_pointer_gesture(),
            _ => {}
        }
    }

    pub(in crate::app) fn cancel_active_pointer_gesture(&mut self) {
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
        if self.transcript.viewport_top() < previous && self.transcript_pointer_gesture.is_none() {
            self.maybe_request_older_messages();
        }
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
            agena_tui::transcript::scroll_for_thumb(metrics, pointer_line, drag.grab_offset);
        let previous = self.transcript.viewport_top();
        self.transcript.relocate_cursor_from_scrollbar(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            target,
        );
        if self.transcript.viewport_top() < previous {
            self.maybe_request_older_messages();
        }
    }

    fn current_transcript_scrollbar_metrics(&mut self) -> Option<TranscriptScrollbarMetrics> {
        let body = self.layout.transcript_body;
        let scrollbar = self.layout.transcript_scrollbar;
        if body.width == 0 || body.height == 0 || scrollbar.height == 0 {
            return None;
        }
        let total_lines = self.transcript.rendered(body.width).lines.len();
        agena_tui::transcript::scrollbar_metrics(
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

    pub(in crate::app) fn copy_active_transcript_text_selection(&mut self) -> bool {
        let Some(selection) = self.transcript.text_selection() else {
            return false;
        };
        self.copy_transcript_text_selection(selection);
        true
    }

    fn copy_transcript_text_selection(&mut self, selection: TranscriptTextSelection) {
        let width = self.layout.transcript_body.width;
        let rendered = self.transcript.rendered(width);
        let text = transcript_text_selection_text(
            rendered.lines.as_slice(),
            rendered.nodes.as_slice(),
            rendered.line_nodes.as_slice(),
            selection,
            spinner_frame(current_spinner_millis()),
        );
        if text.is_empty() {
            return;
        }
        self.request_clipboard_copy(text, ui_text::t(&self.i18n, "flash-copied-mouse-selection"));
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

use crate::app::{
    App, MouseButton, MouseEvent, MouseEventKind, Rect, TranscriptClick, TranscriptPointerGesture,
    TranscriptScrollbarDrag, TranscriptTextPosition, TranscriptTextSelection,
    current_spinner_millis, spinner_frame, transcript_text_selection_text, ui_text,
};
#[cfg(test)]
use crate::app::{
    RenderedLine, RenderedTranscriptNode, Style, TranscriptNodeKey, TranscriptNodeKind,
};
use agena_tui::transcript::TranscriptScrollbarMetrics;

#[cfg(test)]
mod tests {
    use super::{
        Duration, Instant, RenderedLine, RenderedTranscriptNode, Style, TranscriptClick,
        TranscriptNodeKey, TranscriptNodeKind, TranscriptPointerGesture, TranscriptTextPosition,
        TranscriptTextSelection, transcript_click_selects_block, transcript_text_selection_text,
    };

    #[test]
    fn only_a_quick_second_click_on_the_same_line_selects_the_block() {
        let now = Instant::now();
        let recent = TranscriptClick {
            line: 7,
            at: now.checked_sub(Duration::from_millis(200)).unwrap_or(now),
        };
        let old = TranscriptClick {
            line: 7,
            at: now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
        };

        assert!(transcript_click_selects_block(Some(recent), 7, now));
        assert!(!transcript_click_selects_block(Some(recent), 8, now));
        assert!(!transcript_click_selects_block(Some(old), 7, now));
        assert!(!transcript_click_selects_block(None, 7, now));
    }

    #[test]
    fn pointer_gesture_does_not_become_a_range_until_it_moves() {
        let anchor = TranscriptTextPosition { line: 4, column: 7 };
        let mut gesture = TranscriptPointerGesture::new(anchor);
        assert!(!gesture.dragged);
        assert_eq!(gesture.selection().anchor, gesture.selection().head);

        gesture.update(anchor, false);
        assert!(!gesture.dragged, "a press/release pair remains a click");

        gesture.update(anchor, true);
        assert!(
            gesture.dragged,
            "a drag event inside one terminal cell can select that one glyph"
        );

        let mut gesture = TranscriptPointerGesture::new(anchor);
        gesture.update(TranscriptTextPosition { line: 4, column: 8 }, true);
        assert!(gesture.dragged);
        assert_ne!(gesture.selection().anchor, gesture.selection().head);
    }

    fn selection(anchor: (usize, usize), head: (usize, usize)) -> TranscriptTextSelection {
        TranscriptTextSelection {
            anchor: TranscriptTextPosition {
                line: anchor.0,
                column: anchor.1,
            },
            head: TranscriptTextPosition {
                line: head.0,
                column: head.1,
            },
        }
    }

    fn lines(values: &[&str]) -> Vec<RenderedLine> {
        values
            .iter()
            .map(|value| RenderedLine::plain(*value, Style::default()))
            .collect()
    }

    fn copied_text(
        lines: &[RenderedLine],
        selection: TranscriptTextSelection,
        spinner: &str,
    ) -> String {
        let line_nodes = vec![None; lines.len()];
        transcript_text_selection_text(lines, &[], line_nodes.as_slice(), selection, spinner)
    }

    #[test]
    fn mouse_selection_copies_forward_and_backward_cell_ranges() {
        let rendered = lines(&["abcdef"]);
        assert_eq!(copied_text(&rendered, selection((0, 2), (0, 2)), ""), "c");
        assert_eq!(copied_text(&rendered, selection((0, 1), (0, 3)), ""), "bcd");
        assert_eq!(copied_text(&rendered, selection((0, 3), (0, 1)), ""), "bcd");
    }

    #[test]
    fn mouse_selection_preserves_line_breaks_and_partial_endpoints() {
        let rendered = lines(&["zero", "one", "two"]);
        assert_eq!(
            copied_text(&rendered, selection((0, 2), (2, 1)), ""),
            "ro\none\ntw"
        );
    }

    #[test]
    fn mouse_selection_never_splits_wide_or_combining_graphemes() {
        let rendered = lines(&["a你e\u{301}z"]);
        assert_eq!(
            copied_text(&rendered, selection((0, 2), (0, 3)), ""),
            "你e\u{301}"
        );
    }

    #[test]
    fn mouse_selection_copies_the_visible_spinner_instead_of_its_placeholder() {
        let rendered = lines(&["a\u{e000}b"]);
        assert_eq!(
            copied_text(&rendered, selection((0, 0), (0, 2)), "⠋"),
            "a⠋b"
        );
    }

    #[test]
    fn mouse_selection_uses_the_clean_row_projection_instead_of_layout_prefixes() {
        let rendered = vec![
            RenderedLine::plain("  plain text", Style::default())
                .with_copy_projection("plain text", 2),
        ];
        assert_eq!(
            copied_text(&rendered, selection((0, 0), (0, 20)), ""),
            "plain text"
        );
    }

    #[test]
    fn mouse_selection_copies_an_atomic_rich_node_once_from_semantic_source() {
        let lines = lines(&["     a", "  ─────", "     b"]);
        let nodes = vec![RenderedTranscriptNode {
            key: TranscriptNodeKey::MarkdownBlock {
                message_id: 1,
                part_id: 2,
                block_index: 0,
            },
            kind: TranscriptNodeKind::MarkdownMath,
            start_line: 0,
            end_line: 3,
            copy_text: "\\frac{a}{b}".to_string(),
            atomic: true,
            toggleable: false,
            expanded: true,
        }];
        assert_eq!(
            transcript_text_selection_text(
                lines.as_slice(),
                nodes.as_slice(),
                &[Some(0), Some(0), Some(0)],
                selection((0, 0), (2, usize::MAX)),
                "",
            ),
            "\\frac{a}{b}"
        );
    }

    #[test]
    fn mouse_selection_can_slice_a_wrapped_keyboard_semantic_row() {
        let lines = vec![
            RenderedLine::plain("  │1 let very_long =", Style::default())
                .with_copy_projection("let very_long =", 5)
                .with_navigation_unit(1, "let very_long = 42;"),
            RenderedLine::plain("  │    42;          │", Style::default())
                .with_copy_projection("42;", 7)
                .with_navigation_unit(1, "let very_long = 42;"),
        ];
        let nodes = vec![RenderedTranscriptNode {
            key: TranscriptNodeKey::MarkdownBlock {
                message_id: 1,
                part_id: 2,
                block_index: 0,
            },
            kind: TranscriptNodeKind::MarkdownCode,
            start_line: 0,
            end_line: 2,
            copy_text: "let very_long = 42;".to_string(),
            atomic: false,
            toggleable: false,
            expanded: true,
        }];
        assert_eq!(
            transcript_text_selection_text(
                lines.as_slice(),
                nodes.as_slice(),
                &[Some(0), Some(0)],
                selection((0, 6), (1, 8)),
                "",
            ),
            "et very_long =42"
        );
    }
}
