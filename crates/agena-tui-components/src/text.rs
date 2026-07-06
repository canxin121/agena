use std::{borrow::Cow, cmp::min};

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};
use textwrap::{Options as WrapOptions, WordSplitter, wrap};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::layout::wrapped_text_height;

pub struct HeaderRowSpec<'a> {
    pub left: Cow<'a, str>,
    pub right: Option<Cow<'a, str>>,
    pub left_style: Style,
    pub right_style: Style,
}

pub struct WrappedTextSpec<'a> {
    pub text: Cow<'a, str>,
    pub style: Style,
}

pub fn render_header_row(frame: &mut Frame, area: Rect, spec: &HeaderRowSpec<'_>) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let right = spec.right.as_deref().unwrap_or("");
    if right.trim().is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_display_text(spec.left.as_ref(), area.width as usize),
                spec.left_style,
            ))),
            area,
        );
        return;
    }

    let max_right_width = if area.width < 52 {
        area.width.saturating_div(2).max(8)
    } else {
        area.width.saturating_mul(2).saturating_div(5).max(16)
    };
    let truncated_right = truncate_display_text(right, max_right_width as usize);
    let right_width = UnicodeWidthStr::width(truncated_right.as_str()).saturating_add(1) as u16;
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(min(area.width, right_width)),
        ])
        .split(area);
    let truncated_left = truncate_display_text(
        spec.left.as_ref(),
        columns[0].width.saturating_sub(1).max(1) as usize,
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(truncated_left, spec.left_style))),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(truncated_right, spec.right_style)))
            .alignment(Alignment::Right),
        columns[1],
    );
}

pub fn build_wrapped_text_lines(spec: &WrappedTextSpec<'_>, width: u16) -> Vec<Line<'static>> {
    let normalized = trim_empty_line_edges(spec.text.as_ref());
    if normalized.is_empty() {
        return Vec::new();
    }
    let available = width.max(1) as usize;
    let options = WrapOptions::new(available)
        .break_words(false)
        .word_splitter(WordSplitter::NoHyphenation);

    normalized
        .split('\n')
        .flat_map(|line| {
            let wrapped = wrap(line, options.clone());
            if wrapped.is_empty() {
                vec![Line::from(Span::styled(String::new(), spec.style))]
            } else {
                wrapped
                    .into_iter()
                    .map(|segment| Line::from(Span::styled(segment.into_owned(), spec.style)))
                    .collect::<Vec<_>>()
            }
        })
        .collect()
}

pub fn render_wrapped_text(frame: &mut Frame, area: Rect, spec: &WrappedTextSpec<'_>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let lines = build_wrapped_text_lines(spec, area.width);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

pub fn join_inline_segments<I, S>(segments: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    segments
        .into_iter()
        .filter_map(|segment| {
            let segment = segment.as_ref().trim();
            (!segment.is_empty()).then(|| segment.to_string())
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn format_key_value_segment(key: &str, value: &str) -> String {
    format!("{key}={value}")
}

pub fn truncate_display_text(text: &str, max_width: usize) -> String {
    truncate_display_text_with_suffix(text, max_width, "...")
}

pub fn truncate_display_text_with_suffix(text: &str, max_width: usize, suffix: &str) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }

    let suffix_width = UnicodeWidthStr::width(suffix);
    if max_width <= suffix_width {
        return suffix.to_string();
    }

    let budget = max_width.saturating_sub(suffix_width);
    let mut width = 0usize;
    let mut truncated = String::new();
    for grapheme in UnicodeSegmentation::graphemes(text, true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > budget {
            break;
        }
        truncated.push_str(grapheme);
        width = width.saturating_add(grapheme_width);
    }
    truncated.push_str(suffix);
    truncated
}

pub fn trim_empty_line_edges(text: &str) -> String {
    let lines = text.split('\n').collect::<Vec<_>>();
    let Some(first_non_empty) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let last_non_empty = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .unwrap_or(first_non_empty);
    lines[first_non_empty..=last_non_empty].join("\n")
}

pub fn line_plain_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

pub fn text_plain_text(text: &Text<'_>) -> String {
    text.lines
        .iter()
        .map(line_plain_text)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn wrapped_lines_height(lines: &[Line<'_>], width: u16) -> u16 {
    lines
        .iter()
        .map(|line| wrapped_text_height(line_plain_text(line).as_str(), width))
        .sum::<u16>()
        .max(1)
}

pub fn wrapped_text_height_for_text(text: &Text<'_>, width: u16) -> u16 {
    wrapped_lines_height(text.lines.as_slice(), width)
}

pub fn bordered_text_height(text: &Text<'_>, width: u16, min_body: u16, max_body: u16) -> u16 {
    wrapped_text_height_for_text(text, width.saturating_sub(2))
        .clamp(min_body, max_body)
        .saturating_add(2)
}
