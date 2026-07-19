impl App {
    pub(in crate::app) fn handle_transcript_key(&mut self, key: KeyEvent) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        self.transcript.ensure_visual_focus(width, height);
        match resolve_tui_key(KeyContext::Transcript, key) {
            Some(KeyAction::CountDigit(_)) => {}
            Some(KeyAction::EnterInsert) => {
                self.transcript.cancel_text_selection(width, height);
                self.enter_insert_mode();
            }
            Some(KeyAction::Copy) => {
                self.transcript_motion_prefix = None;
                if transcript_yank_key_completes(&mut self.transcript_yank_pending)
                    && !self.copy_active_transcript_text_selection()
                {
                    self.copy_transcript_cursor_node();
                }
            }
            Some(KeyAction::CopyVisible) => self.copy_visible_transcript(),
            Some(KeyAction::CopyAll) => self.copy_loaded_transcript(),
            Some(KeyAction::CopyLast) => self.copy_last_assistant_message(),
            Some(KeyAction::Toggle) => self.toggle_transcript_cursor_node(),
            Some(action @ (KeyAction::MoveLeft | KeyAction::MoveRight)) => {
                self.transcript.cancel_text_selection(width, height);
                let direction = if action == KeyAction::MoveLeft {
                    TranscriptMoveDirection::Up
                } else {
                    TranscriptMoveDirection::Down
                };
                let count = self.transcript_motion_count();
                self.transcript
                    .move_by_blocks(width, height, direction, count);
                if direction == TranscriptMoveDirection::Up {
                    self.maybe_request_older_messages();
                }
            }
            Some(action @ (KeyAction::MoveUp | KeyAction::MoveDown)) => {
                self.transcript.cancel_text_selection(width, height);
                let direction = if action == KeyAction::MoveUp {
                    TranscriptMoveDirection::Up
                } else {
                    TranscriptMoveDirection::Down
                };
                let count = self.transcript_motion_count();
                self.transcript
                    .move_cursor_by_lines(width, height, direction, count);
                if direction == TranscriptMoveDirection::Up {
                    self.maybe_request_older_messages();
                }
            }
            Some(KeyAction::PageUp) => {
                self.transcript_motion_prefix = None;
                self.transcript.move_cursor_by_page(width, height, false);
                self.maybe_request_older_messages();
            }
            Some(KeyAction::PageDown) => {
                self.transcript_motion_prefix = None;
                self.transcript.move_cursor_by_page(width, height, true);
            }
            Some(KeyAction::HalfPageUp) => {
                self.transcript_motion_prefix = None;
                self.transcript
                    .move_cursor_by_half_page(width, height, false);
                self.maybe_request_older_messages();
            }
            Some(KeyAction::HalfPageDown) => {
                self.transcript_motion_prefix = None;
                self.transcript
                    .move_cursor_by_half_page(width, height, true);
            }
            Some(KeyAction::Home) => {
                self.transcript_motion_prefix = None;
                self.transcript.scroll_to_top(width, height);
                self.maybe_request_older_messages();
            }
            Some(KeyAction::End) => {
                self.transcript_motion_prefix = None;
                self.transcript.scroll_to_bottom(width, height);
            }
            _ => self.transcript_motion_prefix = None,
        }
    }
}

fn transcript_yank_key_completes(pending: &mut bool) -> bool {
    if std::mem::take(pending) {
        true
    } else {
        *pending = true;
        false
    }
}

use crate::app::{App, KeyEvent, TranscriptMoveDirection};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};

#[cfg(test)]
mod tests {
    use super::transcript_yank_key_completes;

    #[test]
    fn transcript_yank_requires_two_consecutive_y_keys() {
        let mut pending = false;
        assert!(!transcript_yank_key_completes(&mut pending));
        assert!(pending);
        assert!(transcript_yank_key_completes(&mut pending));
        assert!(!pending);
    }
}
