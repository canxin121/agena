//! Terminal input parsing and key event handling.

use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[cfg(unix)]
const RESIZE_RECHECK_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(not(unix))]
const INPUT_RECHECK_INTERVAL: Duration = Duration::from_millis(8);

const LEGACY_PASTE_INTERVAL: Duration = Duration::from_millis(8);
const LEGACY_PASTE_MIN_CHARS: usize = 3;
const LEGACY_PASTE_MAX_BYTES: usize = 256 * 1024;
/// Terminals that forward the key press which confirmed an input-method (IME)
/// candidate deliver the composed text and that `Enter` inside the same input
/// burst. An application cannot query a terminal for its composition state, so
/// the boundary decides from delivery shape and timing instead: text that
/// arrived faster than a human types, followed immediately by a bare `Enter`.
/// Waiting out this window costs one repeated key press; delivering that
/// `Enter` would submit an answer or advance a wizard page the user never
/// confirmed.
const IME_COMMIT_ENTER_WINDOW: Duration = Duration::from_millis(60);

/// The runtime's sole terminal-input readiness source.
///
/// Crossterm's `EventStream` owns an unjoinable background reader. That makes
/// it impossible to prove that stdin has been released before suspending the
/// TUI for an editor or transfer helper. On Unix, `TerminalInput` instead waits
/// for descriptor readiness without reading bytes. Other platforms use a
/// short, cancellable async poll. `event::read` is called only by the runtime
/// task and only after a non-blocking poll reports a complete event.
pub struct TerminalInput {
    #[cfg(unix)]
    readiness: tokio::io::unix::AsyncFd<StdinDescriptor>,
}

#[cfg(unix)]
#[derive(Debug)]
struct StdinDescriptor;

#[cfg(unix)]
impl std::os::fd::AsRawFd for StdinDescriptor {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        std::io::stdin().as_raw_fd()
    }
}

impl TerminalInput {
    pub fn new() -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                readiness: tokio::io::unix::AsyncFd::new(StdinDescriptor)?,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {})
        }
    }

    pub async fn next(&self) -> io::Result<Event> {
        loop {
            if event::poll(Duration::ZERO)? {
                return event::read();
            }

            #[cfg(unix)]
            match tokio::time::timeout(RESIZE_RECHECK_INTERVAL, self.readiness.readable()).await {
                Ok(Ok(mut readiness)) => readiness.clear_ready(),
                Ok(Err(error)) => return Err(error),
                Err(..) => {
                    // Crossterm also observes SIGWINCH through an internal
                    // signal source, which is not represented by stdin
                    // readiness. The bounded recheck picks up resize events.
                }
            }

            #[cfg(not(unix))]
            tokio::time::sleep(INPUT_RECHECK_INTERVAL).await;
        }
    }
}

/// Normalizes legacy terminals that ignore bracketed-paste mode. The timing
/// heuristic lives at the terminal boundary and is enabled only while the App
/// reports an active text target; generic editors no longer know about tty
/// timing or protocol support.
///
/// The boundary owns the input-method (IME) rule as well. A terminal that
/// forwards the `Enter` used to confirm a candidate would otherwise let that
/// key reach the application as a command while the committed text is applied
/// separately, so the user sees both the text and an action they never asked
/// for. Such an `Enter` is recognized by its delivery, not by a modifier:
/// either it arrives inside the text burst, or it follows machine-delivered
/// text within [`IME_COMMIT_ENTER_WINDOW`]. Both shapes are absorbed here and
/// never become application keys.
#[derive(Debug, Default)]
pub struct InputNormalizer {
    text_input_active: bool,
    pending: Vec<KeyEvent>,
    pending_text: String,
    /// Characters in the current burst; more than one proves machine delivery.
    pending_chars: usize,
    /// A terminal-delivered newline joined the current burst, so the burst is
    /// text: the application must never observe a command key for it.
    pending_has_text_enter: bool,
    /// The terminal-delivered newline is still the last key of the burst, which
    /// makes it the commit key of a candidate rather than pasted content.
    pending_ends_with_text_enter: bool,
    /// Delivery shape of the last burst handed to the target.
    last_burst: Option<TextBurst>,
    last_at: Option<Instant>,
    ready: VecDeque<Event>,
}

