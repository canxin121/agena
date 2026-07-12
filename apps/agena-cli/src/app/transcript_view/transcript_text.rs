use super::transcript_math::{
    fenced_math_language, inline_math_unicode_text, is_display_math_closed, is_display_math_start,
    push_inline_math, push_math_block,
};

pub(in crate::app) fn transcript_message_parts(message: &MessageResource) -> &[MessagePart] {
    message
        .parts
        .as_deref()
        .expect("transcript messages must include full parts")
}

pub(in crate::app) fn transcript_part_content(part: &MessagePart) -> &PartContent {
    part.content
        .as_ref()
        .expect("transcript message parts must include full content")
}

pub(in crate::app) fn operation_block_copy_text(block: &OperationBlock, i18n: &I18n) -> String {
    match block {
        OperationBlock::Text { text }
        | OperationBlock::Markdown { text }
        | OperationBlock::Diff { diff: text, .. } => text.clone(),
        OperationBlock::Command {
            command,
            exit_code,
            stdout,
            stderr,
            ..
        } => {
            let mut parts = vec![format!("$ {command}")];
            if let Some(stdout) = stdout
                && !stdout.trim().is_empty()
            {
                parts.push(stdout.trim().to_string());
            }
            if let Some(stderr) = stderr
                && !stderr.trim().is_empty()
            {
                parts.push(stderr.trim().to_string());
            }
            if let Some(exit_code) = exit_code {
                parts.push(ui_text::operation_command_exit_line(i18n, *exit_code));
            }
            parts.join("\n")
        }
        OperationBlock::SearchResults { query, results } => {
            let mut out = Vec::new();
            if let Some(query) = query {
                out.push(ui_text::operation_search_heading(
                    i18n,
                    Some(query.as_str()),
                ));
            } else {
                out.push(ui_text::operation_search_heading(i18n, None));
            }
            for result in results {
                out.push(result.title.clone());
                out.push(result.uri.clone());
                if let Some(snippet) = &result.snippet
                    && !snippet.trim().is_empty()
                {
                    out.push(snippet.clone());
                }
            }
            out.join("\n")
        }
        OperationBlock::EmbeddedResource { uri, text, .. } => text
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| uri.clone()),
        OperationBlock::Checklist { items } => items
            .iter()
            .map(|item| item.content.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        OperationBlock::FileChanges { changes } => changes
            .iter()
            .map(|change| file_change_list_item_text(change, i18n))
            .collect::<Vec<_>>()
            .join("\n"),
        OperationBlock::ResourceLink { uri, title, .. }
        | OperationBlock::Citation { uri, title, .. } => {
            title.clone().unwrap_or_else(|| uri.clone())
        }
        OperationBlock::Image { url, .. }
        | OperationBlock::Audio { url, .. }
        | OperationBlock::File { url, .. } => url.clone(),
        OperationBlock::Media { artifact, .. } => media_artifact_label(artifact),
        OperationBlock::Progress { message, .. } => message.clone(),
        OperationBlock::NestedTask {
            task_id,
            title,
            status,
        } => ui_text::operation_nested_task_summary(
            i18n,
            title.as_deref().unwrap_or(task_id.as_str()),
            *status,
        ),
        OperationBlock::Json { .. }
        | OperationBlock::Table { .. }
        | OperationBlock::Log { .. }
        | OperationBlock::Custom { .. } => String::new(),
    }
}

pub(in crate::app) fn media_artifact_label(artifact: &agena::message::ArtifactRef) -> String {
    let uri = artifact.uri.trim();
    if uri.starts_with("file://")
        || uri.starts_with('/')
        || uri.starts_with("./")
        || uri.starts_with("../")
    {
        return uri.to_string();
    }

    artifact
        .name
        .clone()
        .unwrap_or_else(|| artifact.uri.clone())
}

pub(in crate::app) fn tool_display_label(tool: &OperationPart) -> String {
    if tool.title.trim().is_empty() {
        tool_invocation_label(&tool.invocation)
    } else {
        tool.title.clone()
    }
}

