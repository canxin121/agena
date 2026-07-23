//! Configurable composer input policy.

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
        event.code == self.code && event.modifiers == self.modifiers
    }
}

#[derive(Debug, Clone)]
pub struct ComposerKeyBindings {
    pub submit: Vec<KeyChord>,
    pub queue: Vec<KeyChord>,
    pub newline: Vec<KeyChord>,
    pub edit_queue: Vec<KeyChord>,
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
            submit: vec![KeyChord::new(KeyCode::Enter, KeyModifiers::CONTROL)],
            queue: vec![KeyChord::new(KeyCode::Enter, KeyModifiers::empty())],
            newline: vec![
                KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT),
                KeyChord::new(KeyCode::Enter, KeyModifiers::ALT),
                KeyChord::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
            ],
            edit_queue: vec![KeyChord::new(KeyCode::Up, KeyModifiers::CONTROL)],
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
            return Some(ComposerAction::AttachClipboardImage);
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
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerAction {
    Submit,
    Queue,
    Newline,
    EditQueue,
    ClearInput,
    FocusItems,
    AttachFile,
    ExternalEditor,
    AttachClipboardImage,
    OpenPendingUserInput,
    OpenPendingPermission,
}

#[cfg(test)]
mod tests {
    use super::{ComposerAction, ComposerKeyBindings};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn default_bindings_distinguish_queue_and_submit() {
        let bindings = ComposerKeyBindings::default();
        assert_eq!(
            bindings.match_action(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(ComposerAction::Queue)
        );
        assert_eq!(
            bindings.match_action(&KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
            Some(ComposerAction::Submit)
        );
    }
}
