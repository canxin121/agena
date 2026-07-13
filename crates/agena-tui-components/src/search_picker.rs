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
    optional_overlay_text_height, render_editor_panel, render_framed_surface, render_text_panel,
    search_navigation_action, split_vertical_sections, text::truncate_display_text, theme,
    workbench::WorkbenchTextSection as PreviewSection,
};

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
pub enum SearchPickerSearchMode {
    None,
    LocalRanked,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchPickerSelectionMode {
    Single,
    Multiple,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchPickerFocus {
    Input,
    Results,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchPickerPreviewMode {
    Hidden,
    Responsive {
        min_total_width: u16,
        left_min_width: u16,
        right_min_width: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub struct SearchPickerConfig {
    pub input_mode: SearchPickerInputMode,
    pub search_mode: SearchPickerSearchMode,
    pub selection_mode: SearchPickerSelectionMode,
    pub preview_mode: SearchPickerPreviewMode,
    pub fill_selected_into_input: bool,
}

impl SearchPickerConfig {
    pub fn searchable() -> Self {
        Self {
            input_mode: SearchPickerInputMode::Search,
            search_mode: SearchPickerSearchMode::LocalRanked,
            selection_mode: SearchPickerSelectionMode::Single,
            preview_mode: SearchPickerPreviewMode::Hidden,
            fill_selected_into_input: false,
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

pub trait SearchPickerInput: Clone {
    fn text(&self) -> &str;
    fn set_text(&mut self, text: String);
    fn handle_line_input_key(&mut self, key: KeyEvent);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchPickerInputResult {
    Close,
    Navigated,
    Edited { changed: bool },
}

#[derive(Debug, Clone)]
pub struct SearchPickerClearAction {
    pub label: String,
    pub detail: String,
    pub current: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SearchPickerNoCustom;

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

    fn set_visible_page_size(&self, page_size: usize) {
        self.visible_page_size.set(page_size.max(1));
    }

    fn visible_page_bounds(&self) -> (usize, usize) {
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
        self.selected = 0;
        self.preview_scroll = 0;
    }

    pub fn move_selection_end(&mut self) {
        self.focus = SearchPickerFocus::Results;
        self.selected = self.row_count().saturating_sub(1);
        self.preview_scroll = 0;
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

#[derive(Debug, Clone)]
pub enum SearchPickerViewState<'a, TItem> {
    Loading { message: &'a str },
    Empty { message: &'a str },
    Error { message: &'a str },
    Selected(&'a TItem),
}

pub struct SearchPickerDialogSpec<'a> {
    loading_message: Cow<'a, str>,
    results_title: Cow<'a, str>,
    preview_title: Cow<'a, str>,
    highlight_style: Style,
    highlight_symbol: Cow<'a, str>,
    checked_symbol: Cow<'a, str>,
    unchecked_symbol: Cow<'a, str>,
    search_label: Cow<'a, str>,
}

impl<'a> SearchPickerDialogSpec<'a> {
    pub fn new(loading_message: Cow<'a, str>, results_title: Cow<'a, str>) -> Self {
        Self {
            loading_message,
            results_title,
            preview_title: Cow::Borrowed("Preview"),
            highlight_style: theme::selection_style(),
            highlight_symbol: Cow::Borrowed(">> "),
            checked_symbol: Cow::Borrowed("[x] "),
            unchecked_symbol: Cow::Borrowed("[ ] "),
            search_label: Cow::Borrowed(""),
        }
    }

    pub fn with_search_label(mut self, label: Cow<'a, str>) -> Self {
        self.search_label = label;
        self
    }
}

pub fn render_search_picker_dialog<TItem, TCustom, TMeta, F>(
    frame: &mut Frame,
    area: Rect,
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    spec: &SearchPickerDialogSpec<'_>,
    normalize_text: F,
) where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
{
    render_search_picker_dialog_with_preview(frame, area, picker, spec, normalize_text, |_| {
        Vec::new()
    });
}

pub fn render_search_picker_dialog_with_preview<TItem, TCustom, TMeta, F, P>(
    frame: &mut Frame,
    area: Rect,
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    spec: &SearchPickerDialogSpec<'_>,
    normalize_text: F,
    build_preview: P,
) where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
    P: Fn(SearchPickerViewState<'_, TItem>) -> Vec<WorkbenchTextSection<'static>>,
{
    let area = search_picker_dialog_area(area);
    let content_width = area.width.saturating_sub(2);
    let prompt_height = optional_overlay_text_height(&picker.prompt, content_width, 1, 2);
    let input_height = if picker.config.input_mode.is_visible() {
        editor_input_panel_height(&picker.input, false)
    } else {
        0
    };
    let footer = picker_footer(picker, spec, &normalize_text);
    let footer_height = optional_overlay_text_height(&footer, content_width, 1, 2);
    let list_height = 6;
    let mut sections = Vec::new();
    if prompt_height > 0 {
        sections.push(VerticalSectionSize::Fixed(prompt_height));
    }
    if input_height > 0 {
        sections.push(VerticalSectionSize::Fixed(input_height));
    }
    sections.push(VerticalSectionSize::Flexible(list_height));
    if footer_height > 0 {
        sections.push(VerticalSectionSize::Fixed(footer_height));
    }
    let framed = render_framed_surface(
        frame,
        area,
        SurfaceMode::Route,
        &FramedSurfaceSpec {
            title: picker_title(picker, &normalize_text).into(),
            target_width: area.width,
            target_height: framed_sections_target_height(&sections),
        },
    );
    let rows = split_vertical_sections(framed.inner, &sections);
    let mut row_index = 0;
    if prompt_height > 0 {
        frame.render_widget(
            Paragraph::new(normalize_text(&picker.prompt)).wrap(Wrap { trim: false }),
            rows[row_index],
        );
        row_index += 1;
    }
    let input_result = if input_height > 0 {
        let result = render_editor_panel(
            frame,
            rows[row_index],
            &EditorPanelSpec {
                title: (!spec.search_label.trim().is_empty()).then(|| spec.search_label.clone()),
                borders: Borders::ALL,
            },
            &picker.input,
        );
        row_index += 1;
        Some(result)
    } else {
        None
    };

    let panels_area = rows[row_index];
    row_index += 1;
    let show_preview = match picker.config.preview_mode {
        SearchPickerPreviewMode::Hidden => false,
        SearchPickerPreviewMode::Responsive {
            min_total_width, ..
        } => panels_area.width >= min_total_width,
    };
    let (list_area, preview_area) = if show_preview {
        let SearchPickerPreviewMode::Responsive {
            left_min_width,
            right_min_width,
            ..
        } = picker.config.preview_mode
        else {
            unreachable!()
        };
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(adaptive_detail_split(
                panels_area.width,
                left_min_width,
                right_min_width,
            ))
            .split(panels_area);
        (split[0], Some(split[1]))
    } else {
        (panels_area, None)
    };

    render_picker_results(frame, list_area, picker, spec, &normalize_text);
    if let Some(preview_area) = preview_area {
        let view_state = picker_view_state(picker, spec.loading_message.as_ref());
        let sections = build_preview(view_state);
        render_preview_sections(
            frame,
            preview_area,
            sections,
            picker.preview_scroll,
            spec.preview_title.clone(),
        );
    }
    if footer_height > 0 {
        frame.render_widget(
            Paragraph::new(footer)
                .style(theme::muted_style())
                .wrap(Wrap { trim: false }),
            rows[row_index],
        );
    }
    if let Some(result) = input_result
        && picker.focus == SearchPickerFocus::Input
    {
        frame.set_cursor_position(result.cursor);
    }
}

/// Canonical centered window for every searchable selection surface.
/// The window scales with the terminal while retaining enough surrounding
/// context to read as a modal, and stops growing on very large terminals.
/// Tiny terminals use the complete area rather than sacrificing usability.
pub fn search_picker_dialog_area(area: Rect) -> Rect {
    if area.width < 48 || area.height < 10 {
        return area;
    }
    let target_width = ((u32::from(area.width) * 88) / 100) as u16;
    let target_height = ((u32::from(area.height) * 82) / 100) as u16;
    SurfaceMode::Overlay.outer_rect(area, target_width.min(126), target_height.min(34))
}

fn picker_view_state<'a, TItem, TCustom, TMeta>(
    picker: &'a SearchPicker<TItem, TCustom, TMeta, Editor>,
    loading_message: &'a str,
) -> SearchPickerViewState<'a, TItem>
where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
{
    if picker.phase.hides_results() {
        SearchPickerViewState::Loading {
            message: loading_message,
        }
    } else if let SearchPickerPhase::Error {
        keep_results: false,
    } = picker.phase
    {
        SearchPickerViewState::Error {
            message: picker.error_message.as_deref().unwrap_or("Search failed"),
        }
    } else if let Some(item) = picker.selected_item() {
        SearchPickerViewState::Selected(item)
    } else {
        SearchPickerViewState::Empty {
            message: &picker.empty_message,
        }
    }
}

fn render_picker_results<TItem, TCustom, TMeta, F>(
    frame: &mut Frame,
    area: Rect,
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    spec: &SearchPickerDialogSpec<'_>,
    normalize_text: &F,
) where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
{
    let page_size = area.height.saturating_sub(2).max(1) as usize;
    picker.set_visible_page_size(page_size);
    let block_title = format!(
        " {} · {} · Page {}/{} ",
        normalize_text(spec.results_title.as_ref()),
        picker.result_count(),
        picker.current_page() + 1,
        picker.page_count(),
    );
    let block = Block::default().borders(Borders::ALL).title(block_title);
    if picker.phase.hides_results() {
        frame.render_widget(
            Paragraph::new(normalize_text(spec.loading_message.as_ref()))
                .style(theme::muted_style())
                .block(block),
            area,
        );
        return;
    }
    if let SearchPickerPhase::Error {
        keep_results: false,
    } = picker.phase
    {
        frame.render_widget(
            Paragraph::new(normalize_text(
                picker.error_message.as_deref().unwrap_or("Search failed"),
            ))
            .style(Style::default().fg(theme::danger_color()))
            .block(block),
            area,
        );
        return;
    }
    if picker.is_empty() {
        frame.render_widget(
            Paragraph::new(normalize_text(&picker.empty_message))
                .style(theme::muted_style())
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
        return;
    }

    let (start, end) = picker.visible_page_bounds();
    let row_width = area.width.saturating_sub(5).max(1) as usize;
    let list_items = (start..end)
        .map(|row| {
            build_picker_row(
                picker,
                row,
                row_width,
                spec.checked_symbol.as_ref(),
                spec.unchecked_symbol.as_ref(),
                normalize_text,
            )
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(picker.selected.saturating_sub(start)));
    let focused = picker.focus == SearchPickerFocus::Results;
    let highlight_symbol = if focused {
        spec.highlight_symbol.to_string()
    } else {
        " ".repeat(UnicodeWidthStr::width(spec.highlight_symbol.as_ref()))
    };
    let list = List::new(list_items)
        .block(block)
        .highlight_style(if focused {
            spec.highlight_style
        } else {
            Style::default()
        })
        .highlight_symbol(highlight_symbol);
    frame.render_stateful_widget(list, area, &mut state);
}

fn build_picker_row<'a, TItem, TCustom, TMeta, F>(
    picker: &'a SearchPicker<TItem, TCustom, TMeta, Editor>,
    row: usize,
    width: usize,
    checked_symbol: &str,
    unchecked_symbol: &str,
    normalize_text: &F,
) -> ListItem<'static>
where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'b> Fn(&'b str) -> String,
{
    let mut index = row;
    if let Some(clear) = picker.clear_action.as_ref() {
        if index == 0 {
            let label = if clear.current {
                format!("✓ {}", clear.label)
            } else {
                clear.label.clone()
            };
            return row_item(
                normalize_text(&label),
                Some(normalize_text(&clear.detail)),
                Style::default().fg(theme::warning_color()),
                theme::muted_style(),
            );
        }
        index -= 1;
    }
    if let Some(custom) = picker.custom_value() {
        if index == 0 {
            return row_item(
                normalize_text(custom.search_picker_label(&picker.meta).as_ref()),
                custom
                    .search_picker_detail(&picker.meta)
                    .map(|detail| normalize_text(detail.as_ref())),
                custom.search_picker_label_style(),
                custom.search_picker_detail_style(),
            );
        }
        index -= 1;
    }
    let Some(matched) = picker.matches.get(index) else {
        return ListItem::new("");
    };
    let Some(item) = picker.items.get(matched.item_index) else {
        return ListItem::new("");
    };
    let checked_prefix = if picker.config.selection_mode == SearchPickerSelectionMode::Multiple {
        if picker
            .checked_keys
            .iter()
            .any(|key| key == item.search_picker_key().as_ref())
        {
            checked_symbol
        } else {
            unchecked_symbol
        }
    } else {
        ""
    };
    let prefix = item
        .search_picker_prefix()
        .map(|prefix| normalize_text(prefix.as_ref()))
        .unwrap_or_default();
    let leading_width = UnicodeWidthStr::width(checked_prefix)
        .saturating_add(UnicodeWidthStr::width(prefix.as_str()));
    let label = normalize_text(item.search_picker_label().as_ref());
    let label = truncate_display_text(&label, width.saturating_sub(leading_width));
    let mut first_line = Line::from(Vec::<Span<'static>>::new());
    if !checked_prefix.is_empty() {
        first_line.spans.push(Span::raw(checked_prefix.to_string()));
    }
    if !prefix.is_empty() {
        first_line
            .spans
            .push(Span::styled(prefix, item.search_picker_prefix_style()));
    }
    first_line.spans.extend(
        highlighted_line(
            "",
            &label,
            &matched.label_ranges,
            item.search_picker_label_style(),
        )
        .spans,
    );
    let detail = item
        .search_picker_disabled_reason()
        .or_else(|| item.search_picker_detail());
    let detail_style = if item.search_picker_disabled_reason().is_some() {
        Style::default().fg(theme::warning_color())
    } else {
        item.search_picker_detail_style()
    };
    if let Some(detail) = detail {
        let used = UnicodeWidthStr::width(label.as_str()).saturating_add(leading_width);
        let detail = truncate_display_text(
            &normalize_text(detail.as_ref()),
            width.saturating_sub(used).saturating_sub(2),
        );
        if !detail.is_empty() {
            first_line.spans.push(Span::raw("  "));
            first_line.spans.push(Span::styled(detail, detail_style));
        }
    }
    ListItem::new(first_line)
}

fn row_item(
    label: String,
    detail: Option<String>,
    label_style: Style,
    detail_style: Style,
) -> ListItem<'static> {
    let mut spans = vec![Span::styled(label, label_style)];
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(detail, detail_style));
    }
    ListItem::new(Line::from(spans))
}

fn highlighted_line(
    prefix: &str,
    label: &str,
    ranges: &[(usize, usize)],
    base: Style,
) -> Line<'static> {
    let chars = label.chars().collect::<Vec<_>>();
    let mut spans = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix.to_string(), theme::muted_style()));
    }
    let mut cursor = 0usize;
    for &(start, end) in ranges {
        let start = start.min(chars.len()).max(cursor);
        let end = end.min(chars.len()).max(start);
        if start > cursor {
            spans.push(Span::styled(
                chars[cursor..start].iter().collect::<String>(),
                base,
            ));
        }
        if end > start {
            spans.push(Span::styled(
                chars[start..end].iter().collect::<String>(),
                base.fg(theme::accent_color()).add_modifier(Modifier::BOLD),
            ));
        }
        cursor = end;
    }
    if cursor < chars.len() {
        spans.push(Span::styled(
            chars[cursor..].iter().collect::<String>(),
            base,
        ));
    }
    Line::from(spans)
}

fn render_preview_sections(
    frame: &mut Frame,
    area: Rect,
    sections: Vec<PreviewSection<'static>>,
    scroll: u16,
    default_title: Cow<'_, str>,
) {
    if sections.is_empty() {
        let empty = Text::from("No preview available");
        render_text_panel(
            frame,
            area,
            &TextPanelSpec {
                title: Some(default_title),
                body: &empty,
                wrap: true,
                scroll: None,
                alignment: None,
            },
        );
        return;
    }
    let constraints = sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            if index + 1 == sections.len() {
                ratatui::layout::Constraint::Min(section.min_body_height.saturating_add(2))
            } else {
                ratatui::layout::Constraint::Length(section.max_body_height.saturating_add(2))
            }
        })
        .collect::<Vec<_>>();
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    for (index, (section, section_area)) in sections.into_iter().zip(areas.iter()).enumerate() {
        render_text_panel(
            frame,
            *section_area,
            &TextPanelSpec {
                title: Some(section.title),
                body: &section.body,
                wrap: true,
                scroll: (index + 1 == areas.len()).then_some((scroll, 0)),
                alignment: None,
            },
        );
    }
}

fn picker_title<TItem, TCustom, TMeta, F>(
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    normalize_text: &F,
) -> String
where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
{
    let status = match picker.phase {
        SearchPickerPhase::Searching { .. } => "searching…".to_string(),
        SearchPickerPhase::Appending => "loading more…".to_string(),
        SearchPickerPhase::Error { .. } => "error".to_string(),
        _ => format!("{} results", picker.result_count()),
    };
    normalize_text(&format!("{} · {}", picker.title, status))
}

fn picker_footer<TItem, TCustom, TMeta, F>(
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    _spec: &SearchPickerDialogSpec<'_>,
    normalize_text: &F,
) -> String
where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
{
    let mut parts = Vec::new();
    if picker.config.selection_mode == SearchPickerSelectionMode::Multiple {
        parts.push(format!(
            "Space toggle · {} selected",
            picker.checked_keys.len()
        ));
        parts.push("Enter confirm".to_string());
    }
    if picker.config.input_mode.is_visible() {
        parts.push(
            "Search ←/→ cursor · Results ←/→ page · ↓ enter · ↑ first row return".to_string(),
        );
    } else {
        parts.push("←/→ page".to_string());
    }
    if !picker.footer.trim().is_empty() {
        parts.push(normalize_text(&picker.footer));
    } else {
        parts.push("↑/↓ navigate · Enter select · Esc close".to_string());
    }
    parts.join(" · ")
}

fn normalize_search_text(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric()
                || matches!(character, '/' | '\\' | '.' | '-' | '_' | '#' | ':')
            {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn score_entry(entry: &SearchIndexEntry, tokens: &[&str]) -> Option<i64> {
    if entry.always_visible {
        return Some(10_000);
    }
    let mut total = 0i64;
    for token in tokens {
        let label_score = score_field(&entry.normalized_label, token, true);
        let document_score = score_field(&entry.normalized_document, token, false);
        let score = match (label_score, document_score) {
            (Some(label), Some(document)) => label.max(document),
            (Some(label), None) => label,
            (None, Some(document)) => document,
            (None, None) => return None,
        };
        total += score;
    }
    Some(total)
}

fn score_field(field: &str, token: &str, primary: bool) -> Option<i64> {
    let weight = if primary { 1_000 } else { 100 };
    if field == token {
        return Some(weight + 500);
    }
    if field.starts_with(token) {
        return Some(weight + 400 - field.len().saturating_sub(token.len()) as i64);
    }
    if field
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| word.starts_with(token))
    {
        return Some(weight + 300);
    }
    if let Some(position) = field.find(token) {
        return Some(weight + 200 - position.min(100) as i64);
    }
    subsequence_score(field, token).map(|score| weight + score)
}

fn subsequence_score(field: &str, token: &str) -> Option<i64> {
    let mut field_chars = field.chars().enumerate();
    let mut last = None;
    let mut gaps = 0usize;
    for wanted in token.chars() {
        let (position, _) = field_chars.find(|(_, character)| *character == wanted)?;
        if let Some(previous) = last {
            gaps += position.saturating_sub(previous + 1);
        }
        last = Some(position);
    }
    Some(100 - gaps.min(100) as i64)
}

fn find_label_ranges(label: &str, tokens: &[&str]) -> Vec<(usize, usize)> {
    let normalized_chars = label
        .chars()
        .map(|character| character.to_lowercase().collect::<String>())
        .collect::<Vec<_>>();
    let mut ranges = Vec::new();
    for token in tokens {
        let token_chars = token.chars().collect::<Vec<_>>();
        if token_chars.is_empty() {
            continue;
        }
        for start in 0..normalized_chars.len() {
            let mut normalized = String::new();
            let mut end = start;
            while end < normalized_chars.len() && normalized.chars().count() < token_chars.len() {
                normalized.push_str(&normalized_chars[end]);
                end += 1;
            }
            if normalized == *token {
                ranges.push((start, end));
                break;
            }
        }
    }
    ranges.sort_unstable_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.0 <= last.1
        {
            last.1 = last.1.max(range.1);
        } else {
            merged.push(range);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::{
        SearchPicker, SearchPickerConfig, SearchPickerCustomValue, SearchPickerDialogSpec,
        SearchPickerFocus, SearchPickerInput, SearchPickerInputMode, SearchPickerItem,
        SearchPickerNoCustom, SearchPickerPreviewMode, SearchPickerSearchMode,
        SearchPickerSelection, SearchPickerViewState, render_search_picker_dialog,
        render_search_picker_dialog_with_preview, search_picker_dialog_area,
    };
    use crate::{Editor, WorkbenchTextSection};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, text::Text};
    use std::borrow::Cow;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Debug)]
    struct Input(String);

    impl SearchPickerInput for Input {
        fn text(&self) -> &str {
            &self.0
        }

        fn set_text(&mut self, text: String) {
            self.0 = text;
        }

        fn handle_line_input_key(&mut self, _: KeyEvent) {}
    }

    #[derive(Clone, Debug)]
    struct Item {
        key: &'static str,
        label: &'static str,
        detail: &'static str,
        pinned: bool,
    }

    impl SearchPickerItem for Item {
        fn search_picker_key(&self) -> Cow<'_, str> {
            Cow::Borrowed(self.key)
        }

        fn search_picker_label(&self) -> Cow<'_, str> {
            Cow::Borrowed(self.label)
        }

        fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.detail))
        }

