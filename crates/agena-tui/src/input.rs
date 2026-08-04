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
    /// the unified Skill/file insertion picker works in both encodings.
    fn matches_legacy_ctrl_a(&self, event: &KeyEvent) -> bool {
        event.code == KeyCode::Char('\u{0001}')
            && event.modifiers.is_empty()
            && self.insert_content.iter().any(|chord| {
                chord.code == KeyCode::Char('a') && chord.modifiers == KeyModifiers::CONTROL
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    #[test]
    fn shift_enter_and_ctrl_j_are_newlines_and_take_precedence_over_queue() {
        let bindings = ComposerKeyBindings::default();
        assert_eq!(
            bindings.match_action(&KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            Some(ComposerAction::Newline)
        );
        assert_eq!(
            bindings.match_action(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            Some(ComposerAction::Newline)
        );
        // A legacy terminal without the keyboard enhancement protocol sends
        // Ctrl+J as the LF control byte, which crossterm surfaces as
        // Char('j') + CONTROL. It must never fall through to the plain-Enter
        // queue action.
        assert_ne!(
            bindings.match_action(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            Some(ComposerAction::Queue)
        );
        assert_ne!(
            bindings.match_action(&KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            Some(ComposerAction::Queue)
        );
    }

    #[test]
    fn ctrl_a_opens_the_unified_file_and_skill_picker() {
        let bindings = ComposerKeyBindings::default();
        assert_eq!(
            bindings.match_action(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Some(ComposerAction::InsertContent)
        );
        assert_ne!(
            bindings.match_action(&KeyEvent::new(
                KeyCode::Char('a'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            )),
            Some(ComposerAction::InsertContent)
        );
        assert_eq!(
            bindings.match_action(&KeyEvent::new(
                KeyCode::Char('\u{0001}'),
                KeyModifiers::NONE
            )),
            Some(ComposerAction::InsertContent)
        );
    }

    #[test]
    fn ctrl_c_is_the_default_clear_composer_binding() {
        let bindings = ComposerKeyBindings::default();
        assert_eq!(
            bindings.match_action(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(ComposerAction::ClearInput)
        );
        assert_ne!(
            bindings.match_action(&KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)),
            Some(ComposerAction::ClearInput)
        );
    }

    #[test]
    fn ctrl_p_edits_the_pending_message() {
        let bindings = ComposerKeyBindings::default();
        assert_eq!(
            bindings.match_action(&KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
            Some(ComposerAction::EditQueue)
        );
        // Arrow keys stay reserved for cursor movement and history; the
        // pending-message edit shortcut is a discoverable Ctrl chord.
        assert_ne!(
            bindings.match_action(&KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
            Some(ComposerAction::EditQueue)
        );
        assert_ne!(
            bindings.match_action(&KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)),
            Some(ComposerAction::EditQueue)
        );
    }

    #[test]
    fn ctrl_x_cancels_the_pending_message() {
        let bindings = ComposerKeyBindings::default();
        assert_eq!(
            bindings.match_action(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Some(ComposerAction::CancelPending)
        );
        assert_ne!(
            bindings.match_action(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            Some(ComposerAction::CancelPending)
        );
    }
}
