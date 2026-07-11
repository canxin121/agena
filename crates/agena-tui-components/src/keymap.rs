//! Shared keymaps for reusable TUI components.
//!
//! Application actions are centralized by `agena-cli::tui_keymap`. This
//! module owns the physical keys shared by generic list, scroll, and dialog
//! components, while `editor.rs` owns the shell-style text-editing map.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationAction {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

pub fn navigation_action(key: KeyEvent) -> Option<NavigationAction> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => Some(NavigationAction::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(NavigationAction::Down),
        KeyCode::PageUp => Some(NavigationAction::PageUp),
        KeyCode::PageDown => Some(NavigationAction::PageDown),
        KeyCode::Home => Some(NavigationAction::Home),
        KeyCode::End => Some(NavigationAction::End),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDialogAction {
    Close,
    Submit,
}

pub fn input_dialog_action(key: KeyEvent, multiline: bool) -> Option<InputDialogAction> {
    if matches!(key.code, KeyCode::Esc) {
        return Some(InputDialogAction::Close);
    }
    if multiline {
        return (matches!(key.code, KeyCode::Char('s'))
            && key.modifiers.contains(KeyModifiers::CONTROL))
        .then_some(InputDialogAction::Submit);
    }
    matches!(key.code, KeyCode::Enter).then_some(InputDialogAction::Submit)
}

#[cfg(test)]
mod tests {
    use super::{InputDialogAction, NavigationAction, input_dialog_action, navigation_action};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn vim_and_arrow_navigation_aliases_share_actions() {
        for (arrow, vim, action) in [
            (KeyCode::Up, 'k', NavigationAction::Up),
            (KeyCode::Down, 'j', NavigationAction::Down),
        ] {
            assert_eq!(
                navigation_action(KeyEvent::new(arrow, KeyModifiers::NONE)),
                Some(action)
            );
            assert_eq!(
                navigation_action(KeyEvent::new(KeyCode::Char(vim), KeyModifiers::NONE)),
                Some(action)
            );
        }
    }

    #[test]
    fn dialog_submit_key_depends_on_editor_mode() {
        assert_eq!(
            input_dialog_action(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), false),
            Some(InputDialogAction::Submit)
        );
        assert_eq!(
            input_dialog_action(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                true
            ),
            Some(InputDialogAction::Submit)
        );
        assert_eq!(
            input_dialog_action(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true),
            None
        );
    }
}
