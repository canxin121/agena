use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use super::{RenderedLine, push_multiline, push_wrapped_line};
use crate::math_render::{MathLinePlacement, layout_config, render_formula, unicode_formula};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum InlineMathSegment {
    Text(String),
    Math(String),
}

pub(in crate::app) fn fenced_math_language(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(marker) = trimmed.chars().next() else {
        return false;
    };
    if marker != '`' && marker != '~' {
        return false;
    }
    let fence_len = trimmed.chars().take_while(|ch| *ch == marker).count();
    if fence_len < 3 {
        return false;
    }
    let language = trimmed[fence_len..]
        .trim()
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '{' | '}'))
        .next()
        .unwrap_or_default()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    matches!(language.as_str(), "math" | "tex" | "latex" | "katex")
}

pub(in crate::app) fn display_math_source(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lines = trimmed.lines().collect::<Vec<_>>();
    if lines.len() >= 2 && fenced_math_language(lines[0]) {
        let body_end = if lines.last().is_some_and(|line| {
            let line = line.trim_start();
            line.starts_with("```") || line.starts_with("~~~")
        }) {
            lines.len().saturating_sub(1)
        } else {
            lines.len()
        };
        return Some(lines[1..body_end].join("\n"));
    }
    if let Some(body) = trimmed
        .strip_prefix("$$")
        .and_then(|text| text.strip_suffix("$$"))
    {
        return Some(body.trim().to_string());
    }
    if let Some(body) = trimmed
        .strip_prefix(r"\[")
        .and_then(|text| text.strip_suffix(r"\]"))
    {
        return Some(body.trim().to_string());
    }
    None
}

pub(in crate::app) fn inline_math_segments(source: &str) -> Vec<InlineMathSegment> {
    let bytes = source.as_bytes();
    let mut segments = Vec::new();
    let mut text_start = 0_usize;
    let mut index = 0_usize;
    let mut code_fence_len = 0_usize;

    while index < bytes.len() {
        if bytes[index] == b'`' && !is_escaped(bytes, index) {
            let run = bytes[index..]
                .iter()
                .take_while(|byte| **byte == b'`')
                .count();
            if code_fence_len == 0 {
                code_fence_len = run;
            } else if run == code_fence_len {
                code_fence_len = 0;
            }
            index += run;
            continue;
        }
        if code_fence_len > 0 {
            index += 1;
            continue;
        }

        let (open_len, close) = if bytes[index] == b'$'
            && bytes.get(index + 1) != Some(&b'$')
            && !is_escaped(bytes, index)
        {
            (1, InlineClose::Dollar)
        } else if bytes[index..].starts_with(br"\(") && !is_escaped(bytes, index) {
            (2, InlineClose::Paren)
        } else {
            index += 1;
            continue;
        };
        let content_start = index + open_len;
        if bytes
            .get(content_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            index += open_len;
            continue;
        }
        let Some(close_start) = find_inline_close(bytes, content_start, close) else {
            index += open_len;
            continue;
        };
        if close_start == content_start
            || bytes
                .get(close_start.saturating_sub(1))
                .is_some_and(u8::is_ascii_whitespace)
        {
            index += open_len;
            continue;
        }
        if text_start < index {
            segments.push(InlineMathSegment::Text(
                source[text_start..index].to_string(),
            ));
        }
        segments.push(InlineMathSegment::Math(
            source[content_start..close_start].to_string(),
        ));
        index = close_start + close.len();
        text_start = index;
    }
    if text_start < source.len() {
        segments.push(InlineMathSegment::Text(source[text_start..].to_string()));
    }
    segments
}

pub(in crate::app) fn inline_math_unicode_text(source: &str) -> String {
    inline_math_segments(source)
        .into_iter()
        .map(|segment| match segment {
            InlineMathSegment::Text(text) => text,
            InlineMathSegment::Math(formula) => unicode_formula(&formula).join(" "),
        })
        .collect()
}

#[derive(Clone, Copy)]
enum InlineClose {
    Dollar,
    Paren,
}

impl InlineClose {
    fn len(self) -> usize {
        match self {
            Self::Dollar => 1,
            Self::Paren => 2,
        }
    }
}

