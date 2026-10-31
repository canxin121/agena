//! Text measurement and rendering helpers.

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

/// Spec of a header row.
pub struct HeaderRowSpec<'a> {
    pub left: Cow<'a, str>,
    pub right: Option<Cow<'a, str>>,
    pub left_style: Style,
    pub right_style: Style,
}

/// Spec of wrapped text.
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

/// Formats a row into fixed-width terminal columns after applying a caller's
/// display-text normalization policy.
pub fn format_fixed_columns<F>(columns: &[(&str, usize)], width: u16, mut normalize: F) -> String
where
    F: FnMut(&str) -> String,
{
    let mut output = String::new();
    for (index, (text, size)) in columns.iter().enumerate() {
        if index > 0 {
            let remaining =
                usize::from(width).saturating_sub(UnicodeWidthStr::width(output.as_str()));
            if remaining == 0 {
                break;
            }
            output.push_str(" ".repeat(remaining.min(2)).as_str());
            if remaining <= 2 {
                break;
            }
        }
        let remaining = usize::from(width).saturating_sub(UnicodeWidthStr::width(output.as_str()));
        if remaining == 0 {
            break;
        }
        let size = (*size).min(remaining);
        let normalized = normalize(text);
        let clipped = truncate_display_text(normalized.as_str(), size);
        output.push_str(clipped.as_str());
        output.push_str(
            " ".repeat(size.saturating_sub(UnicodeWidthStr::width(clipped.as_str())))
                .as_str(),
        );
    }
    output
}

/// Truncate a path-like label in the middle so both its root/context and its
/// distinguishing tail remain visible.
pub fn truncate_display_text_middle(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }

    const ELLIPSIS: &str = "…";
    let ellipsis_width = UnicodeWidthStr::width(ELLIPSIS);
    if max_width <= ellipsis_width {
        return ELLIPSIS.to_string();
    }

    let content_budget = max_width.saturating_sub(ellipsis_width);
    let prefix_budget = content_budget / 2;
    let suffix_budget = content_budget.saturating_sub(prefix_budget);
    let graphemes = UnicodeSegmentation::graphemes(text, true).collect::<Vec<_>>();

    let mut prefix = String::new();
    let mut prefix_width = 0_usize;
    let mut prefix_count = 0_usize;
    for grapheme in &graphemes {
        let width = UnicodeWidthStr::width(*grapheme);
        if prefix_width.saturating_add(width) > prefix_budget {
            break;
        }
        prefix.push_str(grapheme);
        prefix_width = prefix_width.saturating_add(width);
        prefix_count += 1;
    }

    let mut suffix_parts = Vec::new();
    let mut suffix_width = 0_usize;
    for grapheme in graphemes.iter().skip(prefix_count).rev() {
        let width = UnicodeWidthStr::width(*grapheme);
        if suffix_width.saturating_add(width) > suffix_budget {
            break;
        }
        suffix_parts.push(*grapheme);
        suffix_width = suffix_width.saturating_add(width);
    }
    suffix_parts.reverse();

    format!("{prefix}{ELLIPSIS}{}", suffix_parts.concat())
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
        return clip_display_width(suffix, max_width);
    }
    let mut truncated = clip_display_width(text, max_width.saturating_sub(suffix_width));
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
        .fold(0_u16, u16::saturating_add)
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

/// Strip CSI, OSC and single-character terminal escapes without consuming a
/// partial UTF-8 character after a malformed sequence. Does not change text
/// layout or control-character policy; those belong to the display consumer.
pub fn strip_terminal_ansi_sequences(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != 0x1b {
            let ch = text[index..].chars().next().unwrap_or_default();
            out.push(ch);
            index += ch.len_utf8();
            continue;
        }

        index += 1;
        if index >= bytes.len() {
            break;
        }

        match bytes[index] {
            b'[' => {
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    // Escape sequences may only contain ASCII parameter bytes.
                    // A non-ASCII byte means the sequence was truncated and the
                    // rest is real text: stop so it is emitted below and we
                    // never land mid-way through a multi-byte char.
                    if !byte.is_ascii() {
                        break;
                    }
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            b']' => {
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        0x07 => {
                            index += 1;
                            break;
                        }
                        0x1b if bytes.get(index + 1) == Some(&b'\\') => {
                            index += 2;
                            break;
                        }
                        byte if !byte.is_ascii() => break,
                        _ => index += 1,
                    }
                }
            }
            _ => {
                // Single-char escape (e.g. ESC M). Skip it only when it is a
                // plain ASCII byte; a non-ASCII leading byte is the start of a
                // multi-byte char and must be emitted as text.
                if bytes[index].is_ascii() {
                    index += 1;
                }
            }
        }
    }

    out
}

