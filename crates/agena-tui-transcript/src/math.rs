//! Inline and block math rendering integration.

use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::{RenderedLine, TranscriptPointerSelection, push_multiline, push_wrapped_line};
use agena_tui_media::{MathLinePlacement, layout_config, render_formula, unicode_formula};

#[derive(Debug, Clone, PartialEq, Eq)]
/// A semantic row of display math.
pub struct DisplayMathSemanticRow {
    pub source: String,
    /// Relative vertical weight, not a terminal-row count. Native rendering
    /// replaces the Unicode estimate with the real outer-array row metrics.
    pub layout_weight: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A segment of inline math.
pub enum InlineMathSegment {
    Text(String),
    Math(String),
}

/// Cell-row geometry shared by plain and rich native inline graphics. Every
/// graphic contributes its middle cell as an anchor. Formula rasters are
/// normalized to an odd cell height, so that cell contains their exact visual
/// center and can share a row with the surrounding text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineVerticalLayout {
    height: u16,
    anchor_row: u16,
}

impl InlineVerticalLayout {
    pub const fn new(height: u16) -> Self {
        let height = if height == 0 { 1 } else { height };
        Self {
            height,
            anchor_row: height / 2,
        }
    }

    pub const fn height(self) -> u16 {
        self.height
    }

    pub fn text_row(self) -> usize {
        usize::from(self.anchor_row)
    }

    pub fn graphic_top_row(self, graphic_height: u16) -> usize {
        let graphic_height = graphic_height.max(1).min(self.height);
        usize::from(self.anchor_row.saturating_sub(graphic_height / 2))
    }
}

pub fn fenced_math_language(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(marker) = trimmed.chars().next() else {
        return false;
    };
    if marker != '`' && marker != '~' {
        return false;
    }
    let fence_len = trimmed.chars().take_while(|ch| *ch == marker).count();
    if fence_len < 3 {
        return false;
    }
    let language = trimmed[fence_len..]
        .trim()
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '{' | '}'))
        .next()
        .unwrap_or_default()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    matches!(language.as_str(), "math" | "tex" | "latex" | "katex")
}

pub fn display_math_source(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lines = trimmed.lines().collect::<Vec<_>>();
    if lines.len() >= 2 && fenced_math_language(lines[0]) {
        let body_end = if lines.last().is_some_and(|line| {
            let line = line.trim_start();
            line.starts_with("```") || line.starts_with("~~~")
        }) {
            lines.len().saturating_sub(1)
        } else {
            lines.len()
        };
        return Some(lines[1..body_end].join("\n"));
    }
    if let Some(body) = trimmed
        .strip_prefix("$$")
        .and_then(|text| text.strip_suffix("$$"))
    {
        return Some(body.trim().to_string());
    }
    if let Some(body) = trimmed
        .strip_prefix(r"\[")
        .and_then(|text| text.strip_suffix(r"\]"))
    {
        return Some(body.trim().to_string());
    }
    None
}

/// Split only structurally multi-row display environments. Physical source
/// newlines are deliberately ignored, as are row separators nested inside
/// braces or child environments such as matrices and `substack`.
pub fn display_math_semantic_rows(formula: &str) -> Option<Vec<DisplayMathSemanticRow>> {
    let formula = formula.trim();
    let sources = display_math_semantic_row_sources(formula)?;
    if sources.len() < 2 {
        return None;
    }
    // Rendering support is deliberately optional here. The tolerant source
    // scanner owns navigation structure; the semantic renderer only enriches
    // it with more accurate vertical weights when it understands every
    // command in the formula. Unknown extensions and user macros must not
    // collapse otherwise unambiguous top-level equation rows.
    let heights = agena_tui_media::semantic_math_row_heights(formula, true)
        .filter(|heights| heights.len() == sources.len());
    Some(
        sources
            .into_iter()
            .enumerate()
            .map(|(index, source)| DisplayMathSemanticRow {
                source,
                layout_weight: heights
                    .as_ref()
                    .and_then(|heights| heights.get(index))
                    .copied()
                    .unwrap_or(1)
                    .max(1),
            })
            .collect(),
    )
}

