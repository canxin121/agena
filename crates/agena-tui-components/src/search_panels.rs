use std::{borrow::Cow, cmp::max};

use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::{Direction, Layout, Rect},
    style::Style,
    widgets::{Borders, ListItem, Paragraph, Wrap},
};

use crate::{
    Editor, EditorPanelSpec, FramedSurfaceSpec, InputDialogAction, NavigationAction,
    SearchInputKeyResult, SearchListInput, WorkbenchTextSection, bordered_text_height,
    input_dialog_action,
    layout::{
        SurfaceMode, VerticalSectionSize, adaptive_detail_split, editor_input_panel_height,
        estimated_horizontal_panel_widths, framed_sections_target_height,
        optional_overlay_text_height, should_stack_detail_layout, split_vertical_sections,
        top_aligned_panel_rect, top_aligned_vertical_areas,
    },
    navigation_action,
    panels::{
        BoundedListPanelHeight, ListPanelState, TextPanelSpec, render_list_panel_state,
        render_text_panel,
    },
    render_editor_panel, render_framed_surface, search_navigation_action,
    selection::{
        clamp_selected_index, move_selected_index, move_selected_index_end,
        move_selected_index_home, move_selected_index_page,
    },
};

#[derive(Debug, Clone)]
pub struct SearchPanelsOverlay<TItem, TMeta, TInput> {
    pub title: String,
    pub prompt: String,
    pub empty_message: String,
    pub footer: String,
    pub input: TInput,
    pub all_items: Vec<TItem>,
    pub items: Vec<TItem>,
    pub selected: usize,
    pub loading: bool,
    pub meta: TMeta,
}

pub enum SearchPanelsDialogState<'a, TItem> {
    Loading { message: &'a str },
    Empty { message: &'a str },
    Selected(&'a TItem),
}

impl<TItem, TMeta, TInput> SearchPanelsOverlay<TItem, TMeta, TInput>
where
    TInput: SearchListInput,
{
    pub fn new(
        title: String,
        prompt: String,
        empty_message: String,
        footer: String,
        input: TInput,
        loading: bool,
        meta: TMeta,
    ) -> Self {
        Self {
            title,
            prompt,
            empty_message,
            footer,
            input,
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            loading,
            meta,
        }
    }

    pub fn selected_item(&self) -> Option<&TItem> {
        self.items.get(self.selected)
    }

    pub fn list_selected_for_render(&self) -> Option<usize> {
        (!self.loading && !self.items.is_empty()).then_some(self.selected)
    }

    pub fn dialog_state<'a>(
        &'a self,
        loading_message: &'a str,
    ) -> SearchPanelsDialogState<'a, TItem> {
        if self.loading {
            SearchPanelsDialogState::Loading {
                message: loading_message,
            }
        } else if self.items.is_empty() {
            SearchPanelsDialogState::Empty {
                message: self.empty_message.as_str(),
            }
        } else {
            SearchPanelsDialogState::Selected(&self.items[self.selected])
        }
    }

    pub fn clamp_selection(&mut self) {
        clamp_selected_index(&mut self.selected, self.items.len());
    }

    pub fn move_selection(&mut self, delta: isize) {
        move_selected_index(&mut self.selected, self.items.len(), delta);
    }

    pub fn move_selection_page(&mut self, delta: isize, page_size: usize) {
        move_selected_index_page(&mut self.selected, self.items.len(), delta, page_size);
    }

    pub fn move_selection_home(&mut self) {
        move_selected_index_home(&mut self.selected);
    }

    pub fn move_selection_end(&mut self) {
        move_selected_index_end(&mut self.selected, self.items.len());
    }

    pub fn handle_navigation_key(&mut self, key: KeyEvent, page_size: usize) -> bool {
        self.handle_navigation_action(navigation_action(key), page_size)
    }

    fn handle_search_navigation_key(&mut self, key: KeyEvent, page_size: usize) -> bool {
        self.handle_navigation_action(search_navigation_action(key), page_size)
    }

    fn handle_navigation_action(
        &mut self,
        action: Option<NavigationAction>,
        page_size: usize,
    ) -> bool {
        match action {
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
        if self.handle_search_navigation_key(key, page_size) {
            return SearchInputKeyResult::Navigated;
        }

        let before = self.input.text().to_string();
        self.input.handle_line_input_key(key);
        self.input.flush_all_pending_input();
        SearchInputKeyResult::Edited {
            changed: self.input.text() != before,
        }
    }
}

pub fn refresh_search_panels_overlay<TItem, TMeta, TInput, F>(
    dialog: &mut SearchPanelsOverlay<TItem, TMeta, TInput>,
    mut matches_query: F,
) where
    TItem: Clone,
    TInput: SearchListInput,
    F: FnMut(&TItem, &str) -> bool,
{
    let query = dialog.input.text().trim().to_ascii_lowercase();
    dialog.items = dialog
        .all_items
        .iter()
        .filter(|item| query.is_empty() || matches_query(item, query.as_str()))
        .cloned()
        .collect();
    dialog.clamp_selection();
}

struct SearchPanelsSpec<'a> {
    pub title: Cow<'a, str>,
    pub prompt: Cow<'a, str>,
    pub footer: Cow<'a, str>,
    pub target_width: u16,
    pub left_min_width: u16,
    pub right_min_width: u16,
    pub left_panel_state: ListPanelState<'a, BoundedListPanelHeight>,
    pub right_sections: Vec<WorkbenchTextSection<'a>>,
}

