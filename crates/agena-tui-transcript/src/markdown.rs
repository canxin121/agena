//! Markdown rendering pipeline for transcript content.

use crate::RenderedLine;
use crate::math::{
    display_math_source, inline_math_unicode_text, push_inline_math, push_math_block,
};
pub use agena_tui_components::text::{sanitize_terminal_text, strip_terminal_ansi_sequences};
use agena_tui_components::{line_plain_text, trim_empty_line_edges};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use tui_markdown::from_str as markdown_to_text;
use unicode_linebreak::linebreaks;
use unicode_width::UnicodeWidthStr;

pub fn push_multiline(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    style: Style,
    width: u16,
) {
    let sanitized = sanitize_terminal_text(text);
    let normalized = trim_empty_line_edges(sanitized.as_str());
    if normalized.is_empty() {
        return;
    }
    for raw_line in normalized.split('\n') {
        push_wrapped_line(out, prefix, prefix, raw_line, style, width);
    }
}

pub fn push_markdown(out: &mut Vec<RenderedLine>, prefix: &str, text: &str, width: u16) {
    let sanitized = sanitize_terminal_text(text);
    let markdown = trim_empty_line_edges(sanitized.as_str());
    if markdown.is_empty() {
        return;
    }
    if display_math_source(&markdown).is_some() {
        push_math_block(out, prefix, &markdown, width);
        return;
    }
    if !markdown.contains('\n') && push_inline_math(out, prefix, &markdown, width) {
        return;
    }

    let lines = markdown.lines().collect::<Vec<_>>();
    let mut chunk = Vec::<&str>::new();
    let mut active_fence = None::<MarkdownFence>;
    let mut index = 0_usize;

    while index < lines.len() {
        let line = lines[index];
        if let Some(delimiter) = markdown_fence_delimiter(line) {
            if let Some(active) = active_fence {
                if delimiter.marker == active.marker && delimiter.len >= active.len {
                    active_fence = None;
                }
            } else {
                active_fence = Some(delimiter);
            }
            chunk.push(line);
            index += 1;
            continue;
        }

        if active_fence.is_none()
            && index + 1 < lines.len()
            && is_markdown_table_header(lines[index], lines[index + 1])
        {
            flush_markdown_chunk(out, prefix, &mut chunk, width);
            let mut table_lines = vec![lines[index], lines[index + 1]];
            index += 2;
            while index < lines.len() && looks_like_markdown_table_row(lines[index]) {
                table_lines.push(lines[index]);
                index += 1;
            }
            push_markdown_table(out, prefix, table_lines.as_slice(), width);
            continue;
        }

        chunk.push(line);
        index += 1;
    }

    flush_markdown_chunk(out, prefix, &mut chunk, width);
}

pub fn push_wrapped_line(
    out: &mut Vec<RenderedLine>,
    initial_prefix: &str,
    continuation_prefix: &str,
    text: &str,
    style: Style,
    width: u16,
) {
    // Share the display-cell and Unicode break rules with Markdown prose.
    // In particular, long identifiers must wrap instead of being clipped.
    for (index, logical_line) in text.split('\n').enumerate() {
        let prefix = if index == 0 {
            initial_prefix
        } else {
            continuation_prefix
        };
        push_wrapped_rich_line(
            out,
            prefix,
            continuation_prefix,
            Line::styled(logical_line.to_owned(), style),
            width,
        );
    }
}