        fn search_picker_always_visible(&self) -> bool {
            self.pinned
        }
    }

    #[derive(Clone, Debug)]
    struct Custom(String);

    impl SearchPickerCustomValue<()> for Custom {
        fn search_picker_from_input(input: &str, _: &()) -> Option<Self> {
            let input = input.trim();
            (!input.is_empty()).then(|| Self(input.to_string()))
        }

        fn search_picker_label(&self, _: &()) -> Cow<'_, str> {
            Cow::Borrowed("Use typed value")
        }

        fn search_picker_detail(&self, _: &()) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(&self.0))
        }

        fn search_picker_input_text(&self) -> Cow<'_, str> {
            Cow::Borrowed(&self.0)
        }
    }

    fn picker(query: &str) -> SearchPicker<Item, SearchPickerNoCustom, (), Input> {
        let mut picker = SearchPicker::new(
            "Models".into(),
            String::new(),
            String::new(),
            "No models".into(),
            Input(query.into()),
            SearchPickerConfig::searchable(),
            None,
            (),
        );
        picker.replace_items(vec![
            Item {
                key: "haiku",
                label: "Claude Haiku",
                detail: "fast anthropic model",
                pinned: false,
            },
            Item {
                key: "sonnet",
                label: "Claude Sonnet",
                detail: "balanced anthropic model",
                pinned: false,
            },
            Item {
                key: "gpt",
                label: "GPT",
                detail: "openai model",
                pinned: false,
            },
        ]);
        picker
    }

    #[test]
    fn ranked_search_prefers_primary_label_matches() {
        let picker = picker("son");
        assert_eq!(picker.result_count(), 1);
        assert_eq!(picker.selected_item().unwrap().key, "sonnet");
    }

    #[test]
    fn every_picker_uses_the_same_centered_responsive_window() {
        assert_eq!(
            search_picker_dialog_area(ratatui::layout::Rect::new(0, 0, 120, 30)),
            ratatui::layout::Rect::new(7, 3, 105, 24)
        );
        assert_eq!(
            search_picker_dialog_area(ratatui::layout::Rect::new(0, 0, 200, 50)),
            ratatui::layout::Rect::new(37, 8, 126, 34)
        );
        assert_eq!(
            search_picker_dialog_area(ratatui::layout::Rect::new(0, 0, 8, 4)),
            ratatui::layout::Rect::new(0, 0, 8, 4)
        );
    }

    #[test]
    fn search_matches_secondary_document_fields() {
        let picker = picker("openai");
        assert_eq!(picker.result_count(), 1);
        assert_eq!(picker.selected_item().unwrap().key, "gpt");
    }

    #[test]
    fn exact_item_value_does_not_render_a_duplicate_custom_row() {
        let mut config = SearchPickerConfig::searchable();
        config.input_mode = SearchPickerInputMode::SearchWithCustomValue;
        let mut picker = SearchPicker::<Item, Custom, (), Input>::new(
            "Models".into(),
            String::new(),
            String::new(),
            "No models".into(),
            Input("Claude Sonnet".into()),
            config,
            None,
            (),
        );
        picker.replace_items(vec![Item {
            key: "sonnet",
            label: "Claude Sonnet",
            detail: "balanced model",
            pinned: false,
        }]);

        assert_eq!(picker.result_count(), 1);
        assert_eq!(picker.row_count(), 1);
        assert!(matches!(
            picker.selected_row(),
            Some(SearchPickerSelection::Item(item)) if item.key == "sonnet"
        ));

        picker.input.set_text("custom-model".into());
        picker.refresh_results();
        assert_eq!(picker.result_count(), 0);
        assert_eq!(picker.row_count(), 1);
        assert!(matches!(
            picker.selected_row(),
            Some(SearchPickerSelection::Custom(Custom(value))) if value == "custom-model"
        ));
    }

    #[test]
    fn refresh_preserves_selection_by_stable_key() {
        let mut picker = picker("");
        picker.move_selection(1);
        assert_eq!(picker.selected_item().unwrap().key, "sonnet");
        picker.replace_items(vec![
            Item {
                key: "gpt",
                label: "GPT",
                detail: "openai model",
                pinned: false,
            },
            Item {
                key: "haiku",
                label: "Claude Haiku",
                detail: "fast anthropic model",
                pinned: false,
            },
            Item {
                key: "sonnet",
                label: "Claude Sonnet",
                detail: "balanced anthropic model",
                pinned: false,
            },
        ]);
        assert_eq!(picker.selected_item().unwrap().key, "sonnet");
    }

    #[test]
    fn every_query_change_returns_to_the_first_page() {
        let mut picker = picker("");
        picker.set_visible_page_size(2);
        picker.move_selection(2);
        assert_eq!(picker.current_page(), 1);

        picker.input.set_text("model".into());
        picker.refresh_results();

        assert_eq!(picker.result_count(), 3);
        assert_eq!(picker.selected, 0);
        assert_eq!(picker.current_page(), 0);
        assert_eq!(picker.selected_item().unwrap().key, "haiku");
    }

    #[test]
    fn external_query_edits_reset_before_and_after_results_arrive() {
        let mut config = SearchPickerConfig::searchable();
        config.search_mode = SearchPickerSearchMode::External;
        let mut picker = SearchPicker::<Item, SearchPickerNoCustom, (), Editor>::new(
            "Sessions".into(),
            String::new(),
            String::new(),
            "No sessions".into(),
            Editor::default(),
            config,
            None,
            (),
        );
        let items = vec![
            Item {
                key: "one",
                label: "One",
                detail: "first",
                pinned: false,
            },
            Item {
                key: "two",
                label: "Two",
                detail: "second",
                pinned: false,
            },
            Item {
                key: "three",
                label: "Three",
                detail: "third",
                pinned: false,
            },
        ];
        picker.replace_items(items.clone());
        picker.set_visible_page_size(2);
        picker.move_selection(2);
        assert_eq!(picker.current_page(), 1);

        let result = picker.handle_input_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(
            result,
            super::SearchPickerInputResult::Edited { changed: true }
        );
        assert_eq!(picker.selected, 0);
        assert_eq!(picker.current_page(), 0);

        picker.replace_items(items);
        assert_eq!(picker.selected, 0);
        assert_eq!(picker.current_page(), 0);
    }

    #[test]
    fn selection_moves_across_stable_pages_instead_of_sliding_the_window() {
        let mut picker = picker("");
        picker.replace_items(vec![
            Item {
                key: "one",
                label: "One",
                detail: "first",
                pinned: false,
            },
            Item {
                key: "two",
                label: "Two",
                detail: "second",
                pinned: false,
            },
            Item {
                key: "three",
                label: "Three",
                detail: "third",
                pinned: false,
            },
            Item {
                key: "four",
                label: "Four",
                detail: "fourth",
                pinned: false,
            },
            Item {
                key: "five",
                label: "Five",
                detail: "fifth",
                pinned: false,
            },
            Item {
                key: "six",
                label: "Six",
                detail: "sixth",
                pinned: false,
            },
            Item {
                key: "seven",
                label: "Seven",
                detail: "seventh",
                pinned: false,
            },
        ]);
        picker.set_visible_page_size(3);

        assert_eq!(picker.page_count(), 3);
        assert_eq!(picker.visible_page_bounds(), (0, 3));
        picker.move_selection(2);
        assert_eq!(picker.visible_page_bounds(), (0, 3));
        picker.move_selection(1);
        assert_eq!(picker.visible_page_bounds(), (3, 6));

        picker.move_selection_home();
        picker.move_selection(1);
        picker.move_selection_page(1);
        assert_eq!(picker.selected, 4);
        assert_eq!(picker.visible_page_bounds(), (3, 6));
        picker.move_selection_page(1);
        assert_eq!(picker.selected, 6);
        assert_eq!(picker.visible_page_bounds(), (6, 7));
    }

    #[test]
    fn horizontal_keys_follow_the_active_input_or_results_focus() {
        let mut config = SearchPickerConfig::searchable();
        config.search_mode = SearchPickerSearchMode::None;
        let mut picker = SearchPicker::<Item, SearchPickerNoCustom, (), Editor>::new(
            "Models".into(),
            String::new(),
            String::new(),
            "No models".into(),
            Editor::from_text("ab".into()),
            config,
            None,
            (),
        );
        picker.replace_items(vec![
            Item {
                key: "one",
                label: "One",
                detail: "first",
                pinned: false,
            },
            Item {
                key: "two",
                label: "Two",
                detail: "second",
                pinned: false,
            },
            Item {
                key: "three",
                label: "Three",
                detail: "third",
                pinned: false,
            },
        ]);
        picker.set_visible_page_size(2);

        assert_eq!(picker.focus, SearchPickerFocus::Input);
        assert_eq!(picker.input.cursor(), 2);
        let _ = picker.handle_input_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(picker.input.cursor(), 1);
        assert_eq!(picker.selected, 0);

        let _ = picker.handle_input_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(picker.focus, SearchPickerFocus::Results);
        assert_eq!(picker.selected, 0);
        let _ = picker.handle_input_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(picker.current_page(), 1);
        assert_eq!(picker.selected, 2);
        assert_eq!(picker.input.cursor(), 1);

        picker.move_selection_home();
        let _ = picker.handle_input_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(picker.focus, SearchPickerFocus::Input);
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn external_mode_does_not_filter_remote_page_again() {
        let mut picker = picker("does-not-match");
        picker.config.search_mode = SearchPickerSearchMode::External;
        picker.refresh_results();
        assert_eq!(picker.result_count(), 3);
    }

    #[test]
    fn appending_remote_batches_preserves_the_current_selection() {
        let mut picker = picker("");
        picker.config.search_mode = SearchPickerSearchMode::External;
        picker.move_selection(1);
        picker.append_items(vec![Item {
            key: "opus",
            label: "Claude Opus",
            detail: "large anthropic model",
            pinned: false,
        }]);

        assert_eq!(picker.result_count(), 4);
        assert_eq!(picker.selected_item().unwrap().key, "sonnet");
        assert_eq!(picker.items.last().unwrap().key, "opus");
    }

    #[test]
    fn pinned_actions_remain_visible_for_unrelated_queries() {
        let mut picker = picker("unrelated");
        let mut items = picker.items.clone();
        items.insert(
            0,
            Item {
                key: "create",
                label: "Create model",
                detail: "Add a custom model",
                pinned: true,
            },
        );
        picker.replace_items(items);
        assert_eq!(picker.result_count(), 1);
        assert_eq!(picker.selected_item().unwrap().key, "create");
    }

    #[test]
    fn request_generations_reject_stale_external_results() {
        let mut picker = picker("");
        let first = picker.begin_external_search();
        let second = picker.begin_external_search();
        assert!(!picker.accepts_generation(first));
        assert!(picker.accepts_generation(second));
    }

    #[derive(Debug)]
    struct CountedItem {
        key: String,
        label: String,
        clones: Arc<AtomicUsize>,
    }

    impl Clone for CountedItem {
        fn clone(&self) -> Self {
            self.clones.fetch_add(1, Ordering::Relaxed);
            Self {
                key: self.key.clone(),
                label: self.label.clone(),
                clones: Arc::clone(&self.clones),
            }
        }
    }

    impl SearchPickerItem for CountedItem {
        fn search_picker_key(&self) -> Cow<'_, str> {
            Cow::Borrowed(&self.key)
        }

        fn search_picker_label(&self) -> Cow<'_, str> {
            Cow::Borrowed(&self.label)
        }

        fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
            None
        }
    }

    #[test]
    fn repeated_filtering_does_not_clone_catalog_items() {
        let clones = Arc::new(AtomicUsize::new(0));
        let items = (0..1_000)
            .map(|index| CountedItem {
                key: index.to_string(),
                label: format!("Model {index}"),
                clones: Arc::clone(&clones),
            })
            .collect();
        let mut picker = SearchPicker::<CountedItem, SearchPickerNoCustom, (), Input>::new(
            "Models".into(),
            String::new(),
            String::new(),
            "No models".into(),
            Input(String::new()),
            SearchPickerConfig::searchable(),
            None,
            (),
        );
        picker.replace_items(items);
        for query in ["m", "mo", "model", "model 9", "missing", ""] {
            picker.input.set_text(query.to_owned());
            picker.refresh_results();
        }
        assert_eq!(clones.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn unicode_queries_match_without_ascii_only_lowercasing() {
        let mut picker = picker("");
        let mut items = picker.items.clone();
        items.push(Item {
            key: "zh-model",
            label: "推理模型",
            detail: "适合代码分析",
            pinned: false,
        });
        picker.replace_items(items);
        picker.input.set_text("代码".into());
        picker.refresh_results();
        assert_eq!(picker.result_count(), 1);
        assert_eq!(picker.selected_item().unwrap().key, "zh-model");
    }

    fn rendered_buffer(backend: &TestBackend) -> String {
        let buffer = backend.buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn responsive_picker_renders_results_and_preview_without_full_catalog_rows() {
        let mut picker = SearchPicker::<Item, SearchPickerNoCustom, (), Editor>::new(
            "Models".into(),
            "Choose a runtime model".into(),
            "Enter select · Esc close".into(),
            "No models".into(),
            Editor::from_text("claude".into()),
            SearchPickerConfig {
                preview_mode: SearchPickerPreviewMode::Responsive {
                    min_total_width: 90,
                    left_min_width: 40,
                    right_min_width: 40,
                },
                ..SearchPickerConfig::searchable()
            },
            None,
            (),
        );
        picker.replace_items(vec![
            Item {
                key: "sonnet",
                label: "Claude Sonnet",
                detail: "balanced model",
                pinned: false,
            },
            Item {
                key: "haiku",
                label: "Claude Haiku",
                detail: "fast model",
                pinned: false,
            },
        ]);
        let backend = TestBackend::new(120, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_picker_dialog_with_preview(
                    frame,
                    area,
                    &picker,
                    &SearchPickerDialogSpec::new("Loading…".into(), "Results".into()),
                    str::to_owned,
                    |state| {
                        let body = match state {
                            SearchPickerViewState::Selected(item) => {
                                Text::from(format!("Selected: {}", item.label))
                            }
                            SearchPickerViewState::Loading { message }
                            | SearchPickerViewState::Empty { message }
                            | SearchPickerViewState::Error { message } => {
                                Text::from(message.to_owned())
                            }
                        };
                        vec![WorkbenchTextSection::new("Details".into(), body, 4, 10)]
                    },
                );
            })
            .unwrap();
        let rendered = rendered_buffer(terminal.backend());
        assert!(rendered.contains("Models · 2 results"));
        assert!(rendered.contains("Results · 2 · Page 1/1"));
        assert!(rendered.contains("Claude Sonnet"));
        assert!(rendered.contains("Details"));
        assert!(rendered.contains("Selected:"), "{rendered}");
    }

    #[test]
    fn rendered_page_size_tracks_the_available_terminal_height() {
        let picker = SearchPicker::<Item, SearchPickerNoCustom, (), Editor>::new(
            "Models".into(),
            String::new(),
            String::new(),
            "No models".into(),
            Editor::default(),
            SearchPickerConfig::searchable(),
            None,
            (),
        );
        let spec = SearchPickerDialogSpec::new("Loading…".into(), "Results".into());

        let small_backend = TestBackend::new(80, 10);
        let mut small_terminal = Terminal::new(small_backend).unwrap();
        small_terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_picker_dialog(frame, area, &picker, &spec, str::to_owned);
            })
            .unwrap();
        let small_page_size = picker.page_size();

        let tall_backend = TestBackend::new(80, 24);
        let mut tall_terminal = Terminal::new(tall_backend).unwrap();
        tall_terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_picker_dialog(frame, area, &picker, &spec, str::to_owned);
            })
            .unwrap();
        let tall_page_size = picker.page_size();

        assert!(small_page_size >= 1);
        assert!(tall_page_size > small_page_size);
    }

    #[test]
    fn changing_focus_does_not_shift_result_columns() {
        let mut picker = SearchPicker::<Item, SearchPickerNoCustom, (), Editor>::new(
            "Models".into(),
            String::new(),
            String::new(),
            "No models".into(),
            Editor::default(),
            SearchPickerConfig::searchable(),
            None,
            (),
        );
        picker.replace_items(vec![Item {
            key: "sonnet",
            label: "Claude Sonnet",
            detail: "balanced model",
            pinned: false,
        }]);
        let spec = SearchPickerDialogSpec::new("Loading…".into(), "Results".into());
        let backend = TestBackend::new(80, 18);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_picker_dialog(frame, area, &picker, &spec, str::to_owned);
            })
            .unwrap();
        let input_focused = rendered_buffer(terminal.backend());
        let input_column = input_focused
            .lines()
            .find_map(|line| line.find("Claude Sonnet"))
            .unwrap();

        picker.focus = SearchPickerFocus::Results;
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_picker_dialog(frame, area, &picker, &spec, str::to_owned);
            })
            .unwrap();
        let results_focused = rendered_buffer(terminal.backend());
        let results_column = results_focused
            .lines()
            .find_map(|line| line.find("Claude Sonnet"))
            .unwrap();

        assert_eq!(input_column, results_column);
    }

    #[test]
    fn picker_gracefully_renders_in_a_tiny_terminal() {
        let mut picker = SearchPicker::<Item, SearchPickerNoCustom, (), Editor>::new(
            "Models".into(),
            String::new(),
            String::new(),
            "No models".into(),
            Editor::default(),
            SearchPickerConfig::searchable(),
            None,
            (),
        );
        picker.replace_items(Vec::new());
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_picker_dialog(
                    frame,
                    area,
                    &picker,
                    &SearchPickerDialogSpec::new("Loading…".into(), "Results".into()),
                    str::to_owned,
                );
            })
            .unwrap();
        let rendered = rendered_buffer(terminal.backend());
        assert!(rendered.contains("Models"), "{rendered}");
    }

    #[test]
    fn custom_capability_is_derived_from_input_mode() {
        let mut config = SearchPickerConfig::searchable();
        assert!(!config.input_mode.allows_custom_value());
        config.input_mode = SearchPickerInputMode::SearchWithCustomValue;
        assert!(config.input_mode.allows_custom_value());

        fn assert_no_custom_impl<T: SearchPickerCustomValue<()>>() {}
        assert_no_custom_impl::<SearchPickerNoCustom>();
    }
}
