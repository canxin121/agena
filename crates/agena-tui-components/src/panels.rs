use std::borrow::Cow;

use crate::layout::list_panel_height;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::theme;

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
        let body_height = u16::try_from(
            panel
                .items
                .iter()
                .map(ListItem::height)
                .sum::<usize>()
                .max(1),
        )
        .unwrap_or(u16::MAX);
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
    let block = match spec.title.as_ref() {
        Some(title) => Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", title)),
        None => Block::default().borders(Borders::ALL),
    };
    let list = List::new(spec.items.iter().cloned())
        .block(block)
        .highlight_style(spec.highlight_style)
        .highlight_symbol(spec.highlight_symbol.as_ref());
    let mut state = empty_list_state();
    state.select(spec.selected);
    frame.render_stateful_widget(list, area, &mut state);
}

fn empty_list_state() -> ListState {
    ListState::default()
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
    let block = match spec.title.as_ref() {
        Some(title) => Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", title)),
        None => Block::default().borders(Borders::ALL),
    };
    frame.render_widget(paragraph.block(block), area);
}

/// Keep selected rows visible while making the active pane unambiguous.
pub fn panel_highlight_style(active: bool) -> Style {
    if active {
        theme::selection_style()
    } else {
        Style::default()
            .fg(theme::accent_color())
            .add_modifier(Modifier::BOLD)
    }
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
                theme::muted_style(),
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
        value_style: Style::default().fg(theme::accent_color()),
        detail_style: theme::muted_style(),
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

pub fn build_vertical_divider(height: u16) -> Text<'static> {
    Text::from((0..height).map(|_| Line::from("│")).collect::<Vec<_>>())
}

pub fn build_horizontal_divider(width: u16) -> Text<'static> {
    Text::from("─".repeat(usize::from(width)))
}

#[cfg(test)]
mod tests {
    use super::panel_highlight_style;

    #[test]
    fn inactive_panel_selection_keeps_an_accent_without_active_background() {
        let active = panel_highlight_style(true);
        let inactive = panel_highlight_style(false);

        assert_ne!(active, inactive);
        assert_eq!(inactive.fg, Some(crate::theme::accent_color()));
        assert_eq!(inactive.bg, None);
    }
}