pub fn push_wrapped_rich_line(
    out: &mut Vec<RenderedLine>,
    initial_prefix: &str,
    continuation_prefix: &str,
    line: Line<'static>,
    width: u16,
) {
    let line_style = line.style;
    let line_alignment = line.alignment;
    let initial_prefix_width = UnicodeWidthStr::width(initial_prefix);
    if line.spans.is_empty() {
        out.push(
            RenderedLine::rich(Line {
                style: line_style,
                alignment: line_alignment,
                spans: vec![Span::raw(initial_prefix.to_string())],
            })
            .with_copy_projection(String::new(), initial_prefix_width),
        );
        return;
    }

    let plain_text = line_plain_text(&line);
    let available_width = width.max(1) as usize;
    let continuation_prefix_width = UnicodeWidthStr::width(continuation_prefix);
    if available_width <= initial_prefix_width.max(continuation_prefix_width) {
        out.push(
            RenderedLine::rich(prefix_rich_line(initial_prefix, line))
                .with_copy_projection(plain_text, initial_prefix_width),
        );
        return;
    }

    let wrapped_lines = wrap_rich_line(
        line.spans.as_slice(),
        available_width.saturating_sub(initial_prefix_width).max(1),
        available_width
            .saturating_sub(continuation_prefix_width)
            .max(1),
    );
    if wrapped_lines.is_empty() {
        out.push(
            RenderedLine::rich(prefix_rich_line(initial_prefix, line))
                .with_copy_projection(plain_text, initial_prefix_width),
        );
        return;
    }

    for (index, wrapped_line) in wrapped_lines.into_iter().enumerate() {
        let prefix = if index == 0 {
            initial_prefix
        } else {
            continuation_prefix
        };
        let copy_text = wrapped_line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        out.push(
            RenderedLine::rich(prefix_rich_line(
                prefix,
                Line {
                    style: line_style,
                    alignment: line_alignment,
                    spans: wrapped_line.spans,
                },
            ))
            .with_copy_projection(copy_text, UnicodeWidthStr::width(prefix)),
        );
    }
}

pub fn prefix_rich_line(prefix: &str, line: Line<'static>) -> Line<'static> {
    if prefix.is_empty() {
        return line;
    }
    let style = line.style;
    let alignment = line.alignment;
    let mut spans = Vec::with_capacity(line.spans.len().saturating_add(1));
    spans.push(Span::raw(prefix.to_string()));
    spans.extend(line.spans);
    Line {
        style,
        alignment,
        spans,
    }
}

pub fn owned_line(line: &Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .iter()
            .map(|span| Span::styled(span.content.to_string(), span.style))
            .collect::<Vec<_>>(),
    }
}

/// Bound a rich inline line without discarding per-span Markdown styling.
/// This is the rich equivalent of `truncate_display_width` and is primarily
/// used by one-line Activity headlines.
pub fn truncate_rich_line(mut line: Line<'static>, max_width: usize) -> Line<'static> {
    let visible_width = line
        .spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum::<usize>();
    if visible_width <= max_width {
        return line;
    }
    if max_width == 0 {
        line.spans.clear();
        return line;
    }

    let content_width = max_width.saturating_sub(1);
    let mut used_width = 0_usize;
    let mut tokens = Vec::new();
    'spans: for span in &line.spans {
        for grapheme in span.content.as_ref().graphemes(true) {
            let width = UnicodeWidthStr::width(grapheme);
            if used_width.saturating_add(width) > content_width {
                break 'spans;
            }
            tokens.push(StyledGrapheme {
                text: grapheme.to_owned(),
                style: span.style,
                width,
                whitespace: grapheme.chars().all(char::is_whitespace),
            });
            used_width = used_width.saturating_add(width);
        }
    }
    while tokens.last().is_some_and(|token| token.whitespace) {
        tokens.pop();
    }
    let ellipsis_style = tokens.last().map_or(line.style, |token| token.style);
    let mut truncated = styled_tokens_to_line(tokens);
    truncated.spans.push(Span::styled("…", ellipsis_style));
    truncated.style = line.style;
    truncated.alignment = line.alignment;
    truncated
}

pub fn flush_markdown_chunk(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    chunk: &mut Vec<&str>,
    width: u16,
) {
    if chunk.is_empty() {
        return;
    }
    let chunk_text = chunk.join("\n");
    let rendered = markdown_to_text(chunk_text.as_str());
    for line in rendered.lines {
        push_wrapped_rich_line(out, prefix, prefix, owned_line(&line), width);
    }
    chunk.clear();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A fenced code block in markdown.
pub struct MarkdownFence {
    pub marker: char,
    pub len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Alignment of a markdown table column.
pub enum TableColumnAlignment {
    Left,
    Center,
    Right,
}

// Keep Markdown tables compact: one terminal cell on each horizontal side of
// the content. This is half of the previous two-cell visual inset while still
// leaving the text clear of the box-drawing characters.
const MARKDOWN_TABLE_HORIZONTAL_PADDING: usize = 1;
const MARKDOWN_TABLE_MIN_CONTENT_WIDTH: usize = 1;

pub fn markdown_fence_delimiter(line: &str) -> Option<MarkdownFence> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|ch| *ch == marker).count();
    (len >= 3).then_some(MarkdownFence { marker, len })
}

