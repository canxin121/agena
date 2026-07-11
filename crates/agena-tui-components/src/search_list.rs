use std::path::PathBuf;

use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::{Borders, ListItem, Paragraph, Wrap},
};

use crate::theme;

use crate::{
    Editor, EditorPanelSpec, FramedSurfaceSpec, InputDialogAction, NavigationAction,
    input_dialog_action,
    layout::{
        SurfaceMode, VerticalSectionSize, editor_input_panel_height, framed_sections_target_height,
        optional_overlay_text_height, overlay_text_height, split_vertical_sections,
    },
    navigation_action,
    panels::{ListPanelState, MeasuredListPanelHeight, render_list_panel_state},
    render_editor_panel, render_framed_surface,
    selection::{
        clamp_selected_index, move_selected_index, move_selected_index_end,
        move_selected_index_home, move_selected_index_page,
    },
    truncate_display_text,
};

pub trait SearchListInput: Clone {
    fn text(&self) -> &str;
    fn set_text(&mut self, text: String);
    fn handle_line_input_key(&mut self, key: KeyEvent);
    fn flush_all_pending_input(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchInputKeyResult {
    Close,
    Navigated,
    Edited { changed: bool },
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
    pub input_enabled: bool,
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
        theme::muted_style()
    }
}

pub trait SearchListCustomValue<TContext>: Clone {
    fn search_list_from_input(input: &str, context: &TContext) -> Option<Self>;
    fn search_list_label(&self, context: &TContext) -> String;
    fn search_list_detail(&self, context: &TContext) -> Option<String>;
    fn search_list_input_text(&self) -> String;

    fn search_list_label_style(&self) -> Style {
        Style::default().fg(theme::accent_color())
    }

    fn search_list_detail_style(&self) -> Style {
        theme::muted_style()
    }
}

pub(crate) struct SearchListOverlayRenderSpec<'a> {
    pub list_title: Option<std::borrow::Cow<'a, str>>,
}

pub enum SearchListDialogState<'a, TItem, TCustom> {
    Loading { message: &'a str },
    Empty { message: &'a str },
    Rows(Vec<SearchListRow<TItem, TCustom>>),
}

pub struct SearchListDialogSpec<'a> {
    pub loading_message: std::borrow::Cow<'a, str>,
    pub list_title: Option<std::borrow::Cow<'a, str>>,
    pub highlight_style: Style,
    pub highlight_symbol: std::borrow::Cow<'a, str>,
}

impl<'a> SearchListDialogSpec<'a> {
    pub fn new(
        loading_message: std::borrow::Cow<'a, str>,
        list_title: Option<std::borrow::Cow<'a, str>>,
        highlight_style: Style,
        highlight_symbol: std::borrow::Cow<'a, str>,
    ) -> Self {
        Self {
            loading_message,
            list_title,
            highlight_style,
            highlight_symbol,
        }
    }
}

enum SearchListPanelContent<'a> {
    Empty { message: String },
    Panel(ListPanelState<'a, MeasuredListPanelHeight>),
}

impl<TContext> SearchListCustomValue<TContext> for SearchListNoCustom {
    fn search_list_from_input(_: &str, _: &TContext) -> Option<Self> {
        None
    }

    fn search_list_label(&self, _: &TContext) -> String {
        String::new()
    }

