use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};

use agena_api::resource::{MessageAttachment, MessageAttachmentKind, MessageAttachmentSource};
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

use super::{
    MarkdownBlock, RenderedCopySegment, RenderedLine, TableColumnAlignment, TranscriptNodeKind,
    TranscriptPointerSelection, fit_table_column_widths, push_markdown_code_block,
    push_markdown_rule, push_single_line, push_table_border, push_wrapped_rich_line,
    sanitize_terminal_text, trim_empty_line_edges, wrap_rich_line,
};
use crate::{InlineVerticalLayout, push_math_block};
use agena_tui_media::{
    MathLinePlacement, bounded_image_data_url, layout_config, positional_unicode_text,
    render_markdown_image, render_markdown_svg, unicode_formula,
};

mod render;
pub use render::*;
mod parse;
pub use parse::*;

#[derive(Debug, Clone, PartialEq, Eq)]
/// A node of the markdown AST.
pub enum MarkdownNode {
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
        dimensions: MarkdownImageDimensions,
        link_url: Option<String>,
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
/// A list item of the markdown AST.
pub struct MarkdownListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<MarkdownNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A description item of the markdown AST.
pub struct MarkdownDescriptionItem {
    pub term: Vec<MarkdownInline>,
    pub details: Vec<MarkdownNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A table row of the markdown AST.
pub struct MarkdownTableRow {
    pub header: bool,
    pub cells: Vec<Vec<MarkdownInline>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Alignment of a markdown element.
pub enum MarkdownAlignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Kind of a markdown alert.
pub enum MarkdownAlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Inline markdown content.
pub enum MarkdownInline {
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
        dimensions: MarkdownImageDimensions,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Dimensions of a markdown image.
pub struct MarkdownImageDimensions {
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
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
            } else if let [
                MarkdownInline::Image {
                    url,
                    title,
                    alt,
                    dimensions,
                },
            ] = content.as_slice()
            {
                Some(MarkdownNode::Image {
                    url: url.clone(),
                    title: title.clone(),
                    alt: alt.clone(),
                    dimensions: *dimensions,
                    link_url: None,
                })
            } else if let [
                MarkdownInline::Link {
                    url: link_url,
                    label,
                    ..
                },
            ] = content.as_slice()
                && let [
                    MarkdownInline::Image {
                        url,
                        title,
                        alt,
                        dimensions,
                    },
                ] = label.as_slice()
            {
                Some(MarkdownNode::Image {
                    url: url.clone(),
                    title: title.clone(),
                    alt: alt.clone(),
                    dimensions: *dimensions,
                    link_url: Some(link_url.clone()),
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
            if let Some(MarkdownInline::Image {
                url,
                title,
                alt,
                dimensions,
            }) = safe_html_image(&html.literal)
            {
                Some(MarkdownNode::Image {
                    url,
                    title,
                    alt,
                    dimensions,
                    link_url: None,
                })
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
    let tag = html_image_tag(html)?;
    let src = html_attribute(tag, "src")?;
    if src.trim().is_empty() {
        return None;
    }
    Some(MarkdownInline::Image {
        url: src,
        title: html_attribute(tag, "title")
            .or_else(|| html_image_container_caption(html))
            .unwrap_or_default(),
        alt: html_attribute(tag, "alt").unwrap_or_default(),
        dimensions: MarkdownImageDimensions {
            width_px: html_image_dimension(tag, "width"),
            height_px: html_image_dimension(tag, "height"),
        },
    })
}

fn html_image_container_caption(html: &str) -> Option<String> {
    ["figcaption", "p"]
        .into_iter()
        .find_map(|name| html_element_text(html, name))
}

fn html_element_text(html: &str, name: &str) -> Option<String> {
    static TAG: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?s)<[^>]*>").expect("HTML tag stripping regex is valid")
    });

    let lowercase = html.to_ascii_lowercase();
    let opening = format!("<{name}");
    let start = lowercase.find(&opening)?;
    let content_start = lowercase.get(start..)?.find('>')? + start + 1;
    let closing = format!("</{name}>");
    let content_end = lowercase.get(content_start..)?.find(&closing)? + content_start;
    let text = TAG
        .replace_all(html.get(content_start..content_end)?, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then_some(text)
}

fn html_image_tag(html: &str) -> Option<&str> {
    let trimmed = html.trim_start();
    let lowercase = trimmed.to_ascii_lowercase();
    let direct_image = html_starts_with_tag(&lowercase, "img");
    let safe_container = ["div", "figure", "picture", "p", "center"]
        .into_iter()
        .any(|name| {
            html_starts_with_tag(&lowercase, name)
                && lowercase.trim_end().ends_with(&format!("</{name}>"))
        });
    if !direct_image && !safe_container {
        return None;
    }
    if [
        "<!--",
        "<script",
        "<style",
        "<template",
        "<object",
        "<embed",
    ]
    .into_iter()
    .any(|marker| lowercase.contains(marker))
    {
        return None;
    }

    let (start, end) = find_html_image_tag(trimmed, 0)?;
    if find_html_image_tag(trimmed, end).is_some() {
        return None;
    }
    trimmed.get(start..end)
}

fn html_starts_with_tag(lowercase: &str, name: &str) -> bool {
    lowercase
        .strip_prefix('<')
        .and_then(|value| value.strip_prefix(name))
        .and_then(|value| value.as_bytes().first().copied())
        .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
}

fn find_html_image_tag(html: &str, search_from: usize) -> Option<(usize, usize)> {
    let lowercase = html.to_ascii_lowercase();
    let mut search_from = search_from;
    while let Some(relative) = lowercase.get(search_from..)?.find("<img") {
        let start = search_from + relative;
        let next = lowercase.as_bytes().get(start + 4).copied();
        if next.is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')) {
            let mut quote = None;
            for (relative, character) in html[start..].char_indices() {
                match (quote, character) {
                    (None, '\'' | '"') => quote = Some(character),
                    (Some(active), current) if active == current => quote = None,
                    (None, '>') => return Some((start, start + relative + 1)),
                    _ => {}
                }
            }
            return None;
        }
        search_from = start.saturating_add(4);
    }
    None
}

fn html_image_dimension(tag: &str, requested: &str) -> Option<u32> {
    html_attribute(tag, requested)
        .and_then(|value| parse_html_pixel_dimension(&value))
        .or_else(|| {
            html_attribute(tag, "style").and_then(|style| {
                style.split(';').find_map(|declaration| {
                    let (name, value) = declaration.split_once(':')?;
                    name.trim()
                        .eq_ignore_ascii_case(requested)
                        .then(|| parse_html_pixel_dimension(value))
                        .flatten()
                })
            })
        })
}

fn parse_html_pixel_dimension(value: &str) -> Option<u32> {
    const MAX_HTML_IMAGE_DIMENSION_PX: u32 = 8_192;

    let value = value.trim();
    let value = value
        .get(..value.len().saturating_sub(2))
        .filter(|_| {
            value
                .get(value.len().saturating_sub(2)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case("px"))
        })
        .unwrap_or(value)
        .trim();
    value
        .parse::<u32>()
        .ok()
        .filter(|dimension| (1..=MAX_HTML_IMAGE_DIMENSION_PX).contains(dimension))
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
                dimensions: MarkdownImageDimensions::default(),
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
            dimensions: MarkdownImageDimensions::default(),
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

pub(crate) fn inline_plain_text(inlines: &[MarkdownInline]) -> String {
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

#[cfg(test)]
mod tests {
    use agena_api::resource::{MessageAttachment, MessageAttachmentKind, MessageAttachmentSource};
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

    use super::*;

    fn seed_remote_png(source: &str, width: u32, height: u32) {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([20, 40, 60, 255]),
        ));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("fixture PNG should encode");
        agena_tui_media::test_support::seed_remote_image(source, bytes);
    }

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
            MarkdownNode::Image { url, alt, title, .. }
                if url == "icon.svg" && alt == "Logo" && title == "Diagram"
        ));

