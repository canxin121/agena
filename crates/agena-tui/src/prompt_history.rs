//! Prompt-history picker presentation and input reducer.
//!
//! History persistence and replacing the live composer draft remain App
//! effects. This module owns the display rows and makes text available only
//! after an explicit accept action.

use std::borrow::Cow;

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use crate::{
    i18n::I18n,
    keymap::{KeyAction, KeyContext, resolve},
    sanitize_picker_text,
};
use agena_tui_components::{
    Editor, SearchPicker, SearchPickerDialogSpec, SearchPickerItem, SearchPickerNoCustom,
    render_search_picker_dialog,
};

#[derive(Debug, Clone)]
pub struct PromptHistorySearchResult {
    pub history_index: usize,
    pub text: String,
}

impl SearchPickerItem for PromptHistorySearchResult {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(self.history_index.to_string())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.text.as_str())
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        None
    }

    fn search_picker_prefix(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(format!("#{:<3} ", self.history_index + 1)))
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.text.as_str())
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.text.as_str())
    }
}

pub type PromptHistorySearchState =
    SearchPicker<PromptHistorySearchResult, SearchPickerNoCustom, (), Editor>;

#[derive(Debug, PartialEq, Eq)]
pub enum PromptHistoryPickerEffect {
    KeepOpen,
    Close,
    UseText(String),
}

/// Reduces picker keyboard input without mutating an App composer.
pub fn handle_key(
    search: &mut PromptHistorySearchState,
    key: KeyEvent,
) -> PromptHistoryPickerEffect {
    match resolve(KeyContext::PromptHistory, key) {
        Some(KeyAction::Close) => PromptHistoryPickerEffect::Close,
        Some(KeyAction::Accept) => search
            .selected_item()
            .map(|result| PromptHistoryPickerEffect::UseText(result.text.clone()))
            .unwrap_or(PromptHistoryPickerEffect::Close),
        Some(KeyAction::Previous | KeyAction::Next) => {
            let _ = search.handle_input_key(key);
            PromptHistoryPickerEffect::KeepOpen
        }
        Some(KeyAction::Older) => {
            search.move_selection(1);
            PromptHistoryPickerEffect::KeepOpen
        }
        Some(KeyAction::Newer) if search.selected == 0 => PromptHistoryPickerEffect::Close,
        Some(KeyAction::Newer) => {
            search.move_selection(-1);
            PromptHistoryPickerEffect::KeepOpen
        }
        Some(KeyAction::NewerKeepOpen) => {
            search.move_selection(-1);
            PromptHistoryPickerEffect::KeepOpen
        }
        _ => {
            let _ = search.handle_input_key(key);
            PromptHistoryPickerEffect::KeepOpen
        }
    }
}

/// Renders the prompt-history picker from TUI-owned presentation state. The
/// App retains history persistence and applying the accepted draft text.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &PromptHistorySearchState,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-attach-matches").into(),
    )
    .with_search_label(i18n.text("composer-prompt-history-label").into());
    render_search_picker_dialog(frame, area, state, &spec, sanitize_picker_text);
}

#[cfg(test)]
mod tests {
    use super::{
        PromptHistoryPickerEffect, PromptHistorySearchResult, PromptHistorySearchState, handle_key,
    };
    use agena_tui_components::{Editor, SearchPickerConfig};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn only_explicit_accept_returns_history_text() {
        let mut search = PromptHistorySearchState::new(
            "History".to_owned(),
            String::new(),
            String::new(),
            "No matches".to_owned(),
            Editor::default(),
            SearchPickerConfig::searchable(),
            None,
            (),
        );
        search.replace_items(vec![PromptHistorySearchResult {
            history_index: 0,
            text: "saved prompt".to_owned(),
        }]);

        assert_eq!(
            handle_key(
                &mut search,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
            ),
            PromptHistoryPickerEffect::KeepOpen
        );
        assert_eq!(
            handle_key(
                &mut search,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            PromptHistoryPickerEffect::UseText("saved prompt".to_owned())
        );
    }
}
