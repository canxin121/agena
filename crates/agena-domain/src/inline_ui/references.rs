//! Immutable reply references. A prepared result is data, not a rendered tool card.
use super::InlineDocument;
use comrak::{Arena, Options, nodes::NodeValue, parse_document};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range};

pub type InlineUiReferences = BTreeMap<String, InlineDocument>;
pub const INLINE_UI_MAX_REFERENCES: usize = 32;
pub const INLINE_UI_REFERENCE_PREFIX: &str = "```inline-ui";
const INLINE_UI_FENCE_LANGUAGE: &str = "inline-ui";

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
        format!("```inline-ui\n{}\n```", self.reference)
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
    // A reference can arrive before its closing fence, or span transport pages.
    // Find only bounded opaque IDs here; Markdown boundaries govern display.
    for (start, _) in text.match_indices("ui_") {
        if start > 0 && text.as_bytes()[start - 1].is_ascii_alphanumeric() {
            continue;
        }
        let tail = &text.as_bytes()[start..];
        let length = tail
            .iter()
            .take(64)
            .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
            .count();
        let reference = &text[start..start + length];
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

/// A top-level, explicitly closed `inline-ui` backtick fence containing one
/// opaque reference embeds rich content. Other code/quotes/lists remain literal.
pub fn inline_reference_spans(text: &str, streaming: bool) -> Vec<InlineReferenceSpan> {
    if !text.contains(INLINE_UI_FENCE_LANGUAGE) {
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
        let first_line = data.sourcepos.start.line.saturating_sub(1);
        let Some(&start) = offsets.get(first_line) else {
            continue;
        };
        let end = offsets
            .get(data.sourcepos.end.line)
            .copied()
            .unwrap_or(text.len());
        if let NodeValue::CodeBlock(code) = &data.value {
            if !code.fenced
                || code.fence_char != b'`'
                || code.info.trim() != INLINE_UI_FENCE_LANGUAGE
            {
                continue;
            }
            let reference = code.literal.trim();
            if code.closed && inline_reference_tool_id(reference).is_some() {
                spans.push(InlineReferenceSpan {
                    range: start..end,
                    reference: Some(reference.to_owned()),
                });
            } else if !code.closed && streaming && end == text.len() && code.literal.len() <= 512 {
                // Never flash an unfinished reference as code while its fence
                // is streaming. A malformed completed block stays literal.
                spans.push(InlineReferenceSpan {
                    range: start..end,
                    reference: None,
                });
            }
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