fn find_inline_close(bytes: &[u8], mut index: usize, close: InlineClose) -> Option<usize> {
    while index < bytes.len() {
        let matches = match close {
            InlineClose::Dollar => {
                bytes[index] == b'$'
                    && bytes.get(index + 1) != Some(&b'$')
                    && !is_escaped(bytes, index)
            }
            InlineClose::Paren => bytes[index..].starts_with(br"\)") && !is_escaped(bytes, index),
        };
        if matches {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let slash_count = bytes[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    slash_count % 2 == 1
}

pub(in crate::app) fn push_math_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    source: &str,
    width: u16,
) {
    let formula = display_math_source(source).unwrap_or_else(|| source.trim().to_string());
    let config = layout_config();
    if config.native_graphics
        && let Ok(artifact) = render_formula(&formula, true)
    {
        let prefix_width = UnicodeWidthStr::width(prefix) as u16;
        let available = width.saturating_sub(prefix_width).max(1);
        let render_width = artifact.size.width.min(available);
        let render_height = if artifact.size.width > render_width {
            u32::from(artifact.size.height)
                .saturating_mul(u32::from(render_width))
                .div_ceil(u32::from(artifact.size.width))
                .max(1) as u16
        } else {
            artifact.size.height
        };
        let column = prefix_width + available.saturating_sub(render_width) / 2;
        let start = out.len();
        for _ in 0..render_height {
            out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
        }
        out[start].math.push(MathLinePlacement {
            column,
            artifact,
            size: ratatui::layout::Size::new(render_width, render_height),
        });
        return;
    }
    push_unicode_canvas(out, prefix, &unicode_formula(&formula), width);
}

pub(in crate::app) fn push_inline_math(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    source: &str,
    width: u16,
) -> bool {
    let segments = inline_math_segments(source);
    if !segments
        .iter()
        .any(|segment| matches!(segment, InlineMathSegment::Math(_)))
    {
        return false;
    }
    let prefix_width = UnicodeWidthStr::width(prefix) as u16;
    let available = width.saturating_sub(prefix_width).max(1);
    let config = layout_config();

    if config.native_graphics && source.contains('\n') {
        for line in source.lines() {
            if !push_inline_math(out, prefix, line, width) {
                super::push_markdown(out, prefix, line, width);
            }
        }
        return true;
    }

    if config.native_graphics {
        let mut items = Vec::new();
        let mut total_width = 0_u16;
        let mut height = 1_u16;
        let mut render_failed = false;
        for segment in &segments {
            match segment {
                InlineMathSegment::Text(text) => {
                    let text = inline_markdown_plain_text(text);
                    total_width = total_width.saturating_add(
                        UnicodeWidthStr::width(text.as_str()).min(usize::from(u16::MAX)) as u16,
                    );
                    items.push(InlineItem::Text(text));
                }
                InlineMathSegment::Math(formula) => match render_formula(formula, false) {
                    Ok(artifact) => {
                        total_width = total_width.saturating_add(artifact.size.width);
                        height = height.max(artifact.size.height);
                        items.push(InlineItem::Math(artifact));
                    }
                    Err(_) => render_failed = true,
                },
            }
        }
        if !render_failed && total_width <= available {
            let start = out.len();
            for _ in 0..height {
                out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
            }
            let mut line = String::from(prefix);
            let mut column = prefix_width;
            for item in items {
                match item {
                    InlineItem::Text(text) => {
                        column = column.saturating_add(
                            UnicodeWidthStr::width(text.as_str()).min(usize::from(u16::MAX)) as u16,
                        );
                        line.push_str(&text);
                    }
                    InlineItem::Math(artifact) => {
                        out[start].math.push(MathLinePlacement {
                            column,
                            size: artifact.size,
                            artifact: std::sync::Arc::clone(&artifact),
                        });
                        line.push_str(&" ".repeat(usize::from(artifact.size.width)));
                        column = column.saturating_add(artifact.size.width);
                    }
                }
            }
            out[start + usize::from(height.saturating_sub(1))] =
                RenderedLine::plain(line, Style::default());
            return true;
        }
    }

    // Unicode fallback uses the same line-box model, so fractions, roots and
    // matrices remain two-dimensional even without an image protocol.
    let mut rows = vec![String::new()];
    for segment in segments {
        let block = match segment {
            InlineMathSegment::Text(text) => vec![inline_markdown_plain_text(&text)],
            InlineMathSegment::Math(formula) => unicode_formula(&formula),
        };
        append_bottom_aligned(&mut rows, &block);
    }
    if rows
        .iter()
        .all(|line| UnicodeWidthStr::width(line.as_str()) <= usize::from(available))
    {
        push_unicode_canvas(out, prefix, &rows, width);
    } else {
        for row in rows {
            push_wrapped_line(out, prefix, prefix, &row, Style::default(), width);
        }
    }
    true
}

enum InlineItem {
    Text(String),
    Math(std::sync::Arc<crate::math_render::MathArtifact>),
}

fn inline_markdown_plain_text(source: &str) -> String {
    let rendered = tui_markdown::from_str(source);
    rendered
        .lines
        .iter()
        .map(agena_tui_components::line_plain_text)
        .collect::<Vec<_>>()
        .join(" ")
}

fn append_bottom_aligned(canvas: &mut Vec<String>, block: &[String]) {
    let height = canvas.len().max(block.len()).max(1);
    if canvas.len() < height {
        let mut padded = vec![String::new(); height - canvas.len()];
        padded.append(canvas);
        *canvas = padded;
    }
    let block_width = block
        .iter()
        .map(|line| UnicodeWidthStr::width(line.as_str()))
        .max()
        .unwrap_or(0);
    let offset = height.saturating_sub(block.len());
    for (row, canvas_row) in canvas.iter_mut().enumerate().take(height) {
        if let Some(value) = row.checked_sub(offset).and_then(|index| block.get(index)) {
            canvas_row.push_str(value);
            let padding = block_width.saturating_sub(UnicodeWidthStr::width(value.as_str()));
            canvas_row.push_str(&" ".repeat(padding));
        } else {
            canvas_row.push_str(&" ".repeat(block_width));
        }
    }
}

fn push_unicode_canvas(out: &mut Vec<RenderedLine>, prefix: &str, rows: &[String], width: u16) {
    if rows.is_empty() {
        return;
    }
    for row in rows {
        push_multiline(out, prefix, row, Style::default(), width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_parser_ignores_code_and_escaped_dollars() {
        let segments = inline_math_segments(r"cost \$5, `$code$`, and $x^2$");
        assert_eq!(
            segments
                .iter()
                .filter(|segment| matches!(segment, InlineMathSegment::Math(_)))
                .count(),
            1
        );
        let fallback = inline_math_unicode_text("value $x^2$ and `$code$`");
        assert!(fallback.contains("x²"));
        assert!(fallback.contains("`$code$`"));
    }

    #[test]
    fn extracts_supported_display_delimiters() {
        assert_eq!(display_math_source("$$x+y$$").as_deref(), Some("x+y"));
        assert_eq!(display_math_source(r"\[x+y\]").as_deref(), Some("x+y"));
        assert_eq!(
            display_math_source("```math\nx+y\n```").as_deref(),
            Some("x+y")
        );
        assert_eq!(display_math_source("```math\nx+y").as_deref(), Some("x+y"));
    }

    #[test]
    fn unicode_math_fallback_never_reserves_blank_formula_rows() {
        let mut display = Vec::new();
        push_math_block(
            &mut display,
            "  ",
            r"$$\frac{-b\pm\sqrt{b^2-4ac}}{2a}$$",
            80,
        );
        assert!(!display.is_empty());
        assert!(display.iter().any(|line| !line.text.trim().is_empty()));
        assert!(display.iter().all(|line| line.math.is_empty()));

        let mut inline = Vec::new();
        assert!(push_inline_math(
            &mut inline,
            "  ",
            r"等号当且仅当 $a_1=a_2=\cdots=a_n$ 时成立。",
            80,
        ));
        let rendered = inline
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("等号当且仅当"));
        assert!(inline.iter().all(|line| line.math.is_empty()));
    }
}
