use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};

use comrak::{
    Arena, Options,
    nodes::{AlertType, AstNode, ListDelimType, ListType, NodeValue, TableAlignment},
    parse_document,
};
use ratatui::{
    layout::Size,
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use super::transcript_math::push_math_block;
use super::{
    MarkdownBlock, RenderedLine, TableColumnAlignment, TranscriptNodeKind,
    compute_table_column_widths, push_markdown_code_block, push_markdown_rule, push_single_line,
    push_table_border, push_wrapped_rich_line, sanitize_terminal_text, trim_empty_line_edges,
    wrap_rich_line,
};
use crate::math_render::{
    MathLinePlacement, layout_config, positional_unicode_text, render_markdown_image,
    render_markdown_svg, unicode_formula,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum MarkdownNode {
    Paragraph(Vec<MarkdownInline>),
    Heading {
        level: u8,
        content: Vec<MarkdownInline>,
    },
    Quote(Vec<MarkdownNode>),
    Alert {
        kind: MarkdownAlertKind,
        title: Option<String>,
        blocks: Vec<MarkdownNode>,
    },
    Code {
        language: String,
        literal: String,
        fenced: bool,
    },
    Diagram {
        language: String,
        literal: String,
    },
    List {
        ordered: bool,
        start: usize,
        delimiter: char,
        tight: bool,
        items: Vec<MarkdownListItem>,
    },
    DescriptionList(Vec<MarkdownDescriptionItem>),
    Table {
        alignments: Vec<MarkdownAlignment>,
        rows: Vec<MarkdownTableRow>,
    },
    ThematicBreak,
    Math {
        literal: String,
        display: bool,
    },
    Image {
        url: String,
        title: String,
        alt: String,
    },
    FootnoteDefinition {
        name: String,
        blocks: Vec<MarkdownNode>,
    },
    FrontMatter(String),
    Html(String),
    Subtext(Vec<MarkdownInline>),
    Directive {
        info: String,
        blocks: Vec<MarkdownNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct MarkdownListItem {
    pub(in crate::app) checked: Option<bool>,
    pub(in crate::app) blocks: Vec<MarkdownNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct MarkdownDescriptionItem {
    pub(in crate::app) term: Vec<MarkdownInline>,
    pub(in crate::app) details: Vec<MarkdownNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct MarkdownTableRow {
    pub(in crate::app) header: bool,
    pub(in crate::app) cells: Vec<Vec<MarkdownInline>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum MarkdownAlignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum MarkdownAlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum MarkdownInline {
    Text(String),
    Code(String),
    Emphasis(Vec<MarkdownInline>),
    Strong(Vec<MarkdownInline>),
    Strikethrough(Vec<MarkdownInline>),
    Underline(Vec<MarkdownInline>),
    Highlight(Vec<MarkdownInline>),
    Insert(Vec<MarkdownInline>),
    Superscript(Vec<MarkdownInline>),
    Subscript(Vec<MarkdownInline>),
    Spoiler(Vec<MarkdownInline>),
    Link {
        url: String,
        title: String,
        label: Vec<MarkdownInline>,
    },
    WikiLink {
        url: String,
        label: Vec<MarkdownInline>,
    },
    Image {
        url: String,
        title: String,
        alt: String,
    },
    Math {
        literal: String,
        display: bool,
    },
    FootnoteReference(String),
    Html(String),
    Emoji(String),
    SoftBreak,
    HardBreak,
}

const MAX_MARKDOWN_CACHE_DOCUMENTS: usize = 256;
const MAX_MARKDOWN_CACHE_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHEABLE_MARKDOWN_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct MarkdownParseCache {
    entries: HashMap<String, Arc<Vec<MarkdownBlock>>>,
    recency: VecDeque<String>,
    source_bytes: usize,
}

impl MarkdownParseCache {
    fn get(&mut self, source: &str) -> Option<Arc<Vec<MarkdownBlock>>> {
        let blocks = self.entries.get(source).cloned()?;
        self.recency.retain(|candidate| candidate != source);
        self.recency.push_back(source.to_string());
        Some(blocks)
    }

    fn insert(&mut self, source: String, blocks: Arc<Vec<MarkdownBlock>>) {
        if self.entries.insert(source.clone(), blocks).is_none() {
            self.source_bytes = self.source_bytes.saturating_add(source.len());
        }
        self.recency.retain(|candidate| candidate != &source);
        self.recency.push_back(source);
        while self.entries.len() > MAX_MARKDOWN_CACHE_DOCUMENTS
            || self.source_bytes > MAX_MARKDOWN_CACHE_SOURCE_BYTES
        {
            let Some(expired) = self.recency.pop_front() else {
                break;
            };
            if self.entries.remove(&expired).is_some() {
                self.source_bytes = self.source_bytes.saturating_sub(expired.len());
            }
        }
    }
}

static MARKDOWN_PARSE_CACHE: LazyLock<Mutex<MarkdownParseCache>> =
    LazyLock::new(|| Mutex::new(MarkdownParseCache::default()));

pub(in crate::app) fn parse_markdown_document(text: &str) -> Vec<MarkdownBlock> {
    let sanitized = sanitize_terminal_text(text);
    let markdown = trim_empty_line_edges(sanitized.as_str());
    if markdown.is_empty() {
        return Vec::new();
    }
    let cacheable = markdown.len() <= MAX_CACHEABLE_MARKDOWN_BYTES;
    if cacheable
        && let Some(blocks) = MARKDOWN_PARSE_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&markdown)
    {
        return blocks.as_ref().clone();
    }

    let blocks = parse_sanitized_markdown(&markdown);
    if cacheable {
        MARKDOWN_PARSE_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(markdown.to_string(), Arc::new(blocks.clone()));
    }
    blocks
}

fn parse_sanitized_markdown(markdown: &str) -> Vec<MarkdownBlock> {
    let arena = Arena::new();
    let options = markdown_options();
    let protected_display_math = protect_multiline_display_math(markdown);
    let protected_markdown = protect_inline_math_table_pipes(protected_display_math.as_ref());
    let root = parse_document(&arena, protected_markdown.as_ref(), &options);
    let source_lines = markdown.lines().collect::<Vec<_>>();
    let mut previous_end_line = 0_usize;
    let mut blocks = root
        .children()
        .filter_map(|node| {
            let data = node.data();
            let start_line = data.sourcepos.start.line.max(1);
            let end_line = data.sourcepos.end.line.max(start_line);
            drop(data);
            let parsed = convert_block(node)?;
            let source = source_lines
                .get(start_line.saturating_sub(1)..end_line.min(source_lines.len()))
                .unwrap_or_default()
                .join("\n");
            let leading_blank_line = previous_end_line > 0 && start_line > previous_end_line + 1;
            previous_end_line = end_line;
            let copy_text = match &parsed {
                MarkdownNode::Code { literal, .. } | MarkdownNode::Diagram { literal, .. } => {
                    literal.trim_end_matches('\n').to_string()
                }
                _ => source.clone(),
            };
            Some(MarkdownBlock {
                kind: markdown_node_kind(&parsed),
                source,
                copy_text,
                leading_blank_line,
                parsed,
            })
        })
        .collect::<Vec<_>>();
    renumber_footnotes(&mut blocks);
    blocks
}

const MATH_PIPE_PLACEHOLDER: &str = "&#124;";

/// Keep GFM's table scanner from treating a vertical bar inside inline math as
/// a cell separator. Comrak identifies tables before it identifies math spans,
/// so the character is hidden temporarily and restored in `convert_inline`.
fn protect_inline_math_table_pipes(markdown: &str) -> Cow<'_, str> {
    if !markdown.contains('|') {
        return Cow::Borrowed(markdown);
    }

    let mut protected = String::with_capacity(markdown.len());
    let mut changed = false;
    let mut markdown_fence = None::<(char, usize)>;
    for (line_index, line) in markdown.split('\n').enumerate() {
        if line_index > 0 {
            protected.push('\n');
        }
        if let Some((marker, minimum_len)) = markdown_fence {
            protected.push_str(line);
            if closing_markdown_fence(line, marker, minimum_len) {
                markdown_fence = None;
            }
            continue;
        }
        if let Some((marker, len)) = opening_markdown_fence(line) {
            markdown_fence = Some((marker, len));
            protected.push_str(line);
            continue;
        }

        let ranges = inline_math_byte_ranges(line);
        if ranges.is_empty() {
            protected.push_str(line);
            continue;
        }
        let mut range_index = 0_usize;
        for (byte_index, ch) in line.char_indices() {
            while ranges
                .get(range_index)
                .is_some_and(|(_, end)| byte_index >= *end)
            {
                range_index += 1;
            }
            let in_math = ranges
                .get(range_index)
                .is_some_and(|(start, end)| byte_index >= *start && byte_index < *end);
            if ch == '|' && in_math {
                protected.push_str(MATH_PIPE_PLACEHOLDER);
                changed = true;
            } else {
                protected.push(ch);
            }
        }
    }
    if changed {
        Cow::Owned(protected)
    } else {
        Cow::Borrowed(markdown)
    }
}

fn inline_math_byte_ranges(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0_usize;
    let mut code_span = None::<usize>;
    while index < bytes.len() {
        if bytes[index] == b'`' && !escaped_at(bytes, index) {
            let run = bytes[index..]
                .iter()
                .take_while(|byte| **byte == b'`')
                .count();
            if code_span == Some(run) {
                code_span = None;
            } else if code_span.is_none() {
                code_span = Some(run);
            }
            index += run;
            continue;
        }
        if code_span.is_some() {
            index += 1;
            continue;
        }

        let (content_start, closing) = if bytes[index] == b'$' && !escaped_at(bytes, index) {
            let run = bytes[index..]
                .iter()
                .take_while(|byte| **byte == b'$')
                .count();
            if !(1..=2).contains(&run)
                || run == 1
                    && bytes
                        .get(index + 1)
                        .is_none_or(|byte| byte.is_ascii_whitespace())
            {
                index += run;
                continue;
            }
            (index + run, InlineMathClosing::Dollar(run))
        } else if bytes[index..].starts_with(br"\(") && !escaped_at(bytes, index) {
            (index + 2, InlineMathClosing::Paren)
        } else {
            index += 1;
            continue;
        };
        let Some(close_start) = find_inline_math_close(bytes, content_start, closing) else {
            index = content_start;
            continue;
        };
        ranges.push((content_start, close_start));
        index = close_start + closing.len();
    }
    ranges
}

#[derive(Clone, Copy)]
enum InlineMathClosing {
    Dollar(usize),
    Paren,
}

impl InlineMathClosing {
    const fn len(self) -> usize {
        match self {
            Self::Dollar(len) => len,
            Self::Paren => 2,
        }
    }
}

fn find_inline_math_close(
    bytes: &[u8],
    mut index: usize,
    closing: InlineMathClosing,
) -> Option<usize> {
    while index < bytes.len() {
        let found = match closing {
            InlineMathClosing::Dollar(len) => {
                let has_closing_run = bytes
                    .get(index..index.saturating_add(len))
                    .is_some_and(|candidate| candidate.iter().all(|byte| *byte == b'$'));
                has_closing_run
                    && (len == 2 || bytes.get(index + 1) != Some(&b'$'))
                    && !escaped_at(bytes, index)
                    && (len == 2
                        || bytes
                            .get(index.wrapping_sub(1))
                            .is_some_and(|byte| !byte.is_ascii_whitespace()))
                    && (len == 2
                        || bytes
                            .get(index + 1)
                            .is_none_or(|byte| !byte.is_ascii_digit()))
            }
            InlineMathClosing::Paren => {
                bytes[index..].starts_with(br"\)") && !escaped_at(bytes, index)
            }
        };
        if found {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn escaped_at(bytes: &[u8], index: usize) -> bool {
    bytes[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn restore_math_placeholders(literal: String) -> String {
    literal.replace(MATH_PIPE_PLACEHOLDER, "|")
}

/// Protect standalone `$$ ... $$` and `\[ ... \]` blocks from CommonMark
/// block parsing.
///
/// Comrak recognizes dollar math while parsing inlines, after it has already
/// identified headings, thematic breaks, lists, and other blocks. A formula
/// line containing only `=` can therefore become a Setext heading underline
/// before the dollar-math extension sees it. Turning the delimiters into a
/// temporary math code fence makes the entire body opaque to block parsing;
/// `convert_block` already maps math fences back to `MarkdownNode::Math`.
/// Delimiter replacement preserves line count, so source ranges still select
/// the original Markdown rather than the temporary representation.
fn protect_multiline_display_math(markdown: &str) -> Cow<'_, str> {
    if !markdown.contains("$$") && !markdown.contains(r"\[") {
        return Cow::Borrowed(markdown);
    }
    let lines = markdown.split('\n').collect::<Vec<_>>();
    let mut replacements = Vec::<(usize, String)>::new();
    let mut index = 0_usize;
    let mut markdown_fence = None::<(char, usize)>;

    while index < lines.len() {
        let line = lines[index];
        if let Some((marker, minimum_len)) = markdown_fence {
            if closing_markdown_fence(line, marker, minimum_len) {
                markdown_fence = None;
            }
            index += 1;
            continue;
        }

        if let Some((marker, len)) = opening_markdown_fence(line) {
            markdown_fence = Some((marker, len));
            index += 1;
            continue;
        }

        let (opening_delimiter, closing_delimiter) =
            if standalone_math_delimiter_prefix_at(&lines, index, "$$").is_some() {
                ("$$", "$$")
            } else if standalone_math_delimiter_prefix_at(&lines, index, r"\[").is_some() {
                (r"\[", r"\]")
            } else {
                index += 1;
                continue;
            };
        let opening_prefix = standalone_math_delimiter_prefix_at(&lines, index, opening_delimiter)
            .expect("opening delimiter was identified above");
        let Some(closing_index) = lines[index + 1..]
            .iter()
            .enumerate()
            .position(|(offset, _)| {
                standalone_math_delimiter_prefix_at(&lines, index + offset + 1, closing_delimiter)
                    == Some(opening_prefix)
            })
            .map(|offset| index + offset + 1)
        else {
            index += 1;
            continue;
        };

        let body = &lines[index + 1..closing_index];
        let (marker, len) = collision_free_math_fence(body);
        let fence = marker.to_string().repeat(len);
        replacements.push((index, format!("{opening_prefix}{fence}math")));
        let closing_prefix =
            standalone_math_delimiter_prefix_at(&lines, closing_index, closing_delimiter)
                .expect("closing delimiter was identified above");
        replacements.push((closing_index, format!("{closing_prefix}{fence}")));
        index = closing_index + 1;
    }

    if replacements.is_empty() {
        return Cow::Borrowed(markdown);
    }

    let mut replacements = replacements.into_iter().peekable();
    let mut protected = String::with_capacity(markdown.len().saturating_add(16));
    for (line_index, line) in lines.into_iter().enumerate() {
        if line_index > 0 {
            protected.push('\n');
        }
        if replacements
            .peek()
            .is_some_and(|(replacement_index, _)| *replacement_index == line_index)
        {
            let (_, replacement) = replacements.next().expect("replacement was peeked above");
            protected.push_str(&replacement);
        } else {
            protected.push_str(line);
        }
    }
    Cow::Owned(protected)
}

fn standalone_math_delimiter_prefix_at<'a>(
    lines: &[&'a str],
    index: usize,
    delimiter: &str,
) -> Option<&'a str> {
    let line = *lines.get(index)?;
    let (quote_prefix, content) = block_quote_prefix_and_content(line);
    let indentation = content.chars().take_while(|ch| *ch == ' ').count();
    if content[indentation..].trim_end() != delimiter {
        return None;
    }
    let prefix_len = quote_prefix.len().saturating_add(indentation);
    if indentation <= 3 {
        return Some(&line[..prefix_len]);
    }
    list_container_content_indent(lines, index, quote_prefix, indentation)
        .filter(|content_indent| indentation <= content_indent.saturating_add(3))
        .map(|_| &line[..prefix_len])
}

fn list_container_content_indent(
    lines: &[&str],
    index: usize,
    quote_prefix: &str,
    indentation: usize,
) -> Option<usize> {
    let mut minimum_intervening_indent = usize::MAX;
    for candidate in lines[..index].iter().rev() {
        if candidate.trim().is_empty() {
            continue;
        }
        let (candidate_quote_prefix, content) = block_quote_prefix_and_content(candidate);
        if candidate_quote_prefix != quote_prefix {
            break;
        }
        let candidate_indent = content.chars().take_while(|ch| *ch == ' ').count();
        if candidate_indent >= indentation {
            continue;
        }
        let trimmed = &content[candidate_indent..];
        let Some(marker_width) = markdown_list_marker_width(trimmed) else {
            minimum_intervening_indent = minimum_intervening_indent.min(candidate_indent);
            continue;
        };
        let content_indent = candidate_indent.saturating_add(marker_width);
        if content_indent <= indentation && minimum_intervening_indent >= content_indent {
            return Some(content_indent);
        }
        minimum_intervening_indent = minimum_intervening_indent.min(candidate_indent);
    }
    None
}

fn block_quote_prefix_and_content(line: &str) -> (&str, &str) {
    let bytes = line.as_bytes();
    let mut position = 0_usize;
    loop {
        let container_start = position;
        let indentation_start = position;
        while position.saturating_sub(indentation_start) < 3 && bytes.get(position) == Some(&b' ') {
            position += 1;
        }
        if bytes.get(position) != Some(&b'>') {
            position = container_start;
            break;
        }
        position += 1;
        if bytes
            .get(position)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            position += 1;
        }
    }
    (&line[..position], &line[position..])
}

fn markdown_list_marker_width(content: &str) -> Option<usize> {
    let bytes = content.as_bytes();
    let marker_end = if bytes
        .first()
        .is_some_and(|byte| matches!(byte, b'-' | b'*' | b'+'))
    {
        1
    } else {
        let digits = bytes
            .iter()
            .take(9)
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0
            || !bytes
                .get(digits)
                .is_some_and(|byte| matches!(byte, b'.' | b')'))
        {
            return None;
        }
        digits + 1
    };
    let whitespace = bytes[marker_end..]
        .iter()
        .take(4)
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    (whitespace > 0).then_some(marker_end + whitespace)
}

fn opening_markdown_fence(line: &str) -> Option<(char, usize)> {
    let (_, trimmed) = markdown_container_content(line)?;
    let marker = trimmed.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let len = trimmed
        .chars()
        .take_while(|candidate| *candidate == marker)
        .count();
    (len >= 3).then_some((marker, len))
}

fn closing_markdown_fence(line: &str, marker: char, minimum_len: usize) -> bool {
    let Some((_, trimmed)) = markdown_container_content(line) else {
        return false;
    };
    let len = trimmed
        .chars()
        .take_while(|candidate| *candidate == marker)
        .count();
    len >= minimum_len && trimmed[len..].trim().is_empty()
}

/// Return the block-container prefix and the content that CommonMark sees on
/// the line. Up to three indentation spaces and nested blockquote markers are
/// supported; four leading spaces deliberately remain an indented code block.
fn markdown_container_content(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    let mut position = 0_usize;
    loop {
        let indentation_start = position;
        while bytes.get(position) == Some(&b' ') {
            position += 1;
        }
        if position.saturating_sub(indentation_start) > 3 {
            return None;
        }
        if bytes.get(position) != Some(&b'>') {
            break;
        }
        position += 1;
        if bytes.get(position) == Some(&b' ') {
            position += 1;
        }
    }
    Some((&line[..position], &line[position..]))
}

fn collision_free_math_fence(lines: &[&str]) -> (char, usize) {
    let longest_backticks = longest_marker_run(lines, '`');
    let longest_tildes = longest_marker_run(lines, '~');
    if longest_backticks <= longest_tildes {
        ('`', longest_backticks.saturating_add(1).max(3))
    } else {
        ('~', longest_tildes.saturating_add(1).max(3))
    }
}

fn longest_marker_run(lines: &[&str], marker: char) -> usize {
    lines
        .iter()
        .flat_map(|line| line.split(|candidate| candidate != marker))
        .map(str::len)
        .max()
        .unwrap_or(0)
}

fn renumber_footnotes(blocks: &mut [MarkdownBlock]) {
    let mut ordinals = HashMap::new();
    for block in blocks.iter() {
        collect_footnote_definitions(&block.parsed, &mut ordinals);
    }
    for block in blocks {
        apply_footnote_ordinals(&mut block.parsed, &ordinals);
    }
}

fn collect_footnote_definitions(node: &MarkdownNode, ordinals: &mut HashMap<String, usize>) {
    match node {
        MarkdownNode::FootnoteDefinition { name, blocks } => {
            let next = ordinals.len() + 1;
            ordinals.entry(name.clone()).or_insert(next);
            for block in blocks {
                collect_footnote_definitions(block, ordinals);
            }
        }
        MarkdownNode::Quote(blocks)
        | MarkdownNode::Alert { blocks, .. }
        | MarkdownNode::Directive { blocks, .. } => {
            for block in blocks {
                collect_footnote_definitions(block, ordinals);
            }
        }
        MarkdownNode::List { items, .. } => {
            for item in items {
                for block in &item.blocks {
                    collect_footnote_definitions(block, ordinals);
                }
            }
        }
        MarkdownNode::DescriptionList(items) => {
            for item in items {
                for detail in &item.details {
                    collect_footnote_definitions(detail, ordinals);
                }
            }
        }
        _ => {}
    }
}

fn apply_footnote_ordinals(node: &mut MarkdownNode, ordinals: &HashMap<String, usize>) {
    match node {
        MarkdownNode::Paragraph(inlines)
        | MarkdownNode::Heading {
            content: inlines, ..
        }
        | MarkdownNode::Subtext(inlines) => apply_inline_footnote_ordinals(inlines, ordinals),
        MarkdownNode::Quote(blocks)
        | MarkdownNode::Alert { blocks, .. }
        | MarkdownNode::Directive { blocks, .. } => {
            for block in blocks {
                apply_footnote_ordinals(block, ordinals);
            }
        }
        MarkdownNode::List { items, .. } => {
            for item in items {
                for block in &mut item.blocks {
                    apply_footnote_ordinals(block, ordinals);
                }
            }
        }
        MarkdownNode::DescriptionList(items) => {
            for item in items {
                apply_inline_footnote_ordinals(&mut item.term, ordinals);
                for detail in &mut item.details {
                    apply_footnote_ordinals(detail, ordinals);
                }
            }
        }
        MarkdownNode::Table { rows, .. } => {
            for row in rows {
                for cell in &mut row.cells {
                    apply_inline_footnote_ordinals(cell, ordinals);
                }
            }
        }
        MarkdownNode::FootnoteDefinition { name, blocks } => {
            if let Some(ordinal) = ordinals.get(name) {
                *name = ordinal.to_string();
            }
            for block in blocks {
                apply_footnote_ordinals(block, ordinals);
            }
        }
        _ => {}
    }
}

fn apply_inline_footnote_ordinals(
    inlines: &mut [MarkdownInline],
    ordinals: &HashMap<String, usize>,
) {
    for inline in inlines {
        match inline {
            MarkdownInline::FootnoteReference(name) => {
                if let Some(ordinal) = ordinals.get(name) {
                    *name = ordinal.to_string();
                }
            }
            MarkdownInline::Emphasis(children)
            | MarkdownInline::Strong(children)
            | MarkdownInline::Strikethrough(children)
            | MarkdownInline::Underline(children)
            | MarkdownInline::Highlight(children)
            | MarkdownInline::Insert(children)
            | MarkdownInline::Superscript(children)
            | MarkdownInline::Subscript(children)
            | MarkdownInline::Spoiler(children)
            | MarkdownInline::Link {
                label: children, ..
            }
            | MarkdownInline::WikiLink {
                label: children, ..
            } => apply_inline_footnote_ordinals(children, ordinals),
            _ => {}
        }
    }
}

fn markdown_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.superscript = true;
    options.extension.footnotes = true;
    options.extension.inline_footnotes = true;
    options.extension.description_lists = true;
    options.extension.front_matter_delimiter = Some("---".to_string());
    options.extension.multiline_block_quotes = true;
    options.extension.alerts = true;
    options.extension.math_dollars = true;
    options.extension.math_latex = true;
    options.extension.math_code = true;
    options.extension.shortcodes = true;
    options.extension.wikilinks_title_after_pipe = true;
    // Keep CommonMark/GFM's `__strong__` meaning. Comrak's underline extension
    // reassigns the same delimiter and therefore cannot be enabled globally.
    options.extension.underline = false;
    options.extension.subscript = true;
    options.extension.spoiler = true;
    options.extension.cjk_friendly_emphasis = true;
    options.extension.subtext = true;
    options.extension.highlight = true;
    options.extension.insert = true;
    options.extension.block_directive = true;
    options.extension.header_attributes = true;
    options.extension.fenced_code_attributes = true;
    options.extension.inline_code_attributes = true;
    options.extension.link_attributes = true;
    options.parse.smart = true;
    options.parse.relaxed_tasklist_matching = true;
    options.parse.tasklist_in_table = true;
    options.parse.relaxed_autolinks = true;
    options.parse.leave_footnote_definitions = true;
    options
}

fn convert_block<'a>(node: &'a AstNode<'a>) -> Option<MarkdownNode> {
    let value = node.data().value.clone();
    match value {
        NodeValue::Paragraph => {
            let content = convert_inlines(node);
            if let [
                MarkdownInline::Math {
                    literal,
                    display: true,
                },
            ] = content.as_slice()
            {
                Some(MarkdownNode::Math {
                    literal: literal.clone(),
                    display: true,
                })
            } else if let [MarkdownInline::Image { url, title, alt }] = content.as_slice() {
                Some(MarkdownNode::Image {
                    url: url.clone(),
                    title: title.clone(),
                    alt: alt.clone(),
                })
            } else {
                Some(MarkdownNode::Paragraph(content))
            }
        }
        NodeValue::Heading(heading) => Some(MarkdownNode::Heading {
            level: heading.level,
            content: convert_inlines(node),
        }),
        NodeValue::BlockQuote | NodeValue::MultilineBlockQuote(_) => {
            Some(MarkdownNode::Quote(convert_blocks(node)))
        }
        NodeValue::Alert(alert) => Some(MarkdownNode::Alert {
            kind: match alert.alert_type {
                AlertType::Note => MarkdownAlertKind::Note,
                AlertType::Tip => MarkdownAlertKind::Tip,
                AlertType::Important => MarkdownAlertKind::Important,
                AlertType::Warning => MarkdownAlertKind::Warning,
                AlertType::Caution => MarkdownAlertKind::Caution,
            },
            title: alert.title,
            blocks: convert_blocks(node),
        }),
        NodeValue::CodeBlock(code) => {
            let mut language = code
                .info
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_matches(['{', '}'])
                .trim_start_matches('.')
                .to_ascii_lowercase();
            if language.is_empty()
                && let Some(attribute_language) = node.data().attrs.as_deref().and_then(|attrs| {
                    attrs.classes.iter().find(|class| {
                        !matches!(
                            class.to_ascii_lowercase().as_str(),
                            "numberlines" | "number-lines" | "line-numbers" | "nowrap"
                        )
                    })
                })
            {
                language = attribute_language.to_ascii_lowercase();
            }
            if matches!(language.as_str(), "math" | "tex" | "latex" | "katex") {
                Some(MarkdownNode::Math {
                    literal: code.literal.trim_end_matches('\n').to_string(),
                    display: true,
                })
            } else if is_diagram_language(&language) {
                Some(MarkdownNode::Diagram {
                    language,
                    literal: code.literal,
                })
            } else {
                Some(MarkdownNode::Code {
                    language,
                    literal: code.literal,
                    fenced: code.fenced,
                })
            }
        }
        NodeValue::List(list) => Some(MarkdownNode::List {
            ordered: list.list_type == ListType::Ordered,
            start: list.start.max(1),
            delimiter: if list.delimiter == ListDelimType::Paren {
                ')'
            } else {
                '.'
            },
            tight: list.tight,
            items: node
                .children()
                .filter_map(|item| {
                    if !matches!(
                        item.data().value,
                        NodeValue::Item(_) | NodeValue::TaskItem(_)
                    ) {
                        return None;
                    }
                    let checked = item.descendants().find_map(|candidate| {
                        if let NodeValue::TaskItem(task) = candidate.data().value {
                            Some(task.symbol.is_some())
                        } else {
                            None
                        }
                    });
                    let blocks = item
                        .children()
                        .filter(|child| !matches!(child.data().value, NodeValue::TaskItem(_)))
                        .filter_map(convert_block)
                        .collect();
                    Some(MarkdownListItem { checked, blocks })
                })
                .collect(),
        }),
        NodeValue::DescriptionList => {
            let items = node
                .children()
                .filter_map(|item| {
                    if !matches!(item.data().value, NodeValue::DescriptionItem(_)) {
                        return None;
                    }
                    let mut term = Vec::new();
                    let mut details = Vec::new();
                    for child in item.children() {
                        match child.data().value {
                            NodeValue::DescriptionTerm => term = convert_inlines(child),
                            NodeValue::DescriptionDetails => details = convert_blocks(child),
                            _ => {}
                        }
                    }
                    Some(MarkdownDescriptionItem { term, details })
                })
                .collect();
            Some(MarkdownNode::DescriptionList(items))
        }
        NodeValue::Table(table) => Some(MarkdownNode::Table {
            alignments: table
                .alignments
                .iter()
                .map(|alignment| match alignment {
                    TableAlignment::None => MarkdownAlignment::None,
                    TableAlignment::Left => MarkdownAlignment::Left,
                    TableAlignment::Center => MarkdownAlignment::Center,
                    TableAlignment::Right => MarkdownAlignment::Right,
                })
                .collect(),
            rows: node
                .children()
                .filter_map(|row| {
                    let NodeValue::TableRow(header) = row.data().value else {
                        return None;
                    };
                    Some(MarkdownTableRow {
                        header,
                        cells: row.children().map(convert_inlines).collect(),
                    })
                })
                .collect(),
        }),
        NodeValue::ThematicBreak => Some(MarkdownNode::ThematicBreak),
        NodeValue::Math(math) => Some(MarkdownNode::Math {
            literal: restore_math_placeholders(math.literal),
            display: math.display_math,
        }),
        NodeValue::FootnoteDefinition(footnote) => Some(MarkdownNode::FootnoteDefinition {
            name: footnote.name,
            blocks: convert_blocks(node),
        }),
        NodeValue::FrontMatter(front_matter) => Some(MarkdownNode::FrontMatter(front_matter)),
        NodeValue::HtmlBlock(html) => {
            if let Some(MarkdownInline::Image { url, title, alt }) = safe_html_image(&html.literal)
            {
                Some(MarkdownNode::Image { url, title, alt })
            } else {
                Some(MarkdownNode::Html(html.literal))
            }
        }
        NodeValue::Subtext => Some(MarkdownNode::Subtext(convert_inlines(node))),
        NodeValue::BlockDirective(directive) => Some(MarkdownNode::Directive {
            info: directive.info,
            blocks: convert_blocks(node),
        }),
        NodeValue::DescriptionTerm => Some(MarkdownNode::Paragraph(convert_inlines(node))),
        NodeValue::DescriptionDetails | NodeValue::Item(_) => {
            Some(MarkdownNode::Quote(convert_blocks(node)))
        }
        NodeValue::TaskItem(_) | NodeValue::TableRow(_) | NodeValue::TableCell => None,
        _ if node.first_child().is_some() => Some(MarkdownNode::Paragraph(convert_inlines(node))),
        _ => None,
    }
}

fn convert_blocks<'a>(node: &'a AstNode<'a>) -> Vec<MarkdownNode> {
    node.children().filter_map(convert_block).collect()
}

fn convert_inlines<'a>(node: &'a AstNode<'a>) -> Vec<MarkdownInline> {
    let mut converted = Vec::new();
    let mut html_styles: Vec<SafeHtmlInlineStyle> = Vec::new();
    for child in node.children() {
        if let NodeValue::HtmlInline(html) = &child.data().value {
            if let Some(mut image) = safe_html_image(html) {
                for style in html_styles.iter().rev() {
                    image = style.wrap(image);
                }
                converted.push(image);
                continue;
            }
            if let Some(action) = safe_html_inline_action(html) {
                match action {
                    SafeHtmlInlineAction::Open(style) => html_styles.push(style),
                    SafeHtmlInlineAction::Close(style) => {
                        if let Some(position) =
                            html_styles.iter().rposition(|active| *active == style)
                        {
                            html_styles.truncate(position);
                        }
                    }
                    SafeHtmlInlineAction::Break => converted.push(MarkdownInline::HardBreak),
                }
                continue;
            }
        }
        let mut additions = match &child.data().value {
            NodeValue::Text(text) => split_obsidian_embeds(text),
            _ => convert_inline(child).into_iter().collect(),
        };
        for mut inline in additions.drain(..) {
            for style in html_styles.iter().rev() {
                inline = style.wrap(inline);
            }
            converted.push(inline);
        }
    }
    converted
}

fn safe_html_image(html: &str) -> Option<MarkdownInline> {
    let trimmed = html.trim();
    if !trimmed
        .get(..trimmed.len().min(4))
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("<img"))
    {
        return None;
    }
    let src = html_attribute(trimmed, "src")?;
    if src.trim().is_empty() {
        return None;
    }
    Some(MarkdownInline::Image {
        url: src,
        title: html_attribute(trimmed, "title").unwrap_or_default(),
        alt: html_attribute(trimmed, "alt").unwrap_or_default(),
    })
}

fn html_attribute(tag: &str, requested: &str) -> Option<String> {
    static ATTRIBUTE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?i)([a-z_:][a-z0-9_:.-]*)\s*=\s*(?:\"([^\"]*)\"|'([^']*)'|([^\s>]+))"#,
        )
        .expect("HTML attribute regex is valid")
    });
    ATTRIBUTE.captures_iter(tag).find_map(|captures| {
        captures
            .get(1)
            .filter(|name| name.as_str().eq_ignore_ascii_case(requested))
            .and_then(|_| {
                captures
                    .get(2)
                    .or_else(|| captures.get(3))
                    .or_else(|| captures.get(4))
            })
            .map(|value| value.as_str().to_string())
    })
}