/// Terminal-safe multiline display text. Preserve LF and tab for callers that
/// lay out paragraphs or expand tab stops; remove CR and bidi controls, and
/// replace other control characters with spaces. This never mutates source or
/// clipboard text. Editor storage applies its own explicit tab expansion.
pub fn sanitize_terminal_text(text: &str) -> String {
    let stripped = strip_terminal_ansi_sequences(text).replace('\r', "");
    stripped
        .chars()
        .filter_map(|ch| match ch {
            '\n' | '\t' => Some(ch),
            '\u{200e}' | '\u{200f}' => None,
            '\u{202a}'..='\u{202e}' => None,
            '\u{2066}'..='\u{2069}' => None,
            ch if ch.is_control() => Some(' '),
            _ => Some(ch),
        })
        .collect()
}

/// Take a prefix that fits the display-column budget, without splitting a
/// grapheme (including combining marks and ZWJ emoji). The input is already
/// display text: sanitization and ellipsis policy belong to the caller.
pub fn clip_display_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut width = 0_usize;
    text.graphemes(true)
        .take_while(|grapheme| {
            let next = width.saturating_add(UnicodeWidthStr::width(*grapheme));
            if next > max_width {
                return false;
            }
            width = next;
            true
        })
        .collect()
}

/// Copy complete graphemes overlapping a display-column range. Do not apply
/// display sanitization: the caller has already selected its copy projection.
pub fn display_cell_slice(text: &str, range: std::ops::Range<usize>) -> String {
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

#[cfg(test)]
mod terminal_text_tests {
    use super::*;

    #[test]
    fn display_sanitization_preserves_multiline_layout_and_unicode() {
        let text = "\u{1b}[31m中\u{1b}[0m\t🙂\r\nnext\u{7}\u{202e}";
        assert_eq!(sanitize_terminal_text(text), "中\t🙂\nnext ");
        assert_eq!(text, "\u{1b}[31m中\u{1b}[0m\t🙂\r\nnext\u{7}\u{202e}");
    }

    #[test]
    fn strips_osc_and_preserves_unicode_after_partial_escapes() {
        assert_eq!(strip_terminal_ansi_sequences("a\u{1b}]0;title\u{7}b"), "ab");
        assert_eq!(
            strip_terminal_ansi_sequences("a\u{1b}]0;title\u{1b}\\b"),
            "ab"
        );
        for text in ["\u{1b}[中", "\u{1b}]中", "\u{1b}中"] {
            assert_eq!(strip_terminal_ansi_sequences(text), "中");
        }
    }

    #[test]
    fn clipping_never_splits_wide_or_combining_graphemes() {
        assert_eq!(clip_display_width("中a", 1), "");
        assert_eq!(clip_display_width("中a", 2), "中");
        assert_eq!(clip_display_width("e\u{301}x", 1), "e\u{301}");
        assert_eq!(clip_display_width("👩\u{200d}💻x", 2), "👩\u{200d}💻");
        assert_eq!(clip_display_width("a", 0), "");
    }

    #[test]
    fn cell_slices_copy_whole_overlapping_graphemes() {
        assert_eq!(display_cell_slice("a中b", 2..3), "中");
        assert_eq!(display_cell_slice("a👩\u{200d}💻b", 2..3), "👩\u{200d}💻");
        assert_eq!(display_cell_slice("ae\u{301}b", 1..2), "e\u{301}");
    }
}
