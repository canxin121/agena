use std::path::PathBuf;

use ratatui::style::{Color, Style};

pub trait SearchListInput: Clone {
    fn text(&self) -> &str;
    fn set_text(&mut self, text: String);
}

#[derive(Debug, Clone)]
pub struct SearchListOverlay<TItem, TCustom, TMeta, TInput> {
    pub title: String,
    pub prompt: String,
    pub footer: String,
    pub empty_message: String,
    pub input: TInput,
    pub items: Vec<TItem>,
    pub selected: usize,
    pub loading: bool,
    pub config: SearchListOverlayConfig,
    pub clear_action: Option<SearchListClearAction>,
    pub meta: TMeta,
    custom: std::marker::PhantomData<TCustom>,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchListOverlayConfig {
    pub target_width: u16,
    pub search_enabled: bool,
    pub custom_value_enabled: bool,
    pub fill_selected_into_input: bool,
    pub min_list_body_height: u16,
    pub max_list_body_height: u16,
}

#[derive(Debug, Clone)]
pub struct SearchListClearAction {
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub enum SearchListRow<TItem, TCustom> {
    Clear(SearchListClearAction),
    Custom(TCustom),
    Item(TItem),
}

#[derive(Debug, Clone, Default)]
pub struct SearchListNoCustom;

pub trait SearchListItem: Clone {
    fn search_list_label(&self) -> String;
    fn search_list_detail(&self) -> Option<String>;
    fn search_list_fill_value(&self) -> String;
    fn search_list_matches_query(&self, query: &str) -> bool;

    fn search_list_label_style(&self) -> Style {
        Style::default()
    }

    fn search_list_detail_style(&self) -> Style {
        Style::default().fg(Color::DarkGray)
    }
}

pub trait SearchListCustomValue: Clone {
    fn search_list_from_input(input: &str) -> Option<Self>;
    fn search_list_label(&self) -> String;
    fn search_list_detail(&self) -> Option<String>;
    fn search_list_input_text(&self) -> String;

    fn search_list_label_style(&self) -> Style {
        Style::default().fg(Color::Cyan)
    }

    fn search_list_detail_style(&self) -> Style {
        Style::default().fg(Color::DarkGray)
    }
}

impl SearchListCustomValue for SearchListNoCustom {
    fn search_list_from_input(_: &str) -> Option<Self> {
        None
    }

    fn search_list_label(&self) -> String {
        String::new()
    }

    fn search_list_detail(&self) -> Option<String> {
        None
    }

    fn search_list_input_text(&self) -> String {
        String::new()
    }
}

impl SearchListItem for PathBuf {
    fn search_list_label(&self) -> String {
        self.display().to_string()
    }

    fn search_list_detail(&self) -> Option<String> {
        None
    }

