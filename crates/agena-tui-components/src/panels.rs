use std::borrow::Cow;

use crate::layout::list_panel_height;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

pub trait ListPanelHeightResolver {
    fn resolve_placeholder_height(&self) -> u16;
    fn resolve_items_height(&self, panel: &ListPanelSpec<'_>) -> u16;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundedListPanelHeight {
    pub lines_per_item: u16,
    pub min_body_height: u16,
    pub max_body_height: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeasuredListPanelHeight {
    pub min_body_height: u16,
    pub max_body_height: u16,
}

impl ListPanelHeightResolver for BoundedListPanelHeight {
    fn resolve_placeholder_height(&self) -> u16 {
        list_panel_height(1, 1, self.min_body_height, self.max_body_height)
    }

    fn resolve_items_height(&self, panel: &ListPanelSpec<'_>) -> u16 {
        list_panel_height(
            panel.items.len().max(1),
            self.lines_per_item,
            self.min_body_height,
            self.max_body_height,
        )
    }
}

impl ListPanelHeightResolver for MeasuredListPanelHeight {
    fn resolve_placeholder_height(&self) -> u16 {
        list_panel_height(1, 1, self.min_body_height, self.max_body_height)
    }

    fn resolve_items_height(&self, panel: &ListPanelSpec<'_>) -> u16 {
        let body_height = panel
            .items
            .iter()
            .map(ListItem::height)
            .sum::<usize>()
            .max(1) as u16;
        body_height
            .clamp(self.min_body_height, self.max_body_height)
            .saturating_add(2)
    }
}

#[derive(Clone)]
pub struct ListPanelSpec<'a> {
    pub title: Option<Cow<'a, str>>,
    pub items: &'a [ListItem<'a>],
    pub selected: Option<usize>,
    pub highlight_style: Style,
    pub highlight_symbol: Cow<'a, str>,
}

#[derive(Clone)]
pub struct TextPanelSpec<'a> {
    pub title: Option<Cow<'a, str>>,
    pub body: &'a Text<'a>,
    pub wrap: bool,
    pub scroll: Option<(u16, u16)>,
    pub alignment: Option<Alignment>,
}

#[derive(Clone)]
pub struct TwoLineListItemSpec<'a> {
    pub label: Cow<'a, str>,
    pub value: Option<Cow<'a, str>>,
    pub detail: Option<Cow<'a, str>>,
    pub label_style: Style,
    pub value_style: Style,
    pub detail_style: Style,
    pub separator: Cow<'a, str>,
}

#[derive(Clone)]
pub enum ListPanelState<'a, H> {
    Loading {
        title: Option<Cow<'a, str>>,
        message: Cow<'a, str>,
        panel_height: H,
    },
    Empty {
        title: Option<Cow<'a, str>>,
        message: Cow<'a, str>,
        panel_height: H,
    },
    Items {
        panel_height: H,
        panel: ListPanelSpec<'a>,
    },
}

impl<'a> ListPanelSpec<'a> {
    pub fn new(
        title: Option<Cow<'a, str>>,
        items: &'a [ListItem<'a>],
        selected: Option<usize>,
        highlight_style: Style,
        highlight_symbol: Cow<'a, str>,
    ) -> Self {
        Self {
            title,
            items,
            selected,
            highlight_style,
            highlight_symbol,
        }
    }
}

impl<'a, H> ListPanelState<'a, H> {
    pub fn loading(title: Option<Cow<'a, str>>, message: Cow<'a, str>, panel_height: H) -> Self {
        Self::Loading {
            title,
            message,
            panel_height,
        }
    }

    pub fn empty(title: Option<Cow<'a, str>>, message: Cow<'a, str>, panel_height: H) -> Self {
        Self::Empty {
            title,
            message,
            panel_height,
        }
    }

    pub fn items(
        panel_height: H,
        title: Option<Cow<'a, str>>,
        items: &'a [ListItem<'a>],
        selected: Option<usize>,
        highlight_style: Style,
        highlight_symbol: Cow<'a, str>,
    ) -> Self {
        Self::Items {
            panel_height,
            panel: ListPanelSpec::new(title, items, selected, highlight_style, highlight_symbol),
        }
    }
}

pub fn render_list_panel(frame: &mut Frame, area: Rect, spec: &ListPanelSpec<'_>) {
    let mut block = Block::default().borders(Borders::ALL);
    if let Some(title) = spec.title.as_ref() {
        block = block.title(format!(" {} ", title));
    }
    let list = List::new(spec.items.iter().cloned())
        .block(block)
        .highlight_style(spec.highlight_style)
        .highlight_symbol(spec.highlight_symbol.as_ref());
    let mut state = ListState::default();
    state.select(spec.selected);
    frame.render_stateful_widget(list, area, &mut state);
}

