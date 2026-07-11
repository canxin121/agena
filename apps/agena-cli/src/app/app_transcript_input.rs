impl App {
    pub(in crate::app) fn handle_transcript_key(&mut self, key: KeyEvent) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        if matches!(key.code, KeyCode::Char('1'..='9'))
            || matches!(key.code, KeyCode::Char('0')) && self.transcript_motion_prefix.is_some()
        {
            return;
        } else if matches!(key.code, KeyCode::Char('i')) {
            self.enter_insert_mode();
        } else if matches!(key.code, KeyCode::Char('y')) {
            self.copy_transcript_cursor_node();
        } else if matches!(key.code, KeyCode::Char('Y')) {
            self.copy_visible_transcript();
        } else if matches!(key.code, KeyCode::Char('C')) {
            self.copy_loaded_transcript();
        } else if matches!(key.code, KeyCode::Char('c')) {
            self.copy_last_assistant_message();
        } else if matches!(
            key.code,
            KeyCode::Enter | KeyCode::Char('o') | KeyCode::Char(' ')
        ) {
            self.toggle_transcript_cursor_node();
        } else if let Some(direction) = transcript_message_navigation_direction(key.code) {
            let count = self.transcript_motion_count();
            self.transcript
                .move_by_blocks(width, height, direction, count);
            if direction == TranscriptMoveDirection::Up {
                self.maybe_request_older_messages();
            }
        } else if let Some(direction) = transcript_vertical_navigation_direction(key.code) {
            let count = self.transcript_motion_count();
            self.transcript
                .scroll_by_lines_with_blocks(width, height, direction, count);
            if direction == TranscriptMoveDirection::Up {
                self.maybe_request_older_messages();
            }
        } else if matches!(key.code, KeyCode::PageUp)
            || matches!(key.code, KeyCode::Char('b'))
                && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_page(width, height, false);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::PageDown)
            || matches!(key.code, KeyCode::Char('f'))
                && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_page(width, height, true);
        } else if matches!(key.code, KeyCode::Char(' '))
            && key.modifiers.contains(KeyModifiers::SHIFT)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_page(width, height, false);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::Char(' ')) {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_page(width, height, true);
        } else if matches!(key.code, KeyCode::Char('u'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_half_page(width, height, false);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::Char('d'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_half_page(width, height, true);
        } else if matches!(key.code, KeyCode::Home | KeyCode::Char('g')) {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_to_top(width, height);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::End | KeyCode::Char('G')) {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_to_bottom(width, height);
        } else {
            self.transcript_motion_prefix = None;
        }
    }
}
use crate::app::{
    App, KeyCode, KeyEvent, KeyModifiers, TranscriptMoveDirection,
    transcript_message_navigation_direction, transcript_vertical_navigation_direction,
};