        let centered = parse_markdown_document(concat!(
            "<div align=\"center\">\n",
            "  <img src=\"https://example.com/diagram.png\" alt=\"Centered\" width=\"400\">\n",
            "  <p>Figure 1</p>\n",
            "</div>",
        ));
        assert!(matches!(
            &centered[0].parsed,
            MarkdownNode::Image {
                url,
                alt,
                title,
                dimensions: MarkdownImageDimensions {
                    width_px: Some(400),
                    height_px: None,
                },
                ..
            } if url == "https://example.com/diagram.png"
                && alt == "Centered"
                && title == "Figure 1"
        ));

        let styled = safe_html_image(
            r#"<img src="icon.svg" style="width: 320px; height: 180PX" alt="Styled">"#,
        )
        .expect("safe HTML image");
        assert!(matches!(
            styled,
            MarkdownInline::Image {
                dimensions: MarkdownImageDimensions {
                    width_px: Some(320),
                    height_px: Some(180),
                },
                ..
            }
        ));

        assert!(
            safe_html_image("<!-- <img src=\"https://example.com/tracker.png\"> -->").is_none()
        );
        assert!(
            safe_html_image(concat!(
                "<div>",
                "<img src=\"first.png\">",
                "<img src=\"second.png\">",
                "</div>",
            ))
            .is_none()
        );
    }

