use super::*;

pub fn render_parsed_markdown_block(
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
            let start = out.len();
            render_diagram(out, prefix, language, literal, width);
            mark_rendered_semantic_unit(
                out,
                start,
                format!("```{language}\n{}\n```", literal.trim_end_matches('\n')),
            );
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
        MarkdownNode::Image {
            url,
            title,
            alt,
            dimensions,
            link_url,
        } => {
            let start = out.len();
            render_image_block(
                out,
                prefix,
                alt,
                url,
                title,
                *dimensions,
                link_url.as_deref(),
                width,
            );
            let target = if title.is_empty() {
                url.clone()
            } else {
                format!("{url} \"{title}\"")
            };
            let image = format!("![{alt}]({target})");
            mark_rendered_semantic_unit(
                out,
                start,
                link_url
                    .as_deref()
                    .map_or(image.clone(), |link| format!("[{image}]({link})")),
            );
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
    render_inline_flow(out, prefix, prefix, inlines, Style::default(), width);
}

/// Render one inline flow with distinct first-line and continuation prefixes.
///
/// Lists and headings use this to place their marker on the row that actually
/// contains the surrounding text. Multi-row formula/image canvases use the
/// continuation prefix above and below that anchor, so structural markers are
/// never duplicated just because a graphic is taller than one terminal row.
fn render_inline_flow(
    out: &mut Vec<RenderedLine>,
    initial_prefix: &str,
    continuation_prefix: &str,
    inlines: &[MarkdownInline],
    style: Style,
    width: u16,
) {
    let rich_start = out.len();
    if inlines_contain_rich_graphics(inlines)
        && push_rich_inline_graphics(
            out,
            initial_prefix,
            continuation_prefix,
            inlines,
            style,
            width,
        )
    {
        mark_rendered_semantic_unit(out, rich_start, inline_plain_text(inlines));
        return;
    }
    let mut first = true;
    for line in rich_inline_lines(inlines, style) {
        let line_prefix = if first {
            initial_prefix
        } else {
            continuation_prefix
        };
        push_wrapped_rich_line(out, line_prefix, continuation_prefix, line, width);
        first = false;
    }
}

fn mark_rendered_semantic_unit(
    out: &mut [RenderedLine],
    start: usize,
    copy_text: impl Into<String>,
) {
    let copy_text = copy_text.into();
    for line in out.get_mut(start..).unwrap_or_default() {
        line.navigation_unit = Some(start);
        line.navigation_copy_text.clone_from(&copy_text);
        line.pointer_selection = TranscriptPointerSelection::SemanticUnit;
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
    render_inline_flow(out, &first_prefix, &continuation, inlines, style, width);
}

#[derive(Debug)]
enum RichInlineAtom {
    Text(Span<'static>),
    Math(String),
    Image {
        url: String,
        alt: String,
        dimensions: MarkdownImageDimensions,
    },
}

fn push_rich_inline_graphics(
    out: &mut Vec<RenderedLine>,
    initial_prefix: &str,
    continuation_prefix: &str,
    inlines: &[MarkdownInline],
    base_style: Style,
    width: u16,
) -> bool {
    let mut atoms = Vec::new();
    if !append_rich_inline_atoms(&mut atoms, inlines, base_style) {
        return false;
    }
    let prefix_width = u16::try_from(UnicodeWidthStr::width(initial_prefix)).unwrap_or(u16::MAX);
    let continuation_width =
        u16::try_from(UnicodeWidthStr::width(continuation_prefix)).unwrap_or(u16::MAX);
    // A native image occupies the same columns on every row. If a future
    // caller supplies unequal prefix widths, use the normal text fallback
    // rather than letting a continuation marker overlap the image.
    if continuation_width != prefix_width {
        return false;
    }
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
                    let Ok(artifact) = agena_tui_media::render_formula(&literal, false) else {
                        return false;
                    };
                    total_width = total_width.saturating_add(artifact.size.width);
                    height = height.max(artifact.size.height);
                    let size = artifact.size;
                    rendered.push((None, Some((artifact, size)), size.width));
                }
                RichInlineAtom::Image {
                    url,
                    alt,
                    dimensions,
                } => {
                    if let Ok(artifact) = render_markdown_image(&url) {
                        let size = fit_image_size(
                            artifact.image.width(),
                            artifact.image.height(),
                            dimensions,
                            available.min(12),
                            4,
                        );
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
        let vertical = InlineVerticalLayout::new(height);
        let start = out.len();
        for row in 0..vertical.height() {
            let row_prefix = if usize::from(row) == vertical.text_row() {
                initial_prefix
            } else {
                continuation_prefix
            };
            out.push(RenderedLine::plain(
                row_prefix.to_string(),
                Style::default(),
            ));
        }
        let mut spans = vec![Span::raw(initial_prefix.to_string())];
        let mut column = prefix_width;
        for (span, graphic, atom_width) in rendered {
            if let Some(span) = span {
                spans.push(span);
            } else if let Some((artifact, size)) = graphic {
                let placement_row = vertical.graphic_top_row(size.height);
                out[start + placement_row].math.push(MathLinePlacement {
                    column,
                    size,
                    artifact,
                });
                spans.push(Span::raw(" ".repeat(usize::from(atom_width))));
            }
            column = column.saturating_add(atom_width);
        }
        out[start + vertical.text_row()]
            .replace_content_preserving_math(RenderedLine::rich(Line::from(spans)));
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
            RichInlineAtom::Image { url, alt, .. } => vec![vec![Span::styled(
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
    // The Unicode renderer bottom-aligns surrounding text and formula blocks,
    // so the last row is its text anchor. Keep a list/heading marker there and
    // use only the continuation indentation on the taller formula rows.
    let text_row = rows.len().saturating_sub(1);
    for (index, mut row) in rows.into_iter().enumerate() {
        let row_prefix = if index == text_row {
            initial_prefix
        } else {
            continuation_prefix
        };
        row.insert(0, Span::raw(row_prefix.to_string()));
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
            MarkdownInline::Image {
                url,
                alt,
                dimensions,
                ..
            } => atoms.push(RichInlineAtom::Image {
                url: url.clone(),
                alt: alt.clone(),
                dimensions: *dimensions,
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
        let continuation = format!(
            "{prefix}{}",
            " ".repeat(UnicodeWidthStr::width(marker.as_str()) + 1)
        );
        let mut first = true;
        for block in &item.blocks {
            match block {
                MarkdownNode::Paragraph(inlines) if first => {
                    render_inline_flow(
                        out,
                        &first_prefix,
                        &continuation,
                        inlines,
                        Style::default(),
                        width,
                    );
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
                _ if first => {
                    render_first_list_block(out, &first_prefix, &continuation, block, width)
                }
                _ => render_markdown_node(out, &continuation, block, width),
            }
            first = false;
        }
        if item.blocks.is_empty() {
            push_single_line(out, &first_prefix, "", Style::default(), width);
        }
    }
}

/// Render a non-paragraph first block with continuation indentation, then add
/// exactly one list marker. Existing block renderers deliberately accept one
/// repeated prefix (for quote rails, table indentation, code cards, and image
/// canvases); passing a bullet through that interface would duplicate it on
/// every physical row. Native graphics place the marker on their center row,
/// while ordinary block content uses its first row.
fn render_first_list_block(
    out: &mut Vec<RenderedLine>,
    initial_prefix: &str,
    continuation_prefix: &str,
    block: &MarkdownNode,
    width: u16,
) {
    let start = out.len();
    render_markdown_node(out, continuation_prefix, block, width);
    let rendered = &out[start..];
    if rendered.is_empty() {
        return;
    }
    let marker_row = rendered
        .iter()
        .enumerate()
        .find_map(|(row, line)| {
            line.math
                .first()
                .map(|placement| row.saturating_add(usize::from(placement.size.height / 2)))
        })
        .filter(|row| *row < rendered.len())
        .unwrap_or(0);
    replace_rendered_line_prefix(
        &mut out[start + marker_row],
        continuation_prefix,
        initial_prefix,
    );
}

fn replace_rendered_line_prefix(line: &mut RenderedLine, old: &str, new: &str) {
    let Some(rest) = line.text.strip_prefix(old).map(str::to_string) else {
        return;
    };
    let replacement_spans = line.rich_line.as_ref().and_then(|rich| {
        let mut remaining = old.len();
        let mut spans = Vec::with_capacity(rich.spans.len().saturating_add(1));
        spans.push(Span::raw(new.to_string()));
        for span in &rich.spans {
            let content = span.content.as_ref();
            if remaining == 0 {
                spans.push(span.clone());
            } else if remaining >= content.len() {
                remaining -= content.len();
            } else {
                if !content.is_char_boundary(remaining) {
                    return None;
                }
                spans.push(Span::styled(content[remaining..].to_string(), span.style));
                remaining = 0;
            }
        }
        (remaining == 0).then_some(spans)
    });
    if line.rich_line.is_some() && replacement_spans.is_none() {
        return;
    }

    line.text = format!("{new}{rest}");
    if let (Some(rich), Some(spans)) = (line.rich_line.as_mut(), replacement_spans) {
        rich.spans = spans;
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

pub(super) fn is_diagram_language(language: &str) -> bool {
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
        let size = fit_image_size(
            artifact.image.width(),
            artifact.image.height(),
            MarkdownImageDimensions::default(),
            available,
            24,
        );
        let (render_width, render_height) = (size.width, size.height);
        let column = prefix_width + available.saturating_sub(render_width) / 2;
        let start = out.len();
        for _ in 0..render_height {
            out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
        }
        out[start].math.push(MathLinePlacement {
            column,
            artifact,
            size,
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
    let column_count = rows.iter().map(|row| row.cells.len()).max().unwrap_or(0);
    if column_count == 0 {
        return;
    }
    let separator_width = column_count.saturating_mul(3).saturating_add(1);
    let prefix_width = UnicodeWidthStr::width(prefix);
    let budget = usize::from(width)
        .saturating_sub(prefix_width)
        .saturating_sub(separator_width);
    // Size columns from the representation that is actually drawn. Rich
    // Markdown adds visible text that is absent from `inline_plain_text`, most
    // notably the destination suffix on links and wiki links. Measuring only
    // the label leaves unused terminal width and then wraps the suffix inside
    // an unnecessarily narrow cell.
    let natural_widths = (0..column_count)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.cells.get(column))
                .map(|cell| {
                    rich_inline_lines(cell, Style::default())
                        .iter()
                        .map(|line| rich_spans_width(line.spans.as_slice()))
                        .max()
                        .unwrap_or(0)
                })
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let widths = fit_table_column_widths(natural_widths.as_slice(), budget);
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
        let navigation_unit = out.len();
        render_rich_table_row(out, prefix, row, &widths, &table_alignments, width);
        let navigation_copy_text = row
            .cells
            .iter()
            .map(|cell| inline_plain_text(cell))
            .collect::<Vec<_>>()
            .join("\t");
        for line in &mut out[navigation_unit..] {
            line.navigation_unit = Some(navigation_unit);
            line.navigation_copy_text.clone_from(&navigation_copy_text);
        }
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
        let mut copy_cells = Vec::with_capacity(widths.len());
        let mut copy_segments = Vec::with_capacity(widths.len());
        let mut display_column = UnicodeWidthStr::width(prefix).saturating_add(1);
        let mut spans = vec![
            Span::raw(prefix.to_string()),
            Span::styled("│", border_style),
        ];
        for (column, cell_width) in widths.iter().enumerate() {
            spans.push(Span::raw(" "));
            display_column = display_column.saturating_add(1);
            let content_line = cells
                .get(column)
                .and_then(|lines| lines.get(line_index))
                .cloned()
                .unwrap_or_default();
            let copy_text = content_line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            copy_cells.push(copy_text.clone());
            let mut content = content_line.spans;
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
            display_column = display_column.saturating_add(left);
            if !copy_text.is_empty() {
                copy_segments.push(RenderedCopySegment {
                    display_column,
                    text: copy_text,
                    separator_before: if column == 0 {
                        String::new()
                    } else {
                        "\t".to_string()
                    },
                });
            }
            spans.append(&mut content);
            display_column = display_column.saturating_add(content_width);
            if right > 0 {
                spans.push(Span::raw(" ".repeat(right)));
            }
            display_column = display_column.saturating_add(right).saturating_add(2);
            spans.push(Span::raw(" "));
            spans.push(Span::styled("│", border_style));
        }
        let line = Line::from(spans);
        if UnicodeWidthStr::width(line.to_string().as_str()) <= usize::from(width) {
            out.push(
                RenderedLine::rich(line)
                    .with_copy_projection(
                        copy_cells.join("\t"),
                        UnicodeWidthStr::width(prefix).saturating_add(1),
                    )
                    .with_copy_segments(copy_segments),
            );
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
        let navigation_unit = out.len();
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
        let navigation_copy_text = row
            .cells
            .iter()
            .map(|cell| inline_plain_text(cell))
            .collect::<Vec<_>>()
            .join("\t");
        for line in &mut out[navigation_unit..] {
            line.navigation_unit = Some(navigation_unit);
            line.navigation_copy_text.clone_from(&navigation_copy_text);
        }
    }
}

pub(crate) fn render_image_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    alt: &str,
    url: &str,
    title: &str,
    dimensions: MarkdownImageDimensions,
    link_url: Option<&str>,
    width: u16,
) {
    let caption = markdown_image_caption(alt, title, url);
    if layout_config().native_graphics
        && let Ok(artifact) = render_markdown_image(url)
    {
        let prefix_width = u16::try_from(UnicodeWidthStr::width(prefix)).unwrap_or(u16::MAX);
        let available = width.saturating_sub(prefix_width).max(1);
        let size = fit_image_size(
            artifact.image.width(),
            artifact.image.height(),
            dimensions,
            available,
            24,
        );
        let (render_width, render_height) = (size.width, size.height);
        let column = prefix_width + available.saturating_sub(render_width) / 2;
        let start = out.len();
        for _ in 0..render_height {
            out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
        }
        out[start].math.push(MathLinePlacement {
            column,
            artifact,
            size,
        });
        push_single_line(
            out,
            prefix,
            &format!("🖼  {caption}"),
            Style::default().fg(agena_tui_components::theme::muted_color()),
            width,
        );
        if let Some(link_url) = link_url {
            push_image_source_line(out, prefix, "↗", link_url, width);
        }
        return;
    }

    // Remote images use an asynchronous bounded cache. Until a download
    // completes—or when no native image protocol exists—retain an accessible
    // source preview without blocking terminal input.
    push_wrapped_rich_line(
        out,
        prefix,
        prefix,
        Line::from(vec![
            Span::styled(
                "🖼  ",
                Style::default().fg(agena_tui_components::theme::accent_color()),
            ),
            Span::styled(caption, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        width,
    );
    push_image_source_line(out, prefix, "↳", markdown_image_source_label(url), width);
    if let Some(link_url) = link_url {
        push_image_source_line(out, prefix, "↗", link_url, width);
    }
}

fn push_image_source_line(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    marker: &str,
    target: &str,
    width: u16,
) {
    let source_prefix = format!("{prefix}   ");
    push_wrapped_rich_line(
        out,
        &source_prefix,
        &source_prefix,
        Line::from(Span::styled(
            format!("{marker} {target}"),
            Style::default()
                .fg(agena_tui_components::theme::info_color())
                .add_modifier(Modifier::UNDERLINED),
        )),
        width,
    );
}

/// Render an image attachment through the same bounded, workspace-confined
/// graphics pipeline used by Markdown images. Returning `false` means the
/// attachment is not an image or has no renderable source and should retain
/// the caller's ordinary attachment presentation.
pub fn render_attachment_image(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    item: &MessageAttachment,
    width: u16,
) -> bool {
    if item.kind != MessageAttachmentKind::Image {
        return false;
    }
    let Some(source) = attachment_image_source(item) else {
        return false;
    };
    let alt = item
        .filename
        .as_deref()
        .or(item.title.as_deref())
        .unwrap_or("Image");
    render_image_block(
        out,
        prefix,
        alt,
        source.as_ref(),
        item.title.as_deref().unwrap_or_default(),
        MarkdownImageDimensions::default(),
        None,
        width,
    );
    true
}

pub(super) fn attachment_image_source(item: &MessageAttachment) -> Option<Cow<'_, str>> {
    match &item.source {
        MessageAttachmentSource::Url { url }
        | MessageAttachmentSource::DataUrl { url }
        | MessageAttachmentSource::LocalPath { path: url } => Some(Cow::Borrowed(url.as_str())),
        MessageAttachmentSource::Base64 { data } => bounded_image_data_url(&item.mime, data)
            .ok()
            .map(Cow::Owned),
        MessageAttachmentSource::FileId { .. } => None,
    }
}

pub(super) fn markdown_image_caption(alt: &str, title: &str, url: &str) -> String {
    let alt = alt.trim();
    let title = title.trim();
    let label = if alt.is_empty() {
        markdown_image_filename(url).unwrap_or("Image")
    } else {
        alt
    };
    if title.is_empty() || title == label {
        label.to_string()
    } else {
        format!("{label} — {title}")
    }
}

fn markdown_image_filename(source: &str) -> Option<&str> {
    let source = source.split(['?', '#']).next().unwrap_or(source);
    source
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && !name.contains(':'))
}

pub(super) fn markdown_image_source_label(source: &str) -> &str {
    if source.trim_start().starts_with("data:") {
        "embedded image"
    } else {
        source
    }
}

pub(super) fn fit_image_size(
    image_width: u32,
    image_height: u32,
    dimensions: MarkdownImageDimensions,
    max_width: u16,
    max_height: u16,
) -> Size {
    let config = layout_config();
    let natural_width = u64::from(image_width.max(1));
    let natural_height = u64::from(image_height.max(1));
    let (mut pixel_width, mut pixel_height) = match (
        dimensions.width_px.map(u64::from),
        dimensions.height_px.map(u64::from),
    ) {
        // HTML can request both dimensions, but native terminal protocols do
        // not all interpret a conflicting rectangle identically. Treat it as
        // a bounding box and retain the source aspect ratio on every backend.
        (Some(width), Some(height)) => {
            fit_pixels_to_box(natural_width, natural_height, width.max(1), height.max(1))
        }
        (Some(width), None) => (
            width.max(1),
            natural_height
                .saturating_mul(width.max(1))
                .div_ceil(natural_width)
                .max(1),
        ),
        (None, Some(height)) => (
            natural_width
                .saturating_mul(height.max(1))
                .div_ceil(natural_height)
                .max(1),
            height.max(1),
        ),
        (None, None) => (natural_width, natural_height),
    };

    // Fit in pixel space before rounding to terminal cells. Scaling the
    // already-rounded cell rectangle compounds the independent horizontal and
    // vertical rounding errors and can visibly elongate an image, especially
    // for small images or narrow viewports.
    let cell_width = u64::from(config.cell_width.max(1));
    let cell_height = u64::from(config.cell_height.max(1));
    let max_pixel_width = u64::from(max_width.max(1)).saturating_mul(cell_width);
    let max_pixel_height = u64::from(max_height.max(1)).saturating_mul(cell_height);
    (pixel_width, pixel_height) =
        fit_pixels_to_box(pixel_width, pixel_height, max_pixel_width, max_pixel_height);

    Size::new(
        pixel_width
            .div_ceil(cell_width)
            .clamp(1, u64::from(max_width.max(1))) as u16,
        pixel_height
            .div_ceil(cell_height)
            .clamp(1, u64::from(max_height.max(1))) as u16,
    )
}

fn fit_pixels_to_box(
    mut width: u64,
    mut height: u64,
    max_width: u64,
    max_height: u64,
) -> (u64, u64) {
    width = width.max(1);
    height = height.max(1);
    let max_width = max_width.max(1);
    let max_height = max_height.max(1);
    if width > max_width {
        height = height.saturating_mul(max_width).div_ceil(width).max(1);
        width = max_width;
    }
    if height > max_height {
        width = width.saturating_mul(max_height).div_ceil(height).max(1);
        height = max_height;
    }
    (width, height)
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
