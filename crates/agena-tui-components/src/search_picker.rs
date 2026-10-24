//! Unified searchable selection surface for local catalogs, externally loaded
//! pages, editable values, multi-selection, pinned actions, and responsive
//! preview panes. Results are presented as terminal-height-aware pages so the
//! visible page is stable instead of scrolling around the active row.
//!
//! The picker keeps canonical items separate from lightweight match indices.
//! Filtering therefore does not clone domain items, and rendering only builds
//! the visible window instead of allocating one Ratatui row per result.

use std::{borrow::Cow, cell::Cell, path::PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    Editor, EditorPanelSpec, FramedSurfaceSpec, InputDialogAction, NavigationAction, SurfaceMode,
    TextPanelSpec, VerticalSectionSize, WorkbenchTextSection, adaptive_detail_split,
    editor_input_panel_height, framed_sections_target_height, input_dialog_action,
    optional_overlay_text_height, render_editor_panel, render_text_panel, search_navigation_action,
    split_vertical_sections, text::truncate_display_text, theme,
    workbench::WorkbenchTextSection as PreviewSection,
};

mod render;
pub use render::*;
use render::{find_label_ranges, normalize_search_text, score_entry};

/// The role played by the text editor at the top of a picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchPickerInputMode {
    Hidden,
    Search,
    EditableValue,
    SearchWithCustomValue,
}

impl SearchPickerInputMode {
    pub fn is_visible(self) -> bool {
        !matches!(self, Self::Hidden)
    }