pub fn is_markdown_table_header(header: &str, delimiter: &str) -> bool {
    if !header.contains('|') {
        return false;
    }
    parse_markdown_table_alignment(delimiter).is_some()
}

pub fn looks_like_markdown_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.contains('|')
}

pub fn push_markdown_table(out: &mut Vec<RenderedLine>, prefix: &str, lines: &[&str], width: u16) {
    if lines.len() < 2 {
        for line in lines {
            push_multiline(out, prefix, line, Style::default(), width);
        }
        return;
    }

    let Some(alignments) = parse_markdown_table_alignment(lines[1]) else {
        for line in lines {
            push_multiline(out, prefix, line, Style::default(), width);
        }
        return;
    };

    let mut rows = lines
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 1)
        .map(|(_, line)| parse_markdown_table_row(line))
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return;
    }

    let column_count = rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or_else(|| alignments.len().max(1));
    if column_count == 0 {
        return;
    }

    let alignments = normalize_table_alignments(alignments, column_count);
    for row in &mut rows {
        row.resize(column_count, String::new());
        for cell in row.iter_mut() {
            *cell = markdown_table_cell_text(cell.as_str());
        }
    }

    // Every cell owns its padding; the remaining fixed width is the left
    // border, one separator after each cell, and both horizontal insets.
    let separator_width = column_count
        .saturating_mul(MARKDOWN_TABLE_HORIZONTAL_PADDING.saturating_mul(2))
        .saturating_add(column_count.saturating_add(1));
    let prefix_width = UnicodeWidthStr::width(prefix);
    let available_width = width.max(1) as usize;
    let table_width_budget = available_width.saturating_sub(prefix_width);
    let min_content_width = column_count.saturating_mul(MARKDOWN_TABLE_MIN_CONTENT_WIDTH);
    if table_width_budget <= separator_width.saturating_add(min_content_width) {
        push_markdown_table_fallback(out, prefix, &rows, width);
        return;
    }

    let column_widths = compute_table_column_widths(
        rows.as_slice(),
        table_width_budget.saturating_sub(separator_width),
    );
    if column_widths.is_empty() {
        push_markdown_table_fallback(out, prefix, &rows, width);
        return;
    }

    let header_style = Style::default()
        .fg(agena_tui_components::theme::accent_color())
        .add_modifier(Modifier::BOLD);
    let separator_style = Style::default().fg(agena_tui_components::theme::muted_color());
    let body_style = Style::default();

    push_table_border(
        out,
        prefix,
        column_widths.as_slice(),
        "┌",
        "┬",
        "┐",
        separator_style,
    );
    for (row_index, row) in rows.iter().enumerate() {
        let navigation_unit = out.len();
        render_table_row(
            out,
            prefix,
            row.as_slice(),
            column_widths.as_slice(),
            alignments.as_slice(),
            if row_index == 0 {
                header_style
            } else {
                body_style
            },
        );
        let navigation_copy_text = row.join("\t");
        for line in &mut out[navigation_unit..] {
            line.navigation_unit = Some(navigation_unit);
            line.navigation_copy_text.clone_from(&navigation_copy_text);
        }
        if row_index + 1 < rows.len() {
            push_table_border(
                out,
                prefix,
                column_widths.as_slice(),
                "├",
                "┼",
                "┤",
                separator_style,
            );
        }
    }
    push_table_border(
        out,
        prefix,
        column_widths.as_slice(),
        "└",
        "┴",
        "┘",
        separator_style,
    );
}

