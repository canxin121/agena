//! Slash-command suggestion presentation and reducer.
//!
//! The App obtains built-in/plugin catalogs and maps a selected key to its
//! concrete command action. This module owns only picker display state and
//! semantic fill/accept/dismiss intents.

use std::borrow::Cow;

use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
};

use crate::{
    i18n::I18n,
    keymap::{KeyAction, KeyContext, resolve},
    sanitize_picker_text,
};
use agena_tui_components::{
    Editor, SearchPicker, SearchPickerDialogSpec, SearchPickerInputResult, SearchPickerItem,
    SearchPickerNoCustom, render_search_picker_dialog, theme,
};

#[derive(Debug, Clone)]
pub struct SlashCommandSuggestionMeta {
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct SlashCommandSuggestionItem {
    /// Stable App-owned action-map key; it is not a provider/plugin object.
    pub key: String,
    pub label: String,
    pub detail: String,
}

impl SearchPickerItem for SlashCommandSuggestionItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.key)
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.detail))
    }

    fn search_picker_label_style(&self) -> Style {
        Style::default()
            .fg(theme::accent_color())
            .add_modifier(Modifier::BOLD)
    }
}

pub type SlashCommandSuggestionState = SearchPicker<
    SlashCommandSuggestionItem,
    SearchPickerNoCustom,
    SlashCommandSuggestionMeta,
    Editor,
>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommandSuggestionEffect {
    KeepOpen,
    Dismiss,
    Fill { key: String },
    Accept { key: String },
    Unhandled,
}

pub fn handle_key(
    state: &mut SlashCommandSuggestionState,
    key: KeyEvent,
) -> SlashCommandSuggestionEffect {
    match resolve(KeyContext::Suggestion, key) {
        Some(KeyAction::Previous) => {
            if matches!(key.code, crossterm::event::KeyCode::Up) {
                let _ = state.handle_input_key(key);
            } else {
                state.move_selection(-1);
            }
            SlashCommandSuggestionEffect::KeepOpen
        }
        Some(KeyAction::Next) => {
            if matches!(key.code, crossterm::event::KeyCode::Down) {
                let _ = state.handle_input_key(key);
            } else {
                state.move_selection(1);
            }
            SlashCommandSuggestionEffect::KeepOpen
        }
        Some(KeyAction::Close) => SlashCommandSuggestionEffect::Dismiss,
        Some(KeyAction::Fill) => selected_effect(state, false),
        Some(KeyAction::Accept) => selected_effect(state, true),
        _ => {
            let before_cursor = state.input.cursor();
            match state.handle_input_key(key) {
                SearchPickerInputResult::Navigated => SlashCommandSuggestionEffect::KeepOpen,
                SearchPickerInputResult::Edited { changed } => {
                    if changed || state.input.cursor() != before_cursor {
                        SlashCommandSuggestionEffect::KeepOpen
                    } else {
                        SlashCommandSuggestionEffect::Unhandled
                    }
                }
                SearchPickerInputResult::Close => SlashCommandSuggestionEffect::Unhandled,
            }
        }
    }
}

/// Renders slash-command suggestions from TUI-owned picker state. The App
/// retains built-in/plugin catalogs and performs the selected command effect.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &SlashCommandSuggestionState,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-attach-matches").into(),
    );
    render_search_picker_dialog(frame, area, state, &spec, sanitize_picker_text);
}

fn selected_effect(
    state: &SlashCommandSuggestionState,
    submit: bool,
) -> SlashCommandSuggestionEffect {
    let Some(item) = state.selected_item() else {
        return SlashCommandSuggestionEffect::KeepOpen;
    };
    if submit {
        SlashCommandSuggestionEffect::Accept {
            key: item.key.clone(),
        }
    } else {
        SlashCommandSuggestionEffect::Fill {
            key: item.key.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SlashCommandSuggestionEffect, SlashCommandSuggestionItem, SlashCommandSuggestionMeta,
        SlashCommandSuggestionState, handle_key,
    };
    use agena_tui_components::{Editor, SearchPickerConfig};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn explicit_accept_emits_only_the_stable_action_key() {
        let mut state = SlashCommandSuggestionState::new(
            "Commands".to_owned(),
            String::new(),
            String::new(),
            "Empty".to_owned(),
            Editor::default(),
            SearchPickerConfig::searchable(),
            None,
            SlashCommandSuggestionMeta {
                fingerprint: "draft".to_owned(),
            },
        );
        state.replace_items(vec![SlashCommandSuggestionItem {
            key: "command:help".to_owned(),
            label: "/help".to_owned(),
            detail: "Show help".to_owned(),
        }]);

        assert_eq!(
            handle_key(
                &mut state,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            SlashCommandSuggestionEffect::Accept {
                key: "command:help".to_owned()
            }
        );
    }
}
