//! Immutable reply references. A prepared result is data, not a rendered tool card.
use super::InlineDocument;
use comrak::{Arena, Options, nodes::NodeValue, parse_document};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range};

pub type InlineUiReferences = BTreeMap<String, InlineDocument>;
pub const INLINE_UI_MAX_REFERENCES: usize = 32;
pub const INLINE_UI_REFERENCE_PREFIX: &str = "[[inline-ui:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreparedInlineUi {
    pub reference: String,
    pub document: InlineDocument,
}

impl PreparedInlineUi {
    pub fn validate(&self) -> Result<(), String> {
        inline_reference_tool_id(&self.reference).ok_or("invalid inline UI reference")?;
        self.document.validate()
    }
    pub fn marker(&self) -> String {
        format!("[[inline-ui:{}]]", self.reference)
    }
}

pub fn is_inline_ui_prepare_tool(name: &str) -> bool {
    matches!(name, "agena.inline_ui.prepare" | "inline_ui.prepare")
}

/// The random suffix is checked against the stored result. The numeric prefix
/// locates a tool through session membership, never through a global registry.
pub fn inline_reference_tool_id(reference: &str) -> Option<i64> {
    let (id, nonce) = reference.strip_prefix("ui_")?.split_once('_')?;
    if id.starts_with('0')
        || id.len() > 19
        || !id.bytes().all(|c| c.is_ascii_digit())
        || nonce.len() != 32
        || !nonce
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return None;
    }
    id.parse::<i64>().ok().filter(|id| *id > 0)
}

/// Bounded lexical discovery for authorized sidecar loading. This does not decide
/// where to render: Markdown AST boundaries are checked by each renderer.
pub fn inline_reference_candidates(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    for (start, _) in text.match_indices(INLINE_UI_REFERENCE_PREFIX) {
        let tail = &text[start + INLINE_UI_REFERENCE_PREFIX.len()..];
        let mut limit = tail.len().min(96);
        while !tail.is_char_boundary(limit) {
            limit -= 1;
        }
        let tail = &tail[..limit];
        let Some(end) = tail.find("]]").filter(|end| *end <= 80) else {
            continue;
        };
        let reference = &tail[..end];
        if inline_reference_tool_id(reference).is_some() && !refs.iter().any(|r| r == reference) {
            refs.push(reference.to_owned());
            if refs.len() == INLINE_UI_MAX_REFERENCES {
                break;
            }
        }
    }
    refs
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineReferenceSpan {
    pub range: Range<usize>,
    /// None is a trailing unfinished marker while the answer is streaming.
    pub reference: Option<String>,
}

/// Only a complete, standalone top-level paragraph is an embed. Quoted/listed
/// examples, inline code, fences (including open fences), HTML and escapes stay text.
pub fn inline_reference_spans(text: &str, streaming: bool) -> Vec<InlineReferenceSpan> {
    if !text.contains(INLINE_UI_REFERENCE_PREFIX) {
        return Vec::new();
    }
    let arena = Arena::new();
    let root = parse_document(&arena, text, &Options::default());
    let mut offsets = vec![0];
    for (i, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(i + 1);
        }
    }
    let mut spans = Vec::new();
    for node in root.children() {
        let data = node.data.borrow();
        if !matches!(data.value, NodeValue::Paragraph)
            || data.sourcepos.start.line != data.sourcepos.end.line
        {
            continue;
        }
        let line = data.sourcepos.start.line.saturating_sub(1);
        let Some(&start) = offsets.get(line) else {
            continue;
        };
        let end = offsets.get(line + 1).copied().unwrap_or(text.len());
        let source = &text[start..end];
        let trimmed = source.trim();
        // Restrict the marker to literal unindented/at-most-three-space source.
        if source.starts_with('\t') || source.starts_with("    ") {
            continue;
        }
        if let Some(reference) = trimmed
            .strip_prefix(INLINE_UI_REFERENCE_PREFIX)
            .and_then(|s| s.strip_suffix("]]"))
        {
            if inline_reference_tool_id(reference).is_some() {
                spans.push(InlineReferenceSpan {
                    range: start..end,
                    reference: Some(reference.to_owned()),
                });
            }
        } else if streaming
            && end == text.len()
            && !source.ends_with('\n')
            && trimmed.starts_with(INLINE_UI_REFERENCE_PREFIX)
            && trimmed.len() <= 96
            && !trimmed[INLINE_UI_REFERENCE_PREFIX.len()..].contains([' ', '\t', '\n', '<', '>'])
        {
            spans.push(InlineReferenceSpan {
                range: start..end,
                reference: None,
            });
        }
        if spans.len() == INLINE_UI_MAX_REFERENCES {
            break;
        }
    }
    spans
}

/// A readable TUI/copy/export projection. Original bytes and source cursors never
/// change. Missing/revoked references remain explicit, rather than silently vanish.
pub fn expand_inline_references(text: &str, refs: &InlineUiReferences, streaming: bool) -> String {
    let spans = inline_reference_spans(text, streaming);
    if spans.is_empty() {
        return text.to_owned();
    }
    let mut output = String::new();
    let mut previous = 0;
    for span in spans {
        output.push_str(&text[previous..span.range.start]);
        if let Some(reference) = span.reference {
            if let Some(document) = refs.get(&reference) {
                output.push_str(&document.to_markdown());
            } else {
                output.push_str(&format!("> Inline content unavailable ({reference})."));
            }
            output.push('\n');
        }
        previous = span.range.end;
    }
    output.push_str(&text[previous..]);
    output
}