/// How the last text burst reached the application.
#[derive(Debug, Clone, Copy)]
struct TextBurst {
    at: Instant,
    /// More than one character, a non-ASCII character, or a newline delivered
    /// as text: input no human key sequence produced at that cadence.
    machine: bool,
}

impl InputNormalizer {
    pub fn set_text_input_active(&mut self, active: bool) {
        if self.text_input_active != active {
            self.flush_pending(false);
            self.text_input_active = active;
        }
    }

    pub fn accept(&mut self, event: Event) {
        if self.text_input_active
            && let Event::Key(key) = &event
        {
            let key = *key;
            if let Some(ch) = legacy_char_for_key(key) {
                self.accept_character(key, ch);
                return;
            }
            let now = Instant::now();
            if self.absorb_ime_commit_enter(key, now) {
                return;
            }
            if let Some(text) = legacy_text_for_key(key, !self.pending.is_empty()) {
                self.accept_text_enter(key, text, now);
                return;
            }
        }

        self.flush_pending(false);
        self.ready.push_back(event);
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.last_at.map(|last| last + LEGACY_PASTE_INTERVAL)
    }

    pub fn flush_timed_out(&mut self) {
        self.flush_pending(true);
    }

    pub fn flush_all(&mut self) {
        self.flush_pending(false);
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.pending_text.clear();
        self.pending_chars = 0;
        self.pending_has_text_enter = false;
        self.pending_ends_with_text_enter = false;
        self.last_burst = None;
        self.last_at = None;
        self.ready.clear();
    }

    pub fn pop_ready(&mut self) -> Option<Event> {
        self.ready.pop_front()
    }

    pub fn take_ready(&mut self) -> VecDeque<Event> {
        std::mem::take(&mut self.ready)
    }

    pub fn restore_ready(&mut self, mut events: VecDeque<Event>) {
        events.append(&mut self.ready);
        self.ready = events;
    }