fn split_obsidian_embeds(text: &str) -> Vec<MarkdownInline> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("![[") {
        if start > 0 {
            out.push(MarkdownInline::Text(rest[..start].to_string()));
        }
        let after_open = &rest[start + 3..];
        let Some(end) = after_open.find("]]") else {
            out.push(MarkdownInline::Text(rest[start..].to_string()));
            rest = "";
            break;
        };
        let body = &after_open[..end];
        let (target, alias) = body
            .split_once('|')
            .map_or((body, ""), |(target, alias)| (target, alias));
        let target = target.trim();
        let alias = alias.trim();
        if target.is_empty() {
            out.push(MarkdownInline::Text(format!("![[{body}]]")));
        } else if is_raster_image_target(target) {
            out.push(MarkdownInline::Image {
                url: target.to_string(),
                title: String::new(),
                alt: if alias.is_empty() {
                    target.rsplit('/').next().unwrap_or(target).to_string()
                } else {
                    alias.to_string()
                },
            });
        } else {
            out.push(MarkdownInline::WikiLink {
                url: target.to_string(),
                label: vec![MarkdownInline::Text(format!(
                    "↳ {}",
                    if alias.is_empty() { target } else { alias }
                ))],
            });
        }
        rest = &after_open[end + 2..];
    }
    if !rest.is_empty() {
        out.push(MarkdownInline::Text(rest.to_string()));
    }
    if out.is_empty() {
        out.push(MarkdownInline::Text(text.to_string()));
    }
    out
}

