//! Path-browser presentation and reducer.
//!
//! Filesystem enumeration, path resolution, and committing a selected path
//! remain application effects. This module owns only the picker state,
//! display rows, custom-input rendering, and semantic selection intents.

use std::borrow::Cow;

use crossterm::event::KeyEvent;
use ratatui::style::{Modifier, Style};

use crate::i18n::I18n;
use crate::keymap::{KeyAction, KeyContext, resolve};
use agena_tui_components::{
    Editor, SearchPicker, SearchPickerConfig, SearchPickerCustomValue, SearchPickerDialogSpec,
    SearchPickerFocus, SearchPickerInputResult, SearchPickerItem, SearchPickerSelection,
    render_search_picker_dialog,
};
use ratatui::{Frame, layout::Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathBrowserMode {
    AnyPath,
    DirectoryOnly,
}

#[derive(Debug, Clone)]
pub struct PathBrowserMeta {
    pub i18n: I18n,
    pub mode: PathBrowserMode,
}

#[derive(Debug, Clone)]
pub struct PathBrowserCustomValue {
    pub raw: String,
}

impl SearchPickerCustomValue<PathBrowserMeta> for PathBrowserCustomValue {
    fn search_picker_from_input(input: &str, _: &PathBrowserMeta) -> Option<Self> {
        let raw = input.trim().to_owned();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_picker_label(&self, meta: &PathBrowserMeta) -> Cow<'_, str> {
        Cow::Owned(meta.i18n.text("search-picker-custom-path-label"))
    }

    fn search_picker_detail(&self, _: &PathBrowserMeta) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.raw))
    }

    fn search_picker_input_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.raw)
    }
}

#[derive(Debug, Clone)]
pub struct PathBrowserItem {
    /// Stable App-owned path-action key; this is not a filesystem path.
    pub key: String,
    pub label: String,
    /// Full path retained for matching and input filling, but deliberately
    /// not rendered beside every filename: the editable input already shows
    /// the current directory.
    pub search_text: String,
    pub is_dir: bool,
}

impl SearchPickerItem for PathBrowserItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.key)
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        None
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.search_text)
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.search_text)
    }

    fn search_picker_label_style(&self) -> Style {
        if self.is_dir {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    }
}

pub type PathBrowserPresentation =
    SearchPicker<PathBrowserItem, PathBrowserCustomValue, PathBrowserMeta, Editor>;

