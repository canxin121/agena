//! Shared display row for terminal choice pickers.
//!
//! The application supplies localized text and maps a selected `value` to its
//! concrete effect. This module owns only the reusable searchable row and its
//! current-selection presentation.

use std::borrow::Cow;

use agena_tui_components::{
    Editor, SearchPicker, SearchPickerClearAction, SearchPickerConfig, SearchPickerCustomValue,
    SearchPickerDialogSpec, SearchPickerFocus, SearchPickerInputMode, SearchPickerInputResult,
    SearchPickerItem, SearchPickerSelection, SearchPickerSelectionMode,
    render_search_picker_dialog,
};
use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect, style::Style};

use crate::{i18n::I18n, sanitize_picker_text};

#[derive(Debug, Clone)]
pub struct ChoicePickerItem {
    pub label: String,
    pub detail: String,
    pub value: String,
    pub search_text: String,
    pub current: bool,
}

impl SearchPickerItem for ChoicePickerItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.value)
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        (!self.detail.trim().is_empty()).then_some(Cow::Borrowed(self.detail.as_str()))
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.value)
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.search_text)
    }

    fn search_picker_prefix(&self) -> Option<Cow<'_, str>> {
        self.current.then_some(Cow::Borrowed("✓ "))
    }

    fn search_picker_prefix_style(&self) -> Style {
        Style::default().fg(agena_tui_components::theme::accent_color())
    }
}

/// Display-only custom input configuration. The App resolves localized custom
/// row text before construction, while TUI owns when the custom row exists and
/// how it participates in selection.
#[derive(Debug, Clone)]
pub struct ChoicePresentationMeta {
    pub current_value: Option<String>,
    pub custom_label: String,
    pub custom_detail_prefix: String,
    pub custom_detail_suffix: String,
}

#[derive(Debug, Clone)]
pub struct ChoiceCustomValue {
    pub raw: String,
}

impl SearchPickerCustomValue<ChoicePresentationMeta> for ChoiceCustomValue {
    fn search_picker_from_input(input: &str, _: &ChoicePresentationMeta) -> Option<Self> {
        let raw = input.trim().to_owned();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_picker_label(&self, meta: &ChoicePresentationMeta) -> Cow<'_, str> {
        Cow::Owned(meta.custom_label.clone())
    }

    fn search_picker_detail(&self, meta: &ChoicePresentationMeta) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(format!(
            "{}{}{}",
            meta.custom_detail_prefix, self.raw, meta.custom_detail_suffix
        )))
    }

    fn search_picker_input_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.raw.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoicePresentationStyle {
    Searchable,
    SearchableSelect,
    SelectOnly,
}

impl ChoicePresentationStyle {
    fn config(self) -> SearchPickerConfig {
        match self {
            Self::Searchable => SearchPickerConfig {
                input_mode: SearchPickerInputMode::SearchWithCustomValue,
                ..SearchPickerConfig::searchable()
            },
            Self::SearchableSelect => SearchPickerConfig::searchable(),
            Self::SelectOnly => SearchPickerConfig::select_only(),
        }
    }
}

pub type ChoicePresentation =
    SearchPicker<ChoicePickerItem, ChoiceCustomValue, ChoicePresentationMeta, Editor>;

#[allow(clippy::too_many_arguments)]
pub fn new_presentation(
    title: String,
    prompt: String,
    footer: String,
    empty_message: String,
    mut items: Vec<ChoicePickerItem>,
    current_value: Option<String>,
    clear_action: Option<SearchPickerClearAction>,
    style: ChoicePresentationStyle,
    custom_label: String,
    custom_detail_prefix: String,
    custom_detail_suffix: String,
) -> ChoicePresentation {
    mark_current_item(items.as_mut_slice(), current_value.as_deref());
    let mut presentation = ChoicePresentation::new(
        title,
        prompt,
        footer,
        empty_message,
        Editor::default(),
        style.config(),
        clear_action,
        ChoicePresentationMeta {
            current_value,
            custom_label,
            custom_detail_prefix,
            custom_detail_suffix,
        },
    );
    presentation.replace_items(items);
    refresh(&mut presentation);
    select_current(&mut presentation);
    presentation
}

