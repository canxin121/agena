use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};

use agena_api::resource::{PartAttachment, PartAttachmentKind, PartAttachmentSource};
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
mod rich_output;

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
            } else if let Some(image) = standalone_inline_image(&content) {
                Some(image)
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
            } else if code.closed && is_diagram_language(&language) {
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

/// An image occupying its own source line keeps block sizing even when
/// CommonMark groups it with adjacent lines into a single paragraph.
fn standalone_inline_image(inlines: &[MarkdownInline]) -> Option<MarkdownNode> {
    let (image, link_url) = match inlines {
        [image @ MarkdownInline::Image { .. }] => (image, None),
        [MarkdownInline::Link { url, label, .. }] => {
            let [image @ MarkdownInline::Image { .. }] = label.as_slice() else {
                return None;
            };
            (image, Some(url.clone()))
        }
        _ => return None,
    };
    let MarkdownInline::Image {
        url,
        title,
        alt,
        dimensions,
    } = image
    else {
        return None;
    };
    Some(MarkdownNode::Image {
        url: url.clone(),
        title: title.clone(),
        alt: alt.clone(),
        dimensions: *dimensions,
        link_url,
    })
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
