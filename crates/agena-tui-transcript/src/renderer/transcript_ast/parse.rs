use super::*;

static MARKDOWN_PARSE_CACHE: LazyLock<Mutex<MarkdownParseCache>> =
    LazyLock::new(|| Mutex::new(MarkdownParseCache::default()));

pub fn parse_markdown_document(text: &str) -> Vec<MarkdownBlock> {
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

pub(super) fn restore_math_placeholders(literal: String) -> String {
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
pub(super) fn protect_multiline_display_math(markdown: &str) -> Cow<'_, str> {
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