    #[test]
    fn clickable_images_keep_the_graphic_and_destination() {
        let blocks = parse_markdown_document(
            "[![Visit](https://example.com/image.png)](https://example.com)",
        );
        assert!(matches!(
            &blocks[0].parsed,
            MarkdownNode::Image {
                url,
                alt,
                link_url: Some(link_url),
                ..
            } if url == "https://example.com/image.png"
                && alt == "Visit"
                && link_url == "https://example.com"
        ));

        let mut rendered = Vec::new();
        render_parsed_markdown_block(&mut rendered, "", &blocks[0], 100);
        let text = rendered
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("↳ https://example.com/image.png"));
        assert!(text.contains("↗ https://example.com"));
    }

    #[test]
    fn cached_remote_images_create_native_terminal_placements() {
        let source = "https://images.example.test/native-placement.png";
        let bytes = BASE64_STANDARD
            .decode(concat!(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk",
                "+A8AAQUBAScY42YAAAAASUVORK5CYII="
            ))
            .expect("test PNG");
        agena_tui_media::test_support::seed_remote_image(source, bytes);
        let context = agena_tui_media::test_support::test_math_render_context(
            agena_tui_media::MathLayoutConfig {
                native_graphics: true,
                ..agena_tui_media::MathLayoutConfig::default()
            },
        );
        let blocks = parse_markdown_document(&format!("![Remote image]({source})"));
        let mut rendered = Vec::new();
        agena_tui_media::with_math_render_context(&context, || {
            render_parsed_markdown_block(&mut rendered, "", &blocks[0], 80);
        });

        let placements = rendered
            .iter()
            .flat_map(|line| line.math.iter())
            .collect::<Vec<_>>();
        assert_eq!(placements.len(), 1);
        assert_eq!(
            (
                placements[0].artifact.image.width(),
                placements[0].artifact.image.height(),
            ),
            (1, 1)
        );
        assert!(rendered.iter().all(|line| !line.text.contains("↳")));
    }

    #[test]
    fn image_pixels_and_html_dimensions_are_fitted_before_cell_rounding() {
        assert_eq!(
            fit_image_size(
                600,
                200,
                MarkdownImageDimensions {
                    width_px: Some(400),
                    height_px: None,
                },
                80,
                24,
            ),
            Size::new(40, 7)
        );
        assert_eq!(
            fit_image_size(
                600,
                200,
                MarkdownImageDimensions {
                    width_px: Some(400),
                    height_px: Some(200),
                },
                80,
                24,
            ),
            Size::new(40, 7),
            "conflicting HTML dimensions are a bounding box, not permission to distort the image"
        );
        assert_eq!(
            fit_image_size(21, 21, MarkdownImageDimensions::default(), 2, 24),
            Size::new(2, 1),
            "pixel-space fitting must not turn a nearly square image into a two-row rectangle"
        );
    }