fn display_math_semantic_row_sources(formula: &str) -> Option<Vec<String>> {
    let mut candidate = formula.trim();
    // Transparent equation wrappers and outer brace groups are common around
    // `split`/`aligned`. Iteration keeps this tolerant scanner bounded without
    // tying it to whether any command inside the rows can be rendered.
    for _ in 0..32 {
        let content_start = skip_math_ignorable(candidate, 0)?;
        if content_start > 0 {
            candidate = candidate.get(content_start..)?.trim();
            continue;
        }
        if let Some(annotation_end) = math_outer_annotation_end(candidate, 0) {
            candidate = candidate.get(annotation_end..)?.trim();
            continue;
        }
        if candidate.starts_with('{') {
            let group_end = skip_math_environment_argument(candidate, 0)?;
            if group_end == candidate.len() {
                candidate = candidate.get(1..group_end.saturating_sub(1))?.trim();
                continue;
            }
        }
        if let Some(stripped) = strip_math_layout_style(candidate) {
            candidate = stripped.trim();
            continue;
        }

        let (environment, mut body_start) = math_environment_command(candidate, 0, "begin")?;
        if let Some(spec) = navigable_math_environment(environment.as_str()) {
            if spec.optional_position {
                body_start = skip_optional_math_environment_argument(candidate, body_start)?;
            }
            for _ in 0..spec.required_arguments {
                body_start = skip_math_environment_argument(candidate, body_start)?;
            }
            let (body_end, environment_end) =
                matching_math_environment_end(candidate, body_start, environment.as_str())?;
            if !math_outer_annotations_only(candidate.get(environment_end..)?) {
                return None;
            }
            return split_top_level_math_rows(candidate.get(body_start..body_end)?);
        }
        if !transparent_math_environment(environment.as_str()) {
            return None;
        }
        let (body_end, environment_end) =
            matching_math_environment_end(candidate, body_start, environment.as_str())?;
        if !math_outer_annotations_only(candidate.get(environment_end..)?) {
            return None;
        }
        candidate = candidate.get(body_start..body_end)?.trim();
    }
    None
}

fn strip_math_layout_style(source: &str) -> Option<&str> {
    [
        r"\displaystyle",
        r"\textstyle",
        r"\scriptstyle",
        r"\scriptscriptstyle",
    ]
    .into_iter()
    .find_map(|command| {
        source.strip_prefix(command).and_then(|tail| {
            (!tail
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic()))
            .then_some(tail)
        })
    })
}