pub fn push_markdown_table_fallback(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    rows: &[Vec<String>],
    width: u16,
) {
    if rows.is_empty() {
        return;
    }
    for (row_index, row) in rows.iter().enumerate() {
        let navigation_unit = out.len();
        let row_text = row.join(" | ");
        push_multiline(
            out,
            prefix,
            row_text.as_str(),
            if row_index == 0 {
                Style::default()
                    .fg(agena_tui_components::theme::accent_color())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
            width,
        );
        let navigation_copy_text = row.join("\t");
        for line in &mut out[navigation_unit..] {
            line.navigation_unit = Some(navigation_unit);
            line.navigation_copy_text.clone_from(&navigation_copy_text);
        }
    }
}

pub fn parse_markdown_table_alignment(line: &str) -> Option<Vec<TableColumnAlignment>> {
    let cells = parse_markdown_table_row(line);
    if cells.is_empty() {
        return None;
    }

    cells
        .into_iter()
        .map(|cell| {
            let trimmed = cell.trim();
            if trimmed.is_empty()
                || !trimmed.contains('-')
                || !trimmed.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
            {
                return None;
            }
            Some(match (trimmed.starts_with(':'), trimmed.ends_with(':')) {
                (true, true) => TableColumnAlignment::Center,
                (false, true) => TableColumnAlignment::Right,
                _ => TableColumnAlignment::Left,
            })
        })
        .collect()
}

pub fn parse_markdown_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let content = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let content = content.strip_suffix('|').unwrap_or(content);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escape = false;

    for ch in content.chars() {
        if escape {
            cell.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '|' => {
                cells.push(cell.trim().to_string());
                cell.clear();
            }
            _ => cell.push(ch),
        }
    }
    if escape {
        cell.push('\\');
    }
    cells.push(cell.trim().to_string());
    cells
}

pub fn normalize_table_alignments(
    mut alignments: Vec<TableColumnAlignment>,
    column_count: usize,
) -> Vec<TableColumnAlignment> {
    alignments.resize(column_count, TableColumnAlignment::Left);
    alignments
}

pub fn markdown_table_cell_text(cell: &str) -> String {
    let cell = inline_math_unicode_text(cell);
    let rendered = markdown_to_text(&cell);
    let flattened = rendered
        .lines
        .iter()
        .map(|line| line_plain_text(&owned_line(line)))
        .collect::<Vec<_>>()
        .join(" ");
    sanitize_terminal_text(flattened.as_str())
        .trim()
        .to_string()
}

pub fn compute_table_column_widths(rows: &[Vec<String>], budget: usize) -> Vec<usize> {
    if rows.is_empty() {
        return Vec::new();
    }

    let column_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    if column_count == 0 {
        return Vec::new();
    }

    let min_width = MARKDOWN_TABLE_MIN_CONTENT_WIDTH;
    let min_total = min_width.saturating_mul(column_count);
    if budget < min_total {
        return Vec::new();
    }

    let natural_widths = (0..column_count)
        .map(|index| {
            rows.iter()
                .filter_map(|row| row.get(index))
                .map(|cell| UnicodeWidthStr::width(cell.as_str()).max(min_width))
                .max()
                .unwrap_or(min_width)
        })
        .collect::<Vec<_>>();
    fit_table_column_widths(natural_widths.as_slice(), budget)
}

/// Fit measured table content widths into the available content budget.
///
/// Callers rendering rich cells must measure the final visible representation
/// (including link destinations, image labels, and formatted math) before using
/// this allocator. Measuring only the Markdown source label can make a table
/// look artificially narrow while its actual rendered content wraps inside it.
pub fn fit_table_column_widths(natural_widths: &[usize], budget: usize) -> Vec<usize> {
    if natural_widths.is_empty() {
        return Vec::new();
    }

    let column_count = natural_widths.len();
    let min_width = MARKDOWN_TABLE_MIN_CONTENT_WIDTH;
    let min_total = min_width.saturating_mul(column_count);
    if budget < min_total {
        return Vec::new();
    }

    let mut widths = vec![min_width; column_count];
    let mut remaining = budget.saturating_sub(min_total);
    let mut deficits = natural_widths
        .iter()
        .map(|width| width.saturating_sub(min_width))
        .collect::<Vec<_>>();

    while remaining > 0 && deficits.iter().any(|deficit| *deficit > 0) {
        for index in 0..column_count {
            if deficits[index] == 0 || remaining == 0 {
                continue;
            }
            widths[index] += 1;
            deficits[index] -= 1;
            remaining -= 1;
        }
    }

    widths
}

