//! Shared keymaps for reusable TUI components.
//!
//! Application actions are centralized by `agena-cli::tui_keymap`. This
//! module owns the physical keys shared by generic list, scroll, and dialog
//! components, while `editor.rs` owns the shell-style text-editing map.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Navigation action bound to keys.
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
        KeyCode::Left if key.modifiers.is_empty() => Some(NavigationAction::PageUp),
        KeyCode::Right if key.modifiers.is_empty() => Some(NavigationAction::PageDown),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Action of the input dialog.
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