    pub fn allows_custom_value(self) -> bool {
        matches!(self, Self::EditableValue | Self::SearchWithCustomValue)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Search mode of the search picker.
pub enum SearchPickerSearchMode {
    None,
    LocalRanked,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Selection mode of the search picker.
pub enum SearchPickerSelectionMode {
    Single,
    Multiple,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Focus of the search picker.
pub enum SearchPickerFocus {
    Input,
    Results,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Preview mode of the search picker.
pub enum SearchPickerPreviewMode {
    Hidden,
    Responsive {
        min_total_width: u16,
        left_min_width: u16,
        right_min_width: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Phase of the search picker.
pub enum SearchPickerPhase {
    Idle,
    Searching { keep_results: bool },
    Appending,
    Ready { complete: bool },
    Error { keep_results: bool },
}

impl SearchPickerPhase {
    pub fn is_loading(self) -> bool {
        matches!(self, Self::Searching { .. } | Self::Appending)
    }

    fn hides_results(self) -> bool {
        matches!(
            self,
            Self::Searching {
                keep_results: false
            }
        )
    }
}

#[derive(Debug, Clone)]
/// Configuration of the search picker.
pub struct SearchPickerConfig {
    pub input_mode: SearchPickerInputMode,
    pub search_mode: SearchPickerSearchMode,
    pub selection_mode: SearchPickerSelectionMode,
    pub preview_mode: SearchPickerPreviewMode,
    pub fill_selected_into_input: bool,
    /// Path-oriented pickers opt into a soft-wrapped input panel so long
    /// filesystem paths stay visible instead of being horizontally clipped.
    pub wrap_input: bool,
}

impl SearchPickerConfig {
    pub fn searchable() -> Self {
        Self {
            input_mode: SearchPickerInputMode::Search,
            search_mode: SearchPickerSearchMode::LocalRanked,
            selection_mode: SearchPickerSelectionMode::Single,
            preview_mode: SearchPickerPreviewMode::Hidden,
            fill_selected_into_input: false,
            wrap_input: false,
        }
    }

    pub fn select_only() -> Self {
        Self {
            input_mode: SearchPickerInputMode::Hidden,
            search_mode: SearchPickerSearchMode::None,
            ..Self::searchable()
        }
    }
}

/// Input type of the search picker.
pub trait SearchPickerInput: Clone {
    fn text(&self) -> &str;
    fn set_text(&mut self, text: String);
    fn handle_line_input_key(&mut self, key: KeyEvent);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Result of search picker input.
pub enum SearchPickerInputResult {
    Close,
    Navigated,
    Edited { changed: bool },
}

#[derive(Debug, Clone)]
/// Clear action of the search picker.
pub struct SearchPickerClearAction {
    pub label: String,
    pub detail: String,
    pub current: bool,
}

#[derive(Debug, Clone, Default)]
/// Marker type for a search picker without custom values.
pub struct SearchPickerNoCustom;

/// Item type of the search picker.
pub trait SearchPickerItem: Clone {
    /// A stable key used to preserve the selection across filtering and reloads.
    fn search_picker_key(&self) -> Cow<'_, str>;
    fn search_picker_label(&self) -> Cow<'_, str>;
    fn search_picker_detail(&self) -> Option<Cow<'_, str>>;

    /// Non-searchable row chrome displayed before the primary label, such as
    /// a history sequence number or category sigil.
    fn search_picker_prefix(&self) -> Option<Cow<'_, str>> {
        None
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        self.search_picker_label()
    }

    /// Text indexed once when the catalog changes. The default gives the
    /// primary label the strongest weight and includes the detail as fallback.
    fn search_picker_search_text(&self) -> Cow<'_, str> {
        match self.search_picker_detail() {
            Some(detail) => Cow::Owned(format!("{} {}", self.search_picker_label(), detail)),
            None => self.search_picker_label(),
        }
    }

    fn search_picker_label_style(&self) -> Style {
        Style::default()
    }

    fn search_picker_detail_style(&self) -> Style {
        theme::muted_style()
    }

    fn search_picker_prefix_style(&self) -> Style {
        theme::muted_style()
    }

    fn search_picker_disabled_reason(&self) -> Option<Cow<'_, str>> {
        None
    }

    /// Pinned actions such as "create new" remain available for every query.
    fn search_picker_always_visible(&self) -> bool {
        false
    }
}

/// Custom value type of the search picker.
pub trait SearchPickerCustomValue<TContext>: Clone {
    fn search_picker_from_input(input: &str, context: &TContext) -> Option<Self>;
    fn search_picker_label(&self, context: &TContext) -> Cow<'_, str>;
    fn search_picker_detail(&self, context: &TContext) -> Option<Cow<'_, str>>;
    fn search_picker_input_text(&self) -> Cow<'_, str>;

    fn search_picker_label_style(&self) -> Style {
        Style::default().fg(theme::accent_color())
    }

    fn search_picker_detail_style(&self) -> Style {
        theme::muted_style()
    }
}

impl<TContext> SearchPickerCustomValue<TContext> for SearchPickerNoCustom {
    fn search_picker_from_input(_: &str, _: &TContext) -> Option<Self> {
        None
    }

    fn search_picker_label(&self, _: &TContext) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn search_picker_detail(&self, _: &TContext) -> Option<Cow<'_, str>> {
        None
    }

    fn search_picker_input_text(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
}

impl SearchPickerItem for PathBuf {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(self.display().to_string())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Owned(self.display().to_string())
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        None
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Owned(self.display().to_string())
    }
}

#[derive(Debug, Clone)]
/// Selection of the search picker.
pub enum SearchPickerSelection<TItem, TCustom> {
    Clear(SearchPickerClearAction),
    Custom(TCustom),
    Item(TItem),
}

#[derive(Debug, Clone)]
struct SearchIndexEntry {
    key: String,
    normalized_label: String,
    normalized_document: String,
    always_visible: bool,
}

#[derive(Debug, Clone)]
struct SearchMatch {
    item_index: usize,
    score: i64,
    label_ranges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone)]
/// The search picker widget.
pub struct SearchPicker<TItem, TCustom, TMeta, TInput> {
    pub title: String,
    pub prompt: String,
    pub footer: String,
    pub empty_message: String,
    pub error_message: Option<String>,
    pub input: TInput,
    /// Canonical catalog or the current remote page. Filtered results are
    /// represented by `matches`, so items are never cloned per keystroke.
    pub items: Vec<TItem>,
    pub selected: usize,
    pub phase: SearchPickerPhase,
    pub config: SearchPickerConfig,
    pub clear_action: Option<SearchPickerClearAction>,
    pub meta: TMeta,
    pub request_generation: u64,
    pub preview_scroll: u16,
    pub checked_keys: Vec<String>,
    pub focus: SearchPickerFocus,
    index: Vec<SearchIndexEntry>,
    matches: Vec<SearchMatch>,
    selected_key: Option<String>,
    results_query: String,
    custom_duplicates_item: bool,
    visible_page_size: Cell<usize>,
    custom: std::marker::PhantomData<TCustom>,
}

impl<TItem, TCustom, TMeta, TInput> SearchPicker<TItem, TCustom, TMeta, TInput>
where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    TInput: SearchPickerInput,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        title: String,
        prompt: String,
        footer: String,
        empty_message: String,
        input: TInput,
        config: SearchPickerConfig,
        clear_action: Option<SearchPickerClearAction>,
        meta: TMeta,
    ) -> Self {
        let results_query = input.text().to_string();
        let focus = if config.input_mode.is_visible() {
            SearchPickerFocus::Input
        } else {
            SearchPickerFocus::Results
        };
        Self {
            title,
            prompt,
            footer,
            empty_message,
            error_message: None,
            input,
            items: Vec::new(),
            selected: 0,
            phase: SearchPickerPhase::Idle,
            config,
            clear_action,
            meta,
            request_generation: 0,
            preview_scroll: 0,
            checked_keys: Vec::new(),
            focus,
            index: Vec::new(),
            matches: Vec::new(),
            selected_key: None,
            results_query,
            custom_duplicates_item: false,
            visible_page_size: Cell::new(1),
            custom: std::marker::PhantomData,
        }
    }

    pub fn replace_items(&mut self, items: Vec<TItem>) {
        let preserve_selection = self.input.text() == self.results_query;
        let selected_key = preserve_selection
            .then(|| {
                self.selected_item()
                    .map(|item| item.search_picker_key().into_owned())
            })
            .flatten();
        self.items = items;
        self.index = self
            .items
            .iter()
            .map(|item| SearchIndexEntry {
                key: item.search_picker_key().into_owned(),
                normalized_label: normalize_search_text(item.search_picker_label().as_ref()),
                normalized_document: normalize_search_text(
                    item.search_picker_search_text().as_ref(),
                ),
                always_visible: item.search_picker_always_visible(),
            })
            .collect();
        self.selected_key = selected_key;
        self.refresh_results_with_selection(preserve_selection);
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.phase = if loading {
            SearchPickerPhase::Searching {
                keep_results: !self.matches.is_empty(),
            }
        } else {
            SearchPickerPhase::Ready { complete: true }
        };
    }

    pub fn is_loading(&self) -> bool {
        self.phase.is_loading()
    }

    pub fn set_error(&mut self, message: impl Into<String>, keep_results: bool) {
        self.error_message = Some(message.into());
        self.phase = SearchPickerPhase::Error { keep_results };
    }

    pub fn clear_error(&mut self) {
        self.error_message = None;
        if matches!(self.phase, SearchPickerPhase::Error { .. }) {
            self.phase = SearchPickerPhase::Ready { complete: true };
        }
    }

    pub fn begin_external_search(&mut self) -> u64 {
        self.request_generation = self.request_generation.wrapping_add(1);
        self.phase = SearchPickerPhase::Searching {
            keep_results: !self.matches.is_empty(),
        };
        self.request_generation
    }

    pub fn begin_append(&mut self) {
        self.phase = SearchPickerPhase::Appending;
    }

    pub fn accepts_generation(&self, generation: u64) -> bool {
        generation == self.request_generation
    }

    pub fn refresh_results(&mut self) {
        let preserve_selection = self.input.text() == self.results_query;
        self.refresh_results_with_selection(preserve_selection);
    }

    fn refresh_results_with_selection(&mut self, preserve_selection: bool) {
        let selected_key = preserve_selection
            .then(|| {
                self.selected_key.clone().or_else(|| {
                    self.selected_item()
                        .map(|item| item.search_picker_key().into_owned())
                })
            })
            .flatten();
        let query = normalize_search_text(self.input.text().trim());
        self.matches = match self.config.search_mode {
            SearchPickerSearchMode::LocalRanked if !query.is_empty() => {
                let tokens = query.split_whitespace().collect::<Vec<_>>();
                let mut matches = self
                    .index
                    .iter()
                    .enumerate()
                    .filter_map(|(item_index, entry)| {
                        score_entry(entry, tokens.as_slice()).map(|score| SearchMatch {
                            item_index,
                            score,
                            label_ranges: find_label_ranges(
                                self.items[item_index].search_picker_label().as_ref(),
                                tokens.as_slice(),
                            ),
                        })
                    })
                    .collect::<Vec<_>>();
                matches.sort_by(|left, right| {
                    right
                        .score
                        .cmp(&left.score)
                        .then_with(|| left.item_index.cmp(&right.item_index))
                });
                matches
            }
            _ => self
                .items
                .iter()
                .enumerate()
                .map(|(item_index, _)| SearchMatch {
                    item_index,
                    score: 0,
                    label_ranges: Vec::new(),
                })
                .collect(),
        };
        self.custom_duplicates_item = self.raw_custom_value().is_some_and(|custom| {
            let input_text = custom.search_picker_input_text();
            self.matches.iter().any(|matched| {
                self.items[matched.item_index]
                    .search_picker_fill_value()
                    .as_ref()
                    == input_text.as_ref()
            })
        });
        if preserve_selection {
            self.restore_selection(selected_key.as_deref());
        } else {
            self.selected = 0;
            self.clamp_selection();
            self.selected_key = self
                .selected_item()
                .map(|item| item.search_picker_key().into_owned());
            self.preview_scroll = 0;
        }
        self.results_query = self.input.text().to_string();
        if !self.phase.is_loading() && !matches!(self.phase, SearchPickerPhase::Error { .. }) {
            self.phase = SearchPickerPhase::Ready { complete: true };
        }
    }

    pub fn append_items(&mut self, items: Vec<TItem>) {
        let first_new_item = self.items.len();
        self.items.extend(items);
        self.index.extend(
            self.items[first_new_item..]
                .iter()
                .map(|item| SearchIndexEntry {
                    key: item.search_picker_key().into_owned(),
                    normalized_label: normalize_search_text(item.search_picker_label().as_ref()),
                    normalized_document: normalize_search_text(
                        item.search_picker_search_text().as_ref(),
                    ),
                    always_visible: item.search_picker_always_visible(),
                }),
        );
        self.refresh_results();
    }

    fn restore_selection(&mut self, key: Option<&str>) {
        let special = self.special_row_count();
        if let Some(key) = key
            && let Some(index) = self
                .matches
                .iter()
                .position(|matched| self.index[matched.item_index].key == key)
        {
            self.selected = special + index;
            self.selected_key = Some(key.to_string());
            return;
        }
        self.clamp_selection();
        self.selected_key = self
            .selected_item()
            .map(|item| item.search_picker_key().into_owned());
    }

    fn raw_custom_value(&self) -> Option<TCustom> {
        self.config
            .input_mode
            .allows_custom_value()
            .then(|| TCustom::search_picker_from_input(self.input.text(), &self.meta))
            .flatten()
    }

    fn custom_value(&self) -> Option<TCustom> {
        (!self.custom_duplicates_item)
            .then(|| self.raw_custom_value())
            .flatten()
    }

    fn special_row_count(&self) -> usize {
        usize::from(self.clear_action.is_some()) + usize::from(self.custom_value().is_some())
    }

    pub fn row_count(&self) -> usize {
        self.special_row_count() + self.matches.len()
    }

    pub fn result_count(&self) -> usize {
        self.matches.len()
    }

    pub fn query_changed_since_results(&self) -> bool {
        self.input.text() != self.results_query
    }

    pub fn is_empty(&self) -> bool {
        self.row_count() == 0
    }

    fn selected_match(&self) -> Option<&SearchMatch> {
        let result_index = self.selected.checked_sub(self.special_row_count())?;
        self.matches.get(result_index)
    }

    pub fn selected_item(&self) -> Option<&TItem> {
        self.selected_match()
            .and_then(|matched| self.items.get(matched.item_index))
    }

    pub fn selected_item_mut(&mut self) -> Option<&mut TItem> {
        let item_index = self.selected_match()?.item_index;
        self.items.get_mut(item_index)
    }

    pub fn selected_row(&self) -> Option<SearchPickerSelection<TItem, TCustom>> {
        let mut index = self.selected;
        if let Some(clear) = self.clear_action.as_ref() {
            if index == 0 {
                return Some(SearchPickerSelection::Clear(clear.clone()));
            }
            index -= 1;
        }
        if let Some(custom) = self.custom_value() {
            if index == 0 {
                return Some(SearchPickerSelection::Custom(custom));
            }
            index -= 1;
        }
        self.matches
            .get(index)
            .and_then(|matched| self.items.get(matched.item_index))
            .filter(|item| item.search_picker_disabled_reason().is_none())
            .cloned()
            .map(SearchPickerSelection::Item)
    }

    pub fn result_items(&self) -> impl Iterator<Item = &TItem> {
        self.matches
            .iter()
            .filter_map(|matched| self.items.get(matched.item_index))
    }

    pub fn select_item_where(&mut self, mut predicate: impl FnMut(&TItem) -> bool) -> bool {
        let Some(result_index) = self.matches.iter().position(|matched| {
            self.items
                .get(matched.item_index)
                .is_some_and(&mut predicate)
        }) else {
            return false;
        };
        self.selected = self.special_row_count() + result_index;
        self.selected_key = self
            .selected_item()
            .map(|item| item.search_picker_key().into_owned());
        true
    }

    pub fn clamp_selection(&mut self) {
        self.selected = if self.row_count() == 0 {
            0
        } else {
            self.selected.min(self.row_count() - 1)
        };
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.focus = SearchPickerFocus::Results;
        let count = self.row_count();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, count as isize - 1) as usize;
        self.selected_key = self
            .selected_item()
            .map(|item| item.search_picker_key().into_owned());
        self.preview_scroll = 0;
    }