#[derive(Debug, Clone)]
pub enum ChoicePresentationAction {
    Accept,
    Input(KeyEvent),
    Paste(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceSelection {
    Clear,
    Custom { raw: String },
    Item { value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoicePresentationEffect {
    Close,
    Commit(ChoiceSelection),
    KeepOpen,
}

pub fn reduce(
    presentation: &mut ChoicePresentation,
    action: ChoicePresentationAction,
) -> ChoicePresentationEffect {
    match action {
        ChoicePresentationAction::Accept => selection(presentation)
            .map(ChoicePresentationEffect::Commit)
            .unwrap_or(ChoicePresentationEffect::KeepOpen),
        ChoicePresentationAction::Input(key) => {
            if presentation.config.selection_mode == SearchPickerSelectionMode::Multiple
                && presentation.focus == SearchPickerFocus::Results
                && key.code == crossterm::event::KeyCode::Char(' ')
                && key.modifiers.is_empty()
            {
                presentation.toggle_selected();
                return ChoicePresentationEffect::KeepOpen;
            }
            match presentation.handle_input_key(key) {
                SearchPickerInputResult::Close => ChoicePresentationEffect::Close,
                SearchPickerInputResult::Navigated => ChoicePresentationEffect::KeepOpen,
                SearchPickerInputResult::Edited { changed } => {
                    if changed {
                        sync_input(presentation);
                    }
                    ChoicePresentationEffect::KeepOpen
                }
            }
        }
        ChoicePresentationAction::Paste(text) => {
            presentation.input.insert_str(text.as_str());
            sync_input(presentation);
            ChoicePresentationEffect::KeepOpen
        }
    }
}

/// Renders the complete choice picker from TUI-owned state/reduction. The App
/// maps the selected scalar to its concrete settings or session effect.
pub fn render_overlay(frame: &mut Frame<'_>, area: Rect, dialog: &ChoicePresentation, i18n: &I18n) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-attach-matches").into(),
    );
    render_search_picker_dialog(frame, area, dialog, &spec, sanitize_picker_text);
}

pub fn refresh(presentation: &mut ChoicePresentation) {
    presentation.refresh_results();
}

pub fn sync_input(presentation: &mut ChoicePresentation) {
    refresh(presentation);
    select_query_row(presentation);
}

pub fn select_current(presentation: &mut ChoicePresentation) {
    if presentation.select_item_where(|item| item.current) {
        return;
    }
    if presentation.meta.current_value.is_none() && presentation.clear_action.is_some() {
        presentation.selected = 0;
    }
}

/// Mark exactly one projected row as the committed value when it is still
/// available. The App supplies only display rows and the opaque current value;
/// the presentation owns current-row policy and its visible marker.
fn mark_current_item(items: &mut [ChoicePickerItem], current_value: Option<&str>) {
    for item in &mut *items {
        item.current = false;
    }
    let Some(current_value) = current_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    if let Some(item) = items.iter_mut().find(|item| {
        item.value.eq_ignore_ascii_case(current_value)
            || item.label.eq_ignore_ascii_case(current_value)
    }) {
        item.current = true;
    }
}

pub fn select_query_row(presentation: &mut ChoicePresentation) {
    let trimmed = presentation.input.text().trim().to_owned();
    if trimmed.is_empty() {
        select_current(presentation);
        return;
    }
    if presentation.select_item_where(|item| {
        item.value.eq_ignore_ascii_case(&trimmed) || item.label.eq_ignore_ascii_case(&trimmed)
    }) || presentation.select_item_where(|_| true)
    {
        return;
    }
    if presentation.config.input_mode.allows_custom_value()
        && ChoiceCustomValue::search_picker_from_input(
            presentation.input.text(),
            &presentation.meta,
        )
        .is_some()
    {
        presentation.selected = usize::from(presentation.clear_action.is_some());
    } else {
        presentation.clamp_selection();
    }
}

pub fn selection(presentation: &ChoicePresentation) -> Option<ChoiceSelection> {
    match presentation.selected_row()? {
        SearchPickerSelection::Clear(_) => Some(ChoiceSelection::Clear),
        SearchPickerSelection::Custom(value) => Some(ChoiceSelection::Custom { raw: value.raw }),
        SearchPickerSelection::Item(item) => Some(ChoiceSelection::Item { value: item.value }),
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        ChoicePickerItem, ChoicePresentationAction, ChoicePresentationEffect,
        ChoicePresentationStyle, ChoiceSelection, new_presentation, reduce,
    };
    use agena_tui_components::{
        SearchPickerClearAction, SearchPickerFocus, SearchPickerItem, SearchPickerSelectionMode,
    };

    fn item(value: &str) -> ChoicePickerItem {
        ChoicePickerItem {
            label: value.to_owned(),
            detail: format!("{value} detail"),
            value: value.to_owned(),
            search_text: format!("{value} {value} detail"),
            current: false,
        }
    }

    fn presentation(
        current_value: Option<&str>,
        clear_action: bool,
        style: ChoicePresentationStyle,
    ) -> super::ChoicePresentation {
        new_presentation(
            "Choice".to_owned(),
            "Pick a value".to_owned(),
            "Footer".to_owned(),
            "Empty".to_owned(),
            vec![item("alpha"), item("beta")],
            current_value.map(str::to_owned),
            clear_action.then(|| SearchPickerClearAction {
                label: "Clear".to_owned(),
                detail: "Remove value".to_owned(),
                current: current_value.is_none(),
            }),
            style,
            "Custom".to_owned(),
            "Use ".to_owned(),
            String::new(),
        )
    }

    #[test]
    fn current_choice_exposes_stable_search_and_fill_values() {
        let item = ChoicePickerItem {
            label: "Readable label".to_owned(),
            detail: "Detail".to_owned(),
            value: "wire-value".to_owned(),
            search_text: "readable detail wire-value".to_owned(),
            current: true,
        };

        assert_eq!(item.search_picker_key(), "wire-value");
        assert_eq!(item.search_picker_fill_value(), "wire-value");
        assert_eq!(item.search_picker_prefix().as_deref(), Some("✓ "));
    }

    #[test]
    fn presentation_marks_and_selects_the_committed_row() {
        let presentation = presentation(
            Some("beta"),
            true,
            ChoicePresentationStyle::SearchableSelect,
        );

        assert!(presentation.items[1].current);
        assert!(!presentation.items[0].current);
        assert_eq!(
            presentation.selected_item().map(|item| item.value.as_str()),
            Some("beta")
        );
    }

    #[test]
    fn query_prefers_the_matching_item_over_special_rows() {
        let mut presentation = presentation(None, true, ChoicePresentationStyle::Searchable);

        assert_eq!(
            reduce(
                &mut presentation,
                ChoicePresentationAction::Paste("beta".to_owned()),
            ),
            ChoicePresentationEffect::KeepOpen
        );
        assert_eq!(presentation.result_count(), 1);
        assert_eq!(
            presentation.selected_item().map(|item| item.value.as_str()),
            Some("beta")
        );
    }

    #[test]
    fn accept_maps_clear_custom_and_item_rows_to_typed_selection() {
        let mut clear = presentation(None, true, ChoicePresentationStyle::Searchable);
        assert_eq!(
            reduce(&mut clear, ChoicePresentationAction::Accept),
            ChoicePresentationEffect::Commit(ChoiceSelection::Clear)
        );

        let mut custom = presentation(None, false, ChoicePresentationStyle::Searchable);
        let _ = reduce(
            &mut custom,
            ChoicePresentationAction::Paste("standalone".to_owned()),
        );
        assert_eq!(
            reduce(&mut custom, ChoicePresentationAction::Accept),
            ChoicePresentationEffect::Commit(ChoiceSelection::Custom {
                raw: "standalone".to_owned(),
            })
        );

        let mut item = presentation(None, false, ChoicePresentationStyle::SearchableSelect);
        assert!(item.select_item_where(|candidate| candidate.value == "alpha"));
        assert_eq!(
            reduce(&mut item, ChoicePresentationAction::Accept),
            ChoicePresentationEffect::Commit(ChoiceSelection::Item {
                value: "alpha".to_owned(),
            })
        );
    }

    #[test]
    fn escape_closes_and_multi_select_space_toggles_the_selected_item() {
        let mut presentation = presentation(None, false, ChoicePresentationStyle::SearchableSelect);
        assert_eq!(
            reduce(
                &mut presentation,
                ChoicePresentationAction::Input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            ),
            ChoicePresentationEffect::Close
        );

        presentation.config.selection_mode = SearchPickerSelectionMode::Multiple;
        presentation.focus = SearchPickerFocus::Results;
        assert_eq!(
            reduce(
                &mut presentation,
                ChoicePresentationAction::Input(KeyEvent::new(
                    KeyCode::Char(' '),
                    KeyModifiers::NONE,
                )),
            ),
            ChoicePresentationEffect::KeepOpen
        );
        assert_eq!(presentation.checked_keys, vec!["alpha".to_owned()]);
    }
}
