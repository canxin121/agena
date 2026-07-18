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
const TERMINAL_QUERY_RESPONSE_INTERVAL: Duration = Duration::from_millis(20);
const TERMINAL_QUERY_RESPONSE_GRACE: Duration = Duration::from_secs(2);

/// The runtime's sole terminal-input readiness source.
///
/// Crossterm's `EventStream` owns an unjoinable background reader. That makes
/// it impossible to prove that stdin has been released before suspending the
/// TUI for an editor or transfer helper. On Unix, `TerminalInput` instead waits
/// for descriptor readiness without reading bytes. Other platforms use a
/// short, cancellable async poll. `event::read` is called only by the runtime
/// task and only after a non-blocking poll reports a complete event.
pub(super) struct TerminalInput {
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
    pub(super) fn new() -> io::Result<Self> {
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

    pub(super) async fn next(&self) -> io::Result<Event> {
        loop {
            if event::poll(Duration::ZERO)? {
                return event::read();
            }

            #[cfg(unix)]
            match tokio::time::timeout(RESIZE_RECHECK_INTERVAL, self.readiness.readable()).await {
                Ok(Ok(mut readiness)) => readiness.clear_ready(),
                Ok(Err(error)) => return Err(error),
                Err(_) => {
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
#[derive(Debug, Default)]
pub(super) struct InputNormalizer {
    text_input_active: bool,
    terminal_query_filter_until: Option<Instant>,
    terminal_query_pending: Vec<Event>,
    terminal_query_text: String,
    terminal_query_last_at: Option<Instant>,
    pending: Vec<KeyEvent>,
    pending_text: String,
    last_at: Option<Instant>,
    ready: VecDeque<Event>,
}

impl InputNormalizer {
    /// Temporarily accept stripped OSC 4/11 and DSR bodies as query responses
    /// after a synchronous transaction. Explicitly framed replies are always
    /// recognized, and the legacy-paste boundary independently rejects an
    /// exact late color body, so a terminal timeout cannot turn protocol bytes
    /// into composer text even when rendering delays their delivery.
    pub(super) fn arm_terminal_query_response_filter(&mut self) {
        self.terminal_query_filter_until = Some(Instant::now() + TERMINAL_QUERY_RESPONSE_GRACE);
    }

    pub(super) fn set_text_input_active(&mut self, active: bool) {
        if self.text_input_active != active {
            for event in self.finish_terminal_query_candidate(true) {
                self.accept_normalized(event);
            }
            self.flush_pending(false);
            self.text_input_active = active;
        }
    }

    pub(super) fn accept(&mut self, event: Event) {
        for event in self.filter_terminal_query_response(event) {
            self.accept_normalized(event);
        }
    }

    fn accept_normalized(&mut self, event: Event) {
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

        let discarded_terminal_response = self.flush_pending(false);
        if discarded_terminal_response && terminal_query_response_terminator_event(&event) {
            return;
        }
        self.ready.push_back(event);
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        [
            self.last_at.map(|last| last + LEGACY_PASTE_INTERVAL),
            self.terminal_query_last_at
                .map(|last| last + TERMINAL_QUERY_RESPONSE_INTERVAL),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub(super) fn flush_timed_out(&mut self) {
        self.flush_timed_out_at(Instant::now());
    }

    pub(super) fn flush_all(&mut self) {
        for event in self.finish_terminal_query_candidate(true) {
            self.accept_normalized(event);
        }
        self.flush_pending(false);
    }

    pub(super) fn reset(&mut self) {
        self.terminal_query_filter_until = None;
        self.terminal_query_pending.clear();
        self.terminal_query_text.clear();
        self.terminal_query_last_at = None;
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

    fn filter_terminal_query_response(&mut self, event: Event) -> Vec<Event> {
        let now = Instant::now();
        let filter_armed = self
            .terminal_query_filter_until
            .is_some_and(|until| now <= until);
        if !filter_armed {
            self.terminal_query_filter_until = None;
        }
        let explicit_response_start = terminal_query_response_starts_at_event(&event);
        if !filter_armed && self.terminal_query_pending.is_empty() && !explicit_response_start {
            let mut events = self.finish_terminal_query_candidate(true);
            events.push(event);
            return events;
        }

        if let Event::Paste(text) = &event {
            let mut events = self.finish_terminal_query_candidate(true);
            if terminal_query_response_match(text) != TerminalQueryResponseMatch::Complete {
                events.push(event);
            }
            return events;
        }

        let previous_match = terminal_query_response_match(&self.terminal_query_text);
        let previous_len = self.terminal_query_text.len();
        if !append_terminal_response_event(&event, &mut self.terminal_query_text) {
            let mut events = self.finish_terminal_query_candidate(true);
            events.push(event);
            return events;
        }
        let response_match = terminal_query_response_match(&self.terminal_query_text);
        if response_match == TerminalQueryResponseMatch::Invalid
            && previous_match == TerminalQueryResponseMatch::Complete
        {
            self.terminal_query_text.truncate(previous_len);
            let discarded = self.finish_terminal_query_candidate(true);
            debug_assert!(discarded.is_empty());
            return self.filter_terminal_query_response(event);
        }

        self.terminal_query_pending.push(event);
        self.terminal_query_last_at = Some(now);
        match response_match {
            TerminalQueryResponseMatch::Invalid => self.finish_terminal_query_candidate(false),
            TerminalQueryResponseMatch::Complete
                if terminal_query_response_is_delimited(&self.terminal_query_text) =>
            {
                self.finish_terminal_query_candidate(true)
            }
            TerminalQueryResponseMatch::Prefix | TerminalQueryResponseMatch::Complete => Vec::new(),
        }
    }

    fn finish_terminal_query_candidate(&mut self, discard_complete: bool) -> Vec<Event> {
        let complete = terminal_query_response_match(&self.terminal_query_text)
            == TerminalQueryResponseMatch::Complete;
        self.terminal_query_text.clear();
        self.terminal_query_last_at = None;
        if discard_complete && complete {
            self.terminal_query_pending.clear();
            Vec::new()
        } else {
            std::mem::take(&mut self.terminal_query_pending)
        }
    }

    fn flush_timed_out_at(&mut self, now: Instant) {
        let terminal_query_expired = self
            .terminal_query_last_at
            .is_some_and(|last| now >= last + TERMINAL_QUERY_RESPONSE_INTERVAL);
        if terminal_query_expired {
            for event in self.finish_terminal_query_candidate(true) {
                self.accept_normalized(event);
            }
        }

        let legacy_paste_expired = self
            .last_at
            .is_some_and(|last| now >= last + LEGACY_PASTE_INTERVAL);
        if legacy_paste_expired {
            self.flush_pending(true);
        }
    }

    fn flush_pending(&mut self, allow_paste: bool) -> bool {
        if self.pending.is_empty() {
            self.last_at = None;
            return false;
        }

        // A timed-out terminal reply can arrive arbitrarily later, after the
        // query grace period and while a composer happens to be active. At
        // that point Crossterm exposes the visible OSC body as a burst of
        // ordinary character keys, which is indistinguishable from the legacy
        // paste heuristic until the complete body is available. Never publish
        // a structurally exact color reply from that boundary.
        if terminal_color_response_match(&self.pending_text) == TerminalQueryResponseMatch::Complete
        {
            self.pending.clear();
            self.pending_text.clear();
            self.last_at = None;
            return true;
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
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalQueryResponseMatch {
    Prefix,
    Complete,
    Invalid,
}

fn append_terminal_response_event(event: &Event, target: &mut String) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return false;
    }
    match key.code {
        KeyCode::Esc if key.modifiers.is_empty() => target.push('\x1b'),
        KeyCode::Char(ch) if matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            target.push(ch);
        }
        // Crossterm may combine the leading ESC and the following OSC/CSI
        // introducer into one Alt key event. Reconstruct the original bytes
        // only for protocol recognition; the retained Event is forwarded
        // unchanged if the candidate is not a valid query response.
        KeyCode::Char(ch)
            if key.modifiers == KeyModifiers::ALT
                || key.modifiers == (KeyModifiers::ALT | KeyModifiers::SHIFT) =>
        {
            target.push('\x1b');
            target.push(ch);
        }
        KeyCode::Char('g') if key.modifiers == KeyModifiers::CONTROL => target.push('\u{7}'),
        _ => return false,
    }
    true
}

fn terminal_query_response_starts_at_event(event: &Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return false;
    }
    match key.code {
        KeyCode::Esc => key.modifiers.is_empty(),
        KeyCode::Char(']' | '[') => {
            key.modifiers == KeyModifiers::ALT
                || key.modifiers == (KeyModifiers::ALT | KeyModifiers::SHIFT)
        }
        _ => false,
    }
}

fn terminal_query_response_terminator_event(event: &Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return false;
    }
    matches!(key.code, KeyCode::Char('g') if key.modifiers == KeyModifiers::CONTROL)
        || matches!(key.code, KeyCode::Char('\\')
            if key.modifiers == KeyModifiers::ALT
                || key.modifiers == (KeyModifiers::ALT | KeyModifiers::SHIFT))
}

fn terminal_query_response_match(candidate: &str) -> TerminalQueryResponseMatch {
    use TerminalQueryResponseMatch::{Complete, Prefix};

    const STATUS_RESPONSES: [&str; 3] = ["\x1b[0n", "[0n", "0n"];
    if STATUS_RESPONSES.contains(&candidate) {
        return Complete;
    }
    if STATUS_RESPONSES
        .iter()
        .any(|response| response.starts_with(candidate))
    {
        return Prefix;
    }

    terminal_color_response_match(candidate)
}

fn terminal_color_response_match(candidate: &str) -> TerminalQueryResponseMatch {
    use TerminalQueryResponseMatch::{Complete, Invalid, Prefix};

    const PREFIXES: [&str; 6] = [
        "\x1b]4;-2;rgb:",
        "]4;-2;rgb:",
        "4;-2;rgb:",
        "\x1b]11;rgb:",
        "]11;rgb:",
        "11;rgb:",
    ];
    if candidate.is_empty() || PREFIXES.iter().any(|prefix| prefix.starts_with(candidate)) {
        return Prefix;
    }

    let (body, terminated) = if let Some(body) = candidate.strip_suffix('\u{7}') {
        (body, true)
    } else if let Some(body) = candidate.strip_suffix("\x1b\\") {
        (body, true)
    } else if let Some(body) = candidate.strip_suffix('\x1b') {
        // This may be the first byte of an ST terminator.
        (body, false)
    } else {
        (candidate, false)
    };
    let Some(prefix) = PREFIXES.iter().find(|prefix| body.starts_with(**prefix)) else {
        return Invalid;
    };
    let payload = &body[prefix.len()..];
    let components = payload.split('/').collect::<Vec<_>>();
    if components.len() > 3 {
        return Invalid;
    }
    for (index, component) in components.iter().enumerate() {
        let last = index + 1 == components.len();
        if component.is_empty() {
            if last && !terminated {
                return Prefix;
            }
            return Invalid;
        }
        if component.len() > 4 || !component.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Invalid;
        }
    }
    if components.len() == 3 {
        Complete
    } else if terminated {
        Invalid
    } else {
        Prefix
    }
}

pub(crate) fn is_terminal_color_response_text(candidate: &str) -> bool {
    terminal_color_response_match(candidate.trim()) == TerminalQueryResponseMatch::Complete
}

fn terminal_query_response_is_delimited(candidate: &str) -> bool {
    candidate.ends_with('\u{7}')
        || candidate.ends_with("\x1b\\")
        || matches!(candidate, "\x1b[0n" | "[0n" | "0n")
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
        } else if terminal_color_response_match(&self.pending_text)
            == TerminalQueryResponseMatch::Complete
        {
            let mut extended = self.pending_text.clone();
            extended.push(ch);
            if terminal_color_response_match(&extended) == TerminalQueryResponseMatch::Invalid {
                self.pending.clear();
                self.pending_text.clear();
                self.last_at = None;
            }
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

    fn feed_text(input: &mut InputNormalizer, text: &str) {
        for ch in text.chars() {
            input.accept(key(ch));
        }
    }

    fn ready_text(input: &mut InputNormalizer) -> String {
        let mut text = String::new();
        while let Some(event) = input.pop_ready() {
            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Char(ch),
                    ..
                }) => text.push(ch),
                Event::Paste(value) => text.push_str(&value),
                _ => {}
            }
        }
        text
    }

    fn flush_next_deadline(input: &mut InputNormalizer) {
        let deadline = input.deadline().expect("input should have a deadline");
        input.flush_timed_out_at(deadline);
    }

    #[test]
    fn rapid_text_is_emitted_as_one_legacy_paste_only_for_text_targets() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        for ch in ['a', 'b', 'c'] {
            input.accept(key(ch));
        }
        flush_next_deadline(&mut input);
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
    fn delayed_iterm_color_response_never_becomes_user_input() {
        let mut input = InputNormalizer::default();
        input.arm_terminal_query_response_filter();
        feed_text(&mut input, "4;-2;rgb:fae0/fae0/fae0");
        flush_next_deadline(&mut input);
        assert!(input.pop_ready().is_none());

        input.arm_terminal_query_response_filter();
        input.accept(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        feed_text(&mut input, "]4;-2;rgb:ff/80/00");
        input.accept(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));
        assert!(input.pop_ready().is_none());

        input.arm_terminal_query_response_filter();
        input.accept(Event::Key(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::ALT,
        )));
        feed_text(&mut input, "4;-2;rgb:fae0/fae0/fae0");
        input.accept(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));
        assert!(input.pop_ready().is_none());
    }

    #[test]
    fn explicitly_framed_color_response_is_filtered_after_grace_expires() {
        let mut input = InputNormalizer {
            terminal_query_filter_until: Some(Instant::now() - Duration::from_secs(1)),
            ..InputNormalizer::default()
        };
        input.set_text_input_active(true);
        input.accept(Event::Key(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::ALT,
        )));
        feed_text(&mut input, "4;-2;rgb:fae0/fae0/fae0");
        input.accept(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));
        assert!(input.pop_ready().is_none());
    }

    #[test]
    fn bare_late_color_body_cannot_cross_the_legacy_paste_boundary() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        feed_text(&mut input, "4;-2;rgb:fae0/fae0/fae0");
        input.accept(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));
        assert!(input.pop_ready().is_none());

        feed_text(&mut input, "4;-2;rgb:fae0/fae0/fae0");
        input.accept(key('x'));
        input.flush_all();
        assert_eq!(input.pop_ready(), Some(key('x')));
        assert!(input.pop_ready().is_none());

        feed_text(&mut input, "4;-2;rgb:fae0/fae0/fae0");
        flush_next_deadline(&mut input);
        assert!(input.pop_ready().is_none());

        // An actual bracketed paste remains user-authored input. This final
        // boundary applies only to rapid key bursts synthesized by terminals
        // that bypass the response parser.
        let mut input = InputNormalizer::default();
        input.accept(Event::Paste("4;-2;rgb:fae0/fae0/fae0".to_string()));
        assert_eq!(
            input.pop_ready(),
            Some(Event::Paste("4;-2;rgb:fae0/fae0/fae0".to_string()))
        );
    }

    #[test]
    fn delayed_osc11_response_is_filtered_as_keys_or_paste() {
        let mut input = InputNormalizer::default();
        input.arm_terminal_query_response_filter();
        input.accept(Event::Paste("11;rgb:ffff/eeee/dddd".to_string()));
        assert!(input.pop_ready().is_none());

        input.arm_terminal_query_response_filter();
        feed_text(&mut input, "]11;rgb:00/00/00");
        input.flush_all();
        assert!(input.pop_ready().is_none());
    }

    #[test]
    fn trailing_status_response_is_filtered_in_plain_or_alt_decoding() {
        let mut input = InputNormalizer::default();
        input.arm_terminal_query_response_filter();
        feed_text(&mut input, "0n");
        assert!(input.pop_ready().is_none());

        input.arm_terminal_query_response_filter();
        input.accept(Event::Key(KeyEvent::new(
            KeyCode::Char('['),
            KeyModifiers::ALT,
        )));
        feed_text(&mut input, "0n");
        assert!(input.pop_ready().is_none());
    }

    #[test]
    fn input_immediately_after_an_unterminated_response_is_preserved() {
        let mut input = InputNormalizer::default();
        input.arm_terminal_query_response_filter();
        feed_text(&mut input, "4;-2;rgb:fae0/fae0/fae0");
        input.accept(key('x'));
        input.flush_all();
        assert_eq!(input.pop_ready(), Some(key('x')));
        assert!(input.pop_ready().is_none());
    }

    #[test]
    fn color_filter_preserves_near_matches_and_unarmed_text() {
        let mut input = InputNormalizer::default();
        input.arm_terminal_query_response_filter();
        feed_text(&mut input, "4;-2;rgb:fae0/fae0/not-a-color");
        input.flush_all();
        assert_eq!(ready_text(&mut input), "4;-2;rgb:fae0/fae0/not-a-color");

        let mut input = InputNormalizer::default();
        feed_text(&mut input, "4;-2;rgb:fae0/fae0/fae0");
        input.flush_all();
        assert_eq!(ready_text(&mut input), "4;-2;rgb:fae0/fae0/fae0");
    }

    #[test]
    fn color_response_matcher_accepts_only_complete_x11_colors() {
        assert_eq!(
            terminal_query_response_match("4;-2;rgb:fae0/fae0/fae0"),
            TerminalQueryResponseMatch::Complete
        );
        assert_eq!(
            terminal_query_response_match("\x1b]11;rgb:ff/80/00\x1b\\"),
            TerminalQueryResponseMatch::Complete
        );
        assert_eq!(
            terminal_query_response_match("4;-2;rgb:fae0/fae0/"),
            TerminalQueryResponseMatch::Prefix
        );
        assert_eq!(
            terminal_query_response_match("4;-2;rgb:fae0/fae0/zzzz"),
            TerminalQueryResponseMatch::Invalid
        );
    }

    #[test]
    fn independent_deadlines_do_not_flush_an_incomplete_color_candidate_early() {
        let mut input = InputNormalizer::default();
        input.set_text_input_active(true);
        feed_text(&mut input, "abc");
        let paste_deadline = input.deadline().expect("paste should have a deadline");

        input.arm_terminal_query_response_filter();
        feed_text(&mut input, "4");
        let response_deadline = input
            .terminal_query_last_at
            .expect("response should have a timestamp")
            + TERMINAL_QUERY_RESPONSE_INTERVAL;
        assert!(paste_deadline < response_deadline);

        input.flush_timed_out_at(paste_deadline);
        assert_eq!(input.pop_ready(), Some(Event::Paste("abc".to_string())));
        assert_eq!(input.terminal_query_text, "4");

        input.flush_timed_out_at(response_deadline);
        input.flush_all();
        assert_eq!(input.pop_ready(), Some(key('4')));
    }
}
