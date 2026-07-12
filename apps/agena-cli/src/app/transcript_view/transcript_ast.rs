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

use super::transcript_math::{push_inline_math, push_math_block};
use super::{
    MarkdownBlock, RenderedLine, TranscriptNodeKind, push_markdown_code_block,
    push_markdown_heading, push_markdown_rule, push_markdown_table, push_single_line,
    push_wrapped_rich_line, sanitize_terminal_text, trim_empty_line_edges,
};
use crate::math_render::{
    MathLinePlacement, layout_config, render_markdown_image, unicode_formula,
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

pub(in crate::app) fn parse_markdown_document(text: &str) -> Vec<MarkdownBlock> {
    let sanitized = sanitize_terminal_text(text);
    let markdown = trim_empty_line_edges(sanitized.as_str());
    if markdown.is_empty() {
        return Vec::new();
    }

    let arena = Arena::new();
    let options = markdown_options();
    let root = parse_document(&arena, markdown.as_str(), &options);
    let source_lines = markdown.lines().collect::<Vec<_>>();
    let mut previous_end_line = 0_usize;
    root.children()
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
                MarkdownNode::Code { literal, .. } => literal.trim_end_matches('\n').to_string(),
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
        .collect()
}

fn markdown_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.superscript = true;
    options.extension.footnotes = true;
    options.extension.description_lists = true;
    options.extension.front_matter_delimiter = Some("---".to_string());
    options.extension.multiline_block_quotes = true;
    options.extension.alerts = true;
    options.extension.math_dollars = true;
    options.extension.math_code = true;
    options.extension.shortcodes = true;
    options.extension.wikilinks_title_after_pipe = true;
    options.extension.underline = true;
    options.extension.subscript = true;
    options.extension.spoiler = true;
    options.extension.cjk_friendly_emphasis = true;
    options.extension.subtext = true;
    options.extension.highlight = true;
    options.extension.insert = true;
    options.extension.block_directive = true;
    options.parse.smart = true;
    options.parse.relaxed_tasklist_matching = true;
    options.parse.tasklist_in_table = true;
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
            let language = code
                .info
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(language.as_str(), "math" | "tex" | "latex" | "katex") {
                Some(MarkdownNode::Math {
                    literal: code.literal.trim_end_matches('\n').to_string(),
                    display: true,
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
            literal: math.literal,
            display: math.display_math,
        }),
        NodeValue::FootnoteDefinition(footnote) => Some(MarkdownNode::FootnoteDefinition {
            name: footnote.name,
            blocks: convert_blocks(node),
        }),
        NodeValue::FrontMatter(front_matter) => Some(MarkdownNode::FrontMatter(front_matter)),
        NodeValue::HtmlBlock(html) => Some(MarkdownNode::Html(html.literal)),
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
    node.children().filter_map(convert_inline).collect()
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
        NodeValue::WikiLink(link) => Some(MarkdownInline::WikiLink {
            url: link.url,
            label: convert_inlines(node),
        }),
        NodeValue::Image(image) => Some(MarkdownInline::Image {
            url: image.url,
            title: image.title,
            alt: inline_plain_text(&convert_inlines(node)),
        }),
        NodeValue::Math(math) => Some(MarkdownInline::Math {
            literal: math.literal,
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
            let source = inline_render_source(content);
            push_markdown_heading(out, prefix, usize::from(*level), &source, width);
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
    if inlines_contain_math(inlines) {
        let source = inline_render_source(inlines);
        if push_inline_math(out, prefix, &source, width) {
            return;
        }
    }
    for line in rich_inline_lines(inlines, Style::default()) {
        push_wrapped_rich_line(out, prefix, prefix, line, width);
    }
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
    let row_text = rows
        .iter()
        .map(|row| {
            format!(
                "| {} |",
                row.cells
                    .iter()
                    .map(|cell| escape_table_cell(&inline_plain_text(cell)))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        })
        .collect::<Vec<_>>();
    let delimiter = format!(
        "| {} |",
        alignments
            .iter()
            .map(|alignment| match alignment {
                MarkdownAlignment::None | MarkdownAlignment::Left => "---",
                MarkdownAlignment::Center => ":---:",
                MarkdownAlignment::Right => "---:",
            })
            .collect::<Vec<_>>()
            .join(" | ")
    );
    let mut source = Vec::with_capacity(row_text.len() + 1);
    source.push(row_text[0].clone());
    source.push(delimiter);
    source.extend(row_text.into_iter().skip(1));
    let borrowed = source.iter().map(String::as_str).collect::<Vec<_>>();
    push_markdown_table(out, prefix, &borrowed, width);
}

fn escape_table_cell(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', " ↵ ")
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
            MarkdownInline::Superscript(children) | MarkdownInline::Subscript(children) => {
                append_inline_spans(rows, children, style.add_modifier(Modifier::DIM))
            }
            MarkdownInline::Spoiler(children) => append_inline_spans(
                rows,
                children,
                style
                    .fg(agena_tui_components::theme::muted_color())
                    .add_modifier(Modifier::REVERSED),
            ),
            MarkdownInline::Link { url, label, .. } | MarkdownInline::WikiLink { url, label } => {
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
                    unicode_formula(literal).join(" "),
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

fn inlines_contain_math(inlines: &[MarkdownInline]) -> bool {
    inlines.iter().any(|inline| match inline {
        MarkdownInline::Math { .. } => true,
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
        } => inlines_contain_math(children),
        _ => false,
    })
}

fn inline_render_source(inlines: &[MarkdownInline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            MarkdownInline::Text(text) | MarkdownInline::Emoji(text) => out.push_str(text),
            MarkdownInline::Code(code) => out.push_str(&format!("`{code}`")),
            MarkdownInline::Emphasis(children) => {
                out.push('*');
                out.push_str(&inline_render_source(children));
                out.push('*');
            }
            MarkdownInline::Strong(children) => {
                out.push_str("**");
                out.push_str(&inline_render_source(children));
                out.push_str("**");
            }
            MarkdownInline::Strikethrough(children) => {
                out.push_str("~~");
                out.push_str(&inline_render_source(children));
                out.push_str("~~");
            }
            MarkdownInline::Underline(children)
            | MarkdownInline::Highlight(children)
            | MarkdownInline::Insert(children)
            | MarkdownInline::Superscript(children)
            | MarkdownInline::Subscript(children)
            | MarkdownInline::Spoiler(children) => out.push_str(&inline_render_source(children)),
            MarkdownInline::Link { url, label, .. } | MarkdownInline::WikiLink { url, label } => {
                out.push_str(&inline_render_source(label));
                out.push_str(&format!(" ({url})"));
            }
            MarkdownInline::Image { url, alt, .. } => {
                out.push_str(&format!("🖼 {alt} ({url})"));
            }
            MarkdownInline::Math { literal, display } => {
                if *display {
                    out.push_str(&format!("$${literal}$$"));
                } else {
                    out.push_str(&format!("${literal}$"));
                }
            }
            MarkdownInline::FootnoteReference(name) => out.push_str(&format!("[^{name}]")),
            MarkdownInline::Html(html) => {
                if html.to_ascii_lowercase().starts_with("<br") {
                    out.push('\n');
                }
            }
            MarkdownInline::SoftBreak => out.push(' '),
            MarkdownInline::HardBreak => out.push('\n'),
        }
    }
    out
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
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block.parsed, MarkdownNode::FootnoteDefinition { .. }))
        );
    }

    #[test]
    fn preserves_code_span_pipes_inside_table_cells() {
        let blocks = parse_markdown_document("| value |\n| --- |\n| `a\\|b` |");
        let MarkdownNode::Table { rows, .. } = &blocks[0].parsed else {
            panic!("table expected");
        };
        assert_eq!(inline_plain_text(&rows[1].cells[0]), "a|b");
    }
}
