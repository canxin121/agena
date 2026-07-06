use std::borrow::Cow;

use ratatui::{Frame, layout::Rect, style::Style, text::Text, widgets::ListItem};

use crate::{
    ListPanelSection, ListPanelSpec, ParagraphSection, StackedDialogSection,
    StackedDialogSectionHeight, StackedDialogSpec, SurfaceMode, render_stacked_dialog,
};

pub struct DecisionDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub body: &'a Text<'a>,
    pub body_height_bounds: (u16, u16),
    pub list_title: Option<Cow<'a, str>>,
    pub items: &'a [ListItem<'a>],
    pub selected: Option<usize>,
    pub lines_per_item: u16,
    pub list_height_bounds: (u16, u16),
    pub highlight_style: Style,
    pub highlight_symbol: Cow<'a, str>,
    pub footer: &'a Text<'a>,
    pub footer_height_bounds: (u16, u16),
    pub target_width: u16,
}

impl<'a> DecisionDialogSpec<'a> {
    pub fn new(
        title: Cow<'a, str>,
        body: &'a Text<'a>,
        items: &'a [ListItem<'a>],
        footer: &'a Text<'a>,
        target_width: u16,
        highlight_style: Style,
        highlight_symbol: Cow<'a, str>,
    ) -> Self {
        Self {
            title,
            body,
            body_height_bounds: (1, 1),
            list_title: None,
            items,
            selected: None,
            lines_per_item: 1,
            list_height_bounds: (1, 1),
            highlight_style,
            highlight_symbol,
            footer,
            footer_height_bounds: (0, 0),
            target_width,
        }
    }

    pub fn with_body_height_bounds(mut self, body_height_bounds: (u16, u16)) -> Self {
        self.body_height_bounds = body_height_bounds;
        self
    }

    pub fn with_list_title(mut self, list_title: Cow<'a, str>) -> Self {
        self.list_title = Some(list_title);
        self
    }

    pub fn with_list_state(
        mut self,
        selected: Option<usize>,
        lines_per_item: u16,
        list_height_bounds: (u16, u16),
    ) -> Self {
        self.selected = selected;
        self.lines_per_item = lines_per_item;
        self.list_height_bounds = list_height_bounds;
        self
    }

    pub fn with_footer_height_bounds(mut self, footer_height_bounds: (u16, u16)) -> Self {
        self.footer_height_bounds = footer_height_bounds;
        self
    }
}

pub fn render_decision_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &DecisionDialogSpec<'_>,
) {
    render_stacked_dialog(
        frame,
        area,
        surface,
        &StackedDialogSpec {
            title: spec.title.clone(),
            target_width: spec.target_width,
            sections: vec![
                StackedDialogSection::Paragraph(ParagraphSection {
                    height: StackedDialogSectionHeight::AutoText {
                        min: spec.body_height_bounds.0,
                        max: spec.body_height_bounds.1,
                    },
                    title: None,
                    borders: ratatui::widgets::Borders::NONE,
                    body: spec.body.clone(),
                    wrap: true,
                    scroll: None,
                    alignment: None,
                }),
                StackedDialogSection::ListPanel(ListPanelSection {
                    height: StackedDialogSectionHeight::AutoList {
                        lines_per_item: spec.lines_per_item,
                        min_body: spec.list_height_bounds.0,
                        max_body: spec.list_height_bounds.1,
                    },
                    spec: ListPanelSpec {
                        title: spec.list_title.clone(),
                        items: spec.items,
                        selected: spec.selected,
                        highlight_style: spec.highlight_style,
                        highlight_symbol: spec.highlight_symbol.clone(),
                    },
                }),
                StackedDialogSection::Paragraph(ParagraphSection {
                    height: StackedDialogSectionHeight::AutoText {
                        min: spec.footer_height_bounds.0,
                        max: spec.footer_height_bounds.1,
                    },
                    title: None,
                    borders: ratatui::widgets::Borders::NONE,
                    body: spec.footer.clone(),
                    wrap: true,
                    scroll: None,
                    alignment: None,
                }),
            ],
        },
    );
}
