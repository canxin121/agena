use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

const LEGACY_PASTE_INTERVAL: Duration = Duration::from_millis(8);
const LEGACY_PASTE_MIN_CHARS: usize = 3;
const LEGACY_PASTE_MAX_BYTES: usize = 256 * 1024;

/// Normalizes legacy terminals that ignore bracketed-paste mode. The timing
/// heuristic lives at the terminal boundary and is enabled only while the App
/// reports an active text target; generic editors no longer know about tty
/// timing or protocol support.
#[derive(Debug, Default)]
pub(super) struct InputNormalizer {
    text_input_active: bool,
    pending: Vec<KeyEvent>,
    pending_text: String,
    last_at: Option<Instant>,
    ready: VecDeque<Event>,
}

impl InputNormalizer {
    pub(super) fn set_text_input_active(&mut self, active: bool) {
        if self.text_input_active != active {
            self.flush_pending(false);
            self.text_input_active = active;
        }
    }

    pub(super) fn accept(&mut self, event: Event) {
        if self.text_input_active
            && let Event::Key(key) = &event
        {
            let key = *key;
            if let Some(ch) = legacy_char_for_key(key) {
                self.accept_character(key, ch);
                return;
            }
            if let Some(text) = legacy_text_for_key(key, !self.pending.is_empty()) {
                let now = Instant::now();
                let contiguous = self
                    .last_at
                    .is_some_and(|last| now.duration_since(last) <= LEGACY_PASTE_INTERVAL);
                if !contiguous {
                    self.flush_pending(false);
                }
                if self.pending_text.len().saturating_add(text.len()) > LEGACY_PASTE_MAX_BYTES {
                    self.flush_pending(false);
                }
                self.pending.push(key);
                self.pending_text.push_str(text);
                self.last_at = Some(now);
                return;
            }
        }

        self.flush_pending(false);
        self.ready.push_back(event);
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.last_at.map(|last| last + LEGACY_PASTE_INTERVAL)
    }

    pub(super) fn flush_timed_out(&mut self) {
        self.flush_pending(true);
    }

    pub(super) fn flush_all(&mut self) {
        self.flush_pending(false);
    }

    pub(super) fn reset(&mut self) {
        self.pending.clear();
        self.pending_text.clear();
        self.last_at = None;
        self.ready.clear();
    }

    pub(super) fn pop_ready(&mut self) -> Option<Event> {
        self.ready.pop_front()
    }

    pub(super) fn take_ready(&mut self) -> VecDeque<Event> {
        std::mem::take(&mut self.ready)
    }

    pub(super) fn restore_ready(&mut self, mut events: VecDeque<Event>) {
        events.append(&mut self.ready);
        self.ready = events;
    }

    fn flush_pending(&mut self, allow_paste: bool) {
        if self.pending.is_empty() {
            self.last_at = None;
            return;
        }

        if allow_paste && self.pending.len() >= LEGACY_PASTE_MIN_CHARS {
            self.ready
                .push_back(Event::Paste(std::mem::take(&mut self.pending_text)));
            self.pending.clear();
        } else {
            self.ready.extend(self.pending.drain(..).map(Event::Key));
            self.pending_text.clear();
        }
        self.last_at = None;
    }
}

fn legacy_text_for_key(key: KeyEvent, paste_in_progress: bool) -> Option<&'static str> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    match key.code {
        KeyCode::Enter if paste_in_progress && key.modifiers.is_empty() => Some("\n"),
        KeyCode::Tab if paste_in_progress && key.modifiers.is_empty() => Some("\t"),
        _ => None,
    }
}

fn legacy_char_for_key(key: KeyEvent) -> Option<char> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    match key {
        KeyEvent {
            code: KeyCode::Char(ch),
            modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            ..
        } => Some(ch),
        _ => None,
    }
}

// Character key values cannot borrow a temporary UTF-8 buffer, so accept them
// in a dedicated branch before the static control-key mapping above.
impl InputNormalizer {
    fn accept_character(&mut self, key: KeyEvent, ch: char) {
        let now = Instant::now();
        let contiguous = self
            .last_at
            .is_some_and(|last| now.duration_since(last) <= LEGACY_PASTE_INTERVAL);
        if !contiguous {
            self.flush_pending(false);
        }
        if self.pending_text.len().saturating_add(ch.len_utf8()) > LEGACY_PASTE_MAX_BYTES {
            self.flush_pending(false);
        }
        self.pending.push(key);
        self.pending_text.push(ch);
        self.last_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ch: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
    }

    #[test]
    fn rapid_text_is_emitted_as_one_legacy_paste_only_for_text_targets() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        for ch in ['a', 'b', 'c'] {
            input.accept(key(ch));
        }
        input.flush_timed_out();
        assert_eq!(input.pop_ready(), Some(Event::Paste("abc".to_string())));

        input.set_text_input_active(false);
        input.accept(key('j'));
        assert!(matches!(input.pop_ready(), Some(Event::Key(_))));
    }

    #[test]
    fn short_sequences_remain_individual_key_events() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        for ch in ['g', 'g'] {
            input.accept(key(ch));
        }
        input.flush_all();
        assert!(matches!(input.pop_ready(), Some(Event::Key(_))));
        assert!(matches!(input.pop_ready(), Some(Event::Key(_))));
    }

    #[test]
    fn ready_input_survives_a_terminal_suspension_boundary() {
        let mut input = InputNormalizer::default();
        input.accept(key('x'));
        let preserved = input.take_ready();
        input.reset();
        input.restore_ready(preserved);
        assert_eq!(input.pop_ready(), Some(key('x')));
    }
}