    pub fn page_size(&self) -> usize {
        self.visible_page_size.get().max(1)
    }

    pub fn page_count(&self) -> usize {
        self.row_count().max(1).div_ceil(self.page_size())
    }

    pub fn current_page(&self) -> usize {
        if self.row_count() == 0 {
            0
        } else {
            (self.selected / self.page_size()).min(self.page_count().saturating_sub(1))
        }
    }

    /// Let a feature-specific renderer use its actual number of visible rows.
    pub fn set_visible_page_size(&self, page_size: usize) {
        self.visible_page_size.set(page_size.max(1));
    }

    pub fn visible_page_bounds(&self) -> (usize, usize) {
        let start = self.current_page().saturating_mul(self.page_size());
        (start, (start + self.page_size()).min(self.row_count()))
    }

    pub fn move_selection_page(&mut self, delta: isize) {
        let page_size = self.page_size();
        let page = self.current_page();
        let target_page =
            (page as isize + delta).clamp(0, self.page_count().saturating_sub(1) as isize) as usize;
        let row_in_page = self.selected % page_size;
        let target = target_page
            .saturating_mul(page_size)
            .saturating_add(row_in_page)
            .min(self.row_count().saturating_sub(1));
        self.move_selection(target as isize - self.selected as isize);
    }

