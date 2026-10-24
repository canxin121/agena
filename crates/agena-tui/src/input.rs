//! Configurable composer input policy.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A key chord (modifiers plus key).
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    pub fn matches(&self, event: &KeyEvent) -> bool {
        event.code == self.code && event.modifiers == self.modifiers
    }
}

#[derive(Debug, Clone)]
/// Key bindings of the composer.
pub struct ComposerKeyBindings {
    pub submit: Vec<KeyChord>,
    pub queue: Vec<KeyChord>,
    pub newline: Vec<KeyChord>,
    pub edit_queue: Vec<KeyChord>,
    pub cancel_pending: Vec<KeyChord>,
    pub clear_input: Vec<KeyChord>,
    pub focus_items: Vec<KeyChord>,
    pub insert_content: Vec<KeyChord>,
    pub attach_file: Vec<KeyChord>,
    pub external_editor: Vec<KeyChord>,
    pub attach_clipboard_image: Vec<KeyChord>,
    pub open_pending_user_input: Vec<KeyChord>,
    pub open_pending_permission: Vec<KeyChord>,
}

impl Default for ComposerKeyBindings {
    fn default() -> Self {
        Self {
            submit: vec![KeyChord::new(KeyCode::Enter, KeyModifiers::CONTROL)],
            queue: vec![KeyChord::new(KeyCode::Enter, KeyModifiers::empty())],
            newline: vec![
                KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT),
                KeyChord::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
            ],
            edit_queue: vec![KeyChord::new(KeyCode::Char('p'), KeyModifiers::CONTROL)],
            cancel_pending: vec![KeyChord::new(KeyCode::Char('x'), KeyModifiers::CONTROL)],
            clear_input: vec![KeyChord::new(KeyCode::Char('c'), KeyModifiers::CONTROL)],
            focus_items: vec![KeyChord::new(KeyCode::Char('g'), KeyModifiers::CONTROL)],
            insert_content: vec![KeyChord::new(KeyCode::Char('a'), KeyModifiers::CONTROL)],
            attach_file: vec![KeyChord::new(KeyCode::Char('o'), KeyModifiers::CONTROL)],
            external_editor: vec![KeyChord::new(KeyCode::Char('e'), KeyModifiers::CONTROL)],
            attach_clipboard_image: vec![KeyChord::new(KeyCode::Char('t'), KeyModifiers::CONTROL)],
            open_pending_user_input: vec![KeyChord::new(KeyCode::Char('r'), KeyModifiers::CONTROL)],
            open_pending_permission: vec![KeyChord::new(KeyCode::Char('l'), KeyModifiers::CONTROL)],
        }
    }
}

impl ComposerKeyBindings {
    pub fn match_action(&self, event: &KeyEvent) -> Option<ComposerAction> {
        if self.submit.iter().any(|chord| chord.matches(event)) {
            return Some(ComposerAction::Submit);
        }
        if self.newline.iter().any(|chord| chord.matches(event)) {
            return Some(ComposerAction::Newline);
        }
        if self.clear_input.iter().any(|chord| chord.matches(event)) {
            return Some(ComposerAction::ClearInput);
        }
        if self
            .open_pending_user_input
            .iter()
            .any(|chord| chord.matches(event))
        {
            return Some(ComposerAction::OpenPendingUserInput);
        }
        if self
            .open_pending_permission
            .iter()
            .any(|chord| chord.matches(event))
        {
            return Some(ComposerAction::OpenPendingPermission);
        }
        if self.attach_file.iter().any(|chord| chord.matches(event)) {
            return Some(ComposerAction::AttachFile);
        }
        if self.insert_content.iter().any(|chord| chord.matches(event))
            || self.matches_legacy_ctrl_a(event)
        {
            return Some(ComposerAction::InsertContent);
        }
        if self
            .external_editor
            .iter()
            .any(|chord| chord.matches(event))
        {
            return Some(ComposerAction::ExternalEditor);
        }
        if self
            .attach_clipboard_image
            .iter()
            .any(|chord| chord.matches(event))
        {
            return Some(ComposerAction::AttachImage);
        }
        if self.focus_items.iter().any(|chord| chord.matches(event)) {
            return Some(ComposerAction::FocusItems);
        }
        if self.queue.iter().any(|chord| chord.matches(event)) {
            return Some(ComposerAction::Queue);
        }
        if self.edit_queue.iter().any(|chord| chord.matches(event)) {
            return Some(ComposerAction::EditQueue);
        }
        if self.cancel_pending.iter().any(|chord| chord.matches(event)) {
            return Some(ComposerAction::CancelPending);
        }
        None
    }

    /// In terminals without an enhanced keyboard protocol, Ctrl+A can arrive
    /// as the raw SOH control byte. Preserve the configured Ctrl+A binding so
    /// the unified command/file insertion picker works in both encodings.
    fn matches_legacy_ctrl_a(&self, event: &KeyEvent) -> bool {
        event.code == KeyCode::Char('\u{0001}')
            && event.modifiers.is_empty()
            && self.insert_content.iter().any(|chord| {
                chord.code == KeyCode::Char('a') && chord.modifiers == KeyModifiers::CONTROL
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Action bound to a composer key.
pub enum ComposerAction {
    Submit,
    Queue,
    Newline,
    EditQueue,
    CancelPending,
    ClearInput,
    FocusItems,
    InsertContent,
    AttachFile,
    ExternalEditor,
    AttachImage,
    OpenPendingUserInput,
    OpenPendingPermission,
}