    fn search_list_fill_value(&self) -> String {
        self.display().to_string()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        query.trim().is_empty()
            || self
                .display()
                .to_string()
                .to_ascii_lowercase()
                .contains(query.trim().to_ascii_lowercase().as_str())
    }
}

impl<TItem, TCustom, TMeta, TInput> SearchListOverlay<TItem, TCustom, TMeta, TInput>
where
    TItem: SearchListItem,
    TCustom: SearchListCustomValue,
    TInput: SearchListInput,
{
    pub fn new(
        title: String,
        prompt: String,
        footer: String,
        empty_message: String,
        input: TInput,
        config: SearchListOverlayConfig,
        clear_action: Option<SearchListClearAction>,
        meta: TMeta,
    ) -> Self {
        Self {
            title,
            prompt,
            footer,
            empty_message,
            input,
            items: Vec::new(),
            selected: 0,
            loading: false,
            config,
            clear_action,
            meta,
            custom: std::marker::PhantomData,
        }
    }

    pub fn rows(&self) -> Vec<SearchListRow<TItem, TCustom>> {
        let mut rows = Vec::new();
        if let Some(clear) = self.clear_action.as_ref() {
            rows.push(SearchListRow::Clear(clear.clone()));
        }
        if self.config.custom_value_enabled
            && let Some(custom) = TCustom::search_list_from_input(self.input.text())
        {
            rows.push(SearchListRow::Custom(custom));
        }
        rows.extend(self.items.iter().cloned().map(SearchListRow::Item));
        rows
    }

    pub fn selected_row(&self) -> Option<SearchListRow<TItem, TCustom>> {
        self.rows().get(self.selected).cloned()
    }

    pub fn row_count(&self) -> usize {
        self.rows().len()
    }

    pub fn clamp_selection(&mut self) {
        let row_count = self.row_count();
        if row_count == 0 {
            self.selected = 0;
        } else {
            self.selected = std::cmp::min(self.selected, row_count.saturating_sub(1));
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        let row_count = self.row_count();
        if row_count == 0 {
            self.selected = 0;
            return;
        }
        let current = self.selected as isize;
        let last = row_count.saturating_sub(1) as isize;
        self.selected = current.saturating_add(delta).clamp(0, last) as usize;
    }

    pub fn move_selection_page(&mut self, delta: isize, page_size: usize) {
        self.move_selection(delta.saturating_mul(page_size.max(1) as isize));
    }

    pub fn move_selection_home(&mut self) {
        self.selected = 0;
    }

    pub fn move_selection_end(&mut self) {
        let row_count = self.row_count();
        if row_count > 0 {
            self.selected = row_count.saturating_sub(1);
        }
    }

    pub fn fill_input_from_selected(&mut self) -> bool {
        if !self.config.fill_selected_into_input {
            return false;
        }
        let Some(row) = self.selected_row() else {
            return false;
        };
        let value = match row {
            SearchListRow::Clear(_) => String::new(),
            SearchListRow::Custom(custom) => custom.search_list_input_text(),
            SearchListRow::Item(item) => item.search_list_fill_value(),
        };
        if self.input.text() == value {
            return false;
        }
        self.input.set_text(value);
        true
    }
}

impl<TItem, TCustom> SearchListRow<TItem, TCustom>
where
    TItem: SearchListItem,
    TCustom: SearchListCustomValue,
{
    pub fn label(&self) -> String {
        match self {
            Self::Clear(clear) => clear.label.clone(),
            Self::Custom(custom) => custom.search_list_label(),
            Self::Item(item) => item.search_list_label(),
        }
    }

    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Clear(clear) => (!clear.detail.trim().is_empty()).then_some(clear.detail.clone()),
            Self::Custom(custom) => custom.search_list_detail(),
            Self::Item(item) => item.search_list_detail(),
        }
    }

    pub fn label_style(&self) -> Style {
        match self {
            Self::Clear(_) => Style::default().fg(Color::Yellow),
            Self::Custom(custom) => custom.search_list_label_style(),
            Self::Item(item) => item.search_list_label_style(),
        }
    }

    pub fn detail_style(&self) -> Style {
        match self {
            Self::Clear(_) => Style::default().fg(Color::DarkGray),
            Self::Custom(custom) => custom.search_list_detail_style(),
            Self::Item(item) => item.search_list_detail_style(),
        }
    }
}

pub fn refresh_search_list_overlay<TItem, TCustom, TMeta, TInput>(
    dialog: &mut SearchListOverlay<TItem, TCustom, TMeta, TInput>,
    all_items: &[TItem],
) where
    TItem: SearchListItem,
    TCustom: SearchListCustomValue,
    TInput: SearchListInput,
{
    let query = dialog.input.text().trim();
    dialog.items = all_items
        .iter()
        .filter(|item| {
            !dialog.config.search_enabled
                || query.is_empty()
                || item.search_list_matches_query(query)
        })
        .cloned()
        .collect();
    if !dialog.config.search_enabled && query.is_empty() {
        dialog.items = all_items.to_vec();
    }
    dialog.clamp_selection();
}