pub(in crate::app) fn should_render_tool_model_output(
    tool: &OperationPart,
    skipped_text: Option<&str>,
) -> bool {
    let model_output = normalized_tool_text(tool.model_output.text.as_str());
    if model_output.is_empty() {
        return false;
    }
    if tool.invocation.name == "agena_web__search"
        && tool
            .blocks
            .iter()
            .any(|block| matches!(block, OperationBlock::SearchResults { .. }))
    {
        return false;
    }
    let tool_label = normalized_tool_text(tool_display_label(tool).as_str());
    if tool_label == model_output {
        return false;
    }
    if let Some(prefix) = tool_label.strip_suffix(model_output.as_str())
        && prefix.chars().last().is_some_and(|ch| ch.is_whitespace())
        && !prefix.trim().is_empty()
    {
        return false;
    }
    if skipped_text.is_some_and(|candidate| normalized_tool_text(candidate) == model_output) {
        return false;
    }
    !tool
        .blocks
        .iter()
        .filter_map(operation_text_block_text)
        .any(|text| normalized_tool_text(text) == model_output)
}

pub(in crate::app) fn operation_text_block_text(block: &OperationBlock) -> Option<&str> {
    match block {
        OperationBlock::Text { text } | OperationBlock::Markdown { text } => Some(text.as_str()),
        _ => None,
    }
}

pub(in crate::app) fn normalized_tool_text(text: &str) -> String {
    let sanitized = sanitize_terminal_text(text);
    trim_empty_line_edges(sanitized.as_str()).to_string()
}

pub(in crate::app) fn render_limited_tool_text_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    width: u16,
    i18n: &I18n,
) {
    if tool_text_looks_like_markdown(text) {
        push_limited_markdown(out, prefix, text, width, i18n);
    } else {
        push_limited_tool_text(out, prefix, text, Style::default(), width, i18n);
    }
}

pub(in crate::app) fn tool_text_looks_like_markdown(text: &str) -> bool {
    let normalized = normalized_tool_text(text);
    if normalized.is_empty() {
        return false;
    }
    let lines = normalized.lines().collect::<Vec<_>>();
    let unordered_item_count = lines
        .iter()
        .filter(|line| is_markdown_unordered_list_item(line))
        .count();
    let ordered_item_count = lines
        .iter()
        .filter(|line| is_markdown_ordered_list_item(line))
        .count();

    lines.iter().any(|line| {
        let trimmed = line.trim_start();
        markdown_fence_delimiter(trimmed).is_some()
            || trimmed.starts_with("#")
            || trimmed.starts_with("> ")
            || trimmed.starts_with("- [")
            || trimmed.starts_with("* [")
            || trimmed.starts_with("+ [")
    }) || lines
        .windows(2)
        .any(|window| is_markdown_table_header(window[0], window[1]))
        || unordered_item_count >= 2
        || ordered_item_count >= 2
}

pub(in crate::app) fn is_markdown_unordered_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    ["- ", "* ", "+ "]
        .into_iter()
        .any(|prefix| trimmed.starts_with(prefix) && trimmed.len() > prefix.len())
}

pub(in crate::app) fn is_markdown_ordered_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    digit_count > 0
        && trimmed
            .chars()
            .nth(digit_count)
            .is_some_and(|delimiter| delimiter == '.')
        && trimmed
            .chars()
            .nth(digit_count + 1)
            .is_some_and(|separator| separator == ' ')
}