    pub fn move_selection_home(&mut self) {
        self.focus = SearchPickerFocus::Results;
        self.move_selection(-(self.selected as isize));
    }

    pub fn move_selection_end(&mut self) {
        self.focus = SearchPickerFocus::Results;
        let target = self.row_count().saturating_sub(1);
        self.move_selection(target as isize - self.selected as isize);
    }

    pub fn fill_input_from_selected(&mut self) -> bool {
        if !self.config.input_mode.is_visible() || !self.config.fill_selected_into_input {
            return false;
        }
        let Some(selection) = self.selected_row() else {
            return false;
        };
        let value = match selection {
            SearchPickerSelection::Clear(_) => String::new(),
            SearchPickerSelection::Custom(custom) => custom.search_picker_input_text().into_owned(),
            SearchPickerSelection::Item(item) => item.search_picker_fill_value().into_owned(),
        };
        if self.input.text() == value {
            return false;
        }
        self.input.set_text(value);
        self.focus = SearchPickerFocus::Input;
        true
    }

    pub fn toggle_selected(&mut self) -> bool {
        if self.config.selection_mode != SearchPickerSelectionMode::Multiple {
            return false;
        }
        let Some(key) = self
            .selected_item()
            .filter(|item| item.search_picker_disabled_reason().is_none())
            .map(|item| item.search_picker_key().into_owned())
        else {
            return false;
        };
        if let Some(index) = self.checked_keys.iter().position(|checked| checked == &key) {
            self.checked_keys.remove(index);
        } else {
            self.checked_keys.push(key);
        }
        true
    }