pub fn render_table_row(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    cells: &[String],
    widths: &[usize],
    alignments: &[TableColumnAlignment],
    style: Style,
) {
    let wrapped_cells = cells
        .iter()
        .zip(widths.iter())
        .map(|(cell, width)| wrap_table_cell(cell.as_str(), *width))
        .collect::<Vec<_>>();
    let row_height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1).max(1);

    for line_index in 0..row_height {
        let border_style = Style::default().fg(agena_tui_components::theme::muted_color());
        let mut copy_cells = Vec::with_capacity(widths.len());
        let mut spans = vec![
            Span::raw(prefix.to_string()),
            Span::styled("│", border_style),
        ];
        for (column_index, width) in widths.iter().enumerate() {
            spans.push(Span::raw(" ".repeat(MARKDOWN_TABLE_HORIZONTAL_PADDING)));
            let text = wrapped_cells
                .get(column_index)
                .and_then(|lines| lines.get(line_index))
                .cloned()
                .unwrap_or_default();
            copy_cells.push(text.clone());
            spans.push(Span::styled(
                pad_table_cell(
                    text.as_str(),
                    *width,
                    alignments
                        .get(column_index)
                        .copied()
                        .unwrap_or(TableColumnAlignment::Left),
                ),
                style,
            ));
            spans.push(Span::raw(" ".repeat(MARKDOWN_TABLE_HORIZONTAL_PADDING)));
            spans.push(Span::styled("│", border_style));
        }
        out.push(RenderedLine::rich(Line::from(spans)).with_copy_projection(
            copy_cells.join("\t"),
            UnicodeWidthStr::width(prefix).saturating_add(1),
        ));
    }
}

pub fn push_table_border(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    widths: &[usize],
    left: &str,
    middle: &str,
    right: &str,
    style: Style,
) {
    let mut spans = vec![
        Span::raw(prefix.to_string()),
        Span::styled(left.to_string(), style),
    ];
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(middle.to_string(), style));
        }
        spans.push(Span::styled(
            "─".repeat(width.saturating_add(MARKDOWN_TABLE_HORIZONTAL_PADDING.saturating_mul(2))),
            style,
        ));
    }
    spans.push(Span::styled(right.to_string(), style));
    out.push(
        RenderedLine::rich(Line::from(spans))
            .with_copy_projection(String::new(), UnicodeWidthStr::width(prefix)),
    );
}

pub fn wrap_table_cell(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    let normalized = sanitize_terminal_text(text).trim().to_string();
    if normalized.is_empty() {
        return vec![String::new()];
    }

    normalized
        .split('\n')
        .flat_map(|line| wrap_rich_line(&[Span::raw(line.to_owned())], width, width))
        .map(|line| line.to_string())
        .collect()
}

pub fn pad_table_cell(text: &str, width: usize, alignment: TableColumnAlignment) -> String {
    let visible = truncate_display_width(text, width);
    let visible_width = UnicodeWidthStr::width(visible.as_str());
    let padding = width.saturating_sub(visible_width);
    match alignment {
        TableColumnAlignment::Left => format!("{visible}{}", " ".repeat(padding)),
        TableColumnAlignment::Right => format!("{}{visible}", " ".repeat(padding)),
        TableColumnAlignment::Center => {
            let left = padding / 2;
            let right = padding.saturating_sub(left);
            format!("{}{}{}", " ".repeat(left), visible, " ".repeat(right))
        }
    }
}

#[derive(Debug, Clone)]
/// A styled grapheme for rendering.
pub struct StyledGrapheme {
    pub text: String,
    pub style: Style,
    pub width: usize,
    pub whitespace: bool,
}