fn transparent_math_environment(environment: &str) -> bool {
    matches!(
        environment,
        "equation" | "equation*" | "displaymath" | "math"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NavigableMathEnvironment {
    optional_position: bool,
    required_arguments: usize,
}

fn navigable_math_environment(environment: &str) -> Option<NavigableMathEnvironment> {
    let optional_position = matches!(
        environment,
        "aligned"
            | "alignedat"
            | "alignedat*"
            | "gathered"
            | "lgathered"
            | "multlined"
            | "empheq"
            | "IEEEeqnarraybox"
            | "IEEEeqnarraybox*"
    );
    let required_arguments = usize::from(matches!(
        environment,
        "alignedat"
            | "alignedat*"
            | "alignat"
            | "alignat*"
            | "numcases"
            | "subnumcases"
            | "IEEEeqnarray"
            | "IEEEeqnarray*"
            | "IEEEeqnarraybox"
            | "IEEEeqnarraybox*"
            | "empheq"
    ));
    matches!(
        environment,
        "aligned"
            | "alignedat"
            | "alignedat*"
            | "align"
            | "align*"
            | "alignat"
            | "alignat*"
            | "flalign"
            | "flalign*"
            | "eqnarray"
            | "eqnarray*"
            | "gather"
            | "gather*"
            | "gathered"
            | "lgathered"
            | "multline"
            | "multline*"
            | "multlined"
            | "split"
            | "cases"
            | "cases*"
            | "dcases"
            | "dcases*"
            | "rcases"
            | "rcases*"
            | "drcases"
            | "drcases*"
            | "numcases"
            | "subnumcases"
            | "IEEEeqnarray"
            | "IEEEeqnarray*"
            | "IEEEeqnarraybox"
            | "IEEEeqnarraybox*"
            | "empheq"
    )
    .then_some(NavigableMathEnvironment {
        optional_position,
        required_arguments,
    })
}

fn math_environment_command(source: &str, start: usize, command: &str) -> Option<(String, usize)> {
    let marker = format!(r"\{command}{{");
    if !source.get(start..)?.starts_with(marker.as_str()) {
        return None;
    }
    let name_start = start.saturating_add(marker.len());
    let relative_end = source.get(name_start..)?.find('}')?;
    let name_end = name_start.saturating_add(relative_end);
    let name = source.get(name_start..name_end)?;
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '*')
    {
        return None;
    }
    Some((name.to_string(), name_end.saturating_add(1)))
}

fn skip_math_environment_argument(source: &str, start: usize) -> Option<usize> {
    let whitespace = source.get(start..)?.len() - source.get(start..)?.trim_start().len();
    let open = start.saturating_add(whitespace);
    if source.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0_usize;
    let mut index = open;
    while index < source.len() {
        match source.as_bytes()[index] {
            b'\\' => index = index.saturating_add(2),
            b'{' => {
                depth = depth.saturating_add(1);
                index = index.saturating_add(1);
            }
            b'}' => {
                depth = depth.checked_sub(1)?;
                index = index.saturating_add(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index = next_math_char_boundary(source, index),
        }
    }
    None
}

fn skip_optional_math_environment_argument(source: &str, start: usize) -> Option<usize> {
    let whitespace = source.get(start..)?.len() - source.get(start..)?.trim_start().len();
    let open = start.saturating_add(whitespace);
    if source.as_bytes().get(open) != Some(&b'[') {
        return Some(start);
    }
    let mut bracket_depth = 0_usize;
    let mut brace_depth = 0_usize;
    let mut index = open;
    while index < source.len() {
        match source.as_bytes()[index] {
            b'\\' => index = index.saturating_add(2),
            b'{' => {
                brace_depth = brace_depth.saturating_add(1);
                index = index.saturating_add(1);
            }
            b'}' => {
                brace_depth = brace_depth.checked_sub(1)?;
                index = index.saturating_add(1);
            }
            b'[' => {
                bracket_depth = bracket_depth.saturating_add(1);
                index = index.saturating_add(1);
            }
            b']' if brace_depth == 0 => {
                bracket_depth = bracket_depth.checked_sub(1)?;
                index = index.saturating_add(1);
                if bracket_depth == 0 {
                    return Some(index);
                }
            }
            _ => index = next_math_char_boundary(source, index),
        }
    }
    None
}

fn skip_math_ignorable(source: &str, mut index: usize) -> Option<usize> {
    loop {
        let tail = source.get(index..)?;
        let whitespace = tail.len().saturating_sub(tail.trim_start().len());
        index = index.saturating_add(whitespace);
        if source.as_bytes().get(index) != Some(&b'%') || math_character_is_escaped(source, index) {
            return Some(index);
        }
        index = source
            .get(index..)?
            .find('\n')
            .map_or(source.len(), |offset| index.saturating_add(offset + 1));
    }
}

fn math_outer_annotation_end(source: &str, start: usize) -> Option<usize> {
    for command in ["label", "tag"] {
        if let Some(mut end) = math_control_word_end(source, start, command) {
            if command == "tag" && source.as_bytes().get(end) == Some(&b'*') {
                end = end.saturating_add(1);
            }
            return skip_math_environment_argument(source, end);
        }
    }
    ["notag", "nonumber"]
        .into_iter()
        .find_map(|command| math_control_word_end(source, start, command))
}

fn math_outer_annotations_only(source: &str) -> bool {
    let mut index = 0_usize;
    loop {
        let Some(content_start) = skip_math_ignorable(source, index) else {
            return false;
        };
        if content_start == source.len() {
            return true;
        }
        let Some(annotation_end) = math_outer_annotation_end(source, content_start) else {
            return false;
        };
        index = annotation_end;
    }
}

fn matching_math_environment_end(
    source: &str,
    body_start: usize,
    outer_environment: &str,
) -> Option<(usize, usize)> {
    let mut nested = Vec::<String>::new();
    let mut index = body_start;
    while index < source.len() {
        if source.as_bytes()[index] == b'%' && !math_character_is_escaped(source, index) {
            index = source
                .get(index..)?
                .find('\n')
                .map_or(source.len(), |offset| index.saturating_add(offset + 1));
            continue;
        }
        if source.as_bytes()[index] == b'\\' {
            if let Some((environment, end)) = math_environment_command(source, index, "begin") {
                nested.push(environment);
                index = end;
                continue;
            }
            if let Some((environment, end)) = math_environment_command(source, index, "end") {
                if let Some(expected) = nested.pop() {
                    if expected != environment {
                        return None;
                    }
                    index = end;
                    continue;
                }
                return (environment == outer_environment).then_some((index, end));
            }
        }
        index = next_math_char_boundary(source, index);
    }
    None
}

fn split_top_level_math_rows(body: &str) -> Option<Vec<String>> {
    let mut rows = Vec::new();
    let mut nested = Vec::<String>::new();
    let mut brace_depth = 0_usize;
    let mut row_start = 0_usize;
    let mut index = 0_usize;
    while index < body.len() {
        if body.as_bytes()[index] == b'%' && !math_character_is_escaped(body, index) {
            index = body
                .get(index..)?
                .find('\n')
                .map_or(body.len(), |offset| index.saturating_add(offset + 1));
            continue;
        }
        if body.as_bytes()[index] == b'\\' {
            if let Some((environment, end)) = math_environment_command(body, index, "begin") {
                nested.push(environment);
                index = end;
                continue;
            }
            if let Some((environment, end)) = math_environment_command(body, index, "end") {
                if nested.pop().as_deref() != Some(environment.as_str()) {
                    return None;
                }
                index = end;
                continue;
            }
            if brace_depth == 0
                && nested.is_empty()
                && math_source_is_ignorable(body.get(row_start..index)?)
                && let Some(intertext_end) = math_intertext_end(body, index)
            {
                rows.push(body.get(index..intertext_end)?.trim().to_string());
                row_start = intertext_end;
                index = intertext_end;
                continue;
            }
            let row_separator_end = if brace_depth == 0 && nested.is_empty() {
                if body.as_bytes().get(index.saturating_add(1)) == Some(&b'\\') {
                    math_row_separator_end(body, index.saturating_add(2))
                } else {
                    math_control_word_end(body, index, "crcr")
                        .or_else(|| math_control_word_end(body, index, "cr"))
                        .or_else(|| {
                            math_control_word_end(body, index, "tabularnewline")
                                .and_then(|end| math_row_separator_end(body, end))
                        })
                }
            } else {
                None
            };
            if let Some(separator_end) = row_separator_end {
                let row = body.get(row_start..index)?.trim();
                if row.is_empty() {
                    return None;
                }
                rows.push(row.to_string());
                row_start = separator_end;
                index = separator_end;
                continue;
            }
            if matches!(
                body.as_bytes().get(index.saturating_add(1)),
                Some(b'{') | Some(b'}')
            ) {
                index = index.saturating_add(2);
                continue;
            }
        }
        match body.as_bytes()[index] {
            b'{' => brace_depth = brace_depth.saturating_add(1),
            b'}' => brace_depth = brace_depth.checked_sub(1)?,
            _ => {}
        }
        index = next_math_char_boundary(body, index);
    }
    if brace_depth != 0 || !nested.is_empty() {
        return None;
    }
    let final_row = body.get(row_start..)?.trim();
    if final_row.is_empty() {
        return None;
    }
    rows.push(final_row.to_string());
    Some(rows)
}

fn math_source_is_ignorable(source: &str) -> bool {
    skip_math_ignorable(source, 0) == Some(source.len())
}

fn math_intertext_end(source: &str, start: usize) -> Option<usize> {
    ["intertext", "shortintertext"]
        .into_iter()
        .find_map(|command| math_control_word_end(source, start, command))
        .and_then(|end| skip_math_environment_argument(source, end))
}

fn math_row_separator_end(source: &str, mut index: usize) -> Option<usize> {
    if source.as_bytes().get(index) == Some(&b'*') {
        index = index.saturating_add(1);
    }
    let whitespace_start = index;
    while source
        .as_bytes()
        .get(index)
        .is_some_and(u8::is_ascii_whitespace)
    {
        index = index.saturating_add(1);
    }
    if source.as_bytes().get(index) != Some(&b'[') {
        return Some(whitespace_start);
    }
    index = index.saturating_add(1);
    let mut brace_depth = 0_usize;
    while index < source.len() {
        match source.as_bytes()[index] {
            b'\\' => index = index.saturating_add(2),
            b'{' => {
                brace_depth = brace_depth.saturating_add(1);
                index = index.saturating_add(1);
            }
            b'}' => {
                brace_depth = brace_depth.checked_sub(1)?;
                index = index.saturating_add(1);
            }
            b']' if brace_depth == 0 => return Some(index.saturating_add(1)),
            _ => index = next_math_char_boundary(source, index),
        }
    }
    None
}

fn math_control_word_end(source: &str, start: usize, command: &str) -> Option<usize> {
    let marker = format!(r"\{command}");
    if !source.get(start..)?.starts_with(marker.as_str()) {
        return None;
    }
    let end = start.saturating_add(marker.len());
    (!source
        .get(end..)?
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic()))
    .then_some(end)
}

fn math_character_is_escaped(source: &str, index: usize) -> bool {
    source.as_bytes()[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn next_math_char_boundary(source: &str, index: usize) -> usize {
    index
        .saturating_add(
            source
                .get(index..)
                .and_then(|tail| tail.chars().next())
                .map_or(1, char::len_utf8),
        )
        .min(source.len())
}

pub fn inline_math_segments(source: &str) -> Vec<InlineMathSegment> {
    let bytes = source.as_bytes();
    let mut segments = Vec::new();
    let mut text_start = 0_usize;
    let mut index = 0_usize;
    let mut code_fence_len = 0_usize;

    while index < bytes.len() {
        if bytes[index] == b'`' && !is_escaped(bytes, index) {
            let run = bytes[index..]
                .iter()
                .take_while(|byte| **byte == b'`')
                .count();
            if code_fence_len == 0 {
                code_fence_len = run;
            } else if run == code_fence_len {
                code_fence_len = 0;
            }
            index += run;
            continue;
        }
        if code_fence_len > 0 {
            index += 1;
            continue;
        }

        let (open_len, close) = if bytes[index] == b'$'
            && bytes.get(index + 1) != Some(&b'$')
            && !is_escaped(bytes, index)
        {
            (1, InlineClose::Dollar)
        } else if bytes[index..].starts_with(br"\(") && !is_escaped(bytes, index) {
            (2, InlineClose::Paren)
        } else {
            index += 1;
            continue;
        };
        let content_start = index + open_len;
        if bytes
            .get(content_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            index += open_len;
            continue;
        }
        let Some(close_start) = find_inline_close(bytes, content_start, close) else {
            index += open_len;
            continue;
        };
        if close_start == content_start
            || bytes
                .get(close_start.saturating_sub(1))
                .is_some_and(u8::is_ascii_whitespace)
        {
            index += open_len;
            continue;
        }
        if text_start < index {
            segments.push(InlineMathSegment::Text(
                source[text_start..index].to_string(),
            ));
        }
        segments.push(InlineMathSegment::Math(
            source[content_start..close_start].to_string(),
        ));
        index = close_start + close.len();
        text_start = index;
    }
    if text_start < source.len() {
        segments.push(InlineMathSegment::Text(source[text_start..].to_string()));
    }
    segments
}

pub fn inline_math_unicode_text(source: &str) -> String {
    inline_math_segments(source)
        .into_iter()
        .map(|segment| match segment {
            InlineMathSegment::Text(text) => text,
            InlineMathSegment::Math(formula) => unicode_formula(&formula, false).join(" "),
        })
        .collect()
}

#[derive(Clone, Copy)]
enum InlineClose {
    Dollar,
    Paren,
}

impl InlineClose {
    fn len(self) -> usize {
        match self {
            Self::Dollar => 1,
            Self::Paren => 2,
        }
    }
}

fn find_inline_close(bytes: &[u8], mut index: usize, close: InlineClose) -> Option<usize> {
    while index < bytes.len() {
        let matches = match close {
            InlineClose::Dollar => {
                bytes[index] == b'$'
                    && bytes.get(index + 1) != Some(&b'$')
                    && !is_escaped(bytes, index)
            }
            InlineClose::Paren => bytes[index..].starts_with(br"\)") && !is_escaped(bytes, index),
        };
        if matches {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let slash_count = bytes[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    slash_count % 2 == 1
}

pub fn push_math_block(out: &mut Vec<RenderedLine>, prefix: &str, source: &str, width: u16) {
    let formula = display_math_source(source).unwrap_or_else(|| source.trim().to_string());
    let semantic_rows = display_math_semantic_rows(&formula);
    let config = layout_config();
    if config.native_graphics
        && let Ok(artifact) = render_formula(&formula, true)
    {
        let prefix_width = UnicodeWidthStr::width(prefix) as u16;
        let available = width.saturating_sub(prefix_width).max(1);
        let render_width = artifact.size.width.min(available);
        let render_height = if artifact.size.width > render_width {
            u32::from(artifact.size.height)
                .saturating_mul(u32::from(render_width))
                .div_ceil(u32::from(artifact.size.width))
                .max(1) as u16
        } else {
            artifact.size.height
        };
        let native_semantic_rows = semantic_rows.as_ref().map(|rows| {
            let mut rows = rows.clone();
            if artifact.row_layout_weights.len() == rows.len() {
                for (row, weight) in rows.iter_mut().zip(&artifact.row_layout_weights) {
                    row.layout_weight = (*weight).max(1);
                }
            }
            rows
        });
        let navigation_can_map = native_semantic_rows.as_deref().is_none_or(|rows| {
            allocate_math_navigation_ranges(usize::from(render_height), rows).is_some()
        });
        if !navigation_can_map {
            // A downscaled artifact that has fewer terminal rows than semantic
            // equation rows cannot expose honest hit regions. Use the
            // accessible Unicode/source-row fallback below instead.
        } else {
            let column = prefix_width + available.saturating_sub(render_width) / 2;
            let start = out.len();
            for _ in 0..render_height {
                out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
            }
            out[start].math.push(MathLinePlacement {
                column,
                artifact,
                size: ratatui::layout::Size::new(render_width, render_height),
            });
            mark_math_navigation_rows(
                out,
                start,
                usize::from(render_height),
                native_semantic_rows.as_deref(),
                formula.as_str(),
            );
            return;
        }
    }
    let canvas = unicode_formula(&formula, true);
    if let Some(rows) = semantic_rows.as_deref() {
        if let Some(ranges) = allocate_math_navigation_ranges(canvas.len(), rows) {
            for (row, range) in rows.iter().zip(ranges) {
                let start = out.len();
                push_unicode_canvas(out, prefix, &canvas[range], width);
                mark_rendered_math_unit(out, start, row.source.as_str());
            }
            return;
        }
        // A renderer may reject a command even though the outer row structure
        // is unambiguous. Preserve row navigation by rendering an explicit,
        // individually copyable fallback for each structural row.
        for row in rows {
            let start = out.len();
            let row_canvas = unicode_formula(row.source.as_str(), true);
            push_unicode_canvas(out, prefix, row_canvas.as_slice(), width);
            mark_rendered_math_unit(out, start, row.source.as_str());
        }
        return;
    }
    let start = out.len();
    push_unicode_canvas(out, prefix, &canvas, width);
    mark_rendered_math_unit(out, start, formula.as_str());
}

fn mark_math_navigation_rows(
    out: &mut [RenderedLine],
    start: usize,
    height: usize,
    semantic_rows: Option<&[DisplayMathSemanticRow]>,
    fallback_copy_text: &str,
) {
    if let Some(rows) = semantic_rows
        && let Some(ranges) = allocate_math_navigation_ranges(height, rows)
    {
        for (row, range) in rows.iter().zip(ranges) {
            let unit_start = start.saturating_add(range.start);
            for line in out
                .get_mut(unit_start..start.saturating_add(range.end))
                .unwrap_or_default()
            {
                line.navigation_unit = Some(unit_start);
                line.navigation_copy_text.clone_from(&row.source);
                line.pointer_selection = TranscriptPointerSelection::SemanticUnit;
            }
        }
        return;
    }
    mark_rendered_math_unit(out, start, fallback_copy_text);
}

fn mark_rendered_math_unit(out: &mut [RenderedLine], start: usize, copy_text: &str) {
    for line in out.get_mut(start..).unwrap_or_default() {
        line.navigation_unit = Some(start);
        line.navigation_copy_text = copy_text.to_string();
        line.pointer_selection = TranscriptPointerSelection::SemanticUnit;
    }
}

fn allocate_math_navigation_ranges(
    total_height: usize,
    rows: &[DisplayMathSemanticRow],
) -> Option<Vec<std::ops::Range<usize>>> {
    if rows.len() < 2 || total_height < rows.len() {
        return None;
    }
    let weights = rows
        .iter()
        .map(|row| row.layout_weight.max(1))
        .collect::<Vec<_>>();
    let weight_sum = weights.iter().copied().sum::<usize>();
    let heights = if weight_sum == total_height {
        weights
    } else {
        let remaining = total_height.saturating_sub(rows.len());
        let mut heights = vec![1_usize; rows.len()];
        if remaining > 0 {
            let mut distributed = 0_usize;
            let mut remainders = Vec::with_capacity(rows.len());
            for (index, weight) in weights.iter().copied().enumerate() {
                let numerator = remaining.saturating_mul(weight);
                let share = numerator / weight_sum.max(1);
                heights[index] = heights[index].saturating_add(share);
                distributed = distributed.saturating_add(share);
                remainders.push((numerator % weight_sum.max(1), index));
            }
            remainders.sort_by(|left, right| right.cmp(left));
            for (_, index) in remainders
                .into_iter()
                .take(remaining.saturating_sub(distributed))
            {
                heights[index] = heights[index].saturating_add(1);
            }
        }
        heights
    };

    let mut start = 0_usize;
    let ranges = heights
        .into_iter()
        .map(|height| {
            let end = start.saturating_add(height);
            let range = start..end;
            start = end;
            range
        })
        .collect::<Vec<_>>();
    (start == total_height).then_some(ranges)
}

pub fn push_inline_math(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    source: &str,
    width: u16,
) -> bool {
    let segments = inline_math_segments(source);
    if !segments
        .iter()
        .any(|segment| matches!(segment, InlineMathSegment::Math(_)))
    {
        return false;
    }
    let prefix_width = UnicodeWidthStr::width(prefix) as u16;
    let available = width.saturating_sub(prefix_width).max(1);
    let config = layout_config();

    if config.native_graphics && source.contains('\n') {
        for line in source.lines() {
            if !push_inline_math(out, prefix, line, width) {
                crate::push_markdown(out, prefix, line, width);
            }
        }
        return true;
    }

    if config.native_graphics {
        let mut items = Vec::new();
        let mut total_width = 0_u16;
        let mut height = 1_u16;
        let mut render_failed = false;
        for segment in &segments {
            match segment {
                InlineMathSegment::Text(text) => {
                    let text = inline_markdown_plain_text(text);
                    total_width = total_width.saturating_add(
                        UnicodeWidthStr::width(text.as_str()).min(usize::from(u16::MAX)) as u16,
                    );
                    items.push(InlineItem::Text(text));
                }
                InlineMathSegment::Math(formula) => match render_formula(formula, false) {
                    Ok(artifact) => {
                        total_width = total_width.saturating_add(artifact.size.width);
                        height = height.max(artifact.size.height);
                        items.push(InlineItem::Math(artifact));
                    }
                    Err(error) => {
                        tracing::debug!(
                            diagnostic = %error,
                            "native inline math rendering failed; falling back to text rendering"
                        );
                        render_failed = true;
                    }
                },
            }
        }
        if !render_failed && total_width <= available {
            let vertical = InlineVerticalLayout::new(height);
            let start = out.len();
            for _ in 0..vertical.height() {
                out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
            }
            let mut line = String::from(prefix);
            let mut column = prefix_width;
            for item in items {
                match item {
                    InlineItem::Text(text) => {
                        column = column.saturating_add(
                            UnicodeWidthStr::width(text.as_str()).min(usize::from(u16::MAX)) as u16,
                        );
                        line.push_str(&text);
                    }
                    InlineItem::Math(artifact) => {
                        let placement_row = vertical.graphic_top_row(artifact.size.height);
                        out[start + placement_row].math.push(MathLinePlacement {
                            column,
                            size: artifact.size,
                            artifact: std::sync::Arc::clone(&artifact),
                        });
                        line.push_str(&" ".repeat(usize::from(artifact.size.width)));
                        column = column.saturating_add(artifact.size.width);
                    }
                }
            }
            out[start + vertical.text_row()]
                .replace_content_preserving_math(RenderedLine::plain(line, Style::default()));
            return true;
        }
    }

    // Unicode fallback uses the same line-box model, so fractions, roots and
    // matrices remain two-dimensional even without an image protocol.
    let mut rows = vec![String::new()];
    for segment in segments {
        let block = match segment {
            InlineMathSegment::Text(text) => vec![inline_markdown_plain_text(&text)],
            InlineMathSegment::Math(formula) => unicode_formula(&formula, false),
        };
        append_bottom_aligned(&mut rows, &block);
    }
    if rows
        .iter()
        .all(|line| UnicodeWidthStr::width(line.as_str()) <= usize::from(available))
    {
        push_unicode_canvas(out, prefix, &rows, width);
    } else {
        for row in rows {
            push_wrapped_line(out, prefix, prefix, &row, Style::default(), width);
        }
    }
    true
}

enum InlineItem {
    Text(String),
    Math(std::sync::Arc<agena_tui_media::MathArtifact>),
}

fn inline_markdown_plain_text(source: &str) -> String {
    let rendered = tui_markdown::from_str(source);
    rendered
        .lines
        .iter()
        .map(agena_tui_components::line_plain_text)
        .collect::<Vec<_>>()
        .join(" ")
}

fn append_bottom_aligned(canvas: &mut Vec<String>, block: &[String]) {
    let height = canvas.len().max(block.len()).max(1);
    if canvas.len() < height {
        let mut padded = vec![String::new(); height - canvas.len()];
        padded.append(canvas);
        *canvas = padded;
    }
    let block_width = block
        .iter()
        .map(|line| UnicodeWidthStr::width(line.as_str()))
        .max()
        .unwrap_or(0);
    let offset = height.saturating_sub(block.len());
    for (row, canvas_row) in canvas.iter_mut().enumerate().take(height) {
        if let Some(value) = row.checked_sub(offset).and_then(|index| block.get(index)) {
            canvas_row.push_str(value);
            let padding = block_width.saturating_sub(UnicodeWidthStr::width(value.as_str()));
            canvas_row.push_str(&" ".repeat(padding));
        } else {
            canvas_row.push_str(&" ".repeat(block_width));
        }
    }
}

fn push_unicode_canvas(out: &mut Vec<RenderedLine>, prefix: &str, rows: &[String], width: u16) {
    if rows.is_empty() {
        return;
    }
    for row in rows {
        push_multiline(out, prefix, row, Style::default(), width);
    }
}