pub struct SearchPanelsDialogSpec<'a> {
    pub target_width: u16,
    pub left_panel_title: Cow<'a, str>,
    pub left_panel_lines_per_item: u16,
    pub left_panel_min_body_height: u16,
    pub left_panel_max_body_height: u16,
    pub left_min_width: u16,
    pub right_min_width: u16,
    pub loading_message: Cow<'a, str>,
    pub highlight_style: Style,
    pub highlight_symbol: Cow<'a, str>,
}

impl<'a> SearchPanelsDialogSpec<'a> {
    pub fn new(
        target_width: u16,
        left_panel_title: Cow<'a, str>,
        left_panel_lines_per_item: u16,
        left_panel_height_bounds: (u16, u16),
        left_min_width: u16,
        right_min_width: u16,
        loading_message: Cow<'a, str>,
        highlight_style: Style,
        highlight_symbol: Cow<'a, str>,
    ) -> Self {
        Self {
            target_width,
            left_panel_title,
            left_panel_lines_per_item,
            left_panel_min_body_height: left_panel_height_bounds.0,
            left_panel_max_body_height: left_panel_height_bounds.1,
            left_min_width,
            right_min_width,
            loading_message,
            highlight_style,
            highlight_symbol,
        }
    }
}

fn render_search_panels_overlay(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &SearchPanelsSpec<'_>,
    input: &Editor,
) {
    let content_width = surface.content_width(area, spec.target_width);
    let prompt_height = optional_overlay_text_height(spec.prompt.as_ref(), content_width, 1, 2);
    let footer_height = optional_overlay_text_height(spec.footer.as_ref(), content_width, 1, 2);
    let input_height = editor_input_panel_height(input, false);
    let stacked =
        should_stack_detail_layout(content_width, spec.left_min_width, spec.right_min_width);
    let left_panel_height = spec.left_panel_state.resolve_height();
    let right_body_width = if stacked {
        content_width.saturating_sub(2).max(1)
    } else {
        estimated_horizontal_panel_widths(content_width, spec.left_min_width, spec.right_min_width)
            .1
            .saturating_sub(2)
            .max(1)
    };
    let right_section_heights = spec
        .right_sections
        .iter()
        .map(|section| {
            bordered_text_height(
                &section.body,
                right_body_width,
                section.min_body_height,
                section.max_body_height,
            )
        })
        .collect::<Vec<_>>();
    let panels_height = if stacked {
        left_panel_height.saturating_add(right_section_heights.iter().copied().sum::<u16>())
    } else {
        max(
            left_panel_height,
            right_section_heights.iter().copied().sum::<u16>(),
        )
    };
    let mut sections = Vec::new();
    if prompt_height > 0 {
        sections.push(VerticalSectionSize::Fixed(prompt_height));
    }
    sections.push(VerticalSectionSize::Fixed(input_height));
    sections.push(VerticalSectionSize::Flexible(panels_height));
    if footer_height > 0 {
        sections.push(VerticalSectionSize::Fixed(footer_height));
    }
    let frame_surface = render_framed_surface(
        frame,
        area,
        surface,
        &FramedSurfaceSpec {
            title: spec.title.clone(),
            target_width: spec.target_width,
            target_height: framed_sections_target_height(&sections),
        },
    );
    let rows = split_vertical_sections(frame_surface.inner, &sections);

    let mut row_index = 0;
    if prompt_height > 0 {
        frame.render_widget(
            Paragraph::new(spec.prompt.as_ref()).wrap(Wrap { trim: false }),
            rows[row_index],
        );
        row_index += 1;
    }

    let input_area = rows[row_index];
    row_index += 1;
    let result = render_editor_panel(
        frame,
        input_area,
        &EditorPanelSpec {
            title: None,
            borders: Borders::ALL,
        },
        input,
    );

    let panel_area = rows[row_index];
    row_index += 1;
    let (list_area, section_areas) = if stacked {
        let mut heights = Vec::with_capacity(1 + right_section_heights.len());
        heights.push(left_panel_height);
        heights.extend(right_section_heights.iter().copied());
        let areas = match surface {
            SurfaceMode::Overlay => top_aligned_vertical_areas(panel_area, &heights),
            SurfaceMode::Route => split_vertical_sections(
                panel_area,
                &heights
                    .iter()
                    .enumerate()
                    .map(|(index, height)| {
                        if index == 0 {
                            VerticalSectionSize::Flexible(*height)
                        } else {
                            VerticalSectionSize::Fixed(*height)
                        }
                    })
                    .collect::<Vec<_>>(),
            ),
        };
        (areas[0], areas[1..].to_vec())
    } else {
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(adaptive_detail_split(
                panel_area.width,
                spec.left_min_width,
                spec.right_min_width,
            ))
            .split(panel_area);
        let list_area = match surface {
            SurfaceMode::Overlay => top_aligned_panel_rect(split[0], left_panel_height),
            SurfaceMode::Route => split[0],
        };
        let section_areas = match surface {
            SurfaceMode::Overlay => top_aligned_vertical_areas(split[1], &right_section_heights),
            SurfaceMode::Route => split_vertical_sections(
                split[1],
                &right_section_heights
                    .iter()
                    .enumerate()
                    .map(|(index, height)| {
                        if index + 1 == right_section_heights.len() {
                            VerticalSectionSize::Flexible(*height)
                        } else {
                            VerticalSectionSize::Fixed(*height)
                        }
                    })
                    .collect::<Vec<_>>(),
            ),
        };
        (list_area, section_areas)
    };

    render_list_panel_state(frame, list_area, &spec.left_panel_state);

    for (section, section_area) in spec.right_sections.iter().zip(section_areas) {
        render_text_panel(
            frame,
            section_area,
            &TextPanelSpec {
                title: Some(section.title.clone()),
                body: &section.body,
                wrap: true,
                scroll: None,
                alignment: None,
            },
        );
    }

    if footer_height > 0 {
        frame.render_widget(
            Paragraph::new(spec.footer.as_ref()).wrap(Wrap { trim: false }),
            rows[row_index],
        );
    }

    frame.set_cursor_position(result.cursor);
}