pub fn wrap_rich_line(
    spans: &[Span<'static>],
    initial_width: usize,
    continuation_width: usize,
) -> Vec<Line<'static>> {
    let text = spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    if text.is_empty() {
        return vec![Line::default()];
    }
    if spans.len() == 1 && UnicodeWidthStr::width(text.as_str()) <= initial_width {
        return vec![Line::from(spans.to_vec())];
    }

    // Segment the complete text, not individual style spans: a style change
    // can occur inside a word or even an emoji/combining-character sequence.
    let mut breaks = linebreaks(&text).peekable();
    let mut span_index = 0;
    let mut span_end = spans[0].content.len();
    let tokens = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| {
            let end = start + grapheme.len();
            while breaks.peek().is_some_and(|(offset, _)| *offset < end) {
                breaks.next();
            }
            while start >= span_end {
                span_index += 1;
                span_end += spans[span_index].content.len();
            }
            WrapGrapheme {
                start,
                end,
                // Ratatui segments each span separately. A grapheme spanning
                // styles must therefore use its leading character's style as
                // a whole, or emoji and combining marks are drawn incorrectly.
                style: spans[span_index].style,
                width: UnicodeWidthStr::width(grapheme),
                break_after: breaks.peek().is_some_and(|(offset, _)| *offset == end),
                // Non-breaking spaces are content, including at a forced wrap.
                whitespace: grapheme
                    .chars()
                    .all(|ch| ch.is_whitespace() && !matches!(ch, '\u{a0}' | '\u{202f}')),
            }
        })
        .collect::<Vec<_>>();

    let mut ranges = Vec::new();
    let mut start = 0;
    let mut end = 0;
    let mut current_width = 0_usize;
    let mut width_limit = initial_width.max(1);
    let mut last_break_index = None;
    while end < tokens.len() {
        let token = &tokens[end];
        // Trailing breakable spaces do not consume the row's budget. This
        // lets an English word occupy the last cell without being moved to
        // the next row just because its following space did not fit.
        if end > start
            && !token.whitespace
            && current_width.saturating_add(token.width) > width_limit
        {
            let split = last_break_index.unwrap_or(end);
            let mut visible_end = split;
            while visible_end > start && tokens[visible_end - 1].whitespace {
                visible_end -= 1;
            }
            if visible_end > start {
                ranges.push(start..visible_end);
            }
            start = split;
            while start < tokens.len() && tokens[start].whitespace {
                start += 1;
            }
            end = start;
            current_width = 0;
            width_limit = continuation_width.max(1);
            last_break_index = None;
            continue;
        }
        current_width = current_width.saturating_add(token.width);
        end += 1;
        if token.break_after {
            last_break_index = Some(end);
        }
    }
    if start < tokens.len() {
        let mut visible_end = tokens.len();
        if current_width > width_limit {
            while visible_end > start && tokens[visible_end - 1].whitespace {
                visible_end -= 1;
            }
        }
        ranges.push(start..visible_end);
    }
    if ranges.is_empty() {
        return vec![Line::default()];
    }

    // Merge adjacent graphemes with the same style, keeping both allocation
    // and reconstruction linear in the paragraph's length.
    ranges
        .into_iter()
        .map(|range| {
            let mut row = Vec::<Span<'static>>::new();
            for token in &tokens[range] {
                let grapheme = &text[token.start..token.end];
                if let Some(last) = row.last_mut()
                    && last.style == token.style
                {
                    last.content.to_mut().push_str(grapheme);
                } else {
                    row.push(Span::styled(grapheme.to_owned(), token.style));
                }
            }
            Line::from(row)
        })
        .collect()
}

struct WrapGrapheme {
    start: usize,
    end: usize,
    style: Style,
    width: usize,
    break_after: bool,
    whitespace: bool,
}

pub fn styled_tokens_to_line(tokens: Vec<StyledGrapheme>) -> Line<'static> {
    let mut spans = Vec::<Span<'static>>::new();
    for token in tokens {
        if let Some(last) = spans.last_mut()
            && last.style == token.style
        {
            last.content = format!("{}{}", last.content.as_ref(), token.text).into();
        } else {
            spans.push(Span::styled(token.text, token.style));
        }
    }
    Line::from(spans)
}

pub fn truncate_display_width(text: &str, max_width: usize) -> String {
    agena_tui_components::text::clip_display_width(&sanitize_terminal_text(text), max_width)
}
use unicode_segmentation::UnicodeSegmentation;