pub fn new_presentation(
    title: String,
    prompt: String,
    footer: String,
    empty_label: String,
    input: String,
    i18n: I18n,
    mode: PathBrowserMode,
) -> PathBrowserPresentation {
    PathBrowserPresentation::new(
        title,
        prompt,
        footer,
        empty_label,
        Editor::from_text(input),
        SearchPickerConfig {
            wrap_input: true,
            ..SearchPickerConfig::searchable()
        },
        None,
        PathBrowserMeta { i18n, mode },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathBrowserEffect {
    KeepOpen,
    Refresh,
    Close,
    Parent,
    EnterDirectory { key: String },
    SelectItem { key: String, is_dir: bool },
    SelectCustom { raw: String },
}

pub fn handle_key(state: &mut PathBrowserPresentation, key: KeyEvent) -> PathBrowserEffect {
    match resolve(KeyContext::PathBrowser, key) {
        Some(KeyAction::Accept) => return selection_effect(state),
        // Keep ordinary Left/Right editing while the path field is focused.
        // Once results own focus, they become file-browser navigation.
        Some(KeyAction::MoveLeft) if state.focus == SearchPickerFocus::Results => {
            return PathBrowserEffect::Parent;
        }
        Some(KeyAction::MoveRight) if state.focus == SearchPickerFocus::Results => {
            return state
                .selected_item()
                .filter(|item| item.is_dir)
                .map(|item| PathBrowserEffect::EnterDirectory {
                    key: item.key.clone(),
                })
                .unwrap_or(PathBrowserEffect::KeepOpen);
        }
        _ => {}
    }
    match state.handle_input_key(key) {
        SearchPickerInputResult::Close => PathBrowserEffect::Close,
        SearchPickerInputResult::Navigated => PathBrowserEffect::KeepOpen,
        SearchPickerInputResult::Edited { changed } => {
            if changed {
                PathBrowserEffect::Refresh
            } else {
                PathBrowserEffect::KeepOpen
            }
        }
    }
}

fn selection_effect(state: &PathBrowserPresentation) -> PathBrowserEffect {
    match state.selected_row() {
        Some(SearchPickerSelection::Item(item)) => PathBrowserEffect::SelectItem {
            key: item.key.clone(),
            is_dir: item.is_dir,
        },
        Some(SearchPickerSelection::Custom(value)) => PathBrowserEffect::SelectCustom {
            raw: value.raw.clone(),
        },
        Some(SearchPickerSelection::Clear(_)) | None => {
            let raw = state.input.text().trim();
            if raw.is_empty() {
                PathBrowserEffect::KeepOpen
            } else {
                PathBrowserEffect::SelectCustom {
                    raw: raw.to_owned(),
                }
            }
        }
    }
}

/// Renders the complete path-browser picker from TUI-owned presentation
/// state. Workspace enumeration, path resolution, and committing the chosen
/// permission-rule path remain App effects keyed by the opaque row action.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &PathBrowserPresentation,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-path-browser-list-title").into(),
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
    use super::{
        PathBrowserEffect, PathBrowserItem, PathBrowserMode, handle_key, new_presentation,
        render_overlay,
    };
    use crate::i18n::I18n;
    use agena_tui_components::SearchPickerFocus;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn accept_emits_only_the_stable_path_action_key() {
        let mut state = new_presentation(
            "Browse".to_owned(),
            String::new(),
            String::new(),
            "Empty".to_owned(),
            String::new(),
            I18n::english(),
            PathBrowserMode::AnyPath,
        );
        state.replace_items(vec![PathBrowserItem {
            key: "path:0".to_owned(),
            label: "notes.md".to_owned(),
            search_text: "docs/notes.md".to_owned(),
            is_dir: false,
        }]);

        assert_eq!(
            handle_key(
                &mut state,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            PathBrowserEffect::SelectItem {
                key: "path:0".to_owned(),
                is_dir: false,
            }
        );
    }

    #[test]
    fn result_focused_arrows_enter_a_directory_or_go_to_its_parent() {
        let mut state = new_presentation(
            "Browse".to_owned(),
            String::new(),
            String::new(),
            "Empty".to_owned(),
            String::new(),
            I18n::english(),
            PathBrowserMode::AnyPath,
        );
        state.replace_items(vec![PathBrowserItem {
            key: "path:0".to_owned(),
            label: "src/".to_owned(),
            search_text: "src".to_owned(),
            is_dir: true,
        }]);
        state.focus = SearchPickerFocus::Results;

        assert_eq!(
            handle_key(
                &mut state,
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)
            ),
            PathBrowserEffect::EnterDirectory {
                key: "path:0".to_owned()
            }
        );
        assert_eq!(
            handle_key(&mut state, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            PathBrowserEffect::Parent
        );
    }

    #[test]
    fn browser_rows_show_only_the_child_name_not_the_full_path() {
        let i18n = I18n::english();
        let mut state = new_presentation(
            "Browse".to_owned(),
            String::new(),
            String::new(),
            "Empty".to_owned(),
            "/workspace/".to_owned(),
            i18n.clone(),
            PathBrowserMode::AnyPath,
        );
        state.replace_items(vec![PathBrowserItem {
            key: "path:0".to_owned(),
            label: "document.md".to_owned(),
            search_text: "/workspace/document.md".to_owned(),
            is_dir: false,
        }]);
        let backend = TestBackend::new(80, 18);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_overlay(frame, frame.area(), &state, &i18n))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| buffer[(x, y)].symbol()))
            .collect::<String>();

        assert!(rendered.contains("document.md"), "{rendered}");
        assert!(!rendered.contains("/workspace/document.md"), "{rendered}");
    }
}