/// Splits a Markdown text part into independently navigable transcript blocks.
///
/// This deliberately follows the renderer's lightweight Markdown recognition
/// rather than trying to implement a second full Markdown parser.  The source
/// remains Markdown so rendering stays consistent, while fenced code copies as
/// plain code (without its fence) and every other block copies its source.
pub(in crate::app) fn markdown_blocks(text: &str) -> Vec<MarkdownBlock> {
    let sanitized = sanitize_terminal_text(text);
    let markdown = trim_empty_line_edges(sanitized.as_str());
    if markdown.is_empty() {
        return Vec::new();
    }

    let lines = markdown.lines().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut index = 0_usize;
    let mut leading_blank_line = false;

    while index < lines.len() {
        if lines[index].trim().is_empty() {
            leading_blank_line = true;
            index += 1;
            continue;
        }

        let start = index;
        let kind;
        let copy_range;

        if let Some(opening_fence) = markdown_fence_delimiter(lines[index]) {
            kind = if fenced_math_language(lines[index]) {
                TranscriptNodeKind::MarkdownMath
            } else {
                TranscriptNodeKind::MarkdownCode
            };
            index += 1;
            let body_start = index;
            while index < lines.len() {
                if markdown_fence_delimiter(lines[index]).is_some_and(|closing_fence| {
                    closing_fence.marker == opening_fence.marker
                        && closing_fence.len >= opening_fence.len
                }) {
                    break;
                }
                index += 1;
            }
            let body_end = index;
            if index < lines.len() {
                index += 1;
            }
            copy_range = body_start..body_end;
        } else if is_display_math_start(lines[index]) {
            kind = TranscriptNodeKind::MarkdownMath;
            index += 1;
            if !is_display_math_closed(lines[start]) {
                while index < lines.len()
                    && !is_display_math_closed(&lines[start..=index].join("\n"))
                {
                    index += 1;
                }
                if index < lines.len() {
                    index += 1;
                }
            }
            copy_range = start..index;
        } else if markdown_heading(lines[index]).is_some() {
            kind = TranscriptNodeKind::MarkdownParagraph;
            index += 1;
            copy_range = start..index;
        } else if is_markdown_quote_line(lines[index]) {
            kind = TranscriptNodeKind::MarkdownParagraph;
            index += 1;
            while index < lines.len() && is_markdown_quote_line(lines[index]) {
                index += 1;
            }
            copy_range = start..index;
        } else if is_markdown_thematic_break(lines[index]) {
            kind = TranscriptNodeKind::MarkdownParagraph;
            index += 1;
            copy_range = start..index;
        } else if index + 1 < lines.len()
            && is_markdown_table_header(lines[index], lines[index + 1])
        {
            kind = TranscriptNodeKind::MarkdownTable;
            index += 2;
            while index < lines.len() && looks_like_markdown_table_row(lines[index]) {
                index += 1;
            }
            copy_range = start..index;
        } else if is_markdown_list_item(lines[index]) {
            kind = TranscriptNodeKind::MarkdownList;
            index += 1;
            while index < lines.len() {
                let line = lines[index];
                if line.trim().is_empty() {
                    // A blank line only remains part of the list when it
                    // precedes another item or an indented continuation.
                    let Some(next) = lines.get(index + 1) else {
                        break;
                    };
                    if is_markdown_list_item(next) || is_indented_markdown_line(next) {
                        index += 1;
                        continue;
                    }
                    break;
                }
                if is_markdown_list_item(line) || is_indented_markdown_line(line) {
                    index += 1;
                    continue;
                }
                break;
            }
            copy_range = start..index;
        } else {
            kind = TranscriptNodeKind::MarkdownParagraph;
            index += 1;
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && markdown_fence_delimiter(lines[index]).is_none()
                && !is_display_math_start(lines[index])
                && markdown_heading(lines[index]).is_none()
                && !is_markdown_quote_line(lines[index])
                && !is_markdown_thematic_break(lines[index])
                && !(index + 1 < lines.len()
                    && is_markdown_table_header(lines[index], lines[index + 1]))
                && !is_markdown_list_item(lines[index])
            {
                index += 1;
            }
            copy_range = start..index;
        }

        let source = lines[start..index].join("\n");
        if source.trim().is_empty() {
            continue;
        }
        let copy_text = lines[copy_range].join("\n");
        blocks.push(MarkdownBlock {
            kind,
            source,
            copy_text,
            leading_blank_line,
        });
        leading_blank_line = false;
    }

    blocks
}

pub(in crate::app) fn is_markdown_list_item(line: &str) -> bool {
    is_markdown_unordered_list_item(line) || is_markdown_ordered_list_item(line)
}

pub(in crate::app) fn is_indented_markdown_line(line: &str) -> bool {
    line.starts_with("  ") || line.starts_with('\t')
}

pub(in crate::app) fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let text = trimmed.get(level..)?.strip_prefix(' ')?.trim();
    let text = text.trim_end_matches('#').trim_end();
    (!text.is_empty()).then_some((level, text))
}

pub(in crate::app) fn is_markdown_quote_line(line: &str) -> bool {
    markdown_quote_depth_and_text(line).is_some()
}

pub(in crate::app) fn markdown_quote_depth_and_text(line: &str) -> Option<(usize, &str)> {
    let mut depth = 0_usize;
    let mut rest = line.trim_start();
    while let Some(after_marker) = rest.strip_prefix('>') {
        depth += 1;
        rest = after_marker.strip_prefix(' ').unwrap_or(after_marker);
    }
    (depth > 0).then_some((depth, rest))
}