    #[test]
    fn unavailable_images_render_as_compact_accessible_link_previews() {
        let blocks = parse_markdown_document(concat!(
            "![替代文本](https://example.com/image.png \"悬停标题\")\n\n",
            "![带引用式链接的图片][logo]\n\n",
            "[logo]: https://example.com/logo.png \"Placeholder\"",
        ));
        assert_eq!(blocks.len(), 2);

        let mut rendered = Vec::new();
        for block in &blocks {
            render_parsed_markdown_block(&mut rendered, "", block, 100);
        }
        let text = rendered
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            rendered.len(),
            4,
            "each preview should use two lines:\n{text}"
        );
        assert!(text.contains("🖼  替代文本 — 悬停标题"));
        assert!(text.contains("↳ https://example.com/image.png"));
        assert!(text.contains("🖼  带引用式链接的图片 — Placeholder"));
        assert!(text.contains("↳ https://example.com/logo.png"));
        assert!(
            !text.contains(['╭', '╰', '│']),
            "an unloaded image must not masquerade as a rendered card:\n{text}"
        );

        assert_eq!(
            markdown_image_source_label("data:image/png;base64,AAAA"),
            "embedded image"
        );
        assert_eq!(
            markdown_image_caption("", "", "./assets/diagram.png?raw=1"),
            "diagram.png"
        );
    }

    #[test]
    fn base64_image_attachments_enter_the_bounded_image_pipeline() {
        let item = MessageAttachment {
            kind: MessageAttachmentKind::Image,
            mime: "image/png".to_owned(),
            source: MessageAttachmentSource::Base64 {
                data: concat!(
                    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk",
                    "+A8AAQUBAScY42YAAAAASUVORK5CYII="
                )
                .to_owned(),
            },
            filename: Some("pixel.png".to_owned()),
            title: None,
            size_bytes: None,
            sha256: None,
            width: Some(1),
            height: Some(1),
            duration_ms: None,
            page_count: None,
        };
        let source = attachment_image_source(&item).expect("image source");
        let artifact = render_markdown_image(source.as_ref()).expect("attachment image");
        assert_eq!((artifact.image.width(), artifact.image.height()), (1, 1));
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
    fn rich_native_inline_math_centers_formula_and_styled_text_on_one_row() {
        let config = agena_tui_media::MathLayoutConfig {
            native_graphics: true,
            cell_width: 10,
            cell_height: 20,
            ..agena_tui_media::MathLayoutConfig::default()
        };
        let context = agena_tui_media::test_support::test_math_render_context(config);
        let blocks =
            parse_markdown_document(r"**before** \(\begin{bmatrix}a\\b\\c\end{bmatrix}\) after");
        let mut rendered = Vec::new();
        agena_tui_media::with_math_render_context(&context, || {
            render_parsed_markdown_block(&mut rendered, "", &blocks[0], 120);
        });

        let text_row = rendered
            .iter()
            .position(|line| line.text.contains("before"))
            .expect("styled text should occupy the shared anchor row");
        let (top_row, placement) = rendered
            .iter()
            .enumerate()
            .find_map(|(row, line)| line.math.first().map(|placement| (row, placement)))
            .expect("formula should use a native graphics placement");
        assert!(placement.size.height > 1);
        assert_eq!(placement.size.height % 2, 1);
        assert_eq!(top_row + usize::from(placement.size.height / 2), text_row);
        assert!(rendered[text_row].rich_line.as_ref().is_some_and(|line| {
            line.spans
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
        }));
    }

    #[test]
    fn native_inline_formulas_do_not_duplicate_list_markers_across_graphic_rows() {
        let config = agena_tui_media::MathLayoutConfig {
            native_graphics: true,
            cell_width: 10,
            cell_height: 20,
            ..agena_tui_media::MathLayoutConfig::default()
        };
        let context = agena_tui_media::test_support::test_math_render_context(config);
        let blocks = parse_markdown_document(concat!(
            "- $\\text{H}_2\\text{O}$ → H₂O\n",
            "- $\\text{CH}_3\\text{COOH}$ → CH₃COOH\n",
            "- $\\text{C}_6\\text{H}_{12}\\text{O}_6$ → C₆H₁₂O₆\n",
            "- $\\text{NaCl} \\rightleftharpoons \\text{Na}^+ + \\text{Cl}^-$ → NaCl ⇌ Na⁺ + Cl⁻",
        ));
        let mut rendered = Vec::new();
        agena_tui_media::with_math_render_context(&context, || {
            render_parsed_markdown_block(&mut rendered, "  ", &blocks[0], 120);
        });

        let placements = rendered
            .iter()
            .enumerate()
            .flat_map(|(row, line)| line.math.iter().map(move |placement| (row, placement)))
            .collect::<Vec<_>>();
        assert_eq!(placements.len(), 4);
        assert!(
            placements
                .iter()
                .any(|(_, placement)| placement.size.height > 1),
            "fixture must include a formula taller than one terminal row"
        );
        assert_eq!(
            rendered
                .iter()
                .map(|line| line.text.matches('•').count())
                .sum::<usize>(),
            4,
            "each Markdown list item must emit exactly one bullet"
        );

        for (top, placement) in placements {
            let anchor = top + usize::from(placement.size.height / 2);
            assert_eq!(
                rendered[anchor].text.matches('•').count(),
                1,
                "the bullet must share the formula's text anchor row"
            );
            for (row, line) in rendered
                .iter()
                .enumerate()
                .skip(top)
                .take(usize::from(placement.size.height))
            {
                if row != anchor {
                    assert!(
                        !line.text.contains('•'),
                        "formula padding rows must use the continuation indent"
                    );
                }
            }
        }
    }

    #[test]
    fn unicode_inline_formulas_emit_one_list_marker_for_a_multiline_canvas() {
        let blocks =
            parse_markdown_document("- before $\\begin{bmatrix}a\\\\b\\end{bmatrix}$ after");
        let mut rendered = Vec::new();
        render_parsed_markdown_block(&mut rendered, "  ", &blocks[0], 80);

        assert!(rendered.len() > 1, "fixture must use a multiline formula");
        assert_eq!(
            rendered
                .iter()
                .map(|line| line.text.matches('•').count())
                .sum::<usize>(),
            1
        );
        let marker_row = rendered
            .iter()
            .find(|line| line.text.contains('•'))
            .expect("the list marker must remain visible");
        assert!(marker_row.text.contains("before"));
        assert!(marker_row.text.contains("after"));
    }

    #[test]
    fn wrapped_list_paragraph_uses_indentation_after_its_single_marker() {
        let blocks = parse_markdown_document(
            "- This deliberately long list item wraps onto several terminal rows without repeating its marker.",
        );
        let mut rendered = Vec::new();
        render_parsed_markdown_block(&mut rendered, "  ", &blocks[0], 24);

        assert!(rendered.len() > 1, "fixture must wrap");
        assert_eq!(
            rendered
                .iter()
                .map(|line| line.text.matches('•').count())
                .sum::<usize>(),
            1
        );
        assert!(
            rendered
                .iter()
                .skip(1)
                .all(|line| line.text.starts_with("    ") && !line.text.contains('•'))
        );
    }

    #[test]
    fn native_inline_formula_does_not_duplicate_a_heading_marker() {
        let config = agena_tui_media::MathLayoutConfig {
            native_graphics: true,
            cell_width: 10,
            cell_height: 20,
            ..agena_tui_media::MathLayoutConfig::default()
        };
        let context = agena_tui_media::test_support::test_math_render_context(config);
        let blocks = parse_markdown_document("# before $\\frac{a+b}{c+d}$ after");
        let mut rendered = Vec::new();
        agena_tui_media::with_math_render_context(&context, || {
            render_parsed_markdown_block(&mut rendered, "  ", &blocks[0], 80);
        });

        let (top, placement) = rendered
            .iter()
            .enumerate()
            .find_map(|(row, line)| line.math.first().map(|placement| (row, placement)))
            .expect("formula should use a native placement");
        let anchor = top + usize::from(placement.size.height / 2);
        assert_eq!(
            rendered
                .iter()
                .filter(|line| line.text.contains("══"))
                .count(),
            1
        );
        assert!(rendered[anchor].text.contains("══"));
    }

    #[test]
    fn native_inline_image_does_not_duplicate_its_list_marker() {
        let source = "https://images.example.test/tall-inline-list-image.png";
        seed_remote_png(source, 20, 60);

        let config = agena_tui_media::MathLayoutConfig {
            native_graphics: true,
            cell_width: 10,
            cell_height: 20,
            ..agena_tui_media::MathLayoutConfig::default()
        };
        let context = agena_tui_media::test_support::test_math_render_context(config);
        let blocks = parse_markdown_document(&format!("- before ![tall]({source}) after"));
        let mut rendered = Vec::new();
        agena_tui_media::with_math_render_context(&context, || {
            render_parsed_markdown_block(&mut rendered, "  ", &blocks[0], 80);
        });

        let (top, placement) = rendered
            .iter()
            .enumerate()
            .find_map(|(row, line)| line.math.first().map(|placement| (row, placement)))
            .expect("image should use a native placement");
        assert_eq!(placement.size.height, 3);
        let anchor = top + usize::from(placement.size.height / 2);
        assert_eq!(
            rendered
                .iter()
                .map(|line| line.text.matches('•').count())
                .sum::<usize>(),
            1
        );
        assert!(rendered[anchor].text.contains('•'));
        assert!(
            rendered
                .iter()
                .enumerate()
                .filter(|(row, _)| *row != anchor)
                .all(|(_, line)| !line.text.contains('•'))
        );
    }

    #[test]
    fn standalone_native_image_block_emits_one_centered_list_marker() {
        let source = "https://images.example.test/tall-list-image-block.png";
        seed_remote_png(source, 20, 60);
        let config = agena_tui_media::MathLayoutConfig {
            native_graphics: true,
            cell_width: 10,
            cell_height: 20,
            ..agena_tui_media::MathLayoutConfig::default()
        };
        let context = agena_tui_media::test_support::test_math_render_context(config);
        let blocks = parse_markdown_document(&format!("- ![tall]({source})"));
        let mut rendered = Vec::new();
        agena_tui_media::with_math_render_context(&context, || {
            render_parsed_markdown_block(&mut rendered, "  ", &blocks[0], 80);
        });

        let (top, placement) = rendered
            .iter()
            .enumerate()
            .find_map(|(row, line)| line.math.first().map(|placement| (row, placement)))
            .expect("image block should use a native placement");
        let anchor = top + usize::from(placement.size.height / 2);
        assert_eq!(
            rendered
                .iter()
                .map(|line| line.text.matches('•').count())
                .sum::<usize>(),
            1
        );
        assert!(rendered[anchor].text.contains('•'));
        assert!(
            rendered
                .iter()
                .enumerate()
                .filter(|(row, _)| *row != anchor)
                .all(|(_, line)| !line.text.contains('•'))
        );
    }

    #[test]
    fn standalone_display_math_block_emits_one_centered_list_marker() {
        let config = agena_tui_media::MathLayoutConfig {
            native_graphics: true,
            cell_width: 10,
            cell_height: 20,
            ..agena_tui_media::MathLayoutConfig::default()
        };
        let context = agena_tui_media::test_support::test_math_render_context(config);
        let blocks = parse_markdown_document(
            "- $$\n  \\begin{bmatrix}\n  a & b \\\\\n  c & d\n  \\end{bmatrix}\n  $$",
        );
        let mut rendered = Vec::new();
        agena_tui_media::with_math_render_context(&context, || {
            render_parsed_markdown_block(&mut rendered, "  ", &blocks[0], 80);
        });

        let (top, placement) = rendered
            .iter()
            .enumerate()
            .find_map(|(row, line)| line.math.first().map(|placement| (row, placement)))
            .expect("display math block should use a native placement");
        let anchor = top + usize::from(placement.size.height / 2);
        assert_eq!(
            rendered
                .iter()
                .map(|line| line.text.matches('•').count())
                .sum::<usize>(),
            1
        );
        assert!(rendered[anchor].text.contains('•'));
    }

    #[test]
    fn fenced_code_block_at_list_start_emits_only_one_marker() {
        let blocks = parse_markdown_document("- ```text\n  alpha\n  beta\n  ```");
        let mut rendered = Vec::new();
        render_parsed_markdown_block(&mut rendered, "  ", &blocks[0], 80);

        assert!(rendered.len() > 2, "fixture must render a multi-row card");
        assert_eq!(
            rendered
                .iter()
                .map(|line| line.text.matches('•').count())
                .sum::<usize>(),
            1
        );
        assert!(rendered[0].text.contains('•'));
        assert!(rendered.iter().skip(1).all(|line| !line.text.contains('•')));
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