    /// Select visible rows using the same identity bookkeeping as keyboard navigation.
    pub fn handle_pointer(
        &mut self,
        action: &crate::pointer::PointerAction,
        activate: bool,
    ) -> Option<KeyCode> {
        use crate::pointer::PointerAction;
        match *action {
            PointerAction::PickerInput => self.focus = SearchPickerFocus::Input,
            PointerAction::PickerResults => self.focus = SearchPickerFocus::Results,
            PointerAction::PickerRow(index) => {
                if index >= self.row_count() {
                    return None;
                }
                self.focus = SearchPickerFocus::Results;
                if activate {
                    self.move_selection(index as isize - self.selected as isize);
                    if self.config.selection_mode == SearchPickerSelectionMode::Multiple {
                        self.toggle_selected();
                    } else {
                        return Some(KeyCode::Enter);
                    }
                }
            }
            PointerAction::PickerPreview(delta) => {
                self.preview_scroll = self.preview_scroll.saturating_add_signed(delta);
            }
            _ => {}
        }
        None
    }

    pub fn handle_input_key(&mut self, key: KeyEvent) -> SearchPickerInputResult {
        if input_dialog_action(key, false) == Some(InputDialogAction::Close) {
            return SearchPickerInputResult::Close;
        }
        if !self.config.input_mode.is_visible() {
            if self.handle_navigation_action(search_navigation_action(key)) {
                return SearchPickerInputResult::Navigated;
            }
            return SearchPickerInputResult::Edited { changed: false };
        }

        if self.focus == SearchPickerFocus::Input {
            if key.code == KeyCode::Down && key.modifiers.is_empty() && !self.is_empty() {
                self.focus = SearchPickerFocus::Results;
                return SearchPickerInputResult::Navigated;
            }
        } else if let Some(action) = search_navigation_action(key) {
            if action == NavigationAction::Up && self.selected == 0 {
                self.focus = SearchPickerFocus::Input;
            } else {
                self.handle_navigation_action(Some(action));
            }
            return SearchPickerInputResult::Navigated;
        } else if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            return SearchPickerInputResult::Edited { changed: false };
        } else {
            self.focus = SearchPickerFocus::Input;
        }

        let before = self.input.text().to_string();
        self.input.handle_line_input_key(key);
        let changed = self.input.text() != before;
        if changed {
            self.selected = 0;
            self.selected_key = None;
            self.preview_scroll = 0;
        }
        if changed && self.config.search_mode == SearchPickerSearchMode::LocalRanked {
            self.refresh_results();
        } else if changed {
            self.request_generation = self.request_generation.wrapping_add(1);
        }
        SearchPickerInputResult::Edited { changed }
    }

    fn handle_navigation_action(&mut self, action: Option<NavigationAction>) -> bool {
        match action {
            Some(NavigationAction::PageUp) => self.move_selection_page(-1),
            Some(NavigationAction::PageDown) => self.move_selection_page(1),
            Some(NavigationAction::Home) => self.move_selection_home(),
            Some(NavigationAction::End) => self.move_selection_end(),
            Some(NavigationAction::Up) => self.move_selection(-1),
            Some(NavigationAction::Down) => self.move_selection(1),
            None => return false,
        }
        true
    }
}