pub(in crate::app) fn strip_markdown_quote_level(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

pub(in crate::app) fn is_markdown_thematic_break(line: &str) -> bool {
    let mut marker = None;
    let mut count = 0_usize;
    for ch in line.chars().filter(|ch| !ch.is_whitespace()) {
        if !matches!(ch, '-' | '*' | '_') || marker.is_some_and(|value| value != ch) {
            return false;
        }
        marker = Some(ch);
        count += 1;
    }
    count >= 3
}

pub(in crate::app) fn render_markdown_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    block: &MarkdownBlock,
    width: u16,
) {
    match block.kind {
        TranscriptNodeKind::MarkdownCode => {
            push_markdown_code_block(out, prefix, &block.source, width)
        }
        TranscriptNodeKind::MarkdownList => push_markdown_list(out, prefix, &block.source, width),
        TranscriptNodeKind::MarkdownTable => {
            let table_lines = block.source.lines().collect::<Vec<_>>();
            push_markdown_table(out, prefix, table_lines.as_slice(), width);
        }
        TranscriptNodeKind::MarkdownMath => push_math_block(out, prefix, &block.source, width),
        TranscriptNodeKind::MarkdownParagraph => {
            if let Some((level, text)) = markdown_heading(&block.source) {
                push_markdown_heading(out, prefix, level, text, width);
            } else if block.source.lines().all(is_markdown_quote_line) {
                push_markdown_quote(out, prefix, &block.source, width);
            } else if is_markdown_thematic_break(&block.source) {
                push_markdown_rule(out, prefix, width);
            } else if !push_inline_math(out, prefix, &block.source, width) {
                push_markdown(out, prefix, &block.source, width);
            }
        }
        TranscriptNodeKind::Message | TranscriptNodeKind::Activity => {
            push_markdown(out, prefix, &block.source, width);
        }
    }
}

pub(in crate::app) fn should_suppress_markdown_block(
    blocks: &[MarkdownBlock],
    index: usize,
) -> bool {
    let Some(block) = blocks.get(index) else {
        return false;
    };
    let Some(previous) = index.checked_sub(1).and_then(|index| blocks.get(index)) else {
        return false;
    };
    // A heading already carries its own visual rule.  Markdown examples often
    // place `---` right after every heading, which otherwise produces two
    // adjacent separators with no semantic value in the transcript.
    markdown_heading(previous.source.as_str()).is_some()
        && is_markdown_thematic_break(block.source.as_str())
}

pub(in crate::app) fn push_markdown_heading(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    level: usize,
    text: &str,
    width: u16,
) {
    let marker = match level {
        1 => "══",
        2 => "──",
        _ => "›",
    };
    if crate::math_render::layout_config().native_graphics
        && push_inline_math(out, format!("{prefix}{marker} ").as_str(), text, width)
    {
        return;
    }
    let text = markdown_inline_text(text);
    let style = Style::default()
        .fg(if level <= 2 {
            agena_tui_components::theme::accent_color()
        } else {
            agena_tui_components::theme::info_color()
        })
        .add_modifier(Modifier::BOLD);
    let available = width.max(1) as usize;
    let start = format!("{prefix}{marker} {text} ");
    if level <= 2 && UnicodeWidthStr::width(start.as_str()) < available {
        let fill = "─".repeat(available.saturating_sub(UnicodeWidthStr::width(start.as_str())));
        out.push(RenderedLine::plain(format!("{start}{fill}"), style));
    } else {
        push_wrapped_line(
            out,
            prefix,
            prefix,
            format!("{marker} {text}").as_str(),
            style,
            width,
        );
    }
}

pub(in crate::app) fn push_markdown_quote(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    source: &str,
    width: u16,
) {
    let inner_source = source
        .lines()
        .filter_map(strip_markdown_quote_level)
        .collect::<Vec<_>>()
        .join("\n");
    let inner_prefix = format!("{prefix}│ ");
    let blocks = markdown_blocks(inner_source.as_str());
    for block in blocks {
        if block.leading_blank_line {
            out.push(RenderedLine::plain(
                inner_prefix.to_string(),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            ));
        }
        render_markdown_block(out, inner_prefix.as_str(), &block, width);
    }
}

