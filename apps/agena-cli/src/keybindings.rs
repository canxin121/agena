//! Configurable composer key bindings.
//!
//! Two distinct submit actions:
//!
//! * `submit_key`   — fire the message immediately. While the AI is busy
//!   this routes through `steer_input` (Phase 3); when
//!   idle it submits the message directly.
//! * `queue_key`    — append to the local pending queue. While the AI is
//!   busy, the queued message is held until the current
//!   run ends. While idle, behaves like `submit_key`.
//! * `newline_key`  — insert a literal newline.
//! * `edit_queue_key` — pull the queue back into the editor for edit.
//!
//! The defaults follow the user's stated preference (Enter = queue,
//! Ctrl+Enter = submit).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    pub fn matches(&self, event: &KeyEvent) -> bool {
        // We only care about the modifiers we've explicitly listed; ignore
        // KEYPAD/REPEAT bits and similar so that terminals that pass extra
        // flags still match.
        let want =
            self.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let got =
            event.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        event.code == self.code && want == got
    }
}

#[derive(Debug, Clone)]
pub struct ComposerKeyBindings {
    pub submit: Vec<KeyChord>,
    pub queue: Vec<KeyChord>,
    pub newline: Vec<KeyChord>,
    pub edit_queue: Vec<KeyChord>,
    pub history_search: Vec<KeyChord>,
    pub clear_input: Vec<KeyChord>,
    pub focus_items: Vec<KeyChord>,
    pub attach_file: Vec<KeyChord>,
    pub external_editor: Vec<KeyChord>,
    pub attach_clipboard_image: Vec<KeyChord>,
    pub open_pending_user_input: Vec<KeyChord>,
    pub open_pending_permission: Vec<KeyChord>,
}

impl Default for ComposerKeyBindings {
    fn default() -> Self {
        Self {
            // Per user request: Ctrl+Enter sends immediately.
            submit: vec![KeyChord::new(KeyCode::Enter, KeyModifiers::CONTROL)],
            // Per user request: Enter queues (or sends immediately when idle).
            queue: vec![KeyChord::new(KeyCode::Enter, KeyModifiers::empty())],
            newline: vec![
                KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT),
                KeyChord::new(KeyCode::Enter, KeyModifiers::ALT),
                KeyChord::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
            ],
            edit_queue: vec![KeyChord::new(KeyCode::Up, KeyModifiers::empty())],
            history_search: vec![KeyChord::new(KeyCode::Char('r'), KeyModifiers::CONTROL)],
            clear_input: vec![KeyChord::new(KeyCode::Char('l'), KeyModifiers::CONTROL)],
            focus_items: vec![KeyChord::new(KeyCode::F(2), KeyModifiers::empty())],
            attach_file: vec![
                KeyChord::new(KeyCode::F(3), KeyModifiers::empty()),
                KeyChord::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
                KeyChord::new(KeyCode::Char('o'), KeyModifiers::ALT),
            ],
            external_editor: vec![
                KeyChord::new(KeyCode::F(4), KeyModifiers::empty()),
                KeyChord::new(KeyCode::Char('e'), KeyModifiers::ALT),
            ],
            attach_clipboard_image: vec![
                KeyChord::new(KeyCode::F(6), KeyModifiers::empty()),
                KeyChord::new(KeyCode::Char('i'), KeyModifiers::ALT),
            ],
            open_pending_user_input: vec![KeyChord::new(KeyCode::Char('u'), KeyModifiers::ALT)],
            open_pending_permission: vec![KeyChord::new(KeyCode::Char('a'), KeyModifiers::ALT)],
        }
    }
}

impl ComposerKeyBindings {
    pub fn match_action(&self, event: &KeyEvent) -> Option<ComposerAction> {
        // Order matters: more specific (with modifiers) wins. We list submit
        // first so Ctrl+Enter is detected before bare Enter.
        if self.submit.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::Submit);
        }
        if self.newline.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::Newline);
        }
        if self.history_search.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::HistorySearch);
        }
        if self.clear_input.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::ClearInput);
        }
        if self
            .open_pending_user_input
            .iter()
            .any(|c| c.matches(event))
        {
            return Some(ComposerAction::OpenPendingUserInput);
        }
        if self
            .open_pending_permission
            .iter()
            .any(|c| c.matches(event))
        {
            return Some(ComposerAction::OpenPendingPermission);
        }
        if self.attach_file.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::AttachFile);
        }
        if self.external_editor.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::ExternalEditor);
        }
        if self.attach_clipboard_image.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::AttachClipboardImage);
        }
        if self.focus_items.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::FocusItems);
        }
        if self.queue.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::Queue);
        }
        if self.edit_queue.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::EditQueue);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerAction {
    Submit,
    Queue,
    Newline,
    EditQueue,
    HistorySearch,
    ClearInput,
    FocusItems,
    AttachFile,
    ExternalEditor,
    AttachClipboardImage,
    OpenPendingUserInput,
    OpenPendingPermission,
}
