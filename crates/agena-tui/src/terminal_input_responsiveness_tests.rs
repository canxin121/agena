use super::*;

fn large_burst(bytes: usize) -> InputNormalizer {
    let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    InputNormalizer {
        text_input_active: true,
        pending: vec![key; bytes],
        pending_text: "x".repeat(bytes),
        pending_chars: bytes,
        last_at: Some(Instant::now()),
        ..Default::default()
    }
}

#[test]
fn full_legacy_paste_is_one_chunk_not_hundreds_of_thousands_of_keys() {
    let mut input = large_burst(LEGACY_PASTE_MAX_BYTES);
    input.last_at = Some(Instant::now());
    input.accept_character(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE), 'y');
    assert_eq!(input.ready.len(), 1);
    let Some(Event::Paste(text)) = input.pop_ready() else {
        panic!("full chunk must remain paste text");
    };
    assert_eq!(text.len(), LEGACY_PASTE_MAX_BYTES);
    assert!(text.bytes().all(|b| b == b'x'));
    assert_eq!(input.pending_text, "y");
    assert_eq!(input.pending.len(), 1);
    input.flush_timed_out();
    assert!(matches!(input.pop_ready(), Some(Event::Key(key)) if key.code == KeyCode::Char('y')));
}

#[test]
fn multibyte_boundary_preserves_every_character_without_expanding_the_ready_queue() {
    let mut input = large_burst(LEGACY_PASTE_MAX_BYTES - 1);
    input.last_at = Some(Instant::now());
    input.accept_character(KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE), '界');
    assert_eq!(input.ready.len(), 1);
    let Some(Event::Paste(text)) = input.pop_ready() else {
        panic!("expected bounded paste");
    };
    assert_eq!(text.len(), LEGACY_PASTE_MAX_BYTES - 1);
    assert_eq!(input.pending_text, "界");
    assert_eq!(input.pending_chars, 1);
}

#[test]
fn enter_at_chunk_boundary_is_never_replayed_as_a_submit_command() {
    let mut input = large_burst(LEGACY_PASTE_MAX_BYTES);
    input.accept_text_enter(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        "\n",
        Instant::now(),
    );
    assert!(
        matches!(input.pop_ready(), Some(Event::Paste(text)) if text.len() == LEGACY_PASTE_MAX_BYTES)
    );
    input.flush_timed_out();
    assert!(
        input
            .take_ready()
            .iter()
            .all(|event| matches!(event, Event::Paste(_)))
    );
}

#[test]
fn multiline_continuation_after_a_full_chunk_keeps_its_interior_newline() {
    let mut input = large_burst(LEGACY_PASTE_MAX_BYTES);
    input.last_at = Some(Instant::now());
    for code in [KeyCode::Char('b'), KeyCode::Enter, KeyCode::Char('c')] {
        input.accept(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }
    assert!(
        matches!(input.pop_ready(), Some(Event::Paste(text)) if text.len() == LEGACY_PASTE_MAX_BYTES)
    );
    input.flush_timed_out();
    assert!(matches!(input.pop_ready(), Some(Event::Paste(text)) if text == "b\nc"));
    assert!(input.pop_ready().is_none());
}

#[test]
fn a_completed_ime_burst_still_consumes_its_confirmation_enter() {
    let mut input = InputNormalizer::default();
    input.set_text_input_active(true);
    input.accept(Event::Key(KeyEvent::new(
        KeyCode::Char('界'),
        KeyModifiers::NONE,
    )));
    input.flush_timed_out();
    input.take_ready();
    input.accept(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(input.pop_ready().is_none());
    input.accept(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(matches!(input.pop_ready(), Some(Event::Key(key)) if key.code == KeyCode::Enter));
}
