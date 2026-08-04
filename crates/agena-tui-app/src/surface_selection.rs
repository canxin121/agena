//! Mouse-based text selection for the read-only chat surfaces that sit
//! outside the transcript body: the session header (title and subtitle rows),
//! the composer status row (model name etc.), and the composer editor itself.
//!
//! The transcript already supports pointer selection; these surfaces used to
//! swallow mouse events without doing anything, so a user could never copy the
//! session name, session path, model name, or the composer draft text. This
//! module adds a small, self-contained selection model for those rows and
//! turns a committed gesture into clean clipboard text (via the app's OSC 52
//! copy path).

use std::ops::Range;

use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceSelectionKind {
    HeaderTitle,
    HeaderSubtitle,
    ComposerStatus,
    ComposerEditor,
}

impl SurfaceSelectionKind {
    pub(crate) const ALL: [SurfaceSelectionKind; 4] = [
        SurfaceSelectionKind::HeaderTitle,
        SurfaceSelectionKind::HeaderSubtitle,
        SurfaceSelectionKind::ComposerStatus,
        SurfaceSelectionKind::ComposerEditor,
    ];
}

/// A committed pointer selection over one of the chat surfaces.
///
/// Coordinates are absolute terminal cells `(column, row)` so the gesture can
/// be tracked across frames without depending on the current layout; the
/// displayed lines are re-projected from the live surface at copy/render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfaceSelection {
    pub(crate) kind: SurfaceSelectionKind,
    pub(crate) anchor: (u16, u16),
    pub(crate) head: (u16, u16),
}

/// Geometry of the selectable chat surfaces for the current frame.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SurfaceLayout {
    pub(crate) header_title: Rect,
    pub(crate) header_subtitle: Rect,
    pub(crate) composer_status: Rect,
    pub(crate) composer_editor: Rect,
}

impl SurfaceLayout {
    pub(crate) fn rect_for(&self, kind: SurfaceSelectionKind) -> Rect {
        match kind {
            SurfaceSelectionKind::HeaderTitle => self.header_title,
            SurfaceSelectionKind::HeaderSubtitle => self.header_subtitle,
            SurfaceSelectionKind::ComposerStatus => self.composer_status,
            SurfaceSelectionKind::ComposerEditor => self.composer_editor,
        }
    }

    /// The topmost selectable surface containing the cell, if any. Surfaces
    /// never overlap in the main layout, but the header rows and composer are
    /// deliberately checked before the transcript body so clicks on them are
    /// not treated as transcript gestures.
    pub(crate) fn kind_at(&self, column: u16, row: u16) -> Option<SurfaceSelectionKind> {
        SurfaceSelectionKind::ALL.into_iter().find(|kind| {
            let rect = self.rect_for(*kind);
            rect.width > 0
                && rect.height > 0
                && column >= rect.x
                && column < rect.x.saturating_add(rect.width)
                && row >= rect.y
                && row < rect.y.saturating_add(rect.height)
        })
    }
}

/// One displayed line of a selectable surface: the plain text plus the
/// absolute cell coordinate where it starts.
#[derive(Debug, Clone)]
pub(crate) struct SurfaceDisplayLine {
    pub(crate) text: String,
    pub(crate) row: u16,
    pub(crate) column: u16,
}

impl SurfaceDisplayLine {
    pub(crate) fn width(&self) -> u16 {
        UnicodeWidthStr::width(self.text.as_str()) as u16
    }
}

/// Per-line selected cell ranges (in absolute columns) for a committed
/// selection. Lines outside the selection yield `None`.
pub(crate) fn surface_selection_ranges(
    lines: &[SurfaceDisplayLine],
    selection: &SurfaceSelection,
) -> Vec<Option<Range<u16>>> {
    let (anchor, head) = (selection.anchor, selection.head);
    let (start_row, start_col, end_row, end_col) = if (anchor.1, anchor.0) <= (head.1, head.0) {
        (anchor.1, anchor.0, head.1, head.0)
    } else {
        (head.1, head.0, anchor.1, anchor.0)
    };

    lines
        .iter()
        .map(|line| {
            if line.row < start_row || line.row > end_row {
                return None;
            }
            let line_end = line.column.saturating_add(line.width());
            let from = if line.row == start_row {
                start_col
            } else {
                line.column
            };
            let to = if line.row == end_row {
                end_col
            } else {
                line_end
            };
            let from = from.clamp(line.column, line_end);
            let to = to.clamp(line.column, line_end);
            (from < to).then_some(from..to)
        })
        .collect()
}

/// Clean clipboard text for a committed selection over a surface's displayed
/// lines. Wide graphemes are kept whole and lines are joined with newlines.
pub(crate) fn surface_selection_text(
    lines: &[SurfaceDisplayLine],
    selection: &SurfaceSelection,
) -> String {
    let ranges = surface_selection_ranges(lines, selection);
    let mut fragments = Vec::new();
    for (line, range) in lines.iter().zip(ranges) {
        if let Some(range) = range {
            let local = usize::from(range.start.saturating_sub(line.column))
                ..usize::from(range.end.saturating_sub(line.column));
            fragments.push(display_cell_slice(line.text.as_str(), local));
        }
    }
    fragments.join("\n")
}

fn display_cell_slice(text: &str, range: Range<usize>) -> String {
    let mut column = 0_usize;
    text.graphemes(true)
        .filter(|grapheme| {
            let width = UnicodeWidthStr::width(*grapheme);
            let start = column;
            let end = column.saturating_add(width);
            column = end;
            start < range.end && end > range.start
        })
        .collect()
}