    fn search_list_detail(&self, _: &TContext) -> Option<String> {
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
    TCustom: SearchListCustomValue<TMeta>,
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
            && let Some(custom) = TCustom::search_list_from_input(self.input.text(), &self.meta)
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

    pub fn dialog_state<'a>(
        &'a self,
        loading_message: &'a str,
    ) -> SearchListDialogState<'a, TItem, TCustom> {
        if self.loading {
            SearchListDialogState::Loading {
                message: loading_message,
            }
        } else {
            let rows = self.rows();
            if rows.is_empty() {
                SearchListDialogState::Empty {
                    message: self.empty_message.as_str(),
                }
            } else {
                SearchListDialogState::Rows(rows)
            }
        }
    }

    pub fn clamp_selection(&mut self) {
        let row_count = self.row_count();
        clamp_selected_index(&mut self.selected, row_count);
    }

    pub fn move_selection(&mut self, delta: isize) {
        let row_count = self.row_count();
        move_selected_index(&mut self.selected, row_count, delta);
    }

    pub fn move_selection_page(&mut self, delta: isize, page_size: usize) {
        let row_count = self.row_count();
        move_selected_index_page(&mut self.selected, row_count, delta, page_size);
    }

    pub fn move_selection_home(&mut self) {
        move_selected_index_home(&mut self.selected);
    }

    pub fn move_selection_end(&mut self) {
        let row_count = self.row_count();
        move_selected_index_end(&mut self.selected, row_count);
    }

    pub fn fill_input_from_selected(&mut self) -> bool {
        if !self.config.input_enabled || !self.config.fill_selected_into_input {
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

    pub fn handle_navigation_key(&mut self, key: KeyEvent, page_size: usize) -> bool {
        match navigation_action(key) {
            Some(NavigationAction::PageUp) => {
                self.move_selection_page(-1, page_size);
                true
            }
            Some(NavigationAction::PageDown) => {
                self.move_selection_page(1, page_size);
                true
            }
            Some(NavigationAction::Home) => {
                self.move_selection_home();
                true
            }
            Some(NavigationAction::End) => {
                self.move_selection_end();
                true
            }
            Some(NavigationAction::Up) => {
                self.move_selection(-1);
                true
            }
            Some(NavigationAction::Down) => {
                self.move_selection(1);
                true
            }
            _ => false,
        }
    }

    pub fn handle_filter_input_key(
        &mut self,
        key: KeyEvent,
        page_size: usize,
    ) -> SearchInputKeyResult {
        if input_dialog_action(key, false) == Some(InputDialogAction::Close) {
            return SearchInputKeyResult::Close;
        }
        if self.handle_navigation_key(key, page_size) {
            return SearchInputKeyResult::Navigated;
        }
        if !self.config.input_enabled {
            return SearchInputKeyResult::Edited { changed: false };
        }

        let before = self.input.text().to_string();
        self.input.handle_line_input_key(key);
        self.input.flush_all_pending_input();
        SearchInputKeyResult::Edited {
            changed: self.input.text() != before,
        }
    }
}

impl<TItem, TCustom> SearchListRow<TItem, TCustom>
where
    TItem: SearchListItem,
{
    pub fn label<TContext>(&self, context: &TContext) -> String
    where
        TCustom: SearchListCustomValue<TContext>,
    {
        match self {
            Self::Clear(clear) => clear.label.clone(),
            Self::Custom(custom) => custom.search_list_label(context),
            Self::Item(item) => item.search_list_label(),
        }
    }

    pub fn detail<TContext>(&self, context: &TContext) -> Option<String>
    where
        TCustom: SearchListCustomValue<TContext>,
    {
        match self {
            Self::Clear(clear) => (!clear.detail.trim().is_empty()).then_some(clear.detail.clone()),
            Self::Custom(custom) => custom.search_list_detail(context),
            Self::Item(item) => item.search_list_detail(),
        }
    }

    pub fn label_style<TContext>(&self, _: &TContext) -> Style
    where
        TCustom: SearchListCustomValue<TContext>,
    {
        match self {
            Self::Clear(_) => Style::default().fg(theme::warning_color()),
            Self::Custom(custom) => custom.search_list_label_style(),
            Self::Item(item) => item.search_list_label_style(),
        }
    }

    pub fn detail_style<TContext>(&self, _: &TContext) -> Style
    where
        TCustom: SearchListCustomValue<TContext>,
    {
        match self {
            Self::Clear(_) => theme::muted_style(),
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
    TCustom: SearchListCustomValue<TMeta>,
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

fn render_search_list_overlay<TItem, TCustom, TMeta, F>(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    dialog: &SearchListOverlay<TItem, TCustom, TMeta, Editor>,
    spec: &SearchListOverlayRenderSpec<'_>,
    panel_content: &SearchListPanelContent<'_>,
    normalize_text: F,
) where
    TItem: SearchListItem,
    TCustom: SearchListCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
{
    let content_width = surface.content_width(area, dialog.config.target_width);
    let prompt_height = optional_overlay_text_height(dialog.prompt.as_str(), content_width, 1, 2);
    let input_height = if dialog.config.input_enabled {
        editor_input_panel_height(&dialog.input, false)
    } else {
        0
    };
    let footer_height = optional_overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
    let list_height = match panel_content {
        SearchListPanelContent::Empty { message } => search_list_empty_panel_height(
            message.as_str(),
            content_width,
            dialog.config.min_list_body_height,
            dialog.config.max_list_body_height,
        ),
        SearchListPanelContent::Panel(state) => state.resolve_height(),
    };
    let mut section_sizes = Vec::new();
    if prompt_height > 0 {
        section_sizes.push(VerticalSectionSize::Fixed(prompt_height));
    }
    if dialog.config.input_enabled {
        section_sizes.push(VerticalSectionSize::Fixed(input_height));
    }
    section_sizes.push(match surface {
        SurfaceMode::Overlay => VerticalSectionSize::Fixed(list_height),
        SurfaceMode::Route => VerticalSectionSize::Flexible(list_height),
    });
    if footer_height > 0 {
        section_sizes.push(VerticalSectionSize::Fixed(footer_height));
    }
    let frame_surface = render_framed_surface(
        frame,
        area,
        surface,
        &FramedSurfaceSpec {
            title: normalize_text(dialog.title.as_str()).into(),
            target_width: dialog.config.target_width,
            target_height: framed_sections_target_height(&section_sizes),
        },
    );
    let sections = split_vertical_sections(frame_surface.inner, &section_sizes);

    let mut section_index = 0;
    if prompt_height > 0 {
        frame.render_widget(
            Paragraph::new(normalize_text(dialog.prompt.as_str())).wrap(Wrap { trim: false }),
            sections[section_index],
        );
        section_index += 1;
    }

    let input_result = if dialog.config.input_enabled {
        let input_area = sections[section_index];
        section_index += 1;
        Some(render_editor_panel(
            frame,
            input_area,
            &EditorPanelSpec {
                title: None,
                borders: Borders::ALL,
            },
            &dialog.input,
        ))
    } else {
        None
    };

    let list_area = sections[section_index];
    section_index += 1;
    match panel_content {
        SearchListPanelContent::Empty { message } => {
            let list_block = match spec.list_title.as_ref() {
                Some(title) => ratatui::widgets::Block::default()
                    .borders(Borders::ALL)
                    .title(normalize_text(title.as_ref())),
                None => ratatui::widgets::Block::default().borders(Borders::ALL),
            };
            frame.render_widget(
                Paragraph::new(normalize_text(message.as_str()))
                    .style(theme::muted_style())
                    .wrap(Wrap { trim: false })
                    .block(list_block),
                list_area,
            );
        }
        SearchListPanelContent::Panel(state) => {
            render_list_panel_state(frame, list_area, state);
        }
    }

    if footer_height > 0 {
        frame.render_widget(
            Paragraph::new(normalize_text(dialog.footer.as_str())).wrap(Wrap { trim: false }),
            sections[section_index],
        );
    }

    if let Some(result) = input_result {
        frame.set_cursor_position(result.cursor);
    }
}

pub fn render_search_list_dialog<TItem, TCustom, TMeta, F>(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    dialog: &SearchListOverlay<TItem, TCustom, TMeta, Editor>,
    spec: &SearchListDialogSpec<'_>,
    normalize_text: F,
) where
    TItem: SearchListItem,
    TCustom: SearchListCustomValue<TMeta>,
    TMeta: Clone,
    F: for<'a> Fn(&'a str) -> String,
{
    match dialog.dialog_state(spec.loading_message.as_ref()) {
        SearchListDialogState::Loading { message } => {
            let panel_content = SearchListPanelContent::Panel(ListPanelState::loading(
                spec.list_title
                    .clone()
                    .map(|title| normalize_text(title.as_ref()).into()),
                normalize_text(message).into(),
                MeasuredListPanelHeight {
                    min_body_height: dialog.config.min_list_body_height,
                    max_body_height: dialog.config.max_list_body_height,
                },
            ));
            render_search_list_overlay(
                frame,
                area,
                surface,
                dialog,
                &SearchListOverlayRenderSpec {
                    list_title: spec.list_title.clone(),
                },
                &panel_content,
                normalize_text,
            );
        }
        SearchListDialogState::Empty { message } => {
            let panel_content = SearchListPanelContent::Empty {
                message: message.to_string(),
            };
            render_search_list_overlay(
                frame,
                area,
                surface,
                dialog,
                &SearchListOverlayRenderSpec {
                    list_title: spec.list_title.clone(),
                },
                &panel_content,
                normalize_text,
            );
        }
        SearchListDialogState::Rows(rows) => {
            let row_width = surface
                .content_width(area, dialog.config.target_width)
                .saturating_sub(6)
                .max(1) as usize;
            let items = rows
                .iter()
                .map(|row| {
                    let label = truncate_display_text(
                        normalize_text(row.label(&dialog.meta).as_str()).as_str(),
                        row_width,
                    );
                    let mut lines = vec![Line::from(Span::styled(
                        label,
                        row.label_style(&dialog.meta),
                    ))];
                    if let Some(detail) = row.detail(&dialog.meta) {
                        let detail = truncate_display_text(
                            normalize_text(detail.as_str()).as_str(),
                            row_width,
                        );
                        lines.push(Line::from(Span::styled(
                            detail,
                            row.detail_style(&dialog.meta),
                        )));
                    }
                    ListItem::new(Text::from(lines))
                })
                .collect::<Vec<_>>();
            let panel_content = SearchListPanelContent::Panel(ListPanelState::items(
                MeasuredListPanelHeight {
                    min_body_height: dialog.config.min_list_body_height,
                    max_body_height: dialog.config.max_list_body_height,
                },
                spec.list_title
                    .clone()
                    .map(|title| normalize_text(title.as_ref()).into()),
                items.as_slice(),
                Some(std::cmp::min(dialog.selected, rows.len().saturating_sub(1))),
                spec.highlight_style,
                spec.highlight_symbol.clone(),
            ));
            render_search_list_overlay(
                frame,
                area,
                surface,
                dialog,
                &SearchListOverlayRenderSpec {
                    list_title: spec.list_title.clone(),
                },
                &panel_content,
                normalize_text,
            );
        }
    }
}

fn search_list_empty_panel_height(
    message: &str,
    width: u16,
    min_body_height: u16,
    max_body_height: u16,
) -> u16 {
    overlay_text_height(
        message,
        width.saturating_sub(4).max(1),
        min_body_height,
        max_body_height,
    )
    .saturating_add(2)
}
