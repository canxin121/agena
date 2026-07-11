use std::{
    cmp::{max, min},
    ops::Range,
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::search_list::SearchListInput;

const WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";
const PASTE_BURST_MIN_CHARS: u16 = 3;
const PASTE_BURST_CHAR_INTERVAL_MS: u64 = 8;
const PASTE_ENTER_SUPPRESS_WINDOW_MS: u64 = 120;

#[derive(Debug, Clone, Default)]
pub struct Editor {
    text: String,
    cursor: usize,
    preferred_column: Option<usize>,
    kill_buffer: String,
    elements: Vec<EditorElement>,
    paste_burst: PasteBurst,
}

#[derive(Debug, Clone)]
pub struct EditorView {
    pub lines: Vec<Line<'static>>,
    pub cursor_x: u16,
    pub cursor_y: u16,
}

#[derive(Debug, Clone)]
struct EditorElement {
    range: Range<usize>,
}

#[derive(Debug, Clone, Default)]
struct PasteBurst {
    last_plain_char_time: Option<Instant>,
    consecutive_plain_char_burst: u16,
    burst_window_until: Option<Instant>,
    buffer: String,
    active: bool,
    pending_first_char: Option<(char, Instant)>,
}

#[derive(Debug, Clone, Copy)]
enum PasteCharDecision {
    BeginBuffer { retro_chars: u16 },
    BufferAppend,
    RetainFirstChar,
    BeginBufferFromPending,
}

#[derive(Debug, Clone)]
enum PasteFlushResult {
    Paste(String),
    Typed(char),
    None,
}

#[derive(Debug, Clone)]
struct RetroGrab {
    start_byte: usize,
    grabbed: String,
}

impl Editor {
    pub fn from_text(text: String) -> Self {
        let cursor = text.len();
        Self {
            text,
            cursor,
            preferred_column: None,
            kill_buffer: String::new(),
            elements: Vec::new(),
            paste_burst: PasteBurst::default(),
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
        self.text = text;
        self.cursor = self.text.len();
        self.preferred_column = None;
        self.elements.clear();
        self.paste_burst = PasteBurst::default();
    }

    pub fn set_elements(&mut self, elements: Vec<Range<usize>>) {
        let mut normalized = elements
            .into_iter()
            .filter_map(|range| {
                let start = min(range.start, self.text.len());
                let end = min(range.end, self.text.len());
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
        self.paste_burst = PasteBurst::default();
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
        if text.is_empty() {
            return;
        }
        let start = self.clamp_pos_for_insertion(self.cursor);
        self.insert_str_at(start, text);
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
        let hscroll = current_col.saturating_sub(width.saturating_sub(1));
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
                    width,
                    self.elements.as_slice(),
                )
            })
            .collect::<Vec<_>>();

        EditorView {
            lines: visible_lines,
            cursor_x: min(current_col.saturating_sub(hscroll), u16::MAX as usize) as u16,
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
                    width,
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

    pub fn should_insert_newline_on_enter(&mut self) -> bool {
        let now = Instant::now();
        self.flush_pending_input_if_due(now);
        self.paste_burst
            .newline_should_insert_instead_of_submit(now)
    }

    pub fn insert_explicit_newline(&mut self) {
        self.flush_all_pending_input();
        self.insert_newline();
        self.paste_burst.clear_window_after_non_char();
    }

    pub fn insert_newline_from_enter(&mut self) {
        let now = Instant::now();
        self.flush_pending_input_if_due(now);
        if self.paste_burst.append_newline_if_active(now) {
            return;
        }
        self.flush_all_pending_input();
        self.insert_newline();
        self.paste_burst.clear_window_after_non_char();
    }

    pub fn flush_pending_input_if_due(&mut self, now: Instant) {
        match self.paste_burst.flush_if_due(now) {
            PasteFlushResult::Paste(text) => self.insert_str(text.as_str()),
            PasteFlushResult::Typed(ch) => self.insert_char(ch),
            PasteFlushResult::None => {}
        }
    }

    pub fn flush_all_pending_input(&mut self) {
        match self.paste_burst.flush_now() {
            PasteFlushResult::Paste(text) => self.insert_str(text.as_str()),
            PasteFlushResult::Typed(ch) => self.insert_char(ch),
            PasteFlushResult::None => {}
        }
    }

    pub fn cursor_on_first_line(&self) -> bool {
        self.current_line_index() == 0
    }

    fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    fn handle_input_key(&mut self, key: KeyEvent, multiline: bool) {
        let now = Instant::now();
        self.flush_pending_input_if_due(now);

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
                self.prepare_for_command();
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
                self.prepare_for_command();
                self.move_left();
            }
            KeyEvent {
                code: KeyCode::Left,
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                || (modifiers.contains(KeyModifiers::ALT) && !is_altgr(modifiers)) =>
            {
                self.prepare_for_command();
                self.move_word_left();
            }
            KeyEvent {
                code: KeyCode::Char('b'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => {
                self.prepare_for_command();
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
                self.prepare_for_command();
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
                self.prepare_for_command();
                self.move_right();
            }
            KeyEvent {
                code: KeyCode::Right,
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                || (modifiers.contains(KeyModifiers::ALT) && !is_altgr(modifiers)) =>
            {
                self.prepare_for_command();
                self.move_word_right();
            }
            KeyEvent {
                code: KeyCode::Char('f'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => {
                self.prepare_for_command();
                self.move_word_right();
            }
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            } if multiline => {
                self.prepare_for_command();
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
                self.prepare_for_command();
                self.move_up();
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            } if multiline => {
                self.prepare_for_command();
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
                self.prepare_for_command();
                self.move_down();
            }
            KeyEvent {
                code: KeyCode::Home,
                ..
            } => {
                self.prepare_for_command();
                self.move_home(false);
            }
            KeyEvent {
                code: KeyCode::End, ..
            } => {
                self.prepare_for_command();
                self.move_end(false);
            }
            KeyEvent {
                code: KeyCode::Char('h'),
                modifiers,
                ..
            } if modifiers == (KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.prepare_for_command();
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Backspace,
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => {
                self.prepare_for_command();
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Char('\u{007f}'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => {
                self.prepare_for_command();
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0008}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('\u{007f}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.prepare_for_command();
                self.backspace();
            }
            KeyEvent {
                code: KeyCode::Delete,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.prepare_for_command();
                if self.cursor >= self.text.len() {
                    self.backspace();
                } else {
                    self.delete();
                }
            }
            KeyEvent {
                code: KeyCode::Delete,
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => {
                self.prepare_for_command();
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
                self.prepare_for_command();
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
                self.prepare_for_command();
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
                self.prepare_for_command();
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
                self.prepare_for_command();
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
                self.prepare_for_command();
                self.yank();
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if is_altgr(modifiers) => {
                self.handle_plain_char(c, now);
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } => {
                self.handle_plain_char(c, now);
            }
            _ => {}
        }
    }

    fn prepare_for_command(&mut self) {
        self.flush_all_pending_input();
        self.paste_burst.clear_window_after_non_char();
    }

    fn handle_plain_char(&mut self, ch: char, now: Instant) {
        match self.paste_burst.on_plain_char(ch, now) {
            PasteCharDecision::RetainFirstChar => {}
            PasteCharDecision::BufferAppend | PasteCharDecision::BeginBufferFromPending => {
                self.paste_burst.append_char_to_buffer(ch, now);
            }
            PasteCharDecision::BeginBuffer { retro_chars } => {
                if let Some(retro) = self.decide_retro_grab(retro_chars as usize) {
                    self.remove_range(retro.start_byte, self.cursor);
                    self.paste_burst
                        .begin_with_retro_grabbed(retro.grabbed, now);
                    self.paste_burst.append_char_to_buffer(ch, now);
                } else {
                    self.flush_all_pending_input();
                    self.insert_char(ch);
                }
            }
        }
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
        UnicodeWidthStr::width(&self.text[self.current_line_start()..self.cursor])
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

    fn decide_retro_grab(&self, retro_chars: usize) -> Option<RetroGrab> {
        let before = &self.text[..self.cursor];
        let start_byte = retro_start_index(before, retro_chars);
        if self.range_intersects_element(start_byte..self.cursor) {
            return None;
        }
        let grabbed = before[start_byte..].to_string();
        let looks_pastey = grabbed.chars().any(char::is_whitespace)
            || grabbed
                .chars()
                .any(|ch| matches!(ch, '/' | '\\' | ':' | '=' | ',' | '.'))
            || grabbed.chars().count() >= 16;
        looks_pastey.then_some(RetroGrab {
            start_byte,
            grabbed,
        })
    }

    pub fn insert_str_at(&mut self, at: usize, text: &str) {
        let at = self.clamp_pos_for_insertion(at);
        self.text.insert_str(at, text);
        self.update_elements_after_replace(at, at, text.len());
        self.cursor = at + text.len();
        self.preferred_column = None;
    }

    fn find_element_containing(&self, pos: usize) -> Option<usize> {
        self.elements
            .iter()
            .position(|element| pos > element.range.start && pos < element.range.end)
    }

    fn range_intersects_element(&self, range: Range<usize>) -> bool {
        self.elements
            .iter()
            .any(|element| element.range.start < range.end && element.range.end > range.start)
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

impl SearchListInput for Editor {
    fn text(&self) -> &str {
        self.text()
    }

    fn set_text(&mut self, text: String) {
        Self::set_text(self, text);
    }

    fn handle_line_input_key(&mut self, key: KeyEvent) {
        Self::handle_line_input_key(self, key);
    }

    fn flush_all_pending_input(&mut self) {
        Self::flush_all_pending_input(self);
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
        let display_width = UnicodeWidthStr::width(&text[range.clone()]);
        let row_count = display_width.max(1).div_ceil(width);
        for row_index in 0..row_count {
            let start_column = row_index.saturating_mul(width);
            lines.push(WrappedEditorLine {
                range: range.clone(),
                logical_line_index,
                start_column,
                end_column: start_column.saturating_add(width),
            });
        }
    }
    lines
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
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > target_column {
            return offset + index;
        }
        width = width.saturating_add(grapheme_width);
    }
    offset + line.len()
}

fn slice_display_window_styled(
    text: &str,
    range: Range<usize>,
    start_column: usize,
    width: usize,
    elements: &[EditorElement],
) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }

    let end_column = start_column.saturating_add(width);
    let line_text = &text[range.clone()];
    let mut current_column = 0_usize;
    let mut current_style: Option<Style> = None;
    let mut current_segment = String::new();
    let mut spans = Vec::new();

    for (offset, grapheme) in line_text.grapheme_indices(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
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
                .fg(Color::LightCyan)
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

fn is_altgr(modifiers: KeyModifiers) -> bool {
    cfg!(windows)
        && modifiers.contains(KeyModifiers::CONTROL)
        && modifiers.contains(KeyModifiers::ALT)
}

fn is_word_separator(ch: char) -> bool {
    WORD_SEPARATORS.contains(ch)
}

#[cfg(test)]
mod tests {
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
    fn set_cursor_clamps_to_a_utf8_boundary() {
        let mut editor = Editor::from_text("ab中文".to_string());
        editor.set_cursor(3);

        assert_eq!(editor.cursor(), 2);
        editor.set_cursor(usize::MAX);
        assert_eq!(editor.cursor(), editor.text().len());
    }
}

fn is_word_grapheme(grapheme: &str) -> bool {
    grapheme
        .chars()
        .any(|ch| !ch.is_whitespace() && !is_word_separator(ch))
}

fn retro_start_index(before: &str, retro_chars: usize) -> usize {
    let mut index = before.len();
    for _ in 0..retro_chars {
        let previous = previous_grapheme_boundary(before, index);
        if previous == index {
            break;
        }
        index = previous;
    }
    index
}

impl PasteBurst {
    fn on_plain_char(&mut self, ch: char, now: Instant) -> PasteCharDecision {
        let interval = Duration::from_millis(PASTE_BURST_CHAR_INTERVAL_MS);
        match self.last_plain_char_time {
            Some(previous) if now.duration_since(previous) <= interval => {
                self.consecutive_plain_char_burst =
                    self.consecutive_plain_char_burst.saturating_add(1);
            }
            _ => self.consecutive_plain_char_burst = 1,
        }
        self.last_plain_char_time = Some(now);

        if self.active {
            self.extend_window(now);
            return PasteCharDecision::BufferAppend;
        }

        if let Some((held, held_at)) = self.pending_first_char
            && now.duration_since(held_at) <= interval
        {
            self.active = true;
            let _ = self.pending_first_char.take();
            self.buffer.push(held);
            self.extend_window(now);
            return PasteCharDecision::BeginBufferFromPending;
        }

        if self.consecutive_plain_char_burst >= PASTE_BURST_MIN_CHARS {
            return PasteCharDecision::BeginBuffer {
                retro_chars: self.consecutive_plain_char_burst.saturating_sub(1),
            };
        }

        self.pending_first_char = Some((ch, now));
        PasteCharDecision::RetainFirstChar
    }

    fn flush_if_due(&mut self, now: Instant) -> PasteFlushResult {
        let timed_out = self.last_plain_char_time.is_some_and(|previous| {
            now.duration_since(previous) > Duration::from_millis(PASTE_BURST_CHAR_INTERVAL_MS)
        });

        if !timed_out {
            return PasteFlushResult::None;
        }

        self.flush_now()
    }

    fn flush_now(&mut self) -> PasteFlushResult {
        self.last_plain_char_time = None;
        self.consecutive_plain_char_burst = 0;

        if self.active || !self.buffer.is_empty() {
            self.active = false;
            self.burst_window_until = None;
            let text = std::mem::take(&mut self.buffer);
            return PasteFlushResult::Paste(text);
        }

        if let Some((ch, _)) = self.pending_first_char.take() {
            self.burst_window_until = None;
            return PasteFlushResult::Typed(ch);
        }

        PasteFlushResult::None
    }

    fn append_char_to_buffer(&mut self, ch: char, now: Instant) {
        self.buffer.push(ch);
        self.extend_window(now);
    }

    fn begin_with_retro_grabbed(&mut self, grabbed: String, now: Instant) {
        if !grabbed.is_empty() {
            self.buffer.push_str(grabbed.as_str());
        }
        self.active = true;
        self.extend_window(now);
    }

    fn newline_should_insert_instead_of_submit(&self, now: Instant) -> bool {
        self.active
            || self.burst_window_until.is_some_and(|until| now <= until)
            || self.pending_first_char.is_some()
    }

    fn append_newline_if_active(&mut self, now: Instant) -> bool {
        if self.active {
            self.buffer.push('\n');
            self.extend_window(now);
            true
        } else {
            false
        }
    }

    fn extend_window(&mut self, now: Instant) {
        self.burst_window_until = Some(now + Duration::from_millis(PASTE_ENTER_SUPPRESS_WINDOW_MS));
    }

    fn clear_window_after_non_char(&mut self) {
        self.last_plain_char_time = None;
        self.consecutive_plain_char_burst = 0;
        self.burst_window_until = None;
        self.active = false;
        self.pending_first_char = None;
        self.buffer.clear();
    }
}
