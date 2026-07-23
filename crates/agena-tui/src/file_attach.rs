//! File-attachment picker presentation and reducer.
//!
//! The application owns filesystem search and attachment staging. This module
//! owns the editable picker, its display-only rows, and stable selection
//! effects. Rows carry only App-owned action keys, never filesystem paths.

use std::borrow::Cow;

use agena_tui_components::{
    Editor, SearchPicker, SearchPickerConfig, SearchPickerCustomValue, SearchPickerDialogSpec,
    SearchPickerInputMode, SearchPickerInputResult, SearchPickerItem, SearchPickerSearchMode,
    SearchPickerSelection, render_search_picker_dialog,
};
use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use crate::{
    i18n::I18n,
    keymap::{KeyAction, KeyContext, resolve},
};

#[derive(Debug, Clone)]
pub struct FileAttachItem {
    /// Stable application-owned action key; never a filesystem path.
    pub key: String,
    pub label: String,
    pub detail: String,
}

impl SearchPickerItem for FileAttachItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.key)
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.detail))
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.detail)
    }
}

#[derive(Debug, Clone)]
pub struct FileAttachMeta {
    pub i18n: I18n,
}

#[derive(Debug, Clone)]
pub struct FileAttachCustomValue {
    pub raw: String,
}

impl SearchPickerCustomValue<FileAttachMeta> for FileAttachCustomValue {
    fn search_picker_from_input(input: &str, _: &FileAttachMeta) -> Option<Self> {
        let raw = input.trim().to_owned();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_picker_label(&self, meta: &FileAttachMeta) -> Cow<'_, str> {
        Cow::Owned(meta.i18n.text("search-picker-custom-path-label"))
    }

    fn search_picker_detail(&self, _: &FileAttachMeta) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.raw))
    }

    fn search_picker_input_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.raw)
    }
}

pub type FileAttachPresentation =
    SearchPicker<FileAttachItem, FileAttachCustomValue, FileAttachMeta, Editor>;

pub fn new_presentation(
    title: String,
    prompt: String,
    footer: String,
    empty_label: String,
    i18n: I18n,
) -> FileAttachPresentation {
    FileAttachPresentation::new(
        title,
        prompt,
        footer,
        empty_label,
        Editor::default(),
        SearchPickerConfig {
            input_mode: SearchPickerInputMode::EditableValue,
            search_mode: SearchPickerSearchMode::External,
            fill_selected_into_input: true,
            ..SearchPickerConfig::searchable()
        },
        None,
        FileAttachMeta { i18n },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAttachEffect {
    KeepOpen,
    Close,
    Refresh,
    SelectItem { key: String },
    SelectCustom { raw: String },
}

pub fn handle_key(state: &mut FileAttachPresentation, key: KeyEvent) -> FileAttachEffect {
    if resolve(KeyContext::FileAttach, key) == Some(KeyAction::Accept) {
        return selection_effect(state);
    }
    match state.handle_input_key(key) {
        SearchPickerInputResult::Close => FileAttachEffect::Close,
        SearchPickerInputResult::Navigated => FileAttachEffect::KeepOpen,
        SearchPickerInputResult::Edited { changed } if changed => FileAttachEffect::Refresh,
        SearchPickerInputResult::Edited { .. } => FileAttachEffect::KeepOpen,
    }
}

fn selection_effect(state: &FileAttachPresentation) -> FileAttachEffect {
    match state.selected_row() {
        Some(SearchPickerSelection::Item(item)) => FileAttachEffect::SelectItem {
            key: item.key.clone(),
        },
        Some(SearchPickerSelection::Custom(value)) => FileAttachEffect::SelectCustom {
            raw: value.raw.clone(),
        },
        Some(SearchPickerSelection::Clear(_)) | None => {
            let raw = state.input.text().trim();
            if raw.is_empty() {
                FileAttachEffect::KeepOpen
            } else {
                FileAttachEffect::SelectCustom {
                    raw: raw.to_owned(),
                }
            }
        }
    }
}

/// Renders the complete file-attachment picker from TUI-owned presentation
/// state. Filesystem search and attachment staging stay in the App effect
/// adapter, which provides rows keyed by opaque action IDs.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &FileAttachPresentation,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-attach-matches").into(),
    );
    render_search_picker_dialog(frame, area, dialog, &spec, sanitize_display_text);
}

fn sanitize_display_text(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        match ch {
            '\r' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{FileAttachEffect, FileAttachItem, handle_key, new_presentation};
    use crate::i18n::I18n;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn selection_returns_only_a_stable_application_action_key() {
        let mut state = new_presentation(
            "Attach".into(),
            "Path".into(),
            "footer".into(),
            "empty".into(),
            I18n::default(),
        );
        state.replace_items(vec![FileAttachItem {
            key: "file:0".into(),
            label: "notes.md".into(),
            detail: "notes.md".into(),
        }]);
        assert_eq!(
            handle_key(
                &mut state,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            ),
            FileAttachEffect::SelectItem {
                key: "file:0".into(),
            }
        );
    }
}
