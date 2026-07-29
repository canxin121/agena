impl App {
    pub(crate) fn handle_transcript_key(&mut self, key: KeyEvent) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        self.transcript.ensure_visual_focus(width, height);
        let action = resolve_tui_key(KeyContext::Transcript, key);
        if self.handle_pending_transcript_key(key, action, width, height) {
            return;
        }

        match action {
            Some(KeyAction::CountDigit(_)) => {}
            Some(KeyAction::LineStart)
                if matches!(key.code, KeyCode::Char('0'))
                    && self.transcript_motion_prefix.is_some() => {}
            Some(KeyAction::EnterInsert) if self.transcript.has_visual_selection() => {
                self.clear_transcript_pending_command();
                self.transcript_text_object_pending = Some((false, false));
            }
            Some(KeyAction::EnterInsert) => {
                self.clear_transcript_pending_command();
                self.transcript.cancel_text_selection(width, height);
                self.enter_insert_mode();
            }
            Some(KeyAction::ToggleVisualCharacter) => {
                self.clear_transcript_pending_command();
                self.transcript.toggle_visual_selection(
                    width,
                    height,
                    TranscriptVisualSelectionMode::Character,
                );
            }
            Some(KeyAction::ToggleVisualLine) => {
                self.clear_transcript_pending_command();
                self.transcript.toggle_visual_selection(
                    width,
                    height,
                    TranscriptVisualSelectionMode::Line,
                );
            }
            Some(KeyAction::ToggleVisualBlock) => {
                self.clear_transcript_pending_command();
                self.transcript.toggle_visual_selection(
                    width,
                    height,
                    TranscriptVisualSelectionMode::Block,
                );
            }
            Some(KeyAction::CancelSelection) => {
                self.clear_transcript_pending_command();
                self.transcript.cancel_text_selection(width, height);
            }
            Some(KeyAction::Copy) => {
                if self.transcript.has_visual_selection() {
                    self.clear_transcript_pending_command();
                    self.copy_active_transcript_text_selection();
                    self.transcript.cancel_text_selection(width, height);
                } else {
                    self.transcript_yank_pending = true;
                    self.transcript_yank_origin = self.transcript.cursor_text_position(width);
                }
            }
            Some(KeyAction::YankLine) => {
                self.clear_transcript_pending_command();
                self.yank_current_lines(width, height, 1, None);
            }
            Some(KeyAction::Toggle) => {
                self.clear_transcript_pending_command();
                self.toggle_transcript_cursor_node();
            }
            Some(action @ (KeyAction::MoveLeft | KeyAction::MoveRight)) => {
                self.cancel_pointer_selection_if_not_visual(width, height);
                let count = self.transcript_motion_count();
                self.transcript.move_cursor_horizontally(
                    width,
                    height,
                    action == KeyAction::MoveRight,
                    count,
                );
            }
            Some(action @ (KeyAction::MoveUp | KeyAction::MoveDown)) => {
                self.cancel_pointer_selection_if_not_visual(width, height);
                let direction = if action == KeyAction::MoveUp {
                    TranscriptMoveDirection::Up
                } else {
                    TranscriptMoveDirection::Down
                };
                let count = self.transcript_motion_count();
                self.transcript
                    .move_cursor_by_visual_lines(width, height, direction, count);
                if direction == TranscriptMoveDirection::Up {
                    self.maybe_request_older_messages();
                }
            }
            Some(action @ (KeyAction::PreviousMessage | KeyAction::NextMessage)) => {
                self.cancel_pointer_selection_if_not_visual(width, height);
                let direction = if action == KeyAction::PreviousMessage {
                    TranscriptMoveDirection::Up
                } else {
                    TranscriptMoveDirection::Down
                };
                let count = self.transcript_motion_count();
                self.transcript
                    .move_cursor_by_messages(width, height, direction, count);
                if direction == TranscriptMoveDirection::Up {
                    self.maybe_request_older_messages();
                }
            }
            Some(KeyAction::LineStart) => {
                self.cancel_pointer_selection_if_not_visual(width, height);
                self.clear_transcript_pending_command();
                self.transcript
                    .move_cursor_to_line_start(width, height, false);
            }
            Some(KeyAction::LineFirstNonBlank) => {
                self.cancel_pointer_selection_if_not_visual(width, height);
                self.clear_transcript_pending_command();
                self.transcript
                    .move_cursor_to_line_start(width, height, true);
            }
            Some(KeyAction::LineEnd) => {
                self.cancel_pointer_selection_if_not_visual(width, height);
                self.clear_transcript_pending_command();
                self.transcript.move_cursor_to_line_end(width, height);
            }
            Some(
                action @ (KeyAction::WordForward
                | KeyAction::WordBackward
                | KeyAction::WordEnd
                | KeyAction::BigWordForward
                | KeyAction::BigWordBackward
                | KeyAction::BigWordEnd),
            ) => {
                self.cancel_pointer_selection_if_not_visual(width, height);
                let count = self.transcript_motion_count();
                self.move_transcript_word_motion(width, height, action, count);
            }
            Some(
                action @ (KeyAction::FindForward
                | KeyAction::FindBackward
                | KeyAction::FindTillForward
                | KeyAction::FindTillBackward),
            ) => {
                let count = self.transcript_motion_count();
                self.transcript_find_pending = Some((
                    matches!(action, KeyAction::FindForward | KeyAction::FindTillForward),
                    matches!(
                        action,
                        KeyAction::FindTillForward | KeyAction::FindTillBackward
                    ),
                    count,
                ));
            }
            Some(KeyAction::RepeatFind) | Some(KeyAction::RepeatFindReverse) => {
                let reverse = action == Some(KeyAction::RepeatFindReverse);
                let count = self.transcript_motion_count();
                if let Some((forward, till, target)) = self.transcript_last_find {
                    self.cancel_pointer_selection_if_not_visual(width, height);
                    self.transcript.move_cursor_to_find(
                        width,
                        height,
                        if reverse { !forward } else { forward },
                        till,
                        target,
                        count,
                    );
                }
            }
            Some(KeyAction::GotoPrefix) => self.transcript_goto_pending = true,
            Some(KeyAction::End) => {
                let count = self.transcript_motion_count_if_present();
                if let Some(count) = count {
                    self.transcript
                        .move_cursor_to_visual_line_number(width, height, Some(count));
                } else {
                    self.transcript.scroll_to_bottom(width, height);
                }
            }
            Some(KeyAction::ViewportPrefix) => self.transcript_viewport_pending = true,
            Some(KeyAction::ViewTop) => {
                self.clear_transcript_pending_command();
                self.transcript.move_cursor_to_viewport_row(
                    width,
                    height,
                    TranscriptViewportRow::Top,
                );
            }
            Some(KeyAction::TextObjectMessage) => {
                // `M` is middle-of-window normally, but becomes the custom
                // message text object only after `a`/`i` (for example `vaM`).
                self.clear_transcript_pending_command();
                self.transcript.move_cursor_to_viewport_row(
                    width,
                    height,
                    TranscriptViewportRow::Middle,
                );
            }
            Some(KeyAction::ViewBottom) => {
                self.clear_transcript_pending_command();
                self.transcript.move_cursor_to_viewport_row(
                    width,
                    height,
                    TranscriptViewportRow::Bottom,
                );
            }
            Some(KeyAction::SwapVisualEndpoint) => {
                self.clear_transcript_pending_command();
                self.transcript
                    .swap_visual_selection_endpoint(width, height);
            }
            Some(KeyAction::SwapVisualBlockCorner) => {
                self.clear_transcript_pending_command();
                self.transcript.swap_visual_block_corner(width, height);
            }
            Some(KeyAction::AroundTextObject) => {
                self.clear_transcript_pending_command();
                if self.transcript.has_visual_selection() {
                    self.transcript_text_object_pending = Some((false, true));
                }
            }
            Some(KeyAction::PageUp) => {
                self.clear_transcript_pending_command();
                self.transcript.move_cursor_by_page(width, height, false);
                self.maybe_request_older_messages();
            }
            Some(KeyAction::PageDown) => {
                self.clear_transcript_pending_command();
                self.transcript.move_cursor_by_page(width, height, true);
            }
            Some(KeyAction::HalfPageUp) => {
                self.clear_transcript_pending_command();
                self.transcript
                    .move_cursor_by_half_page(width, height, false);
                self.maybe_request_older_messages();
            }
            Some(KeyAction::HalfPageDown) => {
                self.clear_transcript_pending_command();
                self.transcript
                    .move_cursor_by_half_page(width, height, true);
            }
            _ => self.clear_transcript_pending_command(),
        }
    }

    fn handle_pending_transcript_key(
        &mut self,
        key: KeyEvent,
        action: Option<KeyAction>,
        width: u16,
        height: u16,
    ) -> bool {
        if let Some((forward, till, count)) = self.transcript_find_pending.take() {
            if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                && let KeyCode::Char(target) = key.code
            {
                self.cancel_pointer_selection_if_not_visual(width, height);
                self.transcript
                    .move_cursor_to_find(width, height, forward, till, target, count);
                self.transcript_last_find = Some((forward, till, target));
            }
            self.transcript_motion_prefix = None;
            return true;
        }

        if self.transcript_goto_pending {
            self.transcript_goto_pending = false;
            match action {
                Some(KeyAction::GotoPrefix) => {
                    let count = self.transcript_motion_count_if_present();
                    if let Some(count) = count {
                        self.transcript.move_cursor_to_visual_line_number(
                            width,
                            height,
                            Some(count),
                        );
                    } else {
                        self.transcript.scroll_to_top(width, height);
                        self.maybe_request_older_messages();
                    }
                }
                Some(KeyAction::ToggleVisualCharacter) => {
                    self.transcript_motion_prefix = None;
                    self.transcript
                        .reselect_last_visual_selection(width, height);
                }
                Some(KeyAction::WordEnd) => {
                    let count = self.transcript_motion_count();
                    self.move_transcript_word_motion(
                        width,
                        height,
                        KeyAction::WordEndBackward,
                        count,
                    );
                }
                Some(KeyAction::BigWordEnd) => {
                    let count = self.transcript_motion_count();
                    self.move_transcript_word_motion(
                        width,
                        height,
                        KeyAction::BigWordEndBackward,
                        count,
                    );
                }
                _ => self.transcript_motion_prefix = None,
            }
            return true;
        }

        if self.transcript_viewport_pending {
            self.transcript_viewport_pending = false;
            self.transcript_motion_prefix = None;
            match action {
                Some(KeyAction::ViewportPrefix) => self.transcript.place_cursor_in_viewport(
                    width,
                    height,
                    TranscriptViewportRow::Middle,
                ),
                Some(KeyAction::FindTillForward) => self.transcript.place_cursor_in_viewport(
                    width,
                    height,
                    TranscriptViewportRow::Top,
                ),
                Some(KeyAction::WordBackward) => self.transcript.place_cursor_in_viewport(
                    width,
                    height,
                    TranscriptViewportRow::Bottom,
                ),
                _ => {}
            }
            return true;
        }

        if let Some((yank, around)) = self.transcript_text_object_pending.take() {
            let message = matches!(action, Some(KeyAction::TextObjectMessage));
            let selected = match action {
                Some(KeyAction::TextObjectMarkdown | KeyAction::TextObjectMessage) => self
                    .transcript
                    .select_current_text_object(width, height, message),
                Some(KeyAction::WordForward) => self
                    .transcript
                    .select_current_word_text_object(width, height, around),
                Some(KeyAction::TextObjectParagraph) => self
                    .transcript
                    .select_current_paragraph_text_object(width, height, around),
                _ => false,
            };
            if selected && yank {
                self.copy_active_transcript_text_selection();
                self.transcript.cancel_text_selection(width, height);
                self.restore_yank_origin(width, height);
            }
            if yank {
                self.transcript_yank_origin = None;
            }
            self.transcript_motion_prefix = None;
            return true;
        }

        if !self.transcript_yank_pending {
            return false;
        }
        match action {
            Some(KeyAction::CountDigit(_)) => true,
            Some(KeyAction::LineStart)
                if matches!(key.code, KeyCode::Char('0'))
                    && self.transcript_motion_prefix.is_some() =>
            {
                true
            }
            Some(KeyAction::Copy) | Some(KeyAction::YankLine) => {
                let count = self.transcript_motion_count();
                self.transcript_yank_pending = false;
                let origin = self.transcript_yank_origin.take();
                self.yank_current_lines(width, height, count, origin);
                true
            }
            Some(KeyAction::AroundTextObject) | Some(KeyAction::EnterInsert) => {
                self.transcript_yank_pending = false;
                self.transcript_text_object_pending =
                    Some((true, action == Some(KeyAction::AroundTextObject)));
                true
            }
            Some(
                action @ (KeyAction::MoveLeft
                | KeyAction::MoveRight
                | KeyAction::MoveUp
                | KeyAction::MoveDown
                | KeyAction::LineStart
                | KeyAction::LineFirstNonBlank
                | KeyAction::LineEnd
                | KeyAction::WordForward
                | KeyAction::WordBackward
                | KeyAction::WordEnd
                | KeyAction::BigWordForward
                | KeyAction::BigWordBackward
                | KeyAction::BigWordEnd),
            ) => {
                let count = self.transcript_motion_count();
                self.transcript_yank_pending = false;
                let origin = self.transcript_yank_origin.take();
                self.yank_by_motion(width, height, action, count, origin);
                true
            }
            _ => {
                self.clear_transcript_pending_command();
                true
            }
        }
    }

    fn move_transcript_word_motion(
        &mut self,
        width: u16,
        height: u16,
        action: KeyAction,
        count: usize,
    ) {
        let (forward, to_end, big_word) = match action {
            KeyAction::WordForward => (true, false, false),
            KeyAction::WordBackward => (false, false, false),
            KeyAction::WordEnd => (true, true, false),
            KeyAction::WordEndBackward => (false, true, false),
            KeyAction::BigWordForward => (true, false, true),
            KeyAction::BigWordBackward => (false, false, true),
            KeyAction::BigWordEnd => (true, true, true),
            KeyAction::BigWordEndBackward => (false, true, true),
            _ => return,
        };
        self.transcript
            .move_cursor_by_words(width, height, forward, to_end, big_word, count);
    }

    fn yank_current_lines(
        &mut self,
        width: u16,
        height: u16,
        count: usize,
        origin: Option<TranscriptTextPosition>,
    ) {
        if self.copy_active_transcript_text_selection() {
            self.transcript.cancel_text_selection(width, height);
            return;
        }
        if count == 1 {
            self.copy_transcript_cursor_node();
            return;
        }
        self.transcript
            .toggle_visual_selection(width, height, TranscriptVisualSelectionMode::Line);
        if count > 1 {
            self.transcript.move_cursor_by_visual_lines(
                width,
                height,
                TranscriptMoveDirection::Down,
                count.saturating_sub(1),
            );
        }
        self.copy_active_transcript_text_selection();
        self.transcript.cancel_text_selection(width, height);
        if let Some(origin) = origin {
            self.transcript
                .restore_cursor_text_position(width, height, origin);
        }
    }

    fn yank_by_motion(
        &mut self,
        width: u16,
        height: u16,
        action: KeyAction,
        count: usize,
        origin: Option<TranscriptTextPosition>,
    ) {
        let linewise = matches!(action, KeyAction::MoveUp | KeyAction::MoveDown);
        self.transcript.toggle_visual_selection(
            width,
            height,
            if linewise {
                TranscriptVisualSelectionMode::Line
            } else {
                TranscriptVisualSelectionMode::Character
            },
        );
        match action {
            KeyAction::MoveLeft | KeyAction::MoveRight => self.transcript.move_cursor_horizontally(
                width,
                height,
                action == KeyAction::MoveRight,
                count,
            ),
            KeyAction::MoveUp | KeyAction::MoveDown => self.transcript.move_cursor_by_visual_lines(
                width,
                height,
                if action == KeyAction::MoveUp {
                    TranscriptMoveDirection::Up
                } else {
                    TranscriptMoveDirection::Down
                },
                count,
            ),
            KeyAction::LineStart => self
                .transcript
                .move_cursor_to_line_start(width, height, false),
            KeyAction::LineFirstNonBlank => self
                .transcript
                .move_cursor_to_line_start(width, height, true),
            KeyAction::LineEnd => self.transcript.move_cursor_to_line_end(width, height),
            _ => {
                let before = self.transcript.cursor_text_position(width);
                self.move_transcript_word_motion(width, height, action, count);
                if matches!(action, KeyAction::WordForward | KeyAction::BigWordForward)
                    && self.transcript.cursor_text_position(width) != before
                {
                    self.transcript
                        .move_cursor_to_previous_grapheme(width, height);
                }
            }
        }
        self.copy_active_transcript_text_selection();
        self.transcript.cancel_text_selection(width, height);
        if let Some(origin) = origin {
            self.transcript
                .restore_cursor_text_position(width, height, origin);
        }
    }

    fn restore_yank_origin(&mut self, width: u16, height: u16) {
        if let Some(origin) = self.transcript_yank_origin.take() {
            self.transcript
                .restore_cursor_text_position(width, height, origin);
        }
    }

    fn cancel_pointer_selection_if_not_visual(&mut self, width: u16, height: u16) {
        if !self.transcript.has_visual_selection() {
            self.transcript.cancel_text_selection(width, height);
        }
    }
}

use crate::{
    App, KeyEvent, TranscriptMoveDirection, TranscriptTextPosition, TranscriptViewportRow,
    TranscriptVisualSelectionMode,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use crossterm::event::{KeyCode, KeyModifiers};