pub(in crate::app) fn push_markdown_rule(out: &mut Vec<RenderedLine>, prefix: &str, width: u16) {
    let available = (width.max(1) as usize).saturating_sub(UnicodeWidthStr::width(prefix));
    push_single_line(
        out,
        prefix,
        "─".repeat(available.clamp(3, 24)).as_str(),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
}

pub(in crate::app) fn push_markdown_code_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    source: &str,
    width: u16,
) {
    let source_lines = source.lines().collect::<Vec<_>>();
    let opening = source_lines.first().copied().unwrap_or_default();
    let language = code_block_language(opening);
    let has_closing_fence = source_lines
        .last()
        .is_some_and(|line| source_lines.len() > 1 && markdown_fence_delimiter(line).is_some());
    let code_lines = if has_closing_fence {
        &source_lines[1..source_lines.len().saturating_sub(1)]
    } else {
        &source_lines[1..]
    };

    let available = width.max(1) as usize;
    let prefix_width = UnicodeWidthStr::width(prefix);
    let card_width = available.saturating_sub(prefix_width);
    if card_width < 16 {
        push_single_line(
            out,
            prefix,
            format!("[{language}]").as_str(),
            Style::default()
                .fg(agena_tui_components::theme::accent_color())
                .add_modifier(Modifier::BOLD),
            width,
        );
        for line in code_lines {
            for segment in wrap_display_text(line.replace('\t', "    ").as_str(), card_width) {
                push_single_line(out, prefix, segment.as_str(), Style::default(), width);
            }
        }
        return;
    }

    let label = truncate_display_width(language.as_str(), card_width.saturating_sub(7).max(1));
    let top_start = format!("┌─ {label} ");
    let top_fill = "─".repeat(
        card_width
            .saturating_sub(UnicodeWidthStr::width(top_start.as_str()))
            .saturating_sub(1),
    );
    out.push(RenderedLine::rich(Line::from(vec![
        Span::raw(prefix.to_string()),
        Span::styled(
            "┌─ ",
            Style::default().fg(agena_tui_components::theme::muted_color()),
        ),
        Span::styled(
            label,
            Style::default()
                .fg(agena_tui_components::theme::accent_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {top_fill}┐"),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        ),
    ])));

    let line_count_width = code_lines.len().max(1).to_string().len();
    let gutter_width = line_count_width.saturating_add(1);
    let body_width = card_width.saturating_sub(gutter_width).saturating_sub(2);
    for (index, line) in code_lines.iter().enumerate() {
        let number = format!("{:>width$} ", index + 1, width = line_count_width);
        for (segment_index, body) in
            wrap_display_text(line.replace('\t', "    ").as_str(), body_width)
                .into_iter()
                .enumerate()
        {
            let gutter = if segment_index == 0 {
                number.as_str()
            } else {
                ""
            };
            let gutter_padding =
                " ".repeat(gutter_width.saturating_sub(UnicodeWidthStr::width(gutter)));
            let padding =
                " ".repeat(body_width.saturating_sub(UnicodeWidthStr::width(body.as_str())));
            out.push(RenderedLine::rich(Line::from(vec![
                Span::raw(prefix.to_string()),
                Span::styled(
                    "│",
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                ),
                Span::styled(
                    format!("{gutter}{gutter_padding}"),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                ),
                Span::styled(format!("{body}{padding}"), Style::default()),
                Span::styled(
                    "│",
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                ),
            ])));
        }
    }
    if code_lines.is_empty() {
        out.push(RenderedLine::rich(Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled(
                "│",
                Style::default().fg(agena_tui_components::theme::muted_color()),
            ),
            Span::styled(
                "  (empty)".to_string() + &" ".repeat(card_width.saturating_sub(11)),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            ),
            Span::styled(
                "│",
                Style::default().fg(agena_tui_components::theme::muted_color()),
            ),
        ])));
    }
    out.push(RenderedLine::plain(
        format!("{prefix}└{}┘", "─".repeat(card_width.saturating_sub(2))),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    ));
}

pub(in crate::app) fn code_block_language(opening: &str) -> String {
    let Some(fence) = markdown_fence_delimiter(opening) else {
        return "code".to_string();
    };
    let language = opening
        .trim_start()
        .trim_start_matches(fence.marker)
        .split_whitespace()
        .next()
        .unwrap_or("code");
    if language.is_empty() {
        "code".to_string()
    } else {
        language.to_string()
    }
}