    /// Recognize the `Enter` a terminal forwards together with an IME commit
    /// after the committed text already reached the target as its own burst.
    fn absorb_ime_commit_enter(&mut self, key: KeyEvent, now: Instant) -> bool {
        if key.code != KeyCode::Enter || !key.modifiers.is_empty() {
            return false;
        }
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return false;
        }
        let Some(burst) = self.last_burst else {
            return false;
        };
        if !burst.machine || now.duration_since(burst.at) > IME_COMMIT_ENTER_WINDOW {
            return false;
        }
        // One commit owns one `Enter`; a repeated press is a real key again.
        self.last_burst = None;
        true
    }

    /// Keep the forwarded commit key inside the burst it belongs to. It is
    /// tracked as text so [`Self::flush_pending`] can never hand it to the
    /// application as an `Enter` command.
    fn accept_text_enter(&mut self, key: KeyEvent, text: &str, now: Instant) {
        if self.pending_text.len().saturating_add(text.len()) > LEGACY_PASTE_MAX_BYTES {
            self.flush_pending(false);
        }
        self.pending.push(key);
        self.pending_text.push_str(text);
        self.pending_has_text_enter = true;
        self.pending_ends_with_text_enter = true;
        self.last_at = Some(now);
    }

    fn flush_pending(&mut self, allow_paste: bool) {
        if self.pending.is_empty() {
            self.last_at = None;
            return;
        }

        if self.pending_has_text_enter {
            // A terminal-delivered newline ended the burst: an input method
            // confirmed its candidate, or a legacy paste carried line breaks.
            // Such a burst is text and must reach the target as one text
            // value. The trailing newline is the commit key, not content the
            // user wrote, so it is dropped while interior newlines survive.
            let mut text = std::mem::take(&mut self.pending_text);
            if self.pending_ends_with_text_enter {
                text.pop();
            }
            self.record_text_burst(true);
            self.finish_burst();
            self.ready.push_back(Event::Paste(text));
            return;
        }

        self.record_text_burst(self.pending_chars > 1 || !self.pending_text.is_ascii());
        if allow_paste && self.pending.len() >= LEGACY_PASTE_MIN_CHARS {
            let text = std::mem::take(&mut self.pending_text);
            self.finish_burst();
            self.ready.push_back(Event::Paste(text));
            return;
        }
        for key in self.pending.drain(..) {
            self.ready.push_back(Event::Key(key));
        }
        self.finish_burst();
    }

    fn finish_burst(&mut self) {
        self.pending.clear();
        self.pending_text.clear();
        self.pending_chars = 0;
        self.pending_has_text_enter = false;
        self.pending_ends_with_text_enter = false;
        self.last_at = None;
    }

    fn record_text_burst(&mut self, machine: bool) {
        self.last_burst = Some(TextBurst {
            at: Instant::now(),
            machine,
        });
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
        self.pending_chars = self.pending_chars.saturating_add(1);
        // Text follows the delivered newline, so the burst no longer ends with
        // a commit key and that newline is content again.
        self.pending_ends_with_text_enter = false;
        self.last_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ch: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
    }

    fn enter() -> Event {
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
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

    #[test]
    fn an_ime_commit_enter_inside_the_burst_never_becomes_an_application_key() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        // A one-character commit is the case legacy paste detection cannot
        // classify on its own: the committed text reached the target as
        // separate keys and the forwarded Enter would advance the wizard.
        input.accept(key('好'));
        input.accept(enter());
        input.flush_all();
        assert_eq!(input.pop_ready(), Some(Event::Paste("好".to_string())));
        assert!(input.pop_ready().is_none());
    }

    #[test]
    fn a_commit_enter_after_delivered_text_is_absorbed_once() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        for ch in ['a', 'b', 'c'] {
            input.accept(key(ch));
        }
        input.flush_timed_out();
        assert_eq!(input.pop_ready(), Some(Event::Paste("abc".to_string())));

        // The terminal forwarded the commit key after the text burst was
        // already handed over.
        input.accept(enter());
        assert!(input.pop_ready().is_none());

        // The user's own repeat press is a real key again.
        input.accept(enter());
        assert_eq!(input.pop_ready(), Some(enter()));
    }

    #[test]
    fn an_enter_after_a_bracketed_paste_still_submits() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        input.accept(Event::Paste("pasted".to_string()));
        assert_eq!(input.pop_ready(), Some(Event::Paste("pasted".to_string())));
        input.accept(enter());
        assert_eq!(input.pop_ready(), Some(enter()));
    }

    #[test]
    fn interior_newlines_of_a_legacy_paste_survive() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        input.accept(key('a'));
        input.accept(enter());
        input.accept(key('b'));
        input.flush_all();
        assert_eq!(input.pop_ready(), Some(Event::Paste("a\nb".to_string())));
    }

    #[test]
    fn a_legacy_paste_trailing_newline_is_not_content() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        input.accept(key('a'));
        input.accept(key('b'));
        input.accept(enter());
        input.flush_all();
        assert_eq!(input.pop_ready(), Some(Event::Paste("ab".to_string())));
    }

    #[test]
    fn a_late_enter_after_a_single_typed_character_stays_a_key() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        input.accept(key('a'));
        input.flush_timed_out();
        assert_eq!(input.pop_ready(), Some(key('a')));
        // One ASCII character is human cadence: the following Enter is the
        // user's own submit gesture.
        input.accept(enter());
        assert_eq!(input.pop_ready(), Some(enter()));
    }
}