fn is_raster_image_target(target: &str) -> bool {
    let path = target
        .split(['?', '#'])
        .next()
        .unwrap_or(target)
        .to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"]
        .iter()
        .any(|extension| path.ends_with(extension))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafeHtmlInlineStyle {
    Emphasis,
    Strong,
    Underline,
    Highlight,
    Insert,
    Strikethrough,
    Superscript,
    Subscript,
    Keyboard,
}

impl SafeHtmlInlineStyle {
    fn wrap(self, inline: MarkdownInline) -> MarkdownInline {
        let children = vec![inline];
        match self {
            Self::Emphasis => MarkdownInline::Emphasis(children),
            Self::Strong => MarkdownInline::Strong(children),
            Self::Underline => MarkdownInline::Underline(children),
            Self::Highlight | Self::Keyboard => MarkdownInline::Highlight(children),
            Self::Insert => MarkdownInline::Insert(children),
            Self::Strikethrough => MarkdownInline::Strikethrough(children),
            Self::Superscript => MarkdownInline::Superscript(children),
            Self::Subscript => MarkdownInline::Subscript(children),
        }
    }
}

enum SafeHtmlInlineAction {
    Open(SafeHtmlInlineStyle),
    Close(SafeHtmlInlineStyle),
    Break,
}

fn safe_html_inline_action(html: &str) -> Option<SafeHtmlInlineAction> {
    let tag = html.trim().to_ascii_lowercase();
    if tag.starts_with("<br") {
        return Some(SafeHtmlInlineAction::Break);
    }
    let closing = tag.starts_with("</");
    let name = tag
        .trim_start_matches('<')
        .trim_start_matches('/')
        .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '>' | '/'))
        .next()?;
    let style = match name {
        "em" | "i" => SafeHtmlInlineStyle::Emphasis,
        "strong" | "b" => SafeHtmlInlineStyle::Strong,
        "u" => SafeHtmlInlineStyle::Underline,
        "mark" => SafeHtmlInlineStyle::Highlight,
        "ins" => SafeHtmlInlineStyle::Insert,
        "del" | "s" | "strike" => SafeHtmlInlineStyle::Strikethrough,
        "sup" => SafeHtmlInlineStyle::Superscript,
        "sub" => SafeHtmlInlineStyle::Subscript,
        "kbd" => SafeHtmlInlineStyle::Keyboard,
        _ => return None,
    };
    Some(if closing {
        SafeHtmlInlineAction::Close(style)
    } else {
        SafeHtmlInlineAction::Open(style)
    })
}

