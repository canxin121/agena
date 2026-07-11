use std::cmp::min;

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Text},
    widgets::{Block, Borders},
};

use crate::{
    layout::{VerticalSectionSize, inset_rect, split_vertical_sections},
    text::{HeaderRowSpec, render_header_row, truncate_display_text_middle},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderBodyFooterLayout {
    pub header: Rect,
    pub body: Rect,
    pub footer: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposerSurfaceLayout {
    pub status: Option<Rect>,
    pub outer: Rect,
    pub inner: Rect,
    pub items: Option<Rect>,
    pub popup: Option<Rect>,
    pub editor: Rect,
}

pub struct HeaderBodyFooterTextSurfaceSpec<'a> {
    pub title: std::borrow::Cow<'a, str>,
    pub subtitle: Option<std::borrow::Cow<'a, str>>,
    pub right: Option<std::borrow::Cow<'a, str>>,
    pub body: Text<'a>,
    pub body_scroll: (u16, u16),
    pub body_wrap: bool,
    pub footer: Option<Text<'a>>,
    pub title_style: Style,
    pub subtitle_style: Style,
    pub right_style: Style,
}

pub struct ComposerEditorSurfaceSpec<'a> {
    pub editor_lines: Text<'a>,
    pub placeholder: Option<Line<'a>>,
    pub cursor: Option<(u16, u16)>,
}

pub fn pane_header_height(total_height: u16) -> u16 {
    min(3, total_height)
}

pub fn layout_header_body_footer_surface(
    area: Rect,
    header_height: u16,
    footer_height: u16,
    horizontal_inset: u16,
) -> HeaderBodyFooterLayout {
    let header_height = min(header_height, area.height.saturating_sub(1));
    let footer_height = min(
        footer_height,
        area.height.saturating_sub(header_height).saturating_sub(1),
    );
    let sections = if footer_height > 0 {
        vec![
            VerticalSectionSize::Fixed(header_height),
            VerticalSectionSize::Flexible(1),
            VerticalSectionSize::Fixed(footer_height),
        ]
    } else {
        vec![
            VerticalSectionSize::Fixed(header_height),
            VerticalSectionSize::Flexible(1),
        ]
    };
    let split = split_vertical_sections(area, &sections);
    let footer = if footer_height > 0 {
        inset_rect(split[2], horizontal_inset, 0)
    } else {
        Rect::default()
    };

    HeaderBodyFooterLayout {
        header: split[0],
        body: inset_rect(split[1], horizontal_inset, 0),
        footer,
    }
}

pub fn layout_composer_surface(
    area: Rect,
    status_rows: u16,
    item_rows: u16,
    popup_rows: u16,
) -> ComposerSurfaceLayout {
    let (status, outer) = if status_rows > 0 && area.height > status_rows {
        let rows = split_vertical_sections(
            area,
            &[
                VerticalSectionSize::Fixed(status_rows),
                VerticalSectionSize::Flexible(1),
            ],
        );
        (Some(rows[0]), rows[1])
    } else {
        (None, area)
    };

    let inner = Block::default().borders(Borders::ALL).inner(outer);
    if inner.width == 0 || inner.height == 0 {
        return ComposerSurfaceLayout {
            status,
            outer,
            inner,
            items: None,
            popup: None,
            editor: inner,
        };
    }

    let item_rows = item_rows.min(inner.height.saturating_sub(1));
    let popup_rows = min(
        popup_rows,
        inner.height.saturating_sub(item_rows).saturating_sub(1),
    );
    let editor_rows = inner
        .height
        .saturating_sub(item_rows)
        .saturating_sub(popup_rows)
        .max(1);

    let mut sections = Vec::new();
    if item_rows > 0 {
        sections.push(VerticalSectionSize::Fixed(item_rows));
    }
    if popup_rows > 0 {
        sections.push(VerticalSectionSize::Fixed(popup_rows));
    }
    sections.push(VerticalSectionSize::Fixed(editor_rows));
    let rows = split_vertical_sections(inner, &sections);

    let mut next_row = 0;
    let items = if item_rows > 0 {
        let row = Some(rows[next_row]);
        next_row += 1;
        row
    } else {
        None
    };
    let popup = if popup_rows > 0 {
        let row = Some(rows[next_row]);
        next_row += 1;
        row
    } else {
        None
    };

    ComposerSurfaceLayout {
        status,
        outer,
        inner,
        items,
        popup,
        editor: rows[next_row],
    }
}

