//! Text editor widget.

use std::{
    cmp::{max, min},
    ops::Range,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::search_picker::SearchPickerInput;

const WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";
#[derive(Debug, Clone, Default)]
/// A text editor widget.
pub struct Editor {
    text: String,
    cursor: usize,
    preferred_column: Option<usize>,
    kill_buffer: String,
    elements: Vec<EditorElement>,
}

#[derive(Debug, Clone)]
/// View of the text editor.
pub struct EditorView {
    pub lines: Vec<Line<'static>>,
    pub cursor_x: u16,
    pub cursor_y: u16,
}

#[derive(Debug, Clone)]
struct EditorElement {
    range: Range<usize>,
}

impl Editor {
    pub fn from_text(text: String) -> Self {
        let text = sanitize_editor_text(&text);
        let cursor = text.len();
        Self {
            text,
            cursor,
            preferred_column: None,
            kill_buffer: String::new(),
            elements: Vec::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Move the cursor to a byte position while preserving UTF-8 and inline
    /// element boundaries. Composer history uses this to follow the familiar
    /// shell behavior: older entries begin at their start, newer entries and
    /// restored drafts end at their end.
    pub fn set_cursor(&mut self, position: usize) {
        let mut position = min(position, self.text.len());
        while position > 0 && !self.text.is_char_boundary(position) {
            position -= 1;
        }
        self.cursor = self.clamp_pos_to_nearest_boundary(position);
        self.preferred_column = None;
    }

    pub fn set_text(&mut self, text: String) {
        self.text = sanitize_editor_text(&text);
        self.cursor = self.text.len();
        self.preferred_column = None;
        self.elements.clear();
    }

    pub fn set_elements(&mut self, elements: Vec<Range<usize>>) {
        let mut normalized = elements
            .into_iter()
            .filter_map(|range| {
                let start = snap_to_char_boundary(&self.text, range.start);
                let end = snap_to_char_boundary(&self.text, range.end);
                (start < end).then_some(EditorElement { range: start..end })
            })
            .collect::<Vec<_>>();
        normalized.sort_by_key(|element| element.range.start);
        normalized.dedup_by(|a, b| a.range == b.range);
        self.elements = normalized;
        self.cursor = self.clamp_pos_to_nearest_boundary(self.cursor);
    }

    pub fn draft_elements(&self) -> Vec<Range<usize>> {
        self.elements
            .iter()
            .map(|element| element.range.clone())
            .collect()
    }

    pub fn element_texts(&self) -> Vec<String> {
        self.elements
            .iter()
            .filter_map(|element| self.text.get(element.range.clone()).map(str::to_owned))
            .collect()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.preferred_column = None;
        self.elements.clear();
    }

    pub fn insert_char(&mut self, ch: char) {
        let mut buffer = [0_u8; 4];
        self.insert_str(ch.encode_utf8(&mut buffer));
    }

    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let at = self.clamp_pos_for_insertion(self.cursor);
        self.insert_str_at(at, text);
    }

    pub fn insert_element(&mut self, text: &str) {
        let text = sanitize_editor_text(text);
        if text.is_empty() {
            return;
        }
        let start = self.clamp_pos_for_insertion(self.cursor);
        self.insert_str_at(start, &text);
        let end = start + text.len();
        self.elements.push(EditorElement { range: start..end });
        self.elements.sort_by_key(|element| element.range.start);
        self.cursor = end;
        self.preferred_column = None;
    }

    pub fn logical_line_count(&self) -> usize {
        split_editor_lines_with_offsets(self.text.as_str()).len()
    }

    /// Number of terminal rows needed when the editor is soft-wrapped to the
    /// supplied width. Explicit newlines still start a new logical line.
    pub fn wrapped_line_count(&self, width: u16) -> usize {
        wrapped_editor_lines(self.text.as_str(), width).len()
    }

    pub fn handle_line_input_key(&mut self, key: KeyEvent) {
        self.handle_input_key(key, false);
    }

    pub fn handle_multiline_input_key(&mut self, key: KeyEvent) {
        self.handle_input_key(key, true);
    }

    pub fn render_view(&self, width: u16, height: u16) -> EditorView {
        let width = max(width as usize, 1);
        let height = max(height as usize, 1);
        let lines = split_editor_lines_with_offsets(self.text.as_str());
        let current_line_index = self.current_line_index();
        let current_col = self.current_display_column();
        // Scroll only once the cursor moves past the last visible column.
        // Keeping the window at column 0 while the text (or the cursor at the
        // end of an exactly-fitting line) still fits avoids hiding the leading
        // characters: e.g. "abcd" in a 4-column input must not show "bcd".
        let hscroll = if current_col > width {
            current_col.saturating_sub(width.saturating_sub(1))
        } else {
            0
        };
        let vscroll = current_line_index.saturating_sub(height.saturating_sub(1));
        let visible_lines = lines
            .iter()
            .skip(vscroll)
            .take(height)
            .map(|range| {
                slice_display_window_styled(
                    self.text.as_str(),
                    range.clone(),
                    hscroll,
                    hscroll.saturating_add(width),
                    self.elements.as_slice(),
                )
            })
            .collect::<Vec<_>>();

        EditorView {
            lines: visible_lines,
            cursor_x: min(
                min(current_col.saturating_sub(hscroll), width.saturating_sub(1)),
                u16::MAX as usize,
            ) as u16,
            cursor_y: min(
                current_line_index.saturating_sub(vscroll),
                u16::MAX as usize,
            ) as u16,
        }
    }

    /// Render a soft-wrapped editor viewport. This is used by the chat
    /// composer so a long prompt grows vertically instead of horizontally
    /// scrolling or hiding its tail.
    pub fn render_wrapped_view(&self, width: u16, height: u16) -> EditorView {
        let width = max(width as usize, 1);
        let height = max(height as usize, 1);
        let lines = wrapped_editor_lines(self.text.as_str(), width as u16);
        let cursor_line_index = self.current_line_index();
        let cursor_column = self.current_display_column();
        let cursor_visual_line = lines
            .iter()
            .enumerate()
            .find(|(_, line)| {
                line.logical_line_index == cursor_line_index
                    && cursor_column >= line.start_column
                    && cursor_column < line.end_column
            })
            .map(|(index, _)| index)
            .or_else(|| {
                lines
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, line)| line.logical_line_index == cursor_line_index)
                    .map(|(index, _)| index)
            })
            .unwrap_or(0);
        let vscroll = cursor_visual_line.saturating_sub(height.saturating_sub(1));
        let visible_lines = lines
            .iter()
            .skip(vscroll)
            .take(height)
            .map(|line| {
                slice_display_window_styled(
                    self.text.as_str(),
                    line.range.clone(),
                    line.start_column,
                    line.end_column,
                    self.elements.as_slice(),
                )
            })
            .collect::<Vec<_>>();
        let cursor_x = cursor_column
            .saturating_sub(
                lines
                    .get(cursor_visual_line)
                    .map(|line| line.start_column)
                    .unwrap_or_default(),
            )
            .min(width.saturating_sub(1));

        EditorView {
            lines: visible_lines,
            cursor_x: min(cursor_x, u16::MAX as usize) as u16,
            cursor_y: min(
                cursor_visual_line.saturating_sub(vscroll),
                u16::MAX as usize,
            ) as u16,
        }
    }

    pub fn insert_explicit_newline(&mut self) {
        self.insert_newline();
    }

    /// Move the cursor up one soft-wrapped visual row. Unlike `move_up`
    /// (which jumps between explicit `\n` logical lines), this walks the same
    /// wrap boundaries used by `render_wrapped_view` so long prompts can be
    /// edited row by row. `width` must match the editor viewport width.
    pub fn move_visual_up(&mut self, width: u16) {
        let cursor_column = self.current_display_column();
        let target_col = self.preferred_column.unwrap_or(cursor_column);
        let lines = wrapped_editor_lines(self.text.as_str(), width);
        let Some(visual_index) =
            visual_row_index_for_cursor(&lines, self.current_line_index(), cursor_column)
        else {
            return;
        };
        if visual_index == 0 {
            // Already on the first visual row: land on the head of the
            // current logical line (matching the existing first-line
            // behavior of the non-wrapped `move_up`).
            let bol = self.current_line_start();
            self.cursor = self.clamp_pos_to_nearest_boundary(bol);
            self.preferred_column = None;
            return;
        }
        let previous = &lines[visual_index - 1];
        if previous.logical_line_index != self.current_line_index() {
            // Crossing into the previous logical line: prefer its head so the
            // transition is deterministic instead of jumping to an arbitrary
            // wrapped column.
            let prev_bol = self.beginning_of_line(previous.range.start);
            self.cursor = self.clamp_pos_to_nearest_boundary(prev_bol);
            self.preferred_column = None;
            return;
        }
        let line_text = &self.text[previous.range.clone()];
        self.cursor = byte_index_at_display_column(
            line_text,
            previous.range.start,
            target_col.saturating_sub(previous.start_column),
        );
        self.cursor = self.clamp_pos_to_nearest_boundary(self.cursor);
        self.preferred_column = Some(target_col);
    }

    /// Move the cursor down one soft-wrapped visual row. See `move_visual_up`.
    pub fn move_visual_down(&mut self, width: u16) {
        let cursor_column = self.current_display_column();
        let target_col = self.preferred_column.unwrap_or(cursor_column);
        let lines = wrapped_editor_lines(self.text.as_str(), width);
        let Some(visual_index) =
            visual_row_index_for_cursor(&lines, self.current_line_index(), cursor_column)
        else {
            return;
        };
        let Some(next) = lines.get(visual_index + 1) else {
            // Last visual row: land on the tail of the current logical line.
            let eol = self.current_line_end();
            self.cursor = self.clamp_pos_to_nearest_boundary(eol);
            self.preferred_column = None;
            return;
        };
        if next.logical_line_index != self.current_line_index() {
            let next_bol = self.beginning_of_line(next.range.start);
            self.cursor = self.clamp_pos_to_nearest_boundary(next_bol);
            self.preferred_column = None;
            return;
        }
        let line_text = &self.text[next.range.clone()];
        self.cursor = byte_index_at_display_column(
            line_text,
            next.range.start,
            target_col.saturating_sub(next.start_column),
        );
        self.cursor = self.clamp_pos_to_nearest_boundary(self.cursor);
        self.preferred_column = Some(target_col);
    }

    pub fn cursor_on_first_line(&self) -> bool {
        self.current_line_index() == 0
    }

    fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    fn handle_input_key(&mut self, key: KeyEvent, multiline: bool) {
        match key {
            KeyEvent {
                code: KeyCode::Char('\u{0001}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_home(true);
            }
            KeyEvent {
                code: KeyCode::Char('\u{0002}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_left();
            }
            KeyEvent {
                code: KeyCode::Left,
                modifiers,
                ..
            } if modifiers == KeyModifiers::CONTROL || modifiers == KeyModifiers::ALT => {
                self.move_word_left();
            }
            // Alt+B is the classic Emacs/macOS Option-as-Meta encoding for
            // word-left (the terminal sends ESC b). CSI-u terminals report the
            // Left cursor key as control code U+0001 (\u001b[1;3u / \u001b[1;5u).
            KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::ALT,
                ..
            } => {
                self.move_word_left();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0001}'),
                modifiers,
                ..
            } if modifiers == KeyModifiers::ALT || modifiers == KeyModifiers::CONTROL => {
                self.move_word_left();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0005}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('e'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_end(true);
            }
            KeyEvent {
                code: KeyCode::Char('\u{0006}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_right();
            }
            KeyEvent {
                code: KeyCode::Right,
                modifiers,
                ..
            } if modifiers == KeyModifiers::CONTROL || modifiers == KeyModifiers::ALT => {
                self.move_word_right();
            }
            // Alt+F and the CSI-u Right-cursor control code U+0002.
            KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::ALT,
                ..
            } => {
                self.move_word_right();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0002}'),
                modifiers,
                ..
            } if modifiers == KeyModifiers::ALT || modifiers == KeyModifiers::CONTROL => {
                self.move_word_right();
            }
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            } if multiline => {
                self.move_up();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0010}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } if multiline => {
                self.move_up();
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            } if multiline => {
                self.move_down();
            }
            KeyEvent {
                code: KeyCode::Char('\u{000e}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } if multiline => {
                self.move_down();
            }
            KeyEvent {
                code: KeyCode::Home,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_home(false);
            }
            KeyEvent {
                code: KeyCode::End,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_end(false);
            }
            // Word-level deletion. Alt+Backspace is Option+Delete on macOS
            // and Alt+Backspace in most Linux terminals; Ctrl+Backspace is the
            // common Windows/Linux word-delete chord. Terminals without the
            // enhanced keyboard protocol send Alt+Backspace as ESC DEL, which
            // crossterm surfaces as an ALT-modified 0x7F char. Ctrl+Alt+H is
            // kept as the historical Emacs-style word-kill chord.
            KeyEvent {
                code: KeyCode::Backspace,
                modifiers,
                ..
            } if modifiers == KeyModifiers::ALT || modifiers == KeyModifiers::CONTROL => {
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Char('\u{007f}'),
                modifiers,
                ..
            } if modifiers == KeyModifiers::ALT || modifiers == KeyModifiers::CONTROL => {
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Char('h'),
                modifiers,
                ..
            } if modifiers == (KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.delete_backward_word();
            }
            // Terminals that send BS (U+0008) instead of DEL for Option/Ctrl
            // + Backspace (\u001b\b arrives as an ALT-modified 0x08 char).
            KeyEvent {
                code: KeyCode::Char('\u{0008}'),
                modifiers,
                ..
            } if modifiers == KeyModifiers::ALT || modifiers == KeyModifiers::CONTROL => {
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Char('\u{007f}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.backspace();
            }
            KeyEvent {
                code: KeyCode::Delete,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if self.cursor >= self.text.len() {
                    self.backspace();
                } else {
                    self.delete();
                }
            }
            // Forward word deletion (Option+Delete on macOS, Ctrl+Delete on
            // Windows/Linux).
            KeyEvent {
                code: KeyCode::Delete,
                modifiers,
                ..
            } if modifiers == KeyModifiers::ALT || modifiers == KeyModifiers::CONTROL => {
                self.delete_forward_word();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0004}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.delete();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0017}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('w'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0015}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.kill_to_start_of_line(multiline);
            }
            KeyEvent {
                code: KeyCode::Char('\u{000b}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.kill_to_end_of_line(multiline);
            }
            KeyEvent {
                code: KeyCode::Char('\u{0019}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('y'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.yank();
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if is_altgr(modifiers) && !c.is_control() => {
                self.handle_plain_char(c);
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } if !c.is_control() => {
                self.handle_plain_char(c);
            }
            _ => {}
        }
    }

    fn handle_plain_char(&mut self, ch: char) {
        self.insert_char(ch);
    }

    fn move_left(&mut self) {
        self.cursor = self.prev_atomic_boundary(self.cursor);
        self.preferred_column = None;
    }

    fn move_right(&mut self) {
        self.cursor = self.next_atomic_boundary(self.cursor);
        self.preferred_column = None;
    }

    fn move_word_left(&mut self) {
        self.cursor = self.beginning_of_previous_word();
        self.preferred_column = None;
    }

    fn move_word_right(&mut self) {
        self.cursor = self.end_of_next_word();
        self.preferred_column = None;
    }

    fn move_home(&mut self, move_up_at_bol: bool) {
        let bol = self.current_line_start();
        if move_up_at_bol && self.cursor == bol && bol > 0 {
            self.cursor = self.clamp_pos_to_nearest_boundary(self.beginning_of_line(bol - 1));
        } else {
            self.cursor = self.clamp_pos_to_nearest_boundary(bol);
        }
        self.preferred_column = None;
    }

    fn move_end(&mut self, move_down_at_eol: bool) {
        let eol = self.current_line_end();
        if move_down_at_eol && self.cursor == eol && eol < self.text.len() {
            self.cursor = self.clamp_pos_to_nearest_boundary(self.end_of_line(eol + 1));
        } else {
            self.cursor = self.clamp_pos_to_nearest_boundary(eol);
        }
        self.preferred_column = None;
    }

    fn move_up(&mut self) {
        let current_start = self.current_line_start();
        if current_start == 0 {
            // First line: there is no previous row, so land on the line head
            // (the beginning of the whole input) instead of doing nothing.
            self.cursor = 0;
            self.preferred_column = None;
            return;
        }
        let target_end = current_start.saturating_sub(1);
        let target_start = self.text[..target_end]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let target_col = self
            .preferred_column
            .unwrap_or_else(|| self.current_display_column());
        self.cursor = byte_index_at_display_column(
            &self.text[target_start..target_end],
            target_start,
            target_col,
        );
        self.cursor = self.clamp_pos_to_nearest_boundary(self.cursor);
        self.preferred_column = Some(target_col);
    }

    fn move_down(&mut self) {
        let current_end = self.current_line_end();
        if current_end >= self.text.len() {
            // Last line: there is no next row, so land on the line tail (the
            // end of the whole input) instead of doing nothing.
            self.cursor = self.text.len();
            self.preferred_column = None;
            return;
        }
        let target_start = current_end + 1;
        let target_end = self.text[target_start..]
            .find('\n')
            .map(|index| target_start + index)
            .unwrap_or(self.text.len());
        let target_col = self
            .preferred_column
            .unwrap_or_else(|| self.current_display_column());
        self.cursor = byte_index_at_display_column(
            &self.text[target_start..target_end],
            target_start,
            target_col,
        );
        self.cursor = self.clamp_pos_to_nearest_boundary(self.cursor);
        self.preferred_column = Some(target_col);
    }

    fn backspace(&mut self) {
        let previous = self.prev_atomic_boundary(self.cursor);
        if previous < self.cursor {
            self.remove_range(previous, self.cursor);
        }
    }

    fn delete(&mut self) {
        let next = self.next_atomic_boundary(self.cursor);
        if next > self.cursor {
            self.remove_range(self.cursor, next);
        }
    }

    fn delete_backward_word(&mut self) {
        let start = self.beginning_of_previous_word();
        self.kill_buffer = self.remove_range(start, self.cursor);
    }

    fn delete_forward_word(&mut self) {
        let end = self.end_of_next_word();
        self.kill_buffer = self.remove_range(self.cursor, end);
    }

    fn kill_to_start_of_line(&mut self, multiline: bool) {
        let start = self.current_line_start();
        if self.cursor == start && multiline && start > 0 {
            self.kill_buffer = self.remove_range(start - 1, start);
        } else {
            self.kill_buffer = self.remove_range(start, self.cursor);
        }
    }

    fn kill_to_end_of_line(&mut self, multiline: bool) {
        let end = self.current_line_end();
        if self.cursor == end && multiline && end < self.text.len() {
            self.kill_buffer = self.remove_range(self.cursor, end + 1);
        } else {
            self.kill_buffer = self.remove_range(self.cursor, end);
        }
    }

    fn yank(&mut self) {
        if !self.kill_buffer.is_empty() {
            let text = self.kill_buffer.clone();
            self.insert_str(text.as_str());
        }
    }

    pub fn remove_range(&mut self, start: usize, end: usize) -> String {
        let range = self.expand_range_to_element_boundaries(
            min(start, self.text.len())..min(end, self.text.len()),
        );
        if range.start >= range.end {
            return String::new();
        }
        let removed = self.text[range.clone()].to_string();
        self.text.replace_range(range.clone(), "");
        self.update_elements_after_replace(range.start, range.end, 0);
        self.cursor = self.clamp_pos_to_nearest_boundary(range.start);
        self.preferred_column = None;
        removed
    }

    fn current_line_index(&self) -> usize {
        self.text[..self.cursor]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
    }

    fn current_line_start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0)
    }

    fn current_line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map(|index| self.cursor + index)
            .unwrap_or(self.text.len())
    }

    fn beginning_of_line(&self, pos: usize) -> usize {
        let pos = min(pos, self.text.len());
        self.text[..pos]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0)
    }

    fn end_of_line(&self, pos: usize) -> usize {
        let pos = min(pos, self.text.len());
        self.text[pos..]
            .find('\n')
            .map(|index| pos + index)
            .unwrap_or(self.text.len())
    }

    fn current_display_column(&self) -> usize {
        self.text[self.current_line_start()..self.cursor]
            .graphemes(true)
            .map(grapheme_cell_width)
            .sum()
    }

    fn beginning_of_previous_word(&self) -> usize {
        let mut pos = self.cursor;
        while pos > 0 {
            let start = self.prev_atomic_boundary(pos);
            if is_word_grapheme(&self.text[start..pos]) {
                break;
            }
            pos = start;
        }
        while pos > 0 {
            let start = self.prev_atomic_boundary(pos);
            if !is_word_grapheme(&self.text[start..pos]) {
                break;
            }
            pos = start;
        }
        self.adjust_pos_out_of_elements(pos, true)
    }

    fn end_of_next_word(&self) -> usize {
        let mut pos = self.cursor;
        while pos < self.text.len() {
            let end = self.next_atomic_boundary(pos);
            if is_word_grapheme(&self.text[pos..end]) {
                break;
            }
            pos = end;
        }
        while pos < self.text.len() {
            let end = self.next_atomic_boundary(pos);
            if !is_word_grapheme(&self.text[pos..end]) {
                break;
            }
            pos = end;
        }
        self.adjust_pos_out_of_elements(pos, false)
    }

    pub fn insert_str_at(&mut self, at: usize, text: &str) {
        let at = self.clamp_pos_for_insertion(at);
        let text = sanitize_editor_text(text);
        if text.is_empty() {
            return;
        }
        self.text.insert_str(at, &text);
        self.update_elements_after_replace(at, at, text.len());
        self.cursor = at + text.len();
        self.preferred_column = None;
    }

    fn find_element_containing(&self, pos: usize) -> Option<usize> {
        self.elements
            .iter()
            .position(|element| pos > element.range.start && pos < element.range.end)
    }

    fn clamp_pos_to_nearest_boundary(&self, mut pos: usize) -> usize {
        pos = min(pos, self.text.len());
        if let Some(index) = self.find_element_containing(pos) {
            let element = &self.elements[index];
            let dist_start = pos.saturating_sub(element.range.start);
            let dist_end = element.range.end.saturating_sub(pos);
            if dist_start <= dist_end {
                element.range.start
            } else {
                element.range.end
            }
        } else {
            pos
        }
    }

    fn clamp_pos_for_insertion(&self, pos: usize) -> usize {
        self.clamp_pos_to_nearest_boundary(pos)
    }

    fn expand_range_to_element_boundaries(&self, mut range: Range<usize>) -> Range<usize> {
        loop {
            let mut changed = false;
            for element in &self.elements {
                if element.range.start < range.end && element.range.end > range.start {
                    let next_start = min(range.start, element.range.start);
                    let next_end = max(range.end, element.range.end);
                    if next_start != range.start || next_end != range.end {
                        range.start = next_start;
                        range.end = next_end;
                        changed = true;
                    }
                }
            }
            if !changed {
                return range;
            }
        }
    }

    fn shift_elements(&mut self, at: usize, removed: usize, inserted: usize) {
        let end = at.saturating_add(removed);
        let delta = inserted as isize - removed as isize;
        self.elements
            .retain(|element| !(element.range.start >= at && element.range.end <= end));

        for element in &mut self.elements {
            if element.range.end <= at {
                continue;
            }
            if element.range.start >= end {
                element.range.start = ((element.range.start as isize) + delta) as usize;
                element.range.end = ((element.range.end as isize) + delta) as usize;
                continue;
            }

            let new_start = min(at, element.range.start);
            let tail = element.range.end.saturating_sub(end);
            element.range.start = new_start;
            element.range.end = at.saturating_add(inserted).saturating_add(tail);
        }
    }

    fn update_elements_after_replace(&mut self, start: usize, end: usize, inserted_len: usize) {
        self.shift_elements(start, end.saturating_sub(start), inserted_len);
    }

    fn prev_atomic_boundary(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        if let Some(index) = self
            .elements
            .iter()
            .position(|element| pos > element.range.start && pos <= element.range.end)
        {
            return self.elements[index].range.start;
        }
        let boundary = previous_grapheme_boundary(self.text.as_str(), pos);
        if let Some(index) = self.find_element_containing(boundary) {
            self.elements[index].range.start
        } else {
            boundary
        }
    }

    fn next_atomic_boundary(&self, pos: usize) -> usize {
        if pos >= self.text.len() {
            return self.text.len();
        }
        if let Some(index) = self
            .elements
            .iter()
            .position(|element| pos >= element.range.start && pos < element.range.end)
        {
            return self.elements[index].range.end;
        }
        let boundary = next_grapheme_boundary(self.text.as_str(), pos);
        if let Some(index) = self.find_element_containing(boundary) {
            self.elements[index].range.end
        } else {
            boundary
        }
    }

    fn adjust_pos_out_of_elements(&self, pos: usize, prefer_start: bool) -> usize {
        if let Some(index) = self.find_element_containing(pos) {
            let element = &self.elements[index];
            if prefer_start {
                element.range.start
            } else {
                element.range.end
            }
        } else {
            pos
        }
    }
}