fn convert_inline<'a>(node: &'a AstNode<'a>) -> Option<MarkdownInline> {
    let value = node.data().value.clone();
    match value {
        NodeValue::Text(text) => Some(MarkdownInline::Text(text.into_owned())),
        NodeValue::Code(code) => Some(MarkdownInline::Code(code.literal)),
        NodeValue::Emph => Some(MarkdownInline::Emphasis(convert_inlines(node))),
        NodeValue::Strong => Some(MarkdownInline::Strong(convert_inlines(node))),
        NodeValue::Strikethrough => Some(MarkdownInline::Strikethrough(convert_inlines(node))),
        NodeValue::Underline => Some(MarkdownInline::Underline(convert_inlines(node))),
        NodeValue::Highlight => Some(MarkdownInline::Highlight(convert_inlines(node))),
        NodeValue::Insert => Some(MarkdownInline::Insert(convert_inlines(node))),
        NodeValue::Superscript => Some(MarkdownInline::Superscript(convert_inlines(node))),
        NodeValue::Subscript => Some(MarkdownInline::Subscript(convert_inlines(node))),
        NodeValue::SpoileredText => Some(MarkdownInline::Spoiler(convert_inlines(node))),
        NodeValue::Link(link) => Some(MarkdownInline::Link {
            url: link.url,
            title: link.title,
            label: convert_inlines(node),
        }),
        NodeValue::WikiLink(link) => {
            let mut url = link.url;
            let mut label = convert_inlines(node);
            let label_text = inline_plain_text(&label);
            if !looks_like_link_target(&url) && looks_like_link_target(&label_text) {
                label = vec![MarkdownInline::Text(url)];
                url = label_text;
            }
            Some(MarkdownInline::WikiLink { url, label })
        }
        NodeValue::Image(image) => Some(MarkdownInline::Image {
            url: image.url,
            title: image.title,
            alt: inline_plain_text(&convert_inlines(node)),
        }),
        NodeValue::Math(math) => Some(MarkdownInline::Math {
            literal: restore_math_placeholders(math.literal),
            display: math.display_math,
        }),
        NodeValue::FootnoteReference(reference) => {
            Some(MarkdownInline::FootnoteReference(reference.name))
        }
        NodeValue::HtmlInline(html) | NodeValue::Raw(html) => Some(MarkdownInline::Html(html)),
        NodeValue::ShortCode(shortcode) => Some(MarkdownInline::Emoji(shortcode.emoji)),
        NodeValue::SoftBreak => Some(MarkdownInline::SoftBreak),
        NodeValue::LineBreak => Some(MarkdownInline::HardBreak),
        NodeValue::Escaped | NodeValue::EscapedTag(_) => Some(MarkdownInline::Text(
            inline_plain_text(&convert_inlines(node)),
        )),
        _ if node.first_child().is_some() => Some(MarkdownInline::Text(inline_plain_text(
            &convert_inlines(node),
        ))),
        _ => None,
    }
}

fn looks_like_link_target(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.contains("://")
        || value.starts_with("mailto:")
        || value.starts_with('#')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.contains('/')
        || [
            ".md",
            ".markdown",
            ".png",
            ".jpg",
            ".jpeg",
            ".gif",
            ".webp",
            ".bmp",
            ".svg",
        ]
        .iter()
        .any(|extension| value.ends_with(extension))
}

pub(in crate::app) fn inline_plain_text(inlines: &[MarkdownInline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            MarkdownInline::Text(text)
            | MarkdownInline::Code(text)
            | MarkdownInline::Html(text)
            | MarkdownInline::Emoji(text)
            | MarkdownInline::FootnoteReference(text) => out.push_str(text),
            MarkdownInline::Emphasis(children)
            | MarkdownInline::Strong(children)
            | MarkdownInline::Strikethrough(children)
            | MarkdownInline::Underline(children)
            | MarkdownInline::Highlight(children)
            | MarkdownInline::Insert(children)
            | MarkdownInline::Superscript(children)
            | MarkdownInline::Subscript(children)
            | MarkdownInline::Spoiler(children) => out.push_str(&inline_plain_text(children)),
            MarkdownInline::Link { label, .. } | MarkdownInline::WikiLink { label, .. } => {
                out.push_str(&inline_plain_text(label));
            }
            MarkdownInline::Image { alt, .. } => out.push_str(alt),
            MarkdownInline::Math { literal, .. } => out.push_str(literal),
            MarkdownInline::SoftBreak => out.push(' '),
            MarkdownInline::HardBreak => out.push('\n'),
        }
    }
    out
}

fn markdown_node_kind(node: &MarkdownNode) -> TranscriptNodeKind {
    match node {
        MarkdownNode::Heading { .. } => TranscriptNodeKind::MarkdownHeading,
        MarkdownNode::Quote(_) => TranscriptNodeKind::MarkdownQuote,
        MarkdownNode::Alert { .. } => TranscriptNodeKind::MarkdownAlert,
        MarkdownNode::Code { .. } | MarkdownNode::FrontMatter(_) | MarkdownNode::Html(_) => {
            TranscriptNodeKind::MarkdownCode
        }
        MarkdownNode::List { .. } | MarkdownNode::DescriptionList(_) => {
            TranscriptNodeKind::MarkdownList
        }
        MarkdownNode::Table { .. } => TranscriptNodeKind::MarkdownTable,
        MarkdownNode::Math { .. } => TranscriptNodeKind::MarkdownMath,
        MarkdownNode::Image { .. } => TranscriptNodeKind::MarkdownImage,
        MarkdownNode::FootnoteDefinition { .. } => TranscriptNodeKind::MarkdownFootnote,
        MarkdownNode::Diagram { .. } => TranscriptNodeKind::MarkdownDiagram,
        _ => TranscriptNodeKind::MarkdownParagraph,
    }
}

pub(in crate::app) fn render_parsed_markdown_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    block: &MarkdownBlock,
    width: u16,
) {
    render_markdown_node(out, prefix, &block.parsed, width);
}