pub fn render_header_body_footer_text_surface(
    frame: &mut Frame,
    layout: HeaderBodyFooterLayout,
    spec: &HeaderBodyFooterTextSurfaceSpec<'_>,
) {
    let header_frame = Block::default().borders(Borders::BOTTOM);
    let header_inner = inset_rect(header_frame.inner(layout.header), 1, 0);
    frame.render_widget(header_frame, layout.header);

    if header_inner.width > 0 && header_inner.height > 0 {
        let title_area = Rect {
            height: 1,
            ..header_inner
        };
        render_header_row(
            frame,
            title_area,
            &HeaderRowSpec {
                left: spec.title.clone(),
                right: spec.right.clone(),
                left_style: spec.title_style,
                right_style: spec.right_style,
            },
        );
        if header_inner.height > 1
            && let Some(subtitle) = spec.subtitle.as_deref()
            && !subtitle.trim().is_empty()
        {
            let subtitle_area = Rect {
                y: header_inner.y.saturating_add(1),
                height: 1,
                ..header_inner
            };
            frame.render_widget(
                ratatui::widgets::Paragraph::new(ratatui::text::Line::from(
                    ratatui::text::Span::styled(
                        truncate_display_text_middle(subtitle, subtitle_area.width as usize),
                        spec.subtitle_style,
                    ),
                )),
                subtitle_area,
            );
        }
    }

    if layout.body.width > 0 && layout.body.height > 0 {
        let mut paragraph =
            ratatui::widgets::Paragraph::new(spec.body.clone()).scroll(spec.body_scroll);
        if spec.body_wrap {
            paragraph = paragraph.wrap(ratatui::widgets::Wrap { trim: false });
        }
        frame.render_widget(paragraph, layout.body);
    }

    if layout.footer.width > 0
        && layout.footer.height > 0
        && let Some(footer) = spec.footer.as_ref()
    {
        frame.render_widget(
            ratatui::widgets::Paragraph::new(footer.clone())
                .wrap(ratatui::widgets::Wrap { trim: false }),
            layout.footer,
        );
    }
}

pub fn render_composer_editor_surface(
    frame: &mut Frame,
    layout: ComposerSurfaceLayout,
    spec: &ComposerEditorSurfaceSpec<'_>,
) {
    let block = Block::default().borders(Borders::ALL);
    frame.render_widget(block, layout.outer);

    if layout.inner.width == 0 || layout.inner.height == 0 {
        return;
    }

    let editor_width = layout.editor.width.saturating_sub(2).max(1);
    let editor_x = layout.editor.x.saturating_add(1);
    frame.render_widget(
        ratatui::widgets::Paragraph::new(spec.editor_lines.clone())
            .alignment(ratatui::layout::Alignment::Left),
        Rect {
            x: editor_x,
            y: layout.editor.y,
            width: editor_width,
            height: layout.editor.height,
        },
    );

    if let Some(placeholder) = spec.placeholder.as_ref() {
        frame.render_widget(
            ratatui::widgets::Paragraph::new(placeholder.clone()),
            Rect {
                x: editor_x,
                y: layout.editor.y,
                width: editor_width,
                height: 1,
            },
        );
    }

    if let Some((cursor_x, cursor_y)) = spec.cursor {
        frame.set_cursor_position((
            editor_x.saturating_add(cursor_x),
            layout.editor.y.saturating_add(cursor_y),
        ));
    }
}