impl SearchPickerInput for Editor {
    fn text(&self) -> &str {
        self.text()
    }

    fn set_text(&mut self, text: String) {
        Self::set_text(self, text);
    }

    fn handle_line_input_key(&mut self, key: KeyEvent) {
        Self::handle_line_input_key(self, key);
    }
}

fn split_editor_lines_with_offsets(text: &str) -> Vec<Range<usize>> {
    let mut start = 0;
    let mut lines = Vec::new();
    for (index, ch) in text.char_indices() {
        if ch == '\n' {
            lines.push(start..index);
            start = index + 1;
        }
    }
    lines.push(start..text.len());
    lines
}

#[derive(Debug, Clone)]
struct WrappedEditorLine {
    range: Range<usize>,
    logical_line_index: usize,
    start_column: usize,
    end_column: usize,
}

fn wrapped_editor_lines(text: &str, width: u16) -> Vec<WrappedEditorLine> {
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();
    for (logical_line_index, range) in split_editor_lines_with_offsets(text)
        .into_iter()
        .enumerate()
    {
        let line_text = &text[range.clone()];
        // Rows are placed by grapheme display width, not by an arithmetic
        // `row_index * width`. A wide (CJK / emoji) grapheme that would
        // straddle a wrap boundary is pushed onto the next row so it never
        // overflows the requested width or offsets the cursor. Each row keeps
        // the absolute start/end display column of its content.
        let mut start_column = 0_usize;
        let mut column = 0_usize;
        for (_, grapheme) in line_text.grapheme_indices(true) {
            let grapheme_width = grapheme_cell_width(grapheme);
            let row_width = column.saturating_sub(start_column);
            if row_width > 0 && row_width.saturating_add(grapheme_width) > width {
                lines.push(WrappedEditorLine {
                    range: range.clone(),
                    logical_line_index,
                    start_column,
                    end_column: column,
                });
                start_column = column;
            }
            column = column.saturating_add(grapheme_width);
        }
        // The final (possibly empty) row of the logical line.
        lines.push(WrappedEditorLine {
            range: range.clone(),
            logical_line_index,
            start_column,
            end_column: column,
        });
    }
    lines
}