pub(in crate::app) fn wrap_display_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0_usize;
    for grapheme in text.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if !line.is_empty() && line_width.saturating_add(grapheme_width) > width {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        line.push_str(grapheme);
        line_width = line_width.saturating_add(grapheme_width);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

pub(in crate::app) fn push_markdown_list(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    source: &str,
    width: u16,
) {
    for line in source.lines() {
        if line.trim().is_empty() {
            out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
            continue;
        }
        if let Some((indent, marker, text)) = markdown_list_item_parts(line) {
            let depth = indent / 2;
            let marker = display_list_marker(marker, text, depth);
            let list_prefix = format!("{prefix}{}{} ", "  ".repeat(depth), marker);
            let continuation = format!(
                "{prefix}{}{}",
                "  ".repeat(depth),
                " ".repeat(UnicodeWidthStr::width(marker.as_str()) + 1)
            );
            let text = display_list_text(text);
            if crate::math_render::layout_config().native_graphics
                && push_inline_math(out, list_prefix.as_str(), text.as_str(), width)
            {
                continue;
            }
            let text = markdown_inline_text(text.as_str());
            push_wrapped_line(
                out,
                list_prefix.as_str(),
                continuation.as_str(),
                text.as_str(),
                Style::default(),
                width,
            );
        } else {
            let indent = line.len().saturating_sub(line.trim_start().len()) / 2;
            push_wrapped_line(
                out,
                format!("{prefix}{}", "  ".repeat(indent)).as_str(),
                format!("{prefix}{}", "  ".repeat(indent)).as_str(),
                line.trim(),
                Style::default().fg(agena_tui_components::theme::muted_color()),
                width,
            );
        }
    }
}

pub(in crate::app) fn markdown_list_item_parts(line: &str) -> Option<(usize, &str, &str)> {
    let indent = line.len().saturating_sub(line.trim_start().len());
    let trimmed = line.trim_start();
    for marker in ["-", "*", "+"] {
        if let Some(text) = trimmed
            .strip_prefix(marker)
            .and_then(|rest| rest.strip_prefix(' '))
        {
            return Some((indent, marker, text));
        }
    }
    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0
        && trimmed
            .get(digits..)
            .is_some_and(|rest| rest.starts_with(". "))
    {
        Some((indent, &trimmed[..digits + 1], &trimmed[digits + 2..]))
    } else {
        None
    }
}

pub(in crate::app) fn display_list_marker(marker: &str, text: &str, depth: usize) -> String {
    if marker.ends_with('.') {
        marker.to_string()
    } else if text.starts_with("[x] ") || text.starts_with("[X] ") {
        "●".to_string()
    } else if text.starts_with("[ ] ") {
        "○".to_string()
    } else {
        ["•", "◦", "▪"][depth.min(2)].to_string()
    }
}

pub(in crate::app) fn display_list_text(text: &str) -> String {
    text.strip_prefix("[ ] ")
        .or_else(|| text.strip_prefix("[x] "))
        .or_else(|| text.strip_prefix("[X] "))
        .unwrap_or(text)
        .to_string()
}

pub(in crate::app) fn markdown_inline_text(text: &str) -> String {
    let text = inline_math_unicode_text(text);
    let rendered = markdown_to_text(&text);
    let plain = rendered
        .lines
        .iter()
        .map(|line| line_plain_text(&owned_line(line)))
        .collect::<Vec<_>>()
        .join(" ");
    sanitize_terminal_text(plain.as_str()).trim().to_string()
}
use super::{
    I18n, Line, MarkdownBlock, MessagePart, MessageResource, Modifier, OperationBlock,
    OperationPart, PartContent, RenderedLine, Span, Style, TranscriptNodeKind, UnicodeWidthStr,
    file_change_list_item_text, is_markdown_table_header, line_plain_text,
    looks_like_markdown_table_row, markdown_fence_delimiter, markdown_to_text, owned_line,
    push_limited_markdown, push_limited_tool_text, push_markdown, push_markdown_table,
    push_single_line, push_wrapped_line, sanitize_terminal_text, tool_invocation_label,
    trim_empty_line_edges, truncate_display_width, ui_text,
};
use unicode_segmentation::UnicodeSegmentation;
