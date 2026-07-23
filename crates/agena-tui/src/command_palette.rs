//! Display-only command-palette presentation and semantic reducer.
//!
//! Concrete command specifications, plugin catalogs, and command execution
//! remain in the App effect adapter. This module owns only opaque row keys and
//! searchable presentation behavior.

use std::borrow::Cow;

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use agena_tui_components::{
    Editor, SearchPicker, SearchPickerConfig, SearchPickerDialogSpec, SearchPickerInputResult,
    SearchPickerItem, SearchPickerNoCustom, render_search_picker_dialog,
};

use crate::{i18n::I18n, sanitize_picker_text};

#[derive(Debug, Clone)]
pub struct CommandPaletteItem {
    pub key: String,
    pub label: String,
    pub detail: String,
    pub search_text: String,
}

impl CommandPaletteItem {
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
        }
    }
}

impl SearchPickerItem for CommandPaletteItem {
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
}

pub type CommandPalettePresentation =
    SearchPicker<CommandPaletteItem, SearchPickerNoCustom, (), Editor>;

pub fn new_presentation(
    title: String,
    prompt: String,
    footer: String,
    empty_message: String,
    items: Vec<CommandPaletteItem>,
) -> CommandPalettePresentation {
    let mut presentation = CommandPalettePresentation::new(
        title,
        prompt,
        footer,
        empty_message,
        Editor::default(),
        SearchPickerConfig::searchable(),
        None,
        (),
    );
    presentation.replace_items(items);
    presentation
}

#[derive(Debug, Clone)]
pub enum CommandPaletteAction {
    Accept,
    Input(KeyEvent),
    Paste(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteEffect {
    Close,
    Activate { key: String },
    KeepOpen,
}

pub fn reduce(
    presentation: &mut CommandPalettePresentation,
    action: CommandPaletteAction,
) -> CommandPaletteEffect {
    match action {
        CommandPaletteAction::Accept => presentation
            .selected_item()
            .map(|item| CommandPaletteEffect::Activate {
                key: item.key.clone(),
            })
            .unwrap_or(CommandPaletteEffect::KeepOpen),
        CommandPaletteAction::Input(key) => match presentation.handle_input_key(key) {
            SearchPickerInputResult::Close => CommandPaletteEffect::Close,
            SearchPickerInputResult::Navigated | SearchPickerInputResult::Edited { .. } => {
                CommandPaletteEffect::KeepOpen
            }
        },
        CommandPaletteAction::Paste(text) => {
            presentation.input.insert_str(text.as_str());
            presentation.refresh_results();
            CommandPaletteEffect::KeepOpen
        }
    }
}

/// Renders the command palette from TUI-owned display state. The App retains
/// the command catalog and executes the opaque selected key.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &CommandPalettePresentation,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-commands-title").into(),
    );
    render_search_picker_dialog(frame, area, dialog, &spec, sanitize_picker_text);
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        CommandPaletteAction, CommandPaletteEffect, CommandPaletteItem, new_presentation, reduce,
    };

    #[test]
    fn activation_returns_only_the_opaque_display_key() {
        let mut presentation = new_presentation(
            "Commands".to_owned(),
            "Search".to_owned(),
            "Footer".to_owned(),
            "Empty".to_owned(),
            vec![CommandPaletteItem::new(
                "command:help",
                "/help",
                "Show help",
                "/help help",
            )],
        );

        assert_eq!(
            reduce(&mut presentation, CommandPaletteAction::Accept),
            CommandPaletteEffect::Activate {
                key: "command:help".to_owned(),
            }
        );
    }

    #[test]
    fn escape_closes_the_presentation() {
        let mut presentation = new_presentation(
            "Commands".to_owned(),
            "Search".to_owned(),
            "Footer".to_owned(),
            "Empty".to_owned(),
            Vec::new(),
        );

        assert_eq!(
            reduce(
                &mut presentation,
                CommandPaletteAction::Input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            ),
            CommandPaletteEffect::Close
        );
    }
}