/// Back the byte index off to the nearest UTF-8 char boundary (0 when it
/// lands inside a multi-byte char). Guards every `text[a..b]` slice in the
/// editor against the "not a char boundary" panic.
fn snap_to_char_boundary(text: &str, mut index: usize) -> usize {
    index = min(index, text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn previous_grapheme_boundary(text: &str, index: usize) -> usize {
    text[..index]
        .grapheme_indices(true)
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_grapheme_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let grapheme = text[index..].graphemes(true).next().unwrap_or_default();
    index + grapheme.len()
}

fn byte_index_at_display_column(line: &str, offset: usize, target_column: usize) -> usize {
    let mut width = 0_usize;
    for (index, grapheme) in line.grapheme_indices(true) {
        let grapheme_width = grapheme_cell_width(grapheme);
        if width.saturating_add(grapheme_width) > target_column {
            return offset + index;
        }
        width = width.saturating_add(grapheme_width);
    }
    offset + line.len()
}

/// Map a cursor position to the index of the visual (soft-wrapped) row that
/// holds the cursor within the wrapped line set for a logical line. The
/// cursor is expressed as an absolute byte offset into the buffer; the cursor
/// column is the display column of the cursor within its logical line.
///
/// Returns `None` when the logical line has no rows (empty buffer), which
/// callers treat as "no row to move within".
fn visual_row_index_for_cursor(
    lines: &[WrappedEditorLine],
    logical_line_index: usize,
    cursor_column: usize,
) -> Option<usize> {
    // Only rows belonging to the requested logical line are candidates; a
    // wrapped line's `start_column`/`end_column` are absolute within that
    // logical line, so we can compare directly against the cursor column.
    let mut index = 0_usize;
    for (row_index, row) in lines.iter().enumerate() {
        if row.logical_line_index != logical_line_index {
            continue;
        }
        if row.end_column == 0 {
            // Empty row (empty logical line): the cursor sits on it.
            return Some(row_index);
        }
        if cursor_column >= row.start_column && cursor_column < row.end_column {
            return Some(row_index);
        }
        index = row_index;
    }
    // The cursor is at or past the last row's end column.
    if lines
        .iter()
        .any(|row| row.logical_line_index == logical_line_index)
    {
        Some(index)
    } else {
        None
    }
}

fn slice_display_window_styled(
    text: &str,
    range: Range<usize>,
    start_column: usize,
    end_column: usize,
    elements: &[EditorElement],
) -> Line<'static> {
    if end_column <= start_column {
        return Line::default();
    }

    let line_text = &text[range.clone()];
    let mut current_column = 0_usize;
    let mut current_style: Option<Style> = None;
    let mut current_segment = String::new();
    let mut spans = Vec::new();

    for (offset, grapheme) in line_text.grapheme_indices(true) {
        let grapheme_width = grapheme_cell_width(grapheme);
        let next_column = current_column.saturating_add(grapheme_width);
        if next_column <= start_column {
            current_column = next_column;
            continue;
        }
        if current_column >= end_column {
            break;
        }

        let absolute_start = range.start + offset;
        let absolute_end = absolute_start + grapheme.len();
        let style = if elements
            .iter()
            .any(|element| element.range.start < absolute_end && element.range.end > absolute_start)
        {
            Style::default()
                .fg(crate::theme::accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        if !current_segment.is_empty() && current_style.is_some_and(|current| style != current) {
            spans.push(Span::styled(
                std::mem::take(&mut current_segment),
                current_style.unwrap_or_default(),
            ));
        }
        current_style = Some(style);
        current_segment.push_str(grapheme);
        current_column = next_column;
    }

    if !current_segment.is_empty() {
        spans.push(Span::styled(
            current_segment,
            current_style.unwrap_or_default(),
        ));
    }

    if spans.is_empty() {
        Line::from("")
    } else {
        Line::from(spans)
    }
}

/// Terminal-safe display text for the editor buffer.
///
/// Mirrors the transcript's `sanitize_terminal_text` so the editor never
/// stores bytes that ratatui's buffer renderer drops or that make the
/// terminal's visible output disagree with the width math used for cursor
/// placement. The transcript sanitizes external content before display; the
/// editor applies the same policy at every mutation entry point (typing,
/// paste, history recall, attachment insertion) so the stored buffer is
/// always exactly what is rendered and measured.
fn sanitize_editor_text(text: &str) -> String {
    let stripped = strip_ansi_sequences(text);
    let mut out = String::with_capacity(stripped.len());
    for ch in stripped.chars() {
        match ch {
            '\n' => out.push(ch),
            '\r' => {}
            '\t' => {
                // unicode-width reports tab as width 1 while ratatui drops it
                // from the buffer (0 cells). Expand tabs to spaces so cursor
                // math and rendering agree; four columns is the common editor
                // tab stop.
                out.push_str("    ");
            }
            '\u{200e}' | '\u{200f}' => {}
            '\u{202a}'..='\u{202e}' => {}
            '\u{2066}'..='\u{2069}' => {}
            ch if ch.is_control() => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

/// Remove ANSI escape sequences (CSI, OSC, and single-char escapes) from text
/// that is about to enter the editor buffer. Pasted terminal output frequently
/// contains color/OSC sequences; leaving them in the buffer would make the
/// visible output and cursor math disagree.
fn strip_ansi_sequences(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0_usize;
    while index < bytes.len() {
        if bytes[index] != b'\x1b' {
            let ch = text[index..].chars().next().unwrap_or_default();
            out.push(ch);
            index = index.saturating_add(ch.len_utf8());
            continue;
        }
        let Some(&next) = bytes.get(index + 1) else {
            break;
        };
        match next {
            b'[' => {
                index = index.saturating_add(2);
                while index < bytes.len() {
                    let byte = bytes[index];
                    // CSI sequences may only contain ASCII parameter bytes; a
                    // non-ASCII byte means the sequence is truncated and the
                    // rest is real text (never land mid-char).
                    if !byte.is_ascii() {
                        break;
                    }
                    index = index.saturating_add(1);
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            b']' => {
                index = index.saturating_add(2);
                while index < bytes.len() {
                    match bytes[index] {
                        0x07 => {
                            index = index.saturating_add(1);
                            break;
                        }
                        0x1b if bytes.get(index + 1) == Some(&b'\\') => {
                            index = index.saturating_add(2);
                            break;
                        }
                        byte if !byte.is_ascii() => break,
                        _ => index = index.saturating_add(1),
                    }
                }
            }
            _ => {
                // Single-char escape (e.g. ESC M). Skip it only when it is a
                // plain ASCII byte; a non-ASCII leading byte is the start of a
                // multi-byte char and must be emitted as text.
                if bytes[index + 1].is_ascii() {
                    index = index.saturating_add(2);
                } else {
                    index = index.saturating_add(1);
                }
            }
        }
    }
    out
}

/// Halfwidth Katakana Voiced Sound Mark (dakuten).
const HALFWIDTH_KATAKANA_VOICED_SOUND_MARK: char = '\u{FF9E}';
/// Halfwidth Katakana Semi-Voiced Sound Mark (handakuten).
const HALFWIDTH_KATAKANA_SEMI_VOICED_SOUND_MARK: char = '\u{FF9F}';

/// Display width of one grapheme in terminal cells, matching ratatui's
/// `CellWidth` (unicode-width plus a +1 adjustment for the halfwidth katakana
/// dakuten/handakuten that terminals render as standalone halfwidth chars).
fn grapheme_cell_width(grapheme: &str) -> usize {
    let width = UnicodeWidthStr::width(grapheme);
    let marks = grapheme
        .chars()
        .filter(|ch| {
            matches!(
                *ch,
                HALFWIDTH_KATAKANA_VOICED_SOUND_MARK | HALFWIDTH_KATAKANA_SEMI_VOICED_SOUND_MARK
            )
        })
        .count();
    width.saturating_add(marks)
}

fn is_altgr(modifiers: KeyModifiers) -> bool {
    cfg!(windows)
        && modifiers.contains(KeyModifiers::CONTROL)
        && modifiers.contains(KeyModifiers::ALT)
}

fn is_word_separator(ch: char) -> bool {
    WORD_SEPARATORS.contains(ch)
}

fn is_word_grapheme(grapheme: &str) -> bool {
    grapheme
        .chars()
        .any(|ch| !ch.is_whitespace() && !is_word_separator(ch))
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::Editor;

    #[test]
    fn wrapped_view_expands_long_logical_lines_into_multiple_terminal_rows() {
        let editor = Editor::from_text("abcdefghij\nxy".to_string());

        assert_eq!(editor.wrapped_line_count(4), 4);
        let view = editor.render_wrapped_view(4, 8);
        let lines = view
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(lines, ["abcd", "efgh", "ij", "xy"]);
    }

    #[test]
    fn wrapped_view_uses_terminal_display_width_for_wide_characters() {
        let editor = Editor::from_text("你好世界".to_string());

        assert_eq!(editor.wrapped_line_count(4), 2);
        let view = editor.render_wrapped_view(4, 2);
        assert_eq!(view.lines.len(), 2);
    }

    #[test]
    fn wrapped_view_breaks_before_a_wide_char_that_crosses_the_boundary() {
        // "abc你" is 5 display columns wide; at width 4 the CJK char wraps
        // onto its own row because it cannot straddle the 4-column boundary.
        let editor = Editor::from_text("abc你".to_string());

        assert_eq!(editor.wrapped_line_count(4), 2);
        let view = editor.render_wrapped_view(4, 8);
        let lines = view
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(lines, ["abc", "你"]);
    }

    #[test]
    fn wrapped_view_places_the_cursor_after_a_wrapped_wide_char() {
        // Cursor at the end of "abc你" (display column 5). The terminal wraps
        // 你 onto row 1 at columns 0-1, so the cursor must sit at column 2 of
        // row 1, not at column 1 of an overflowing row 0.
        let mut editor = Editor::from_text("abc你".to_string());
        editor.set_cursor(editor.text().len());
        let view = editor.render_wrapped_view(4, 8);
        assert_eq!((view.cursor_x, view.cursor_y), (2, 1));
    }

    #[test]
    fn single_line_view_does_not_scroll_when_the_text_fits_exactly() {
        // "abcd" is exactly as wide as the viewport; the cursor at the end
        // must not push the window right and hide the leading character.
        let mut editor = Editor::from_text("abcd".to_string());
        editor.set_cursor(editor.text().len());
        let view = editor.render_view(4, 1);
        let text = view
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(text, ["abcd"]);
        assert_eq!((view.cursor_x, view.cursor_y), (3, 0));
    }

    #[test]
    fn single_line_view_scrolls_only_when_the_cursor_passes_the_width() {
        let mut editor = Editor::from_text("abcdef".to_string());
        editor.set_cursor(editor.text().len());
        let view = editor.render_view(4, 1);
        let text = view
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(text, ["def"]);
        assert_eq!((view.cursor_x, view.cursor_y), (3, 0));
    }

    #[test]
    fn set_cursor_clamps_to_a_utf8_boundary() {
        let mut editor = Editor::from_text("ab中文".to_string());
        editor.set_cursor(3);

        assert_eq!(editor.cursor(), 2);
        editor.set_cursor(usize::MAX);
        assert_eq!(editor.cursor(), editor.text().len());
    }

    #[test]
    fn set_elements_snaps_byte_ranges_to_utf8_boundaries() {
        // A range cut through the middle of a multi-byte char used to produce
        // a non-boundary element, which later panicked text[range] slicing.
        // Boundaries must snap back to the previous char boundary.
        let mut editor = Editor::from_text("ab中文".to_string());
        // "ab中文" = a(0) b(1) 中(2..5) 文(5..8); byte 3 is inside 中 and
        // byte 6 inside 文, both must snap back to 2 and 5.
        #[allow(clippy::single_range_in_vec_init)]
        // set_elements takes ranges, not collected indices
        editor.set_elements(vec![3..6]);

        assert_eq!(editor.draft_elements(), vec![2..5]);
        assert_eq!(
            editor.element_texts().into_iter().collect::<Vec<_>>(),
            vec!["中".to_string()]
        );
    }

    #[test]
    fn text_commands_reject_extra_modifiers() {
        let mut editor = Editor::from_text("one two".to_string());
        editor.set_cursor(editor.text().len());

        editor.handle_line_input_key(KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert_eq!(editor.cursor(), editor.text().len());

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(editor.cursor(), 4);
    }

    #[test]
    fn up_on_the_first_line_lands_on_the_input_head() {
        let mut editor = Editor::from_text("alpha\nbeta\ngamma".to_string());
        editor.set_cursor(2);

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));

        assert_eq!(editor.cursor(), 0);
    }

    #[test]
    fn down_on_the_last_line_lands_on_the_input_tail() {
        let mut editor = Editor::from_text("alpha\nbeta\ngamma".to_string());
        editor.set_cursor(12);

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

        assert_eq!(editor.cursor(), editor.text().len());
    }

    #[test]
    fn up_and_down_between_middle_lines_preserve_the_preferred_column() {
        let mut editor = Editor::from_text("alpha\nbeta\ngamma".to_string());
        editor.set_cursor(9);

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(editor.cursor(), 3);

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(editor.cursor(), 9);
    }

    #[test]
    fn single_line_inputs_ignore_up_and_down() {
        let mut editor = Editor::from_text("alpha".to_string());
        editor.set_cursor(2);

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(editor.cursor(), 2);

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(editor.cursor(), 2);
    }

    #[test]
    fn ctrl_h_is_reserved_for_application_help() {
        let mut editor = Editor::from_text("text".to_string());
        editor.set_cursor(editor.text().len());

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
        editor.handle_line_input_key(KeyEvent::new(KeyCode::Char('\u{0008}'), KeyModifiers::NONE));

        assert_eq!(editor.text(), "text");
    }

    #[test]
    fn alt_left_and_right_move_by_words() {
        let mut editor = Editor::from_text("one two three".to_string());
        editor.set_cursor(editor.text().len());

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 8);
        editor.handle_line_input_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 4);

        // Word-right stops before the trailing space (end of "two"), the
        // same semantics as Ctrl+Right.
        editor.handle_line_input_key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 7);
        editor.handle_line_input_key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 13);
    }

    #[test]
    fn alt_and_ctrl_backspace_delete_backward_word() {
        let mut editor = Editor::from_text("one two three".to_string());
        editor.set_cursor(editor.text().len());

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
        assert_eq!(editor.text(), "one two ");

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));
        assert_eq!(editor.text(), "one ");

        // ESC DEL encoding of Alt+Backspace on terminals without the
        // enhanced keyboard protocol.
        let mut raw = Editor::from_text("one two".to_string());
        raw.set_cursor(raw.text().len());
        raw.handle_line_input_key(KeyEvent::new(KeyCode::Char('\u{007f}'), KeyModifiers::ALT));
        assert_eq!(raw.text(), "one ");
    }

    #[test]
    fn alt_and_ctrl_delete_delete_forward_word() {
        let mut editor = Editor::from_text("one two three".to_string());
        editor.set_cursor(0);

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::ALT));
        assert_eq!(editor.text(), " two three");

        let mut editor = Editor::from_text("one two three".to_string());
        editor.set_cursor(0);
        editor.handle_line_input_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL));
        assert_eq!(editor.text(), " two three");
    }
    #[test]
    fn alt_b_and_alt_f_move_by_words() {
        let mut editor = Editor::from_text("one two three".to_string());
        editor.set_cursor(editor.text().len());

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 8);
        editor.handle_line_input_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 4);

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 7);
        editor.handle_line_input_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 13);
    }

    #[test]
    fn csi_u_control_codes_move_by_words() {
        // CSI u reports the Left cursor key as U+0001 and Right as U+0002,
        // carrying the Alt (\x1b[1;3u) or Ctrl (\x1b[1;5u) modifier.
        let mut editor = Editor::from_text("one two three".to_string());
        editor.set_cursor(editor.text().len());

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Char('\u{0001}'), KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 8);
        editor.handle_line_input_key(KeyEvent::new(KeyCode::Char('\u{0002}'), KeyModifiers::ALT));
        assert_eq!(editor.cursor(), 13);

        let mut editor = Editor::from_text("one two three".to_string());
        editor.set_cursor(editor.text().len());
        editor.handle_line_input_key(KeyEvent::new(
            KeyCode::Char('\u{0001}'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(editor.cursor(), 8);
        editor.handle_line_input_key(KeyEvent::new(
            KeyCode::Char('\u{0002}'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(editor.cursor(), 13);
    }

    #[test]
    fn esc_bs_word_delete() {
        // Terminals that send BS instead of DEL for Option/Ctrl+Backspace
        // surface as an ALT/CONTROL-modified U+0008 char.
        let mut editor = Editor::from_text("one two three".to_string());
        editor.set_cursor(editor.text().len());

        editor.handle_line_input_key(KeyEvent::new(KeyCode::Char('\u{0008}'), KeyModifiers::ALT));
        assert_eq!(editor.text(), "one two ");
        editor.handle_line_input_key(KeyEvent::new(
            KeyCode::Char('\u{0008}'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(editor.text(), "one ");
    }

    #[test]
    fn inserted_text_is_sanitized_for_terminal_display() {
        // ANSI escape sequences are stripped so the terminal output and the
        // width math used for cursor placement agree.
        let mut editor = Editor::default();
        editor.insert_str("\x1b[31mred\x1b[0m");
        assert_eq!(editor.text(), "red");
        assert_eq!(editor.cursor(), 3);

        // Tabs expand to spaces: unicode-width reports a tab as width 1 while
        // ratatui drops it (0 cells), which used to shift the cursor.
        let mut editor = Editor::default();
        editor.insert_str("a\tb");
        assert_eq!(editor.text(), "a    b");
        assert_eq!(editor.cursor(), 6);

        // Control characters become spaces (transcript sanitize policy).
        let mut editor = Editor::default();
        editor.insert_str("x\u{1}y");
        assert_eq!(editor.text(), "x y");

        // Bidi marks are dropped.
        let mut editor = Editor::default();
        editor.insert_str("a\u{200e}b");
        assert_eq!(editor.text(), "ab");

        // CR is removed.
        let mut editor = Editor::default();
        editor.insert_str("a\rb");
        assert_eq!(editor.text(), "ab");

        // An all-escape insertion is a no-op.
        let mut editor = Editor::from_text("abc".to_string());
        editor.insert_str("\x1b[31m");
        assert_eq!(editor.text(), "abc");
        assert_eq!(editor.cursor(), 3);
    }

    #[test]
    fn set_text_and_from_text_sanitize() {
        let mut editor = Editor::default();
        editor.set_text("\x1b[31mred\x1b[0m".to_string());
        assert_eq!(editor.text(), "red");
        assert_eq!(editor.cursor(), 3);

        let editor = Editor::from_text("a\u{1}b".to_string());
        assert_eq!(editor.text(), "a b");
    }

    #[test]
    fn insert_element_keeps_ranges_on_sanitized_text() {
        let mut editor = Editor::default();
        editor.insert_element("\x1b[31mpath\x1b[0m");
        assert_eq!(editor.text(), "path");
        assert_eq!(editor.draft_elements(), vec![0..4]);
    }

    #[test]
    fn cursor_math_matches_ratatui_for_halfwidth_katakana_marks() {
        // unicode-width reports U+FF9E (halfwidth dakuten) as zero width, but
        // ratatui renders it as one cell; the editor math must agree.
        let mut editor = Editor::from_text("a\u{FF9E}".to_string());
        editor.set_cursor(editor.text().len());
        let view = editor.render_wrapped_view(10, 1);
        assert_eq!((view.cursor_x, view.cursor_y), (2, 0));
    }

    #[test]
    fn pasted_escape_sequences_do_not_shift_the_cursor() {
        let mut editor = Editor::from_text("prefix".to_string());
        editor.insert_str("\x1b[31mred\x1b[0m");
        assert_eq!(editor.text(), "prefixred");
        let view = editor.render_wrapped_view(20, 1);
        assert_eq!(view.cursor_x, 9);
    }
}
