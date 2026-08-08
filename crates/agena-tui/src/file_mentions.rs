//! File-mention suggestion presentation and reducer.
//!
//! The App searches the workspace and resolves a selected key to its concrete
//! path. This module owns only picker display state and the selection/query
//! intents needed to drive that application effect.

use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent};
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
    Editor, SearchPicker, SearchPickerConfig, SearchPickerDialogSpec, SearchPickerInputResult,
    SearchPickerItem, SearchPickerNoCustom, SearchPickerSearchMode, render_search_picker_dialog,
    theme,
};

#[derive(Debug, Clone)]
/// Metadata of a file mention suggestion.
pub struct FileMentionSuggestionMeta {
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
/// A file mention suggestion item.
pub struct FileMentionSuggestionItem {
    /// Stable App-owned path-map key; it is not a filesystem path itself.
    pub key: String,
    pub label: String,
    pub detail: String,
}

impl SearchPickerItem for FileMentionSuggestionItem {
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
            .fg(theme::info_color())
            .add_modifier(Modifier::BOLD)
    }
}

pub type FileMentionSuggestionState = SearchPicker<
    FileMentionSuggestionItem,
    SearchPickerNoCustom,
    FileMentionSuggestionMeta,
    Editor,
>;

pub fn new_state(
    title: String,
    prompt: String,
    footer: String,
    empty_label: String,
    query: String,
    fingerprint: String,
) -> FileMentionSuggestionState {
    let mut config = SearchPickerConfig::searchable();
    config.search_mode = SearchPickerSearchMode::External;
    FileMentionSuggestionState::new(
        title,
        prompt,
        footer,
        empty_label,
        Editor::from_text(query),
        config,
        None,
        FileMentionSuggestionMeta { fingerprint },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Effect produced by a file mention suggestion.
pub enum FileMentionSuggestionEffect {
    KeepOpen,
    Dismiss,
    Refresh { query: String },
    Select { key: String },
    Unhandled,
}

pub fn handle_key(
    state: &mut FileMentionSuggestionState,
    key: KeyEvent,
) -> FileMentionSuggestionEffect {
    match resolve(KeyContext::Suggestion, key) {
        Some(KeyAction::Previous) => {
            if matches!(key.code, KeyCode::Up) {
                let _ = state.handle_input_key(key);
            } else {
                state.move_selection(-1);
            }
            FileMentionSuggestionEffect::KeepOpen
        }
        Some(KeyAction::Next) => {
            if matches!(key.code, KeyCode::Down) {
                let _ = state.handle_input_key(key);
            } else {
                state.move_selection(1);
            }
            FileMentionSuggestionEffect::KeepOpen
        }
        Some(KeyAction::Close) => FileMentionSuggestionEffect::Dismiss,
        Some(KeyAction::Fill | KeyAction::Accept) => selected_effect(state),
        _ => {
            let before_cursor = state.input.cursor();
            match state.handle_input_key(key) {
                SearchPickerInputResult::Navigated => FileMentionSuggestionEffect::KeepOpen,
                SearchPickerInputResult::Edited { changed } if changed => {
                    FileMentionSuggestionEffect::Refresh {
                        query: state.input.text().to_owned(),
                    }
                }
                SearchPickerInputResult::Edited { .. } if state.input.cursor() != before_cursor => {
                    FileMentionSuggestionEffect::KeepOpen
                }
                SearchPickerInputResult::Edited { .. } | SearchPickerInputResult::Close => {
                    FileMentionSuggestionEffect::Unhandled
                }
            }
        }
    }
}

/// Renders file-mention suggestions from TUI-owned picker state. The App
/// retains workspace search and maps the opaque key back to a concrete path.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &FileMentionSuggestionState,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-attach-matches").into(),
    );
    render_search_picker_dialog(frame, area, state, &spec, sanitize_picker_text);
}

fn selected_effect(state: &FileMentionSuggestionState) -> FileMentionSuggestionEffect {
    let Some(item) = state.selected_item() else {
        return FileMentionSuggestionEffect::KeepOpen;
    };
    FileMentionSuggestionEffect::Select {
        key: item.key.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{FileMentionSuggestionEffect, FileMentionSuggestionItem, handle_key};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn selection_emits_only_the_stable_path_map_key() {
        let mut state = super::new_state(
            "Files".to_owned(),
            String::new(),
            String::new(),
            "Empty".to_owned(),
            String::new(),
            "draft".to_owned(),
        );
        state.replace_items(vec![FileMentionSuggestionItem {
            key: "path:0".to_owned(),
            label: "notes.md".to_owned(),
            detail: "docs/notes.md".to_owned(),
        }]);

        assert_eq!(
            handle_key(
                &mut state,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            FileMentionSuggestionEffect::Select {
                key: "path:0".to_owned(),
            }
        );
    }
}