pub fn render_search_panels_dialog<TItem, TMeta, FLeft, FRight>(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    dialog: &SearchPanelsOverlay<TItem, TMeta, Editor>,
    spec: &SearchPanelsDialogSpec<'_>,
    build_left_item: FLeft,
    build_right_sections: FRight,
) where
    FLeft: Fn(&TItem) -> ListItem<'static>,
    FRight: Fn(SearchPanelsDialogState<'_, TItem>) -> Vec<WorkbenchTextSection<'static>>,
{
    let left_items = dialog.items.iter().map(build_left_item).collect::<Vec<_>>();
    let panel_height = BoundedListPanelHeight {
        lines_per_item: spec.left_panel_lines_per_item,
        min_body_height: spec.left_panel_min_body_height,
        max_body_height: spec.left_panel_max_body_height,
    };
    let left_panel_state = if dialog.loading {
        ListPanelState::loading(
            Some(spec.left_panel_title.clone()),
            spec.loading_message.clone(),
            panel_height,
        )
    } else if dialog.items.is_empty() {
        ListPanelState::empty(
            Some(spec.left_panel_title.clone()),
            Cow::Owned(dialog.empty_message.clone()),
            panel_height,
        )
    } else {
        ListPanelState::items(
            panel_height,
            Some(spec.left_panel_title.clone()),
            left_items.as_slice(),
            dialog.list_selected_for_render(),
            spec.highlight_style,
            spec.highlight_symbol.clone(),
        )
    };
    let right_sections = build_right_sections(dialog.dialog_state(spec.loading_message.as_ref()));
    render_search_panels_overlay(
        frame,
        area,
        surface,
        &SearchPanelsSpec {
            title: Cow::Borrowed(dialog.title.as_str()),
            prompt: Cow::Borrowed(dialog.prompt.as_str()),
            footer: Cow::Borrowed(dialog.footer.as_str()),
            target_width: spec.target_width,
            left_min_width: spec.left_min_width,
            right_min_width: spec.right_min_width,
            left_panel_state,
            right_sections,
        },
        &dialog.input,
    );
}