pub fn render_text_panel(frame: &mut Frame, area: Rect, spec: &TextPanelSpec<'_>) {
    let mut paragraph = Paragraph::new(spec.body.clone());
    if spec.wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    if let Some(scroll) = spec.scroll {
        paragraph = paragraph.scroll(scroll);
    }
    if let Some(alignment) = spec.alignment {
        paragraph = paragraph.alignment(alignment);
    }
    let mut block = Block::default().borders(Borders::ALL);
    if let Some(title) = spec.title.as_ref() {
        block = block.title(format!(" {} ", title));
    }
    frame.render_widget(paragraph.block(block), area);
}

impl<H: ListPanelHeightResolver> ListPanelState<'_, H> {
    pub fn resolve_height(&self) -> u16 {
        match self {
            Self::Loading { panel_height, .. } | Self::Empty { panel_height, .. } => {
                panel_height.resolve_placeholder_height()
            }
            Self::Items {
                panel_height,
                panel,
            } => panel_height.resolve_items_height(panel),
        }
    }
}

pub fn render_list_panel_state<H>(frame: &mut Frame, area: Rect, state: &ListPanelState<'_, H>) {
    match state {
        ListPanelState::Loading { title, message, .. }
        | ListPanelState::Empty { title, message, .. } => {
            let items = [ListItem::new(Line::from(Span::styled(
                message.as_ref(),
                Style::default().fg(Color::DarkGray),
            )))];
            render_list_panel(
                frame,
                area,
                &ListPanelSpec::new(title.clone(), &items, None, Style::default(), "> ".into()),
            );
        }
        ListPanelState::Items { panel, .. } => render_list_panel(frame, area, panel),
    }
}

pub fn build_two_line_list_item<'a>(spec: TwoLineListItemSpec<'a>) -> ListItem<'a> {
    let mut first_line = vec![Span::styled(spec.label, spec.label_style)];
    if let Some(value) = spec.value.filter(|value| !value.trim().is_empty()) {
        first_line.push(Span::raw(spec.separator));
        first_line.push(Span::styled(value, spec.value_style));
    }

    let mut lines = vec![Line::from(first_line)];
    if let Some(detail) = spec.detail {
        lines.push(Line::from(Span::styled(detail, spec.detail_style)));
    }
    ListItem::new(lines)
}

pub fn build_accented_two_line_list_item<'a>(
    label: Cow<'a, str>,
    value: Option<Cow<'a, str>>,
    detail: Option<Cow<'a, str>>,
) -> ListItem<'a> {
    build_two_line_list_item(TwoLineListItemSpec {
        label,
        value,
        detail,
        label_style: Style::default().add_modifier(Modifier::BOLD),
        value_style: Style::default().fg(Color::Cyan),
        detail_style: Style::default().fg(Color::DarkGray),
        separator: Cow::Borrowed("  "),
    })
}

pub fn build_detail_two_line_list_item<'a>(
    label: Cow<'a, str>,
    detail: Option<Cow<'a, str>>,
    detail_style: Style,
) -> ListItem<'a> {
    build_two_line_list_item(TwoLineListItemSpec {
        label,
        value: None,
        detail,
        label_style: Style::default(),
        value_style: Style::default(),
        detail_style,
        separator: Cow::Borrowed("  "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_line_list_item_omits_empty_value_and_optional_detail() {
        let spec = TwoLineListItemSpec {
            label: Cow::Borrowed("Name"),
            value: Some(Cow::Borrowed("")),
            detail: Some(Cow::Borrowed("Detail")),
            label_style: Style::default(),
            value_style: Style::default(),
            detail_style: Style::default(),
            separator: Cow::Borrowed("  "),
        };
        let item = build_two_line_list_item(spec);
        assert_eq!(item.height(), 2);
    }

    #[test]
    fn accented_two_line_list_item_renders_two_rows_with_value() {
        let item = build_accented_two_line_list_item(
            Cow::Borrowed("Name"),
            Some(Cow::Borrowed("Value")),
            Some(Cow::Borrowed("Detail")),
        );
        assert_eq!(item.height(), 2);
    }

    #[test]
    fn measured_list_panel_height_uses_rendered_item_heights() {
        let items = [
            ListItem::new(vec![Line::from("one"), Line::from("two")]),
            ListItem::new(Line::from("three")),
        ];
        let panel = ListPanelSpec {
            title: None,
            items: &items,
            selected: Some(0),
            highlight_style: Style::default(),
            highlight_symbol: Cow::Borrowed(">> "),
        };
        let height = MeasuredListPanelHeight {
            min_body_height: 2,
            max_body_height: 8,
        }
        .resolve_items_height(&panel);

        assert_eq!(height, 5);
    }
}
