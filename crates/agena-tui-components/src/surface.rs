//! Shared surface geometry and focus state.

use std::cmp::min;

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    layout::{VerticalSectionSize, inset_rect, split_vertical_sections},
    text::{
        HeaderRowSpec, line_plain_text, render_header_row, truncate_display_text_middle,
        truncate_display_text_with_suffix,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderBodyFooterLayout {
    pub header: Rect,
    pub body: Rect,
    pub footer: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposerSurfaceLayout {
    pub outer: Rect,
    pub inner: Rect,
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
    /// Optional status chip rendered against the left corner of the
    /// composer's top border, breaking the horizontal line. The line's spans
    /// should carry the chip background so the border does not show through
    /// as a blank gap.
    pub status: Option<Line<'static>>,
    /// Optional status chip rendered against the top border's right corner.
    pub status_top_right: Option<Line<'static>>,
    /// Optional status chip rendered against the bottom border's left corner.
    pub status_bottom_left: Option<Line<'static>>,
    /// Optional status chip rendered against the bottom border's right corner.
    pub status_bottom_right: Option<Line<'static>>,
}

/// Geometry of the status chip drawn inside the composer's top border.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerStatusPlacement {
    /// The text actually drawn inside the chip (already truncated to fit).
    pub text: String,
    /// Display width of the full chip, including one-cell padding on each
    /// side of the text.
    pub chip_width: u16,
    /// Column of the chip's first cell (its leading padding cell).
    pub column: u16,
    /// Column where the visible text starts (one past [`Self::column`]).
    pub text_column: u16,
    /// Number of `─` cells drawn before the chip.
    pub left_dashes: u16,
    /// Number of `─` cells drawn after the chip.
    pub right_dashes: u16,
}

/// Computes where a status chip sits inside the composer's top border so the
/// renderer and the selection/copy projection agree on the exact cells.
/// Returns `None` when there is no room for a readable chip.
pub fn composer_status_placement(outer: Rect, text: &str) -> Option<ComposerStatusPlacement> {
    if outer.width < 4 || text.trim().is_empty() {
        return None;
    }

    // The corners occupy two cells; keep at least one dash on each side so
    // the chip reads as a deliberate break in the border.
    let max_text_width = usize::from(outer.width.saturating_sub(4));
    let text = if UnicodeWidthStr::width(text) > max_text_width {
        truncate_display_text_with_suffix(text, max_text_width, "…")
    } else {
        text.to_string()
    };
    if text.trim().is_empty() {
        return None;
    }

    let text_width = u16::try_from(UnicodeWidthStr::width(text.as_str())).unwrap_or(u16::MAX);
    let chip_width = text_width.saturating_add(2);
    let dashes = outer.width.saturating_sub(2).saturating_sub(chip_width);
    let left_dashes = dashes / 2;
    let right_dashes = dashes.saturating_sub(left_dashes);
    let column = outer.x.saturating_add(1).saturating_add(left_dashes);

    Some(ComposerStatusPlacement {
        text,
        chip_width,
        column,
        text_column: column.saturating_add(1),
        left_dashes,
        right_dashes,
    })
}

/// Computes where a status chip sits centered inside the composer's top
/// border while reserving `reserve_right` cells for a right-corner chip (the
/// reserved cells cover the corner chip's full width plus one separating
/// dash). Returns `None` when there is no room for a readable chip.
pub fn composer_status_placement_reserving(
    outer: Rect,
    text: &str,
    reserve_right: u16,
) -> Option<ComposerStatusPlacement> {
    if outer.width < 4 || text.trim().is_empty() {
        return None;
    }
    let reserved = if reserve_right > 0 {
        reserve_right.saturating_add(1)
    } else {
        0
    };
    let max_text_width = usize::from(outer.width.saturating_sub(4).saturating_sub(reserved));
    if max_text_width == 0 {
        return None;
    }
    let text = if UnicodeWidthStr::width(text) > max_text_width {
        truncate_display_text_with_suffix(text, max_text_width, "…")
    } else {
        text.to_string()
    };
    if text.trim().is_empty() {
        return None;
    }

    let text_width = u16::try_from(UnicodeWidthStr::width(text.as_str())).unwrap_or(u16::MAX);
    let chip_width = text_width.saturating_add(2);
    let dashes = outer
        .width
        .saturating_sub(2)
        .saturating_sub(reserved)
        .saturating_sub(chip_width);
    let left_dashes = dashes / 2;
    let right_dashes = dashes.saturating_sub(left_dashes);
    let column = outer.x.saturating_add(1).saturating_add(left_dashes);

    Some(ComposerStatusPlacement {
        text,
        chip_width,
        column,
        text_column: column.saturating_add(1),
        left_dashes,
        right_dashes,
    })
}

/// Computes where a status chip sits against the left corner of the
/// composer's top border while reserving `reserve_right` cells for a
/// right-corner chip (the reserved cells cover the corner chip's full width
/// plus one separating dash). Returns `None` when there is no room for a
/// readable chip.
pub fn composer_status_placement_left(
    outer: Rect,
    text: &str,
    reserve_right: u16,
) -> Option<ComposerStatusPlacement> {
    if outer.width < 4 || text.trim().is_empty() {
        return None;
    }
    let reserved = if reserve_right > 0 {
        reserve_right.saturating_add(1)
    } else {
        0
    };
    let max_text_width = usize::from(outer.width.saturating_sub(4).saturating_sub(reserved));
    if max_text_width == 0 {
        return None;
    }
    let text = if UnicodeWidthStr::width(text) > max_text_width {
        truncate_display_text_with_suffix(text, max_text_width, "…")
    } else {
        text.to_string()
    };
    if text.trim().is_empty() {
        return None;
    }

    let text_width = u16::try_from(UnicodeWidthStr::width(text.as_str())).unwrap_or(u16::MAX);
    let chip_width = text_width.saturating_add(2);
    let column = outer.x.saturating_add(1);
    Some(ComposerStatusPlacement {
        text,
        chip_width,
        column,
        text_column: column.saturating_add(1),
        left_dashes: 0,
        right_dashes: outer
            .width
            .saturating_sub(2)
            .saturating_sub(reserved)
            .saturating_sub(chip_width),
    })
}

/// Computes where a status chip sits against the left corner of a border
/// (used for the composer's bottom-left corner). The chip starts one cell
/// after the corner and extends rightward; text is truncated to fit.
pub fn composer_corner_placement_left(outer: Rect, text: &str) -> Option<ComposerStatusPlacement> {
    if outer.width < 4 || text.trim().is_empty() {
        return None;
    }
    let max_text_width = usize::from(outer.width.saturating_sub(3));
    let text = if UnicodeWidthStr::width(text) > max_text_width {
        truncate_display_text_with_suffix(text, max_text_width, "…")
    } else {
        text.to_string()
    };
    if text.trim().is_empty() {
        return None;
    }

    let text_width = u16::try_from(UnicodeWidthStr::width(text.as_str())).unwrap_or(u16::MAX);
    let chip_width = text_width.saturating_add(2);
    let column = outer.x.saturating_add(1);
    Some(ComposerStatusPlacement {
        text,
        chip_width,
        column,
        text_column: column.saturating_add(1),
        left_dashes: 0,
        right_dashes: outer.width.saturating_sub(2).saturating_sub(chip_width),
    })
}

/// Computes where a status chip sits against the right corner of a border
/// (used for the composer's top-right and bottom-right corners). The chip
/// ends one cell before the corner and extends leftward; text is truncated
/// to fit.
pub fn composer_corner_placement_right(outer: Rect, text: &str) -> Option<ComposerStatusPlacement> {
    if outer.width < 4 || text.trim().is_empty() {
        return None;
    }
    let max_text_width = usize::from(outer.width.saturating_sub(3));
    let text = if UnicodeWidthStr::width(text) > max_text_width {
        truncate_display_text_with_suffix(text, max_text_width, "…")
    } else {
        text.to_string()
    };
    if text.trim().is_empty() {
        return None;
    }

    let text_width = u16::try_from(UnicodeWidthStr::width(text.as_str())).unwrap_or(u16::MAX);
    let chip_width = text_width.saturating_add(2);
    let column = outer
        .x
        .saturating_add(outer.width.saturating_sub(1).saturating_sub(chip_width));
    Some(ComposerStatusPlacement {
        text,
        chip_width,
        column,
        text_column: column.saturating_add(1),
        left_dashes: column.saturating_sub(outer.x.saturating_add(1)),
        right_dashes: 0,
    })
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

/// Computes the composer surface geometry. The status chip lives on the top
/// border row, and there is no separate item/popup row, so the editor simply
/// fills the box interior.
pub fn layout_composer_surface(area: Rect) -> ComposerSurfaceLayout {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    ComposerSurfaceLayout {
        outer: area,
        inner,
        editor: inner,
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
    let status = spec.status.as_ref().filter(|line| !line.spans.is_empty());
    let top_right = spec
        .status_top_right
        .as_ref()
        .filter(|line| !line.spans.is_empty());
    let bottom_left = spec
        .status_bottom_left
        .as_ref()
        .filter(|line| !line.spans.is_empty());
    let bottom_right = spec
        .status_bottom_right
        .as_ref()
        .filter(|line| !line.spans.is_empty());

    let top_right_placement = top_right.and_then(|line| {
        composer_corner_placement_right(layout.outer, line_plain_text(line).as_str())
    });
    let status_placement = status.and_then(|line| {
        composer_status_placement_left(
            layout.outer,
            line_plain_text(line).as_str(),
            top_right_placement
                .as_ref()
                .map(|p| p.chip_width)
                .unwrap_or(0),
        )
    });
    let bottom_left_placement = bottom_left.and_then(|line| {
        composer_corner_placement_left(layout.outer, line_plain_text(line).as_str())
    });
    let bottom_right_placement = bottom_right.and_then(|line| {
        composer_corner_placement_right(layout.outer, line_plain_text(line).as_str())
    });

    // The horizontal border rows are drawn by hand so the left-aligned status
    // chip and the corner chips all break the line; the block only paints the
    // two vertical edges.
    let block = Block::default().borders(Borders::LEFT | Borders::RIGHT);
    frame.render_widget(block, layout.outer);

    let mut top_chips = Vec::new();
    if let (Some(line), Some(placement)) = (status, status_placement) {
        top_chips.push((placement, line.clone()));
    }
    if let (Some(line), Some(placement)) = (top_right, top_right_placement) {
        top_chips.push((placement, line.clone()));
    }
    render_composer_border_row(frame, layout.outer, "┌", "┐", &top_chips);

    let mut bottom_chips = Vec::new();
    if let (Some(line), Some(placement)) = (bottom_left, bottom_left_placement) {
        bottom_chips.push((placement, line.clone()));
    }
    if let (Some(line), Some(placement)) = (bottom_right, bottom_right_placement) {
        bottom_chips.push((placement, line.clone()));
    }
    let bottom_row = Rect {
        x: layout.outer.x,
        y: layout
            .outer
            .y
            .saturating_add(layout.outer.height.saturating_sub(1)),
        width: layout.outer.width,
        height: 1,
    };
    render_composer_border_row(frame, bottom_row, "└", "┘", &bottom_chips);

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

/// Draws one horizontal composer border row as `┌── chip ── chip ┐` (or the
/// matching `└ … ┘` row) so status chips visibly break the line instead of
/// floating on a separate row or leaving a blank gap. Chips must be ordered
/// left to right; their geometry comes from the placement functions above.
/// Each chip's spans are taken from its line; the renderer adds one-cell
/// padding on each side using the line's first span style so the chip
/// background covers the border behind it.
fn render_composer_border_row(
    frame: &mut Frame,
    outer: Rect,
    left_corner: &str,
    right_corner: &str,
    chips: &[(ComposerStatusPlacement, Line<'static>)],
) {
    let mut spans = Vec::new();
    spans.push(Span::styled(left_corner.to_string(), Style::default()));
    let mut cursor = outer.x.saturating_add(1);
    for (placement, line) in chips {
        if placement.column > cursor {
            spans.push(Span::styled(
                "─".repeat(usize::from(placement.column.saturating_sub(cursor))),
                Style::default(),
            ));
        }
        let pad_style = line
            .spans
            .first()
            .map(|span| span.style)
            .unwrap_or_default();
        spans.push(Span::styled(" ", pad_style));
        spans.extend(line.spans.iter().cloned());
        spans.push(Span::styled(" ", pad_style));
        cursor = placement.column.saturating_add(placement.chip_width);
    }
    let right_corner_column = outer.x.saturating_add(outer.width.saturating_sub(1));
    if right_corner_column > cursor {
        spans.push(Span::styled(
            "─".repeat(usize::from(right_corner_column.saturating_sub(cursor))),
            Style::default(),
        ));
    }
    spans.push(Span::styled(right_corner.to_string(), Style::default()));
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect {
            x: outer.x,
            y: outer.y,
            width: outer.width,
            height: 1,
        },
    );
}
