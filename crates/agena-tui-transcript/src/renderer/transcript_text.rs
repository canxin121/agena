use super::transcript_ast::{parse_markdown_document, render_parsed_markdown_block};
use super::{
    I18n, Line, MarkdownBlock, Modifier, RenderedLine, Span, Style, TranscriptEntry,
    UnicodeWidthStr, file_change_list_item_text, is_markdown_table_header,
    markdown_fence_delimiter, push_expanded_markdown, push_expanded_tool_text, push_single_line,
    sanitize_terminal_text, tool_invocation_label, trim_empty_line_edges, truncate_display_width,
};
use crate::ui_text;
use crate::{OperationBlockResource, OperationPartResource};
use crate::{TranscriptEntryPart, TranscriptPartContent};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn transcript_message_parts(message: &TranscriptEntry) -> &[TranscriptEntryPart] {
    message.parts.as_slice()
}

pub(crate) fn transcript_part_content(part: &TranscriptEntryPart) -> &TranscriptPartContent {
    &part.content
}

pub(crate) fn operation_block_copy_text(block: &OperationBlockResource, i18n: &I18n) -> String {
    match block {
        OperationBlockResource::Text { text }
        | OperationBlockResource::Markdown { text }
        | OperationBlockResource::Diff { diff: text, .. } => text.clone(),
        OperationBlockResource::Command {
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
        OperationBlockResource::SearchResults { query, results } => {
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
        OperationBlockResource::EmbeddedResource { uri, text, .. } => text
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| uri.clone()),
        OperationBlockResource::Checklist { items } => items
            .iter()
            .map(|item| item.content.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        OperationBlockResource::FileChanges { changes } => changes
            .iter()
            .map(|change| file_change_list_item_text(change, i18n))
            .collect::<Vec<_>>()
            .join("\n"),
        OperationBlockResource::ResourceLink { uri, title, .. }
        | OperationBlockResource::Citation { uri, title, .. } => {
            title.clone().unwrap_or_else(|| uri.clone())
        }
        OperationBlockResource::Image { url, .. }
        | OperationBlockResource::Audio { url, .. }
        | OperationBlockResource::File { url, .. } => url.clone(),
        OperationBlockResource::Media { artifact, .. } => artifact
            .name
            .clone()
            .unwrap_or_else(|| artifact.uri.clone()),
        OperationBlockResource::Progress { message, .. } => message.clone(),
        OperationBlockResource::NestedTask {
            task_id,
            title,
            status,
        } => ui_text::operation_nested_task_summary(
            i18n,
            title.as_deref().unwrap_or(task_id.as_str()),
            *status,
        ),
        OperationBlockResource::Json { .. }
        | OperationBlockResource::Table { .. }
        | OperationBlockResource::Log { .. }
        | OperationBlockResource::Custom { .. } => String::new(),
    }
}

pub(crate) fn tool_display_label(tool: &OperationPartResource) -> String {
    if tool.title.trim().is_empty() {
        tool_invocation_label(&tool.invocation)
    } else {
        tool.title.clone()
    }
}

pub(crate) fn should_render_tool_model_output(
    tool: &OperationPartResource,
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
            .any(|block| matches!(block, OperationBlockResource::SearchResults { .. }))
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

pub(crate) fn operation_text_block_text(block: &OperationBlockResource) -> Option<&str> {
    match block {
        OperationBlockResource::Text { text } | OperationBlockResource::Markdown { text } => {
            Some(text.as_str())
        }
        _ => None,
    }
}

pub(crate) fn normalized_tool_text(text: &str) -> String {
    let sanitized = sanitize_terminal_text(text);
    trim_empty_line_edges(sanitized.as_str()).to_string()
}

pub(crate) fn render_expanded_tool_text_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    width: u16,
) {
    if tool_text_looks_like_markdown(text) {
        push_expanded_markdown(out, prefix, text, width);
    } else {
        push_expanded_tool_text(out, prefix, text, Style::default(), width);
    }
}

pub(crate) fn tool_text_looks_like_markdown(text: &str) -> bool {
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

pub(crate) fn is_markdown_unordered_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    ["- ", "* ", "+ "]
        .into_iter()
        .any(|prefix| trimmed.starts_with(prefix) && trimmed.len() > prefix.len())
}

pub(crate) fn is_markdown_ordered_list_item(line: &str) -> bool {
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

/// Parses a Markdown text part into independently navigable transcript blocks.
pub fn markdown_blocks(text: &str) -> Vec<MarkdownBlock> {
    parse_markdown_document(text)
}

pub(crate) fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let text = trimmed.get(level..)?.strip_prefix(' ')?.trim();
    let text = text.trim_end_matches('#').trim_end();
    (!text.is_empty()).then_some((level, text))
}

pub(crate) fn is_markdown_thematic_break(line: &str) -> bool {
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

pub fn render_markdown_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    block: &MarkdownBlock,
    width: u16,
) {
    render_parsed_markdown_block(out, prefix, block, width);
}

pub(crate) fn should_suppress_markdown_block(blocks: &[MarkdownBlock], index: usize) -> bool {
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

pub(crate) fn push_markdown_rule(out: &mut Vec<RenderedLine>, prefix: &str, width: u16) {
    let available = (width.max(1) as usize).saturating_sub(UnicodeWidthStr::width(prefix));
    push_single_line(
        out,
        prefix,
        "─".repeat(available.clamp(3, 24)).as_str(),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
}

pub(crate) fn push_markdown_code_block(
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
    let palette = agena_tui_components::theme::active_palette();
    let code_uses_terminal_defaults = palette.code_bg == ratatui::style::Color::Reset;
    let code_muted = if code_uses_terminal_defaults {
        palette.code_fg
    } else {
        palette.muted
    };
    let code_accent = if code_uses_terminal_defaults {
        palette.code_fg
    } else {
        palette.accent
    };
    if card_width < 16 {
        push_single_line(
            out,
            prefix,
            format!("[{language}]").as_str(),
            Style::default()
                .fg(code_accent)
                .add_modifier(Modifier::BOLD),
            width,
        );
        if let Some(label) = out.last_mut() {
            label.copy_text.clear();
        }
        for line in code_lines {
            let navigation_unit = out.len();
            for segment in wrap_display_text(line.replace('\t', "    ").as_str(), card_width) {
                push_single_line(out, prefix, segment.as_str(), Style::default(), width);
                if let Some(rendered) = out.last_mut() {
                    rendered.navigation_unit = Some(navigation_unit);
                    rendered.navigation_copy_text = (*line).to_string();
                }
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
    out.push(
        RenderedLine::rich(Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled("┌─ ", Style::default().fg(code_muted).bg(palette.code_bg)),
            Span::styled(
                label,
                Style::default()
                    .fg(code_accent)
                    .bg(palette.code_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {top_fill}┐"),
                Style::default().fg(code_muted).bg(palette.code_bg),
            ),
        ]))
        .with_copy_projection(String::new(), prefix_width),
    );

    let line_count_width = code_lines.len().max(1).to_string().len();
    let gutter_width = line_count_width.saturating_add(1);
    let body_width = card_width.saturating_sub(gutter_width).saturating_sub(2);
    let highlighted_lines = syntax_highlight_lines(&language, code_lines, palette);
    for (index, line) in highlighted_lines.into_iter().enumerate() {
        let navigation_unit = out.len();
        let navigation_copy_text = code_lines.get(index).copied().unwrap_or_default();
        let number = format!("{:>width$} ", index + 1, width = line_count_width);
        for (segment_index, body) in wrap_styled_spans(line, body_width).into_iter().enumerate() {
            let gutter = if segment_index == 0 {
                number.as_str()
            } else {
                ""
            };
            let gutter_padding =
                " ".repeat(gutter_width.saturating_sub(UnicodeWidthStr::width(gutter)));
            let body_display_width = body
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>();
            let body_copy_text = body
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            let padding = " ".repeat(body_width.saturating_sub(body_display_width));
            let mut spans = vec![
                Span::raw(prefix.to_string()),
                Span::styled("│", Style::default().fg(code_muted).bg(palette.code_bg)),
                Span::styled(
                    format!("{gutter}{gutter_padding}"),
                    Style::default().fg(code_muted).bg(palette.code_bg),
                ),
            ];
            spans.extend(body);
            spans.extend([
                Span::styled(padding, Style::default().bg(palette.code_bg)),
                Span::styled("│", Style::default().fg(code_muted).bg(palette.code_bg)),
            ]);
            out.push(
                RenderedLine::rich(Line::from(spans))
                    .with_copy_projection(
                        body_copy_text,
                        prefix_width.saturating_add(1).saturating_add(gutter_width),
                    )
                    .with_navigation_unit(navigation_unit, navigation_copy_text),
            );
        }
    }
    if code_lines.is_empty() {
        out.push(
            RenderedLine::rich(Line::from(vec![
                Span::raw(prefix.to_string()),
                Span::styled("│", Style::default().fg(code_muted).bg(palette.code_bg)),
                Span::styled(
                    "  (empty)".to_string() + &" ".repeat(card_width.saturating_sub(11)),
                    Style::default().fg(palette.code_fg).bg(palette.code_bg),
                ),
                Span::styled("│", Style::default().fg(code_muted).bg(palette.code_bg)),
            ]))
            .with_copy_projection(String::new(), prefix_width),
        );
    }
    out.push(
        RenderedLine::rich(Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled(
                format!("└{}┘", "─".repeat(card_width.saturating_sub(2))),
                Style::default().fg(code_muted).bg(palette.code_bg),
            ),
        ]))
        .with_copy_projection(String::new(), prefix_width),
    );
}

fn syntax_highlight_lines(
    language: &str,
    lines: &[&str],
    palette: agena_tui_components::ThemePalette,
) -> Vec<Vec<Span<'static>>> {
    use std::sync::LazyLock;

    use syntect::{
        easy::HighlightLines,
        highlighting::{FontStyle, ThemeSet},
        parsing::SyntaxSet,
    };

    static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
    static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

    let syntax = SYNTAXES
        .find_syntax_by_token(language)
        .or_else(|| SYNTAXES.find_syntax_by_extension(language))
        .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    let background = match palette.code_bg {
        ratatui::style::Color::Rgb(red, green, blue) => {
            Some(agena_tui_components::TerminalRgb::new(red, green, blue))
        }
        ratatui::style::Color::Reset => None,
        _ => unreachable!("built-in code surface colors are RGB or terminal defaults"),
    };
    let theme_name = if palette.scheme == agena_tui_components::ColorScheme::Light {
        // The GitHub-oriented light theme is a better fit for a white terminal
        // than Solarized, whose low-contrast colors assume a cream canvas.
        "InspiredGitHub"
    } else {
        "base16-ocean.dark"
    };
    let Some(theme) = THEMES.themes.get(theme_name) else {
        return lines
            .iter()
            .map(|line| {
                vec![Span::styled(
                    line.replace('\t', "    "),
                    Style::default().fg(palette.code_fg).bg(palette.code_bg),
                )]
            })
            .collect();
    };
    let mut highlighter = HighlightLines::new(syntax, theme);
    lines
        .iter()
        .map(|line| {
            let expanded = line.replace('\t', "    ");
            let mut terminated = expanded.clone();
            terminated.push('\n');
            let highlighted = highlighter
                .highlight_line(&terminated, &SYNTAXES)
                .unwrap_or_else(|_| Vec::new());
            let mut spans = highlighted
                .into_iter()
                .filter_map(|(syntax_style, text)| {
                    let text = text.strip_suffix('\n').unwrap_or(text);
                    if text.is_empty() {
                        return None;
                    }
                    let foreground = syntax_style.foreground;
                    let foreground =
                        background.map_or(ratatui::style::Color::Reset, |background| {
                            agena_tui_components::theme::readable_text_color(
                                ratatui::style::Color::Rgb(
                                    foreground.r,
                                    foreground.g,
                                    foreground.b,
                                ),
                                background,
                            )
                        });
                    let mut style = Style::default().fg(foreground).bg(palette.code_bg);
                    if syntax_style.font_style.contains(FontStyle::BOLD) {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if syntax_style.font_style.contains(FontStyle::ITALIC) {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if syntax_style.font_style.contains(FontStyle::UNDERLINE) {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    Some(Span::styled(text.to_string(), style))
                })
                .collect::<Vec<_>>();
            if spans.is_empty() && !expanded.is_empty() {
                spans.push(Span::styled(
                    expanded,
                    Style::default().fg(palette.code_fg).bg(palette.code_bg),
                ));
            }
            spans
        })
        .collect()
}

fn wrap_styled_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let mut rows = vec![Vec::new()];
    let mut row_width = 0_usize;
    for span in spans {
        let mut chunk = String::new();
        for grapheme in span.content.graphemes(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            if row_width > 0 && row_width.saturating_add(grapheme_width) > width {
                if !chunk.is_empty() {
                    rows.last_mut()
                        .expect("styled rows are never empty")
                        .push(Span::styled(std::mem::take(&mut chunk), span.style));
                }
                rows.push(Vec::new());
                row_width = 0;
            }
            chunk.push_str(grapheme);
            row_width = row_width.saturating_add(grapheme_width);
        }
        if !chunk.is_empty() {
            rows.last_mut()
                .expect("styled rows are never empty")
                .push(Span::styled(chunk, span.style));
        }
    }
    rows
}

pub(crate) fn code_block_language(opening: &str) -> String {
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

pub(crate) fn wrap_display_text(text: &str, width: usize) -> Vec<String> {
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

#[cfg(test)]
mod syntax_highlight_tests {
    use super::*;

    #[test]
    fn code_tokens_keep_readable_foregrounds_on_both_code_surfaces() {
        let source = [
            "fn main() {",
            "    // comment",
            "    println!(\"hello\");",
            "}",
        ];
        for scheme in [
            agena_tui_components::ColorScheme::Dark,
            agena_tui_components::ColorScheme::Light,
        ] {
            let palette = agena_tui_components::ThemePalette::for_scheme(scheme);
            let ratatui::style::Color::Rgb(red, green, blue) = palette.code_bg else {
                panic!("code background must be RGB")
            };
            let background = agena_tui_components::TerminalRgb::new(red, green, blue);
            let highlighted = syntax_highlight_lines("rust", &source, palette);

            for span in highlighted.iter().flatten() {
                assert_eq!(span.style.bg, Some(palette.code_bg));
                let foreground = span.style.fg.unwrap_or(palette.code_fg);
                assert_eq!(
                    agena_tui_components::theme::readable_text_color(foreground, background),
                    foreground,
                    "{scheme:?} code token lacks readable contrast: {span:?}"
                );
            }
        }
    }

    #[test]
    fn unknown_terminal_background_never_forces_a_dark_code_surface() {
        let palette = agena_tui_components::ThemePalette::for_unknown_background();
        let highlighted = syntax_highlight_lines("rust", &["let answer = 42;"], palette);

        for span in highlighted.iter().flatten() {
            assert_eq!(span.style.fg, Some(ratatui::style::Color::Reset));
            assert_eq!(span.style.bg, Some(ratatui::style::Color::Reset));
        }
    }
}