fn render_markdown_node(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    node: &MarkdownNode,
    width: u16,
) {
    match node {
        MarkdownNode::Paragraph(inlines) => render_paragraph(out, prefix, inlines, width),
        MarkdownNode::Heading { level, content } => {
            render_heading(out, prefix, usize::from(*level), content, width);
        }
        MarkdownNode::Quote(blocks) => {
            let quote_prefix = format!("{prefix}│ ");
            for block in blocks {
                render_markdown_node(out, &quote_prefix, block, width);
            }
        }
        MarkdownNode::Alert {
            kind,
            title,
            blocks,
        } => render_alert(out, prefix, *kind, title.as_deref(), blocks, width),
        MarkdownNode::Code {
            language,
            literal,
            fenced,
        } => {
            let language = if language.is_empty() {
                if *fenced { "text" } else { "indented" }
            } else {
                language.as_str()
            };
            let source = format!("```{language}\n{}\n```", literal.trim_end_matches('\n'));
            push_markdown_code_block(out, prefix, &source, width);
        }
        MarkdownNode::Diagram { language, literal } => {
            render_diagram(out, prefix, language, literal, width)
        }
        MarkdownNode::List {
            ordered,
            start,
            delimiter,
            items,
            ..
        } => render_list(out, prefix, *ordered, *start, *delimiter, items, width, 0),
        MarkdownNode::DescriptionList(items) => {
            for item in items {
                let term = inline_plain_text(&item.term);
                push_single_line(
                    out,
                    prefix,
                    &format!("◆ {term}"),
                    Style::default()
                        .fg(agena_tui_components::theme::accent_color())
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                let detail_prefix = format!("{prefix}  │ ");
                for detail in &item.details {
                    render_markdown_node(out, &detail_prefix, detail, width);
                }
            }
        }
        MarkdownNode::Table { alignments, rows } => {
            render_ast_table(out, prefix, alignments, rows, width)
        }
        MarkdownNode::ThematicBreak => push_markdown_rule(out, prefix, width),
        MarkdownNode::Math { literal, display } => {
            let source = if *display {
                format!("$$\n{literal}\n$$")
            } else {
                format!("${literal}$")
            };
            push_math_block(out, prefix, &source, width);
        }
        MarkdownNode::Image { url, title, alt } => {
            render_image_card(out, prefix, alt, url, title, width)
        }
        MarkdownNode::FootnoteDefinition { name, blocks } => {
            push_single_line(
                out,
                prefix,
                &format!("[^{name}]"),
                Style::default()
                    .fg(agena_tui_components::theme::accent_color())
                    .add_modifier(Modifier::BOLD),
                width,
            );
            let footnote_prefix = format!("{prefix}  ");
            for block in blocks {
                render_markdown_node(out, &footnote_prefix, block, width);
            }
        }
        MarkdownNode::FrontMatter(front_matter) => {
            let source = format!("```yaml\n{}\n```", front_matter_body(front_matter));
            push_markdown_code_block(out, prefix, &source, width);
        }
        MarkdownNode::Html(html) => {
            let source = format!("```html\n{}\n```", html.trim_end_matches('\n'));
            push_markdown_code_block(out, prefix, &source, width);
        }
        MarkdownNode::Subtext(inlines) => {
            for mut line in rich_inline_lines(inlines, Style::default().add_modifier(Modifier::DIM))
            {
                line.spans.insert(0, Span::raw("⌞ "));
                push_wrapped_rich_line(out, prefix, prefix, line, width);
            }
        }
        MarkdownNode::Directive { info, blocks } => {
            push_single_line(
                out,
                prefix,
                &format!("╭─ {info}"),
                Style::default()
                    .fg(agena_tui_components::theme::accent_color())
                    .add_modifier(Modifier::BOLD),
                width,
            );
            let body_prefix = format!("{prefix}│ ");
            for block in blocks {
                render_markdown_node(out, &body_prefix, block, width);
            }
            push_single_line(
                out,
                prefix,
                "╰─",
                Style::default().fg(agena_tui_components::theme::muted_color()),
                width,
            );
        }
    }
}

fn render_paragraph(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    inlines: &[MarkdownInline],
    width: u16,
) {
    if inlines_contain_rich_graphics(inlines)
        && push_rich_inline_graphics(out, prefix, inlines, Style::default(), width)
    {
        return;
    }
    for line in rich_inline_lines(inlines, Style::default()) {
        push_wrapped_rich_line(out, prefix, prefix, line, width);
    }
}

fn render_heading(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    level: usize,
    inlines: &[MarkdownInline],
    width: u16,
) {
    let marker = match level {
        1 => "══",
        2 => "──",
        _ => "›",
    };
    let style = Style::default()
        .fg(if level <= 2 {
            agena_tui_components::theme::accent_color()
        } else {
            agena_tui_components::theme::info_color()
        })
        .add_modifier(Modifier::BOLD);
    let first_prefix = format!("{prefix}{marker} ");
    let continuation = format!("{prefix}{}", " ".repeat(UnicodeWidthStr::width(marker) + 1));
    if inlines_contain_rich_graphics(inlines)
        && push_rich_inline_graphics(out, &first_prefix, inlines, style, width)
    {
        return;
    }
    for line in rich_inline_lines(inlines, style) {
        push_wrapped_rich_line(out, &first_prefix, &continuation, line, width);
    }
}

#[derive(Debug)]
enum RichInlineAtom {
    Text(Span<'static>),
    Math(String),
    Image { url: String, alt: String },
}

fn push_rich_inline_graphics(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    inlines: &[MarkdownInline],
    base_style: Style,
    width: u16,
) -> bool {
    let mut atoms = Vec::new();
    if !append_rich_inline_atoms(&mut atoms, inlines, base_style) {
        return false;
    }
    let prefix_width = u16::try_from(UnicodeWidthStr::width(prefix)).unwrap_or(u16::MAX);
    let available = width.saturating_sub(prefix_width).max(1);
    if layout_config().native_graphics {
        let mut rendered = Vec::with_capacity(atoms.len());
        let mut total_width = 0_u16;
        let mut height = 1_u16;
        for atom in atoms {
            match atom {
                RichInlineAtom::Text(span) => {
                    let span_width = u16::try_from(UnicodeWidthStr::width(span.content.as_ref()))
                        .unwrap_or(u16::MAX);
                    total_width = total_width.saturating_add(span_width);
                    rendered.push((Some(span), None, span_width));
                }
                RichInlineAtom::Math(literal) => {
                    let Ok(artifact) = crate::math_render::render_formula(&literal, false) else {
                        return false;
                    };
                    total_width = total_width.saturating_add(artifact.size.width);
                    height = height.max(artifact.size.height);
                    let size = artifact.size;
                    rendered.push((None, Some((artifact, size)), size.width));
                }
                RichInlineAtom::Image { url, alt } => {
                    if let Ok(artifact) = render_markdown_image(&url) {
                        let size = fit_image_size(artifact.size, available.min(12), 4);
                        let size = Size::new(size.0, size.1);
                        total_width = total_width.saturating_add(size.width);
                        height = height.max(size.height);
                        rendered.push((None, Some((artifact, size)), size.width));
                    } else {
                        let text =
                            format!("🖼 {} ({url})", if alt.is_empty() { "Image" } else { &alt });
                        let span_width = u16::try_from(UnicodeWidthStr::width(text.as_str()))
                            .unwrap_or(u16::MAX);
                        total_width = total_width.saturating_add(span_width);
                        rendered.push((
                            Some(Span::styled(
                                text,
                                Style::default().fg(agena_tui_components::theme::info_color()),
                            )),
                            None,
                            span_width,
                        ));
                    }
                }
            }
        }
        if total_width > available {
            return false;
        }
        let start = out.len();
        for _ in 0..height {
            out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
        }
        let mut spans = vec![Span::raw(prefix.to_string())];
        let mut column = prefix_width;
        for (span, graphic, atom_width) in rendered {
            if let Some(span) = span {
                spans.push(span);
            } else if let Some((artifact, size)) = graphic {
                out[start].math.push(MathLinePlacement {
                    column,
                    size,
                    artifact,
                });
                spans.push(Span::raw(" ".repeat(usize::from(atom_width))));
            }
            column = column.saturating_add(atom_width);
        }
        out[start + usize::from(height.saturating_sub(1))] = RenderedLine::rich(Line::from(spans));
        return true;
    }

    let mut rows: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    for atom in atoms {
        let block = match atom {
            RichInlineAtom::Text(span) => vec![vec![span]],
            RichInlineAtom::Math(literal) => unicode_formula(&literal, false)
                .into_iter()
                .map(|line| {
                    vec![Span::styled(
                        line,
                        Style::default().fg(agena_tui_components::theme::accent_color()),
                    )]
                })
                .collect(),
            RichInlineAtom::Image { url, alt } => vec![vec![Span::styled(
                format!("🖼 {} ({url})", if alt.is_empty() { "Image" } else { &alt }),
                Style::default().fg(agena_tui_components::theme::info_color()),
            )]],
        };
        append_bottom_aligned_rich(&mut rows, block);
    }
    if rows
        .iter()
        .any(|row| rich_spans_width(row) > usize::from(available))
    {
        return false;
    }
    for mut row in rows {
        row.insert(0, Span::raw(prefix.to_string()));
        out.push(RenderedLine::rich(Line::from(row)));
    }
    true
}

fn append_bottom_aligned_rich(
    rows: &mut Vec<Vec<Span<'static>>>,
    mut block: Vec<Vec<Span<'static>>>,
) {
    let row_width = rows.first().map_or(0, |row| rich_spans_width(row));
    let block_width = block
        .iter()
        .map(|row| rich_spans_width(row))
        .max()
        .unwrap_or(0);
    for row in &mut block {
        let padding = block_width.saturating_sub(rich_spans_width(row));
        if padding > 0 {
            row.push(Span::raw(" ".repeat(padding)));
        }
    }
    if rows.len() < block.len() {
        let mut padding = vec![vec![Span::raw(" ".repeat(row_width))]; block.len() - rows.len()];
        padding.append(rows);
        *rows = padding;
    } else if block.len() < rows.len() {
        let mut padding = vec![vec![Span::raw(" ".repeat(block_width))]; rows.len() - block.len()];
        padding.append(&mut block);
        block = padding;
    }
    for (row, addition) in rows.iter_mut().zip(block) {
        row.extend(addition);
    }
}

fn rich_spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn append_rich_inline_atoms(
    atoms: &mut Vec<RichInlineAtom>,
    inlines: &[MarkdownInline],
    style: Style,
) -> bool {
    for inline in inlines {
        match inline {
            MarkdownInline::Text(text) | MarkdownInline::Emoji(text) => {
                atoms.push(RichInlineAtom::Text(Span::styled(text.clone(), style)));
            }
            MarkdownInline::Code(code) => atoms.push(RichInlineAtom::Text(Span::styled(
                code.clone(),
                style
                    .fg(agena_tui_components::theme::warning_color())
                    .add_modifier(Modifier::BOLD),
            ))),
            MarkdownInline::Emphasis(children) => {
                if !append_rich_inline_atoms(atoms, children, style.add_modifier(Modifier::ITALIC))
                {
                    return false;
                }
            }
            MarkdownInline::Strong(children) => {
                if !append_rich_inline_atoms(atoms, children, style.add_modifier(Modifier::BOLD)) {
                    return false;
                }
            }
            MarkdownInline::Strikethrough(children) => {
                if !append_rich_inline_atoms(
                    atoms,
                    children,
                    style.add_modifier(Modifier::CROSSED_OUT),
                ) {
                    return false;
                }
            }
            MarkdownInline::Underline(children) | MarkdownInline::Insert(children) => {
                if !append_rich_inline_atoms(
                    atoms,
                    children,
                    style.add_modifier(Modifier::UNDERLINED),
                ) {
                    return false;
                }
            }
            MarkdownInline::Highlight(children) => {
                if !append_rich_inline_atoms(
                    atoms,
                    children,
                    style
                        .fg(agena_tui_components::theme::warning_color())
                        .add_modifier(Modifier::BOLD),
                ) {
                    return false;
                }
            }
            MarkdownInline::Superscript(children) => {
                if let Some(text) = positional_unicode(children, true) {
                    atoms.push(RichInlineAtom::Text(Span::styled(text, style)));
                } else if !append_rich_inline_atoms(
                    atoms,
                    children,
                    style.add_modifier(Modifier::DIM),
                ) {
                    return false;
                }
            }
            MarkdownInline::Subscript(children) => {
                if let Some(text) = positional_unicode(children, false) {
                    atoms.push(RichInlineAtom::Text(Span::styled(text, style)));
                } else if !append_rich_inline_atoms(
                    atoms,
                    children,
                    style.add_modifier(Modifier::DIM),
                ) {
                    return false;
                }
            }
            MarkdownInline::Spoiler(children) => {
                if !append_rich_inline_atoms(
                    atoms,
                    children,
                    style
                        .fg(agena_tui_components::theme::muted_color())
                        .add_modifier(Modifier::REVERSED),
                ) {
                    return false;
                }
            }
            MarkdownInline::Link { url, title, label } => {
                if !append_rich_inline_atoms(
                    atoms,
                    label,
                    style
                        .fg(agena_tui_components::theme::info_color())
                        .add_modifier(Modifier::UNDERLINED),
                ) {
                    return false;
                }
                atoms.push(RichInlineAtom::Text(Span::styled(
                    link_suffix(url, title),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                )));
            }
            MarkdownInline::WikiLink { url, label } => {
                if !append_rich_inline_atoms(
                    atoms,
                    label,
                    style
                        .fg(agena_tui_components::theme::info_color())
                        .add_modifier(Modifier::UNDERLINED),
                ) {
                    return false;
                }
                atoms.push(RichInlineAtom::Text(Span::styled(
                    format!(" ({url})"),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                )));
            }
            MarkdownInline::Image { url, alt, .. } => atoms.push(RichInlineAtom::Image {
                url: url.clone(),
                alt: alt.clone(),
            }),
            MarkdownInline::Math { literal, .. } => {
                atoms.push(RichInlineAtom::Math(literal.clone()));
            }
            MarkdownInline::FootnoteReference(name) => {
                atoms.push(RichInlineAtom::Text(Span::styled(
                    format!("[^{name}]"),
                    style
                        .fg(agena_tui_components::theme::accent_color())
                        .add_modifier(Modifier::BOLD),
                )))
            }
            MarkdownInline::Html(html) => {
                if html.to_ascii_lowercase().starts_with("<br") {
                    return false;
                }
            }
            MarkdownInline::SoftBreak => {
                atoms.push(RichInlineAtom::Text(Span::styled(" ", style)));
            }
            MarkdownInline::HardBreak => return false,
        }
    }
    true
}

fn render_list(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    ordered: bool,
    start: usize,
    delimiter: char,
    items: &[MarkdownListItem],
    width: u16,
    depth: usize,
) {
    for (offset, item) in items.iter().enumerate() {
        let marker = if let Some(checked) = item.checked {
            if checked {
                "●".to_string()
            } else {
                "○".to_string()
            }
        } else if ordered {
            format!("{}{delimiter}", start.saturating_add(offset))
        } else {
            ["•", "◦", "▪"][depth.min(2)].to_string()
        };
        let first_prefix = format!("{prefix}{marker} ");
        let continuation = format!("{prefix}{}", " ".repeat(marker.chars().count() + 1));
        let mut first = true;
        for block in &item.blocks {
            match block {
                MarkdownNode::Paragraph(inlines) if first => {
                    render_paragraph(out, &first_prefix, inlines, width);
                }
                MarkdownNode::List {
                    ordered,
                    start,
                    delimiter,
                    items,
                    ..
                } => render_list(
                    out,
                    &continuation,
                    *ordered,
                    *start,
                    *delimiter,
                    items,
                    width,
                    depth.saturating_add(1),
                ),
                _ => render_markdown_node(
                    out,
                    if first { &first_prefix } else { &continuation },
                    block,
                    width,
                ),
            }
            first = false;
        }
        if item.blocks.is_empty() {
            push_single_line(out, &first_prefix, "", Style::default(), width);
        }
    }
}

fn render_alert(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    kind: MarkdownAlertKind,
    title: Option<&str>,
    blocks: &[MarkdownNode],
    width: u16,
) {
    let (icon, default_title, color) = match kind {
        MarkdownAlertKind::Note => ("●", "Note", agena_tui_components::theme::info_color()),
        MarkdownAlertKind::Tip => ("◆", "Tip", agena_tui_components::theme::success_color()),
        MarkdownAlertKind::Important => (
            "!",
            "Important",
            agena_tui_components::theme::accent_color(),
        ),
        MarkdownAlertKind::Warning => {
            ("▲", "Warning", agena_tui_components::theme::warning_color())
        }
        MarkdownAlertKind::Caution => ("■", "Caution", agena_tui_components::theme::danger_color()),
    };
    push_single_line(
        out,
        prefix,
        &format!("╭─ {icon} {}", title.unwrap_or(default_title)),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
        width,
    );
    let body_prefix = format!("{prefix}│ ");
    for block in blocks {
        render_markdown_node(out, &body_prefix, block, width);
    }
    push_single_line(out, prefix, "╰─", Style::default().fg(color), width);
}

fn is_diagram_language(language: &str) -> bool {
    matches!(
        language,
        "mermaid"
            | "plantuml"
            | "puml"
            | "dot"
            | "graphviz"
            | "d2"
            | "vega"
            | "vega-lite"
            | "svgbob"
            | "svg"
    )
}

fn render_diagram(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    language: &str,
    literal: &str,
    width: u16,
) {
    if language == "svg"
        && layout_config().native_graphics
        && let Ok(artifact) = render_markdown_svg(literal)
    {
        let prefix_width = u16::try_from(UnicodeWidthStr::width(prefix)).unwrap_or(u16::MAX);
        let available = width.saturating_sub(prefix_width).max(1);
        let (render_width, render_height) = fit_image_size(artifact.size, available, 24);
        let column = prefix_width + available.saturating_sub(render_width) / 2;
        let start = out.len();
        for _ in 0..render_height {
            out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
        }
        out[start].math.push(MathLinePlacement {
            column,
            artifact,
            size: Size::new(render_width, render_height),
        });
        push_single_line(
            out,
            prefix,
            "◇ SVG diagram",
            Style::default()
                .fg(agena_tui_components::theme::accent_color())
                .add_modifier(Modifier::BOLD),
            width,
        );
        return;
    }
    let label = match language {
        "puml" => "PlantUML",
        "dot" | "graphviz" => "Graphviz",
        "vega-lite" => "Vega-Lite",
        "svgbob" => "Svgbob",
        "svg" => "SVG",
        language => language,
    };
    push_single_line(
        out,
        prefix,
        &format!("◇ Diagram · {label}"),
        Style::default()
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    let source = format!("```{language}\n{}\n```", literal.trim_end_matches('\n'));
    push_markdown_code_block(out, prefix, &source, width);
}

fn render_ast_table(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    alignments: &[MarkdownAlignment],
    rows: &[MarkdownTableRow],
    width: u16,
) {
    if rows.is_empty() {
        return;
    }
    let plain_rows = rows
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| inline_plain_text(cell))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let column_count = plain_rows.iter().map(Vec::len).max().unwrap_or(0);
    if column_count == 0 {
        return;
    }
    let separator_width = column_count.saturating_mul(3).saturating_add(1);
    let prefix_width = UnicodeWidthStr::width(prefix);
    let budget = usize::from(width)
        .saturating_sub(prefix_width)
        .saturating_sub(separator_width);
    let widths = compute_table_column_widths(&plain_rows, budget);
    if widths.len() != column_count {
        render_rich_table_fallback(out, prefix, rows, width);
        return;
    }
    let table_alignments = (0..column_count)
        .map(|index| {
            match alignments
                .get(index)
                .copied()
                .unwrap_or(MarkdownAlignment::None)
            {
                MarkdownAlignment::None | MarkdownAlignment::Left => TableColumnAlignment::Left,
                MarkdownAlignment::Center => TableColumnAlignment::Center,
                MarkdownAlignment::Right => TableColumnAlignment::Right,
            }
        })
        .collect::<Vec<_>>();
    let border_style = Style::default().fg(agena_tui_components::theme::muted_color());
    push_table_border(out, prefix, &widths, "┌", "┬", "┐", border_style);
    for (row_index, row) in rows.iter().enumerate() {
        render_rich_table_row(out, prefix, row, &widths, &table_alignments, width);
        if row_index + 1 < rows.len() {
            push_table_border(out, prefix, &widths, "├", "┼", "┤", border_style);
        }
    }
    push_table_border(out, prefix, &widths, "└", "┴", "┘", border_style);
}

fn render_rich_table_row(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    row: &MarkdownTableRow,
    widths: &[usize],
    alignments: &[TableColumnAlignment],
    width: u16,
) {
    let base = if row.header {
        Style::default()
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let cells = widths
        .iter()
        .enumerate()
        .map(|(index, cell_width)| {
            let logical_lines = row
                .cells
                .get(index)
                .map(|cell| rich_inline_lines(cell, base))
                .unwrap_or_default();
            let mut wrapped = logical_lines
                .into_iter()
                .flat_map(|line| wrap_rich_line(&line.spans, *cell_width, *cell_width))
                .collect::<Vec<_>>();
            if wrapped.is_empty() {
                wrapped.push(Line::default());
            }
            wrapped
        })
        .collect::<Vec<_>>();
    let row_height = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let border_style = Style::default().fg(agena_tui_components::theme::muted_color());
    for line_index in 0..row_height {
        let mut spans = vec![
            Span::raw(prefix.to_string()),
            Span::styled("│", border_style),
        ];
        for (column, cell_width) in widths.iter().enumerate() {
            spans.push(Span::raw(" "));
            let mut content = cells
                .get(column)
                .and_then(|lines| lines.get(line_index))
                .cloned()
                .unwrap_or_default()
                .spans;
            let content_width = rich_spans_width(&content).min(*cell_width);
            let padding = cell_width.saturating_sub(content_width);
            let alignment = alignments
                .get(column)
                .copied()
                .unwrap_or(TableColumnAlignment::Left);
            let left = match alignment {
                TableColumnAlignment::Left => 0,
                TableColumnAlignment::Right => padding,
                TableColumnAlignment::Center => padding / 2,
            };
            let right = padding.saturating_sub(left);
            if left > 0 {
                spans.push(Span::raw(" ".repeat(left)));
            }
            spans.append(&mut content);
            if right > 0 {
                spans.push(Span::raw(" ".repeat(right)));
            }
            spans.push(Span::raw(" "));
            spans.push(Span::styled("│", border_style));
        }
        let line = Line::from(spans);
        if UnicodeWidthStr::width(line.to_string().as_str()) <= usize::from(width) {
            out.push(RenderedLine::rich(line));
        }
    }
}

fn render_rich_table_fallback(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    rows: &[MarkdownTableRow],
    width: u16,
) {
    for row in rows {
        let base = if row.header {
            Style::default()
                .fg(agena_tui_components::theme::accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        for (index, cell) in row.cells.iter().enumerate() {
            let marker = if index == 0 { "│ " } else { "├ " };
            let initial_prefix = format!("{prefix}{marker}");
            let continuation_prefix = format!("{prefix}  ");
            let logical_lines = rich_inline_lines(cell, base);
            if logical_lines.is_empty() {
                push_wrapped_rich_line(
                    out,
                    &initial_prefix,
                    &continuation_prefix,
                    Line::default(),
                    width,
                );
                continue;
            }
            for (line_index, line) in logical_lines.into_iter().enumerate() {
                push_wrapped_rich_line(
                    out,
                    if line_index == 0 {
                        &initial_prefix
                    } else {
                        &continuation_prefix
                    },
                    &continuation_prefix,
                    line,
                    width,
                );
            }
        }
    }
}

fn render_image_card(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    alt: &str,
    url: &str,
    title: &str,
    width: u16,
) {
    let label = if alt.trim().is_empty() {
        "Image"
    } else {
        alt.trim()
    };
    if layout_config().native_graphics
        && let Ok(artifact) = render_markdown_image(url)
    {
        let prefix_width = u16::try_from(UnicodeWidthStr::width(prefix)).unwrap_or(u16::MAX);
        let available = width.saturating_sub(prefix_width).max(1);
        let (render_width, render_height) = fit_image_size(artifact.size, available, 24);
        let column = prefix_width + available.saturating_sub(render_width) / 2;
        let start = out.len();
        for _ in 0..render_height {
            out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
        }
        out[start].math.push(MathLinePlacement {
            column,
            artifact,
            size: Size::new(render_width, render_height),
        });
        let caption = if title.trim().is_empty() {
            label.to_string()
        } else {
            format!("{label} — {}", title.trim())
        };
        push_single_line(
            out,
            prefix,
            &format!("🖼  {caption}"),
            Style::default().fg(agena_tui_components::theme::muted_color()),
            width,
        );
        return;
    }
    push_single_line(
        out,
        prefix,
        &format!("╭─ 🖼  {label}"),
        Style::default()
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    if !title.trim().is_empty() {
        push_single_line(
            out,
            prefix,
            &format!("│  {}", title.trim()),
            Style::default(),
            width,
        );
    }
    push_single_line(
        out,
        prefix,
        &format!("╰─ {url}"),
        Style::default()
            .fg(agena_tui_components::theme::info_color())
            .add_modifier(Modifier::UNDERLINED),
        width,
    );
}

fn fit_image_size(size: Size, max_width: u16, max_height: u16) -> (u16, u16) {
    let mut width = size.width.max(1);
    let mut height = size.height.max(1);
    if width > max_width {
        height = u32::from(height)
            .saturating_mul(u32::from(max_width))
            .div_ceil(u32::from(width))
            .clamp(1, u32::from(u16::MAX)) as u16;
        width = max_width;
    }
    if height > max_height {
        width = u32::from(width)
            .saturating_mul(u32::from(max_height))
            .div_ceil(u32::from(height))
            .clamp(1, u32::from(max_width)) as u16;
        height = max_height;
    }
    (width.max(1), height.max(1))
}

fn front_matter_body(front_matter: &str) -> String {
    let mut lines = front_matter.trim_matches('\n').lines().collect::<Vec<_>>();
    if lines.first().is_some_and(|line| line.trim() == "---") {
        lines.remove(0);
    }
    if lines
        .last()
        .is_some_and(|line| matches!(line.trim(), "---" | "..."))
    {
        lines.pop();
    }
    lines.join("\n")
}

fn link_suffix(url: &str, title: &str) -> String {
    if title.trim().is_empty() {
        format!(" ({url})")
    } else {
        format!(" ({url} — {})", title.trim())
    }
}

fn rich_inline_lines(inlines: &[MarkdownInline], base_style: Style) -> Vec<Line<'static>> {
    let mut rows = vec![Vec::new()];
    append_inline_spans(&mut rows, inlines, base_style);
    rows.into_iter().map(Line::from).collect()
}

fn append_inline_spans(
    rows: &mut Vec<Vec<Span<'static>>>,
    inlines: &[MarkdownInline],
    style: Style,
) {
    for inline in inlines {
        match inline {
            MarkdownInline::Text(text) | MarkdownInline::Emoji(text) => {
                rows.last_mut()
                    .expect("inline rows are never empty")
                    .push(Span::styled(text.clone(), style));
            }
            MarkdownInline::Code(code) => rows
                .last_mut()
                .expect("inline rows are never empty")
                .push(Span::styled(
                    code.clone(),
                    style
                        .fg(agena_tui_components::theme::warning_color())
                        .add_modifier(Modifier::BOLD),
                )),
            MarkdownInline::Emphasis(children) => {
                append_inline_spans(rows, children, style.add_modifier(Modifier::ITALIC))
            }
            MarkdownInline::Strong(children) => {
                append_inline_spans(rows, children, style.add_modifier(Modifier::BOLD))
            }
            MarkdownInline::Strikethrough(children) => {
                append_inline_spans(rows, children, style.add_modifier(Modifier::CROSSED_OUT))
            }
            MarkdownInline::Underline(children) | MarkdownInline::Insert(children) => {
                append_inline_spans(rows, children, style.add_modifier(Modifier::UNDERLINED))
            }
            MarkdownInline::Highlight(children) => append_inline_spans(
                rows,
                children,
                style
                    .fg(agena_tui_components::theme::warning_color())
                    .add_modifier(Modifier::BOLD),
            ),
            MarkdownInline::Superscript(children) => {
                if let Some(text) = positional_unicode(children, true) {
                    rows.last_mut()
                        .expect("inline rows are never empty")
                        .push(Span::styled(text, style));
                } else {
                    append_inline_spans(rows, children, style.add_modifier(Modifier::DIM));
                }
            }
            MarkdownInline::Subscript(children) => {
                if let Some(text) = positional_unicode(children, false) {
                    rows.last_mut()
                        .expect("inline rows are never empty")
                        .push(Span::styled(text, style));
                } else {
                    append_inline_spans(rows, children, style.add_modifier(Modifier::DIM));
                }
            }
            MarkdownInline::Spoiler(children) => append_inline_spans(
                rows,
                children,
                style
                    .fg(agena_tui_components::theme::muted_color())
                    .add_modifier(Modifier::REVERSED),
            ),
            MarkdownInline::Link { url, title, label } => {
                append_inline_spans(
                    rows,
                    label,
                    style
                        .fg(agena_tui_components::theme::info_color())
                        .add_modifier(Modifier::UNDERLINED),
                );
                rows.last_mut()
                    .expect("inline rows are never empty")
                    .push(Span::styled(
                        link_suffix(url, title),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                    ));
            }
            MarkdownInline::WikiLink { url, label } => {
                append_inline_spans(
                    rows,
                    label,
                    style
                        .fg(agena_tui_components::theme::info_color())
                        .add_modifier(Modifier::UNDERLINED),
                );
                rows.last_mut()
                    .expect("inline rows are never empty")
                    .push(Span::styled(
                        format!(" ({url})"),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                    ));
            }
            MarkdownInline::Image { url, alt, .. } => rows
                .last_mut()
                .expect("inline rows are never empty")
                .push(Span::styled(
                    format!("🖼 {} ({url})", if alt.is_empty() { "Image" } else { alt }),
                    style.fg(agena_tui_components::theme::info_color()),
                )),
            MarkdownInline::Math { literal, .. } => rows
                .last_mut()
                .expect("inline rows are never empty")
                .push(Span::styled(
                    unicode_formula(literal, false).join(" "),
                    style.fg(agena_tui_components::theme::accent_color()),
                )),
            MarkdownInline::FootnoteReference(name) => rows
                .last_mut()
                .expect("inline rows are never empty")
                .push(Span::styled(
                    format!("[^{name}]"),
                    style
                        .fg(agena_tui_components::theme::accent_color())
                        .add_modifier(Modifier::BOLD),
                )),
            MarkdownInline::Html(html) => {
                if html.trim().eq_ignore_ascii_case("<br>")
                    || html.trim().eq_ignore_ascii_case("<br/>")
                    || html.trim().eq_ignore_ascii_case("<br />")
                {
                    rows.push(Vec::new());
                }
            }
            MarkdownInline::SoftBreak => rows
                .last_mut()
                .expect("inline rows are never empty")
                .push(Span::styled(" ", style)),
            MarkdownInline::HardBreak => rows.push(Vec::new()),
        }
    }
}

fn positional_unicode(inlines: &[MarkdownInline], superscript: bool) -> Option<String> {
    let mut source = String::new();
    for inline in inlines {
        match inline {
            MarkdownInline::Text(text) | MarkdownInline::Emoji(text) => source.push_str(text),
            _ => return None,
        }
    }
    positional_unicode_text(&source, superscript)
}

fn inlines_contain_rich_graphics(inlines: &[MarkdownInline]) -> bool {
    inlines.iter().any(|inline| match inline {
        MarkdownInline::Math { .. } | MarkdownInline::Image { .. } => true,
        MarkdownInline::Emphasis(children)
        | MarkdownInline::Strong(children)
        | MarkdownInline::Strikethrough(children)
        | MarkdownInline::Underline(children)
        | MarkdownInline::Highlight(children)
        | MarkdownInline::Insert(children)
        | MarkdownInline::Superscript(children)
        | MarkdownInline::Subscript(children)
        | MarkdownInline::Spoiler(children)
        | MarkdownInline::Link {
            label: children, ..
        }
        | MarkdownInline::WikiLink {
            label: children, ..
        } => inlines_contain_rich_graphics(children),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_gfm_structure_without_line_heuristics() {
        let blocks = parse_markdown_document(
            "Title\n---\n\n1) first\n   - nested\n\n> [!WARNING]\n> careful\n\nFootnote[^a]\n\n[^a]: detail",
        );
        assert!(matches!(
            blocks[0].parsed,
            MarkdownNode::Heading { level: 2, .. }
        ));
        assert!(matches!(
            blocks[1].parsed,
            MarkdownNode::List { delimiter: ')', .. }
        ));
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block.parsed, MarkdownNode::Alert { .. }))
        );
        assert!(blocks.iter().any(|block| matches!(
            &block.parsed,
            MarkdownNode::FootnoteDefinition { name, .. } if name == "1"
        )));
    }

    #[test]
    fn preserves_code_span_pipes_inside_table_cells() {
        let blocks = parse_markdown_document("| value |\n| --- |\n| `a\\|b` |");
        let MarkdownNode::Table { rows, .. } = &blocks[0].parsed else {
            panic!("table expected");
        };
        assert_eq!(inline_plain_text(&rows[1].cells[0]), "a|b");
    }

    #[test]
    fn preserves_math_pipes_inside_table_cells() {
        let blocks = parse_markdown_document(
            "| expression | meaning |\n| --- | --- |\n| $|x|$ | magnitude |",
        );
        let MarkdownNode::Table { rows, .. } = &blocks[0].parsed else {
            panic!("table expected: {blocks:#?}");
        };
        assert_eq!(rows[1].cells.len(), 2);
        assert!(matches!(
            rows[1].cells[0].as_slice(),
            [MarkdownInline::Math { literal, .. }] if literal == "|x|"
        ));

        let display = parse_markdown_document(
            "| expression | meaning |\n| --- | --- |\n| $$|x|$$ | magnitude |",
        );
        let MarkdownNode::Table { rows, .. } = &display[0].parsed else {
            panic!("table expected: {display:#?}");
        };
        assert_eq!(rows[1].cells.len(), 2);
        assert!(matches!(
            rows[1].cells[0].as_slice(),
            [MarkdownInline::Math { literal, display: true }] if literal == "|x|"
        ));
    }

    #[test]
    fn currency_dollars_do_not_hide_real_table_separators() {
        let blocks = parse_markdown_document(
            "| first | second | third |\n| --- | --- | --- |\n| $5 | next $6 | value |",
        );
        let MarkdownNode::Table { rows, .. } = &blocks[0].parsed else {
            panic!("table expected: {blocks:#?}");
        };
        assert_eq!(rows[1].cells.len(), 3);
        assert_eq!(inline_plain_text(&rows[1].cells[0]), "$5");
        assert_eq!(inline_plain_text(&rows[1].cells[1]), "next $6");
    }

    #[test]
    fn rich_table_cells_preserve_all_explicit_lines() {
        let blocks = parse_markdown_document("| value |\n| --- |\n| first<br>second |");
        let mut rendered = Vec::new();
        render_parsed_markdown_block(&mut rendered, "", &blocks[0], 40);
        let text = rendered
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("first"), "first cell line missing:\n{text}");
        assert!(text.contains("second"), "second cell line missing:\n{text}");
    }

    #[test]
    fn markdown_superscripts_and_subscripts_use_positional_unicode() {
        let blocks = parse_markdown_document("x^2^ and H~2~O");
        let mut rendered = Vec::new();
        render_parsed_markdown_block(&mut rendered, "", &blocks[0], 80);
        let text = rendered
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("x²"),
            "superscript was not positioned: {text}"
        );
        assert!(text.contains("H₂O"), "subscript was not positioned: {text}");
    }

    #[test]
    fn parses_latex_delimiters_and_inline_footnotes() {
        let blocks = parse_markdown_document(
            "Inline \\(x^2\\) and note^[inline **detail**].\n\n\\[\n\\frac{a}{b}\n\\]",
        );
        let MarkdownNode::Paragraph(inlines) = &blocks[0].parsed else {
            panic!("paragraph expected");
        };
        assert!(inlines.iter().any(|inline| matches!(
            inline,
            MarkdownInline::Math {
                literal,
                display: false
            } if literal == "x^2"
        )));
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block.parsed, MarkdownNode::FootnoteDefinition { .. }))
        );
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block.parsed, MarkdownNode::Math { display: true, .. }))
        );
    }

    #[test]
    fn multiline_dollar_math_is_opaque_to_markdown_block_syntax() {
        let source = concat!(
            "### 矩阵乘法\n\n",
            "$$\n",
            "\\begin{bmatrix}\n",
            "a_{11} & a_{12} \\\\\n",
            "a_{21} & a_{22}\n",
            "\\end{bmatrix}\n",
            "=\n",
            "\\begin{bmatrix}\n",
            "b_{11} & b_{12} \\\\\n",
            "b_{21} & b_{22}\n",
            "\\end{bmatrix}\n",
            "$$",
        );
        let blocks = parse_markdown_document(source);

        assert_eq!(blocks.len(), 2);
        assert!(matches!(
            blocks[0].parsed,
            MarkdownNode::Heading { level: 3, .. }
        ));
        let MarkdownNode::Math { literal, display } = &blocks[1].parsed else {
            panic!("display formula must remain one semantic math block: {blocks:#?}");
        };
        assert!(*display);
        assert!(literal.contains("a_{11} & a_{12} \\\\"));
        assert!(literal.contains("\n=\n"));
        assert!(literal.contains("b_{11} & b_{12} \\\\"));
        assert_eq!(blocks[1].source, source.split_once("\n\n").unwrap().1);

        let rendered = unicode_formula(literal, true).join("\n");
        assert!(!rendered.contains("$$"), "dollar fence leaked:\n{rendered}");
        assert!(
            !rendered.contains(r"\begin"),
            "matrix source leaked:\n{rendered}"
        );
        assert!(
            rendered.contains('='),
            "matrix equality missing:\n{rendered}"
        );
        assert!(
            rendered.contains('⎡'),
            "left matrix bracket missing:\n{rendered}"
        );
        assert!(
            rendered.contains('⎤'),
            "right matrix bracket missing:\n{rendered}"
        );
    }

    #[test]
    fn multiline_math_inside_nested_lists_remains_opaque() {
        let blocks =
            parse_markdown_document("- outer\n  - inner\n    $$\n    x\n    =\n    y\n    $$");
        let MarkdownNode::List { items, .. } = &blocks[0].parsed else {
            panic!("outer list expected: {blocks:#?}");
        };
        let nested_math = items[0]
            .blocks
            .iter()
            .flat_map(|block| match block {
                MarkdownNode::List { items, .. } => items
                    .iter()
                    .flat_map(|item| item.blocks.iter())
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .any(|block| matches!(block, MarkdownNode::Math { display: true, .. }));
        assert!(
            nested_math,
            "nested display math was parsed as Markdown syntax"
        );

        let quoted =
            parse_markdown_document("> - item\n>     \\[\n>     a\n>     =\n>     b\n>     \\]");
        assert!(matches!(
            &quoted[0].parsed,
            MarkdownNode::Quote(children)
                if matches!(
                    children.as_slice(),
                    [MarkdownNode::List { items, .. }]
                        if items[0].blocks.iter().any(
                            |block| matches!(block, MarkdownNode::Math { display: true, .. })
                        )
                )
        ));
    }

    #[test]
    fn dollar_delimiters_inside_code_fences_are_not_rewritten_as_math() {
        let source = "```text\n$$\n=\n$$\n```";
        let blocks = parse_markdown_document(source);

        assert!(matches!(
            &blocks[0].parsed,
            MarkdownNode::Code { literal, .. } if literal.contains("$$\n=\n$$")
        ));

        let indented = parse_markdown_document("    $$\n    =\n    $$");
        assert!(matches!(indented[0].parsed, MarkdownNode::Code { .. }));
    }

    #[test]
    fn dollar_math_protection_is_lazy_and_uses_collision_free_fences() {
        assert!(matches!(
            protect_multiline_display_math("plain Markdown"),
            Cow::Borrowed(_)
        ));

        let protected = protect_multiline_display_math("$$\n```\n~~~\n=\n$$");
        let opening = protected.lines().next().unwrap_or_default();
        let closing = protected.lines().last().unwrap_or_default();
        assert!(opening.ends_with("math"));
        assert_eq!(opening.trim_end_matches("math"), closing);
        assert!(opening.len() >= 5);

        let blocks = parse_markdown_document("$$\n```\n~~~\n=\n$$");
        assert!(matches!(
            &blocks[0].parsed,
            MarkdownNode::Math { literal, display: true }
                if literal.contains("```\n~~~\n=")
        ));

        let latex_delimited = parse_markdown_document("\\[\nx\n=\ny\n\\]");
        assert!(matches!(
            &latex_delimited[0].parsed,
            MarkdownNode::Math { literal, display: true } if literal == "x\n=\ny"
        ));

        let quoted = parse_markdown_document("> $$\n> x\n> =\n> y\n> $$");
        assert!(matches!(
            &quoted[0].parsed,
            MarkdownNode::Quote(children)
                if matches!(children.as_slice(), [MarkdownNode::Math { display: true, .. }])
        ));
    }

    #[test]
    fn double_underscore_remains_commonmark_strong_text() {
        let blocks = parse_markdown_document("__strong__");
        let MarkdownNode::Paragraph(inlines) = &blocks[0].parsed else {
            panic!("paragraph expected");
        };
        assert!(matches!(inlines.as_slice(), [MarkdownInline::Strong(_)]));
    }

    #[test]
    fn parses_attributes_safe_html_and_obsidian_embeds() {
        let code = parse_markdown_document("```{.rust #sample}\nfn main() {}\n```");
        assert!(matches!(
            &code[0].parsed,
            MarkdownNode::Code { language, .. } if language == "rust"
        ));

        let html = parse_markdown_document("Press <kbd>Ctrl</kbd> and ![[icon.svg|Logo]].");
        let MarkdownNode::Paragraph(inlines) = &html[0].parsed else {
            panic!("paragraph expected");
        };
        assert!(inlines.iter().any(|inline| matches!(
            inline,
            MarkdownInline::Highlight(children)
                if inline_plain_text(children) == "Ctrl"
        )));
        assert!(inlines.iter().any(|inline| matches!(
            inline,
            MarkdownInline::Image { url, alt, .. }
                if url == "icon.svg" && alt == "Logo"
        )));

        let image = parse_markdown_document(r#"<img src="icon.svg" alt="Logo" title="Diagram">"#);
        assert!(matches!(
            &image[0].parsed,
            MarkdownNode::Image { url, alt, title }
                if url == "icon.svg" && alt == "Logo" && title == "Diagram"
        ));
    }

    #[test]
    fn rich_tables_and_math_keep_inline_styles() {
        let table = parse_markdown_document("| head |\n| --- |\n| *styled* |");
        let mut rendered = Vec::new();
        render_parsed_markdown_block(&mut rendered, "", &table[0], 80);
        assert!(rendered.iter().any(|line| {
            line.rich_line.as_ref().is_some_and(|line| {
                line.spans
                    .iter()
                    .any(|span| span.style.add_modifier.contains(Modifier::ITALIC))
            })
        }));

        let math = parse_markdown_document("**value** \\(\\frac{a}{b}\\)");
        let mut rendered = Vec::new();
        render_parsed_markdown_block(&mut rendered, "", &math[0], 80);
        assert!(!rendered.is_empty());
        assert!(rendered.iter().any(|line| {
            line.rich_line.as_ref().is_some_and(|line| {
                line.spans
                    .iter()
                    .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
            })
        }));
    }

    #[test]
    fn diagram_fences_are_semantic_and_keep_safe_source_fallbacks() {
        let blocks = parse_markdown_document("```mermaid\ngraph TD; A-->B\n```");
        assert_eq!(blocks[0].kind, TranscriptNodeKind::MarkdownDiagram);
        assert!(matches!(
            &blocks[0].parsed,
            MarkdownNode::Diagram { language, literal }
                if language == "mermaid" && literal.contains("A-->B")
        ));
        let mut rendered = Vec::new();
        render_parsed_markdown_block(&mut rendered, "", &blocks[0], 80);
        assert!(
            rendered
                .iter()
                .any(|line| line.text.contains("Diagram · mermaid"))
        );
        assert!(rendered.iter().any(|line| line.text.contains("A-->B")));
    }
}
