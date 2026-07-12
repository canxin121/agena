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
    if !key.modifiers.is_empty() {
        return None;
    }
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

/// Navigation for secondary UI surfaces that intentionally expose only
/// structural keys. Unlike `navigation_action`, this excludes Vim letter
/// aliases so page commands do not accumulate behind invisible mnemonics.
pub fn structural_navigation_action(key: KeyEvent) -> Option<NavigationAction> {
    if !key.modifiers.is_empty() {
        return None;
    }
    match key.code {
        KeyCode::Up => Some(NavigationAction::Up),
        KeyCode::Down => Some(NavigationAction::Down),
        _ => None,
    }
}

/// Navigation used while a searchable text input is active. Printable Vim
/// aliases are deliberately excluded so every character remains typeable.
pub fn search_navigation_action(key: KeyEvent) -> Option<NavigationAction> {
    match key.code {
        KeyCode::Up if key.modifiers.is_empty() => Some(NavigationAction::Up),
        KeyCode::Down if key.modifiers.is_empty() => Some(NavigationAction::Down),
        KeyCode::PageUp if key.modifiers.is_empty() => Some(NavigationAction::PageUp),
        KeyCode::PageDown if key.modifiers.is_empty() => Some(NavigationAction::PageDown),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDialogAction {
    Close,
    Submit,
}

pub fn input_dialog_action(key: KeyEvent, multiline: bool) -> Option<InputDialogAction> {
    if matches!(key.code, KeyCode::Esc) && key.modifiers.is_empty() {
        return Some(InputDialogAction::Close);
    }
    if multiline {
        return (matches!(key.code, KeyCode::Char('s')) && key.modifiers == KeyModifiers::CONTROL)
            .then_some(InputDialogAction::Submit);
    }
    (matches!(key.code, KeyCode::Enter) && key.modifiers.is_empty())
        .then_some(InputDialogAction::Submit)
}

#[cfg(test)]
mod tests {
    use super::{
        InputDialogAction, NavigationAction, input_dialog_action, navigation_action,
        search_navigation_action, structural_navigation_action,
    };
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

    #[test]
    fn navigation_and_dialog_keys_reject_unexpected_modifiers() {
        assert_eq!(
            navigation_action(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            navigation_action(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            input_dialog_action(
                KeyEvent::new(
                    KeyCode::Char('s'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                true,
            ),
            None
        );
        assert_eq!(
            input_dialog_action(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), false,),
            None
        );
    }

    #[test]
    fn searchable_inputs_reserve_printable_vim_keys_for_text() {
        assert_eq!(
            search_navigation_action(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE,)),
            None
        );
        assert_eq!(
            search_navigation_action(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE,)),
            None
        );
        assert_eq!(
            search_navigation_action(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Some(NavigationAction::Down)
        );
        assert_eq!(
            search_navigation_action(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
            Some(NavigationAction::PageUp)
        );
        assert_eq!(
            search_navigation_action(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
            Some(NavigationAction::PageDown)
        );
        assert_eq!(
            search_navigation_action(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            search_navigation_action(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn structural_navigation_has_no_letter_aliases() {
        assert_eq!(
            structural_navigation_action(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            structural_navigation_action(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Some(NavigationAction::Down)
        );
        for code in [
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
        ] {
            assert_eq!(
                structural_navigation_action(KeyEvent::new(code, KeyModifiers::NONE)),
                None
            );
        }
    }
}