/// Apply an inverse-video highlight to the cells of a line that fall inside
/// `selected` (absolute columns). Spans are split at grapheme boundaries so
/// only the selected cells change style; unselected spans keep their styling.
pub(crate) fn apply_cell_range_highlight(
    mut line: Line<'static>,
    selected: Option<Range<u16>>,
) -> Line<'static> {
    let Some(selected) = selected else {
        return line;
    };
    if line.spans.is_empty() || selected.start >= selected.end {
        return line;
    }

    let mut column = 0_u16;
    let mut spans = Vec::new();
    for span in std::mem::take(&mut line.spans) {
        let span_width = UnicodeWidthStr::width(span.content.as_ref()) as u16;
        let span_start = column;
        let span_end = column.saturating_add(span_width);
        column = span_end;

        if span_end <= selected.start || span_start >= selected.end {
            spans.push(span);
            continue;
        }
        spans.extend(split_span_by_selection(span, span_start, &selected));
    }
    line.spans = spans;
    line
}

fn split_span_by_selection(
    span: Span<'static>,
    span_start: u16,
    selected: &Range<u16>,
) -> Vec<Span<'static>> {
    let mut prefix = String::new();
    let mut highlighted = String::new();
    let mut suffix = String::new();
    let mut column = span_start;
    let mut started = false;
    let mut finished = false;

    for grapheme in span.content.graphemes(true) {
        let width = UnicodeWidthStr::width(grapheme) as u16;
        let grapheme_start = column;
        column = column.saturating_add(width);
        if grapheme_start >= selected.end {
            finished = true;
        }
        if finished {
            suffix.push_str(grapheme);
        } else if grapheme_start.saturating_add(width) > selected.start
            && grapheme_start < selected.end
        {
            started = true;
            highlighted.push_str(grapheme);
        } else if started {
            finished = true;
            suffix.push_str(grapheme);
        } else {
            prefix.push_str(grapheme);
        }
    }

    let style = span.style;
    let mut spans = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix, style));
    }
    if !highlighted.is_empty() {
        spans.push(Span::styled(
            highlighted,
            style.add_modifier(Modifier::REVERSED),
        ));
    }
    if !suffix.is_empty() {
        spans.push(Span::styled(suffix, style));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, row: u16, column: u16) -> SurfaceDisplayLine {
        SurfaceDisplayLine {
            text: text.to_owned(),
            row,
            column,
        }
    }

    fn selection(
        kind: SurfaceSelectionKind,
        anchor: (u16, u16),
        head: (u16, u16),
    ) -> SurfaceSelection {
        SurfaceSelection { kind, anchor, head }
    }

    #[test]
    fn single_line_selection_slices_by_display_cells() {
        let lines = vec![line("session title", 3, 2)];
        let selection = selection(SurfaceSelectionKind::HeaderTitle, (2, 3), (8, 3));
        assert_eq!(surface_selection_text(&lines, &selection), "sessio");
        assert_eq!(
            surface_selection_ranges(&lines, &selection),
            vec![Some(2..8)]
        );
    }

    #[test]
    fn reversed_gesture_and_out_of_bounds_head_are_normalized() {
        let lines = vec![line("abcdef", 1, 0)];
        let backward = selection(SurfaceSelectionKind::ComposerStatus, (4, 1), (1, 1));
        assert_eq!(surface_selection_text(&lines, &backward), "bcd");
        let overflow = selection(SurfaceSelectionKind::ComposerStatus, (2, 1), (99, 1));
        assert_eq!(surface_selection_text(&lines, &overflow), "cdef");
    }

    #[test]
    fn multi_line_selection_joins_wrapped_rows_with_newlines() {
        let lines = vec![line("one two", 5, 0), line("three", 6, 0)];
        let selection = selection(SurfaceSelectionKind::ComposerEditor, (4, 5), (3, 6));
        assert_eq!(surface_selection_text(&lines, &selection), "two\nthr");
    }

    #[test]
    fn layout_hit_testing_finds_the_expected_surface() {
        let layout = SurfaceLayout {
            header_title: Rect::new(1, 1, 60, 1),
            header_subtitle: Rect::new(1, 2, 60, 1),
            composer_status: Rect::new(1, 20, 60, 1),
            composer_editor: Rect::new(2, 22, 58, 3),
        };
        assert_eq!(
            layout.kind_at(10, 1),
            Some(SurfaceSelectionKind::HeaderTitle)
        );
        assert_eq!(
            layout.kind_at(10, 2),
            Some(SurfaceSelectionKind::HeaderSubtitle)
        );
        assert_eq!(
            layout.kind_at(30, 20),
            Some(SurfaceSelectionKind::ComposerStatus)
        );
        assert_eq!(
            layout.kind_at(30, 23),
            Some(SurfaceSelectionKind::ComposerEditor)
        );
        assert_eq!(layout.kind_at(30, 10), None);
        assert_eq!(layout.kind_at(0, 0), None);
    }

    #[test]
    fn highlight_only_marks_selected_cells_and_keeps_wide_graphemes_whole() {
        let line = Line::from(Span::styled("ab你c", ratatui::style::Style::default()));
        let highlighted = apply_cell_range_highlight(line, Some(2..4));
        let contents = highlighted
            .spans
            .iter()
            .map(|span| (span.content.as_ref(), span.style.add_modifier))
            .collect::<Vec<_>>();
        assert_eq!(
            contents,
            vec![
                ("ab", Modifier::empty()),
                ("你", Modifier::REVERSED),
                ("c", Modifier::empty()),
            ]
        );
    }
}
