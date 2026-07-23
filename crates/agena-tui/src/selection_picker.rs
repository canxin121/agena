//! Reusable display-only searchable selection-picker presentation.
//!
//! Concrete provider, agent, and inspector objects stay in their App effect
//! adapters. This module carries only opaque row keys and presentation policy.

use std::borrow::Cow;

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect, style::Style};

use agena_tui_components::{
    Editor, SearchPicker, SearchPickerConfig, SearchPickerDialogSpec, SearchPickerInputResult,
    SearchPickerItem, SearchPickerNoCustom, render_search_picker_dialog,
};

use crate::{i18n::I18n, sanitize_picker_text};

#[derive(Debug, Clone)]
pub struct SelectionPickerItem {
    pub key: String,
    pub label: String,
    pub detail: String,
    pub search_text: String,
    pub always_visible: bool,
    pub prefix: Option<String>,
}

impl SelectionPickerItem {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
        search_text: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            detail: detail.into(),
            search_text: search_text.into(),
            always_visible: false,
            prefix: None,
        }
    }

    pub fn always_visible(mut self) -> Self {
        self.always_visible = true;
        self
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }
}

impl SearchPickerItem for SelectionPickerItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.key.as_str())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.label.as_str())
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        (!self.detail.trim().is_empty()).then_some(Cow::Borrowed(self.detail.as_str()))
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.search_text.as_str())
    }

    fn search_picker_always_visible(&self) -> bool {
        self.always_visible
    }

    fn search_picker_prefix(&self) -> Option<Cow<'_, str>> {
        self.prefix.as_deref().map(Cow::Borrowed)
    }

    fn search_picker_prefix_style(&self) -> Style {
        Style::default().fg(agena_tui_components::theme::accent_color())
    }
}

pub type SelectionPickerPresentation =
    SearchPicker<SelectionPickerItem, SearchPickerNoCustom, (), Editor>;

pub fn new_presentation(
    title: String,
    prompt: String,
    footer: String,
    empty_message: String,
    initial_query: String,
) -> SelectionPickerPresentation {
    SelectionPickerPresentation::new(
        title,
        prompt,
        footer,
        empty_message,
        Editor::from_text(initial_query),
        SearchPickerConfig::searchable(),
        None,
        (),
    )
}

#[derive(Debug, Clone)]
pub enum SelectionPickerAction {
    Accept,
    Input(KeyEvent),
    Paste(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionPickerEffect {
    Close,
    Activate { key: String },
    KeepOpen,
}

pub fn reduce(
    presentation: &mut SelectionPickerPresentation,
    action: SelectionPickerAction,
) -> SelectionPickerEffect {
    match action {
        SelectionPickerAction::Accept => presentation
            .selected_item()
            .map(|item| SelectionPickerEffect::Activate {
                key: item.key.clone(),
            })
            .unwrap_or(SelectionPickerEffect::KeepOpen),
        SelectionPickerAction::Input(key) => match presentation.handle_input_key(key) {
            SearchPickerInputResult::Close => SelectionPickerEffect::Close,
            SearchPickerInputResult::Navigated | SearchPickerInputResult::Edited { .. } => {
                SelectionPickerEffect::KeepOpen
            }
        },
        SelectionPickerAction::Paste(text) => {
            presentation.input.insert_str(text.as_str());
            presentation.refresh_results();
            SelectionPickerEffect::KeepOpen
        }
    }
}

/// Renders the selection picker from its TUI-owned rows and reducer state.
/// The App retains only the opaque-key action map and concrete effect.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &SelectionPickerPresentation,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-attach-matches").into(),
    );
    render_search_picker_dialog(frame, area, dialog, &spec, sanitize_picker_text);
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        SelectionPickerAction, SelectionPickerEffect, SelectionPickerItem, new_presentation, reduce,
    };

    #[test]
    fn activation_returns_only_the_opaque_row_key() {
        let mut presentation = new_presentation(
            "Agents".to_owned(),
            "Search".to_owned(),
            "Footer".to_owned(),
            "Empty".to_owned(),
            String::new(),
        );
        presentation.replace_items(vec![SelectionPickerItem::new(
            "agent:review",
            "review",
            "Reviewer",
            "review Reviewer",
        )]);

        assert_eq!(
            reduce(&mut presentation, SelectionPickerAction::Accept),
            SelectionPickerEffect::Activate {
                key: "agent:review".to_owned(),
            }
        );
    }

    #[test]
    fn escape_closes_the_presentation() {
        let mut presentation = new_presentation(
            "Agents".to_owned(),
            "Search".to_owned(),
            "Footer".to_owned(),
            "Empty".to_owned(),
            String::new(),
        );

        assert_eq!(
            reduce(
                &mut presentation,
                SelectionPickerAction::Input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            ),
            SelectionPickerEffect::Close
        );
    }
}
