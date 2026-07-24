//! Semantic LaTeX adapter for the terminal Unicode renderer.
//!
//! `pulldown-latex` owns command parsing, macro expansion, grouping, scripts,
//! and mathematical environments. This module only translates its semantic
//! event stream into the layout tree consumed by `term-maths`.

use std::sync::LazyLock;

use pulldown_latex::{
    Event, Parser, Storage,
    event::{
        Content, Dimension, DimensionUnit, EnvironmentFlow, Font, Grouping, ScriptPosition,
        ScriptType, StateChange, Visual,
    },
};
use regex::Regex;
use rust_latex_parser::{AccentKind, EqNode, MathFontKind, MatrixKind};

use super::positional_unicode_text;

const MAX_SEMANTIC_EVENTS: usize = 16_384;

static CHEMISTRY_ISOTOPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?:\^\s*\{?(\d+)\}?\s*_\s*\{?(\d+)\}?|",
        r"_\s*\{?(\d+)\}?\s*\^\s*\{?(\d+)\}?)\s*([A-Z][a-z]?)",
    ))
    .expect("valid mhchem isotope regex")
});

/// Parse standard LaTeX into the existing `term-maths` layout tree.
///
/// A failed parse is intentional and visible to callers: they retain the
/// original source instead of rendering an incomplete best-effort formula.
pub(super) fn parse(source: &str, display: bool) -> Option<EqNode> {
    let source = compatibility_source(source)?;
    let storage = Storage::new();
    let mut events = Vec::new();
    for event in Parser::new(&source, &storage) {
        if events.len() >= MAX_SEMANTIC_EVENTS {
            return None;
        }
        events.push(event.ok()?);
    }

    let mut adapter = SemanticAdapter::new(&events, display);
    let node = adapter.parse_document()?;
    (adapter.cursor == events.len()).then_some(node)
}

/// Return an optional terminal-layout weight for every top-level row in a
/// semantic multi-row environment. Navigation structure is scanned
/// independently, so unsupported commands may make this geometry enrichment
/// unavailable without erasing otherwise unambiguous equation rows.
pub(super) fn semantic_row_heights(source: &str, display: bool) -> Option<Vec<usize>> {
    let ast = parse(source, display)?;
    let rows = top_level_table_rows(&ast)?;
    (rows.len() > 1).then(|| {
        rows.iter()
            .map(|row| semantic_row_height(row.as_slice()))
            .collect()
    })
}

fn top_level_table_rows(node: &EqNode) -> Option<&[Vec<EqNode>]> {
    match node {
        EqNode::Matrix { rows, .. } => Some(rows.as_slice()),
        EqNode::Delimited { content, .. } => top_level_table_rows(content),
        EqNode::Seq(nodes) => {
            let mut meaningful = nodes.iter().filter(|node| match node {
                EqNode::Space(_) => false,
                EqNode::Text(text) => !text.trim().is_empty(),
                _ => true,
            });
            let only = meaningful.next()?;
            meaningful
                .next()
                .is_none()
                .then(|| top_level_table_rows(only))
                .flatten()
        }
        _ => None,
    }
}

fn semantic_row_height(row: &[EqNode]) -> usize {
    let rendered = row
        .iter()
        .map(term_maths::layout::layout)
        .collect::<Vec<_>>();
    let baseline = rendered
        .iter()
        .map(|cell| cell.baseline())
        .max()
        .unwrap_or(0);
    let below = rendered
        .iter()
        .map(|cell| {
            cell.height()
                .saturating_sub(cell.baseline().saturating_add(1))
        })
        .max()
        .unwrap_or(0);
    baseline.saturating_add(1).saturating_add(below).max(1)
}

struct SemanticAdapter<'events, 'source> {
    events: &'events [Event<'source>],
    cursor: usize,
    display: bool,
    font: Option<Font>,
}

impl<'events, 'source> SemanticAdapter<'events, 'source> {
    fn new(events: &'events [Event<'source>], display: bool) -> Self {
        Self {
            events,
            cursor: 0,
            display,
            font: None,
        }
    }

    fn parse_document(&mut self) -> Option<EqNode> {
        let mut nodes = Vec::new();
        while self.cursor < self.events.len() {
            match self.events.get(self.cursor)? {
                Event::End | Event::EnvironmentFlow(_) => return None,
                Event::StateChange(change) => {
                    self.apply_state_change(*change);
                    self.cursor += 1;
                }
                _ => nodes.push(self.parse_element()?),
            }
        }
        Some(sequence(nodes))
    }

    fn parse_element(&mut self) -> Option<EqNode> {
        while let Some(Event::StateChange(change)) = self.events.get(self.cursor) {
            self.apply_state_change(*change);
            self.cursor += 1;
        }

        let event = self.events.get(self.cursor)?.clone();
        self.cursor += 1;
        match event {
            Event::Content(content) => Some(self.content(content)),
            Event::Begin(grouping) => self.group(grouping),
            Event::Visual(visual) => self.visual(visual),
            Event::Script { ty, position } => self.script(ty, position),
            Event::Space { width, .. } => Some(EqNode::Space(width.map_or(0.0, dimension_points))),
            Event::StateChange(_) => unreachable!("state changes are consumed above"),
            Event::End | Event::EnvironmentFlow(_) => None,
        }
    }

    fn content(&self, content: Content<'source>) -> EqNode {
        let node = match content {
            Content::Text(text) => return EqNode::TextBlock(text.to_string()),
            Content::Number(number) => EqNode::Text(number.to_string()),
            Content::Function(name) if is_limit_function(name) => EqNode::Limit {
                name: name.to_string(),
                lower: None,
            },
            Content::Function("mod") => {
                EqNode::Seq(vec![EqNode::Text("mod".to_string()), EqNode::Space(4.0)])
            }
            Content::Function(name) => EqNode::Text(name.to_string()),
            Content::Ordinary { content: '■', .. } => EqNode::Text("∎".to_string()),
            Content::Ordinary { content, .. } => EqNode::Text(content.to_string()),
            Content::LargeOp { content, .. } => EqNode::BigOp {
                symbol: content.to_string(),
                lower: None,
                upper: None,
            },
            Content::BinaryOp { content: '⋅', .. } => spaced_text("·".to_string()),
            Content::BinaryOp { content, .. } => spaced_text(content.to_string()),
            Content::Relation { content, .. } => {
                let mut buffer = [0_u8; 8];
                let value = std::str::from_utf8(content.encode_utf8_to_buf(&mut buffer))
                    .expect("relation content is valid UTF-8")
                    .to_string();
                spaced_text(value)
            }
            Content::Delimiter { content, .. } | Content::Punctuation(content) => {
                EqNode::Text(content.to_string())
            }
        };
        self.apply_font(node)
    }

    fn group(&mut self, grouping: Grouping) -> Option<EqNode> {
        let previous_font = self.font;
        if !matches!(grouping, Grouping::Normal | Grouping::LeftRight(_, _)) {
            self.font = None;
        }

        let result = match grouping {
            Grouping::Normal => self.group_sequence(),
            Grouping::LeftRight(left, right) => {
                let content = self.group_sequence()?;
                Some(EqNode::Delimited {
                    left: left.map_or_else(String::new, |ch| ch.to_string()),
                    right: right.map_or_else(String::new, |ch| ch.to_string()),
                    content: Box::new(content),
                })
            }
            Grouping::Cases { left } => {
                let rows = self.table_rows()?;
                Some(EqNode::Delimited {
                    left: if left { "{" } else { "" }.to_string(),
                    right: if left { "" } else { "}" }.to_string(),
                    content: Box::new(EqNode::Matrix {
                        kind: MatrixKind::Plain,
                        rows,
                    }),
                })
            }
            Grouping::Array(_)
            | Grouping::Matrix { .. }
            | Grouping::Equation { .. }
            | Grouping::Align { .. }
            | Grouping::Aligned
            | Grouping::SubArray { .. }
            | Grouping::Alignat { .. }
            | Grouping::Alignedat { .. }
            | Grouping::Gather { .. }
            | Grouping::Gathered
            | Grouping::Multline
            | Grouping::Split => Some(EqNode::Matrix {
                kind: MatrixKind::Plain,
                rows: self.table_rows()?,
            }),
        };
        self.font = previous_font;
        result
    }

    fn group_sequence(&mut self) -> Option<EqNode> {
        let mut nodes = Vec::new();
        loop {
            match self.events.get(self.cursor)? {
                Event::End => {
                    self.cursor += 1;
                    return Some(sequence(nodes));
                }
                Event::EnvironmentFlow(_) => return None,
                Event::StateChange(change) => {
                    self.apply_state_change(*change);
                    self.cursor += 1;
                }
                _ => nodes.push(self.parse_element()?),
            }
        }
    }

    fn table_rows(&mut self) -> Option<Vec<Vec<EqNode>>> {
        let default_font = self.font;
        let mut rows = Vec::new();
        let mut row = Vec::new();
        let mut cell = Vec::new();

        loop {
            match self.events.get(self.cursor)? {
                Event::End => {
                    self.cursor += 1;
                    push_cell(&mut row, &mut cell);
                    if !row.is_empty() {
                        rows.push(std::mem::take(&mut row));
                    }
                    return Some(rows);
                }
                Event::EnvironmentFlow(EnvironmentFlow::Alignment) => {
                    self.cursor += 1;
                    push_cell(&mut row, &mut cell);
                    self.font = default_font;
                }
                Event::EnvironmentFlow(EnvironmentFlow::NewLine { .. }) => {
                    self.cursor += 1;
                    push_cell(&mut row, &mut cell);
                    rows.push(std::mem::take(&mut row));
                    self.font = default_font;
                }
                Event::EnvironmentFlow(_) => self.cursor += 1,
                Event::StateChange(change) => {
                    self.apply_state_change(*change);
                    self.cursor += 1;
                }
                _ => cell.push(self.parse_element()?),
            }
        }
    }

    fn visual(&mut self, visual: Visual) -> Option<EqNode> {
        match visual {
            Visual::SquareRoot => Some(EqNode::Sqrt(Box::new(self.parse_element()?))),
            Visual::Root => {
                let radicand = self.parse_element()?;
                let index = self.parse_element()?;
                Some(EqNode::StackRel {
                    base: Box::new(EqNode::Sqrt(Box::new(radicand))),
                    annotation: Box::new(index),
                    over: true,
                })
            }
            Visual::Fraction(line_width) => {
                let numerator = self.parse_element()?;
                let denominator = self.parse_element()?;
                if line_width.is_some_and(|width| width.value == 0.0) {
                    Some(EqNode::Matrix {
                        kind: MatrixKind::Plain,
                        rows: vec![vec![numerator], vec![denominator]],
                    })
                } else {
                    Some(EqNode::Frac(Box::new(numerator), Box::new(denominator)))
                }
            }
            Visual::Negation => Some(negate(self.parse_element()?)),
        }
    }

    fn script(&mut self, ty: ScriptType, position: ScriptPosition) -> Option<EqNode> {
        let base = trim_outer_spaces(self.parse_element()?);
        let (lower, upper) = match ty {
            ScriptType::Subscript => (Some(self.parse_element()?), None),
            ScriptType::Superscript => (None, Some(self.parse_element()?)),
            ScriptType::SubSuperscript => {
                let lower = self.parse_element()?;
                let upper = self.parse_element()?;
                (Some(lower), Some(upper))
            }
        };

        let stacked = matches!(position, ScriptPosition::AboveBelow)
            || matches!(position, ScriptPosition::Movable) && self.display;
        Some(if stacked {
            stacked_scripts(base, lower, upper)
        } else {
            right_scripts(base, lower, upper)
        })
    }

    fn apply_state_change(&mut self, change: StateChange) {
        if let StateChange::Font(font) = change {
            self.font = font;
        }
    }

    fn apply_font(&self, node: EqNode) -> EqNode {
        match self.font.and_then(font_kind) {
            Some(kind) => EqNode::MathFont {
                kind,
                content: Box::new(node),
            },
            None => node,
        }
    }
}

fn push_cell(row: &mut Vec<EqNode>, cell: &mut Vec<EqNode>) {
    row.push(sequence(std::mem::take(cell)));
}

fn sequence(mut nodes: Vec<EqNode>) -> EqNode {
    match nodes.len() {
        0 => EqNode::Text(String::new()),
        1 => nodes.pop().expect("single node exists"),
        _ => EqNode::Seq(nodes),
    }
}

fn spaced_text(value: String) -> EqNode {
    EqNode::Seq(vec![
        EqNode::Space(4.0),
        EqNode::Text(value),
        EqNode::Space(4.0),
    ])
}

fn is_limit_function(name: &str) -> bool {
    matches!(
        name,
        "lim" | "liminf" | "limsup" | "max" | "min" | "inf" | "sup"
    )
}

fn font_kind(font: Font) -> Option<MathFontKind> {
    Some(match font {
        Font::Bold | Font::BoldItalic => MathFontKind::Bold,
        Font::DoubleStruck => MathFontKind::Blackboard,
        Font::Script | Font::BoldScript => MathFontKind::Calligraphic,
        Font::Fraktur | Font::BoldFraktur => MathFontKind::Fraktur,
        Font::Monospace => MathFontKind::Monospace,
        Font::SansSerif
        | Font::SansSerifBoldItalic
        | Font::SansSerifItalic
        | Font::BoldSansSerif => MathFontKind::SansSerif,
        Font::UpRight => MathFontKind::Roman,
        Font::Italic => return None,
    })
}

fn dimension_points(dimension: Dimension) -> f32 {
    let factor = match dimension.unit {
        DimensionUnit::Em => 18.0,
        DimensionUnit::Mu => 1.0,
        DimensionUnit::Ex => 8.0,
        DimensionUnit::Pt | DimensionUnit::Bp => 1.0,
        DimensionUnit::Pc => 12.0,
        DimensionUnit::In => 72.0,
        DimensionUnit::Cm => 28.35,
        DimensionUnit::Mm => 2.835,
        DimensionUnit::Dd => 1.07,
        DimensionUnit::Cc => 12.84,
        DimensionUnit::Sp => 1.0 / 65_536.0,
    };
    dimension.value * factor
}

fn right_scripts(base: EqNode, lower: Option<EqNode>, upper: Option<EqNode>) -> EqNode {
    match (lower, upper) {
        (Some(lower), Some(upper)) => {
            EqNode::SupSub(Box::new(base), Box::new(upper), Box::new(lower))
        }
        (Some(lower), None) => EqNode::Sub(Box::new(base), Box::new(lower)),
        (None, Some(upper)) => EqNode::Sup(Box::new(base), Box::new(upper)),
        (None, None) => base,
    }
}

fn stacked_scripts(base: EqNode, lower: Option<EqNode>, upper: Option<EqNode>) -> EqNode {
    if let EqNode::BigOp {
        symbol,
        lower: None,
        upper: None,
    } = base
    {
        return EqNode::BigOp {
            symbol,
            lower: lower.map(Box::new),
            upper: upper.map(Box::new),
        };
    }
    if let EqNode::Limit { name, lower: None } = base {
        if upper.is_none() {
            return EqNode::Limit {
                name,
                lower: lower.map(Box::new),
            };
        }
        return EqNode::BigOp {
            symbol: name,
            lower: lower.map(Box::new),
            upper: upper.map(Box::new),
        };
    }

    if lower.is_none()
        && let Some(kind) = upper.as_ref().and_then(accent_kind)
    {
        return EqNode::Accent(Box::new(base), kind);
    }

    let with_lower = match lower {
        Some(annotation) => EqNode::StackRel {
            base: Box::new(base),
            annotation: Box::new(trim_outer_spaces(annotation)),
            over: false,
        },
        None => base,
    };
    match upper {
        Some(annotation) => EqNode::StackRel {
            base: Box::new(with_lower),
            annotation: Box::new(trim_outer_spaces(annotation)),
            over: true,
        },
        None => with_lower,
    }
}

fn accent_kind(node: &EqNode) -> Option<AccentKind> {
    match flat_text(node)?.as_str() {
        "^" => Some(AccentKind::Hat),
        "‾" => Some(AccentKind::Bar),
        "~" => Some(AccentKind::Tilde),
        "→" => Some(AccentKind::Vec),
        "˙" => Some(AccentKind::Dot),
        "¨" => Some(AccentKind::DoubleDot),
        _ => None,
    }
}

fn negate(node: EqNode) -> EqNode {
    let Some(text) = flat_text(&node) else {
        return EqNode::StackRel {
            base: Box::new(node),
            annotation: Box::new(EqNode::Text("╱".to_string())),
            over: true,
        };
    };
    let replacement = match text.as_str() {
        "=" => "≠",
        "∈" => "∉",
        "∋" => "∌",
        "<" => "≮",
        ">" => "≯",
        "≤" => "≰",
        "≥" => "≱",
        "⊂" => "⊄",
        "⊃" => "⊅",
        "⊆" => "⊈",
        "⊇" => "⊉",
        "≈" => "≉",
        "≡" => "≢",
        _ => return EqNode::Text(format!("{text}\u{338}")),
    };
    if matches!(node, EqNode::Seq(_)) {
        spaced_text(replacement.to_string())
    } else {
        EqNode::Text(replacement.to_string())
    }
}

fn flat_text(node: &EqNode) -> Option<String> {
    match node {
        EqNode::Text(text) | EqNode::TextBlock(text) => Some(text.clone()),
        EqNode::Space(_) => Some(String::new()),
        EqNode::Seq(nodes) => nodes
            .iter()
            .map(flat_text)
            .collect::<Option<Vec<_>>>()
            .map(|v| v.concat()),
        EqNode::MathFont { content, .. } => flat_text(content),
        _ => None,
    }
}

fn trim_outer_spaces(node: EqNode) -> EqNode {
    let EqNode::Seq(mut nodes) = node else {
        return node;
    };
    while matches!(nodes.first(), Some(EqNode::Space(_))) {
        nodes.remove(0);
    }
    while matches!(nodes.last(), Some(EqNode::Space(_))) {
        nodes.pop();
    }
    sequence(nodes)
}

fn compatibility_source(source: &str) -> Option<String> {
    let mut source = source.to_string();
    for (command, replacement) in [
        ("llbracket", "⟦"),
        ("rrbracket", "⟧"),
        ("mathscr", r"\mathcal"),
        ("qed", "∎"),
        ("QED", "∎"),
    ] {
        source = replace_command(&source, command, replacement);
    }
    for position in ["t", "b", "c"] {
        source = source.replace(&format!(r"\begin{{array}}[{position}]"), r"\begin{array}");
    }
    source = rewrite_braced_command(&source, "substack", |body| {
        format!(r"\begin{{subarray}}{{c}}{body}\end{{subarray}}")
    })?;
    source = rewrite_braced_command(&source, "tag", |body| format!(r"\quad({body})"))?;
    source = rewrite_braced_command(&source, "label", |_| String::new())?;
    rewrite_braced_command(&source, "ce", |body| {
        format!(r"\mathrm{{{}}}", normalize_chemistry(body))
    })
}

fn replace_command(source: &str, command: &str, replacement: &str) -> String {
    let needle = format!(r"\{command}");
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find(&needle) {
        let start = cursor + relative;
        let end = start + needle.len();
        output.push_str(&source[cursor..start]);
        if source[end..]
            .chars()
            .next()
            .is_some_and(char::is_alphabetic)
        {
            output.push_str(&source[start..end]);
        } else {
            output.push_str(replacement);
        }
        cursor = end;
    }
    output.push_str(&source[cursor..]);
    output
}

fn rewrite_braced_command(
    source: &str,
    command: &str,
    rewrite: impl Fn(&str) -> String,
) -> Option<String> {
    let needle = format!(r"\{command}");
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find(&needle) {
        let start = cursor + relative;
        let command_end = start + needle.len();
        if source[command_end..]
            .chars()
            .next()
            .is_some_and(char::is_alphabetic)
        {
            output.push_str(&source[cursor..command_end]);
            cursor = command_end;
            continue;
        }
        let open = skip_whitespace(source, command_end);
        if source.as_bytes().get(open) != Some(&b'{') {
            output.push_str(&source[cursor..command_end]);
            cursor = command_end;
            continue;
        }
        let close = balanced_brace_end(source, open)?;
        output.push_str(&source[cursor..start]);
        output.push_str(&rewrite(&source[open + 1..close]));
        cursor = close + 1;
    }
    output.push_str(&source[cursor..]);
    Some(output)
}

fn skip_whitespace(source: &str, mut cursor: usize) -> usize {
    while source
        .as_bytes()
        .get(cursor)
        .is_some_and(u8::is_ascii_whitespace)
    {
        cursor += 1;
    }
    cursor
}

fn balanced_brace_end(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    let mut escaped = false;
    for (relative, character) in source[open..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + relative);
                }
            }
            _ => {}
        }
    }
    None
}

fn normalize_chemistry(source: &str) -> String {
    let isotopes = CHEMISTRY_ISOTOPE.replace_all(source, |captures: &regex::Captures<'_>| {
        let (mass, atomic_number) =
            if let (Some(mass), Some(atomic_number)) = (captures.get(1), captures.get(2)) {
                (mass.as_str(), atomic_number.as_str())
            } else {
                (
                    captures.get(4).expect("alternate isotope mass").as_str(),
                    captures
                        .get(3)
                        .expect("alternate isotope atomic number")
                        .as_str(),
                )
            };
        let element = captures.get(5).expect("isotope element").as_str();
        format!(
            "{}{}{element}",
            positional_unicode_text(mass, true).expect("ASCII isotope mass has superscripts"),
            positional_unicode_text(atomic_number, false)
                .expect("ASCII atomic number has subscripts"),
        )
    });
    let arrows = isotopes
        .replace("<=>", "⇌")
        .replace("<->", "↔")
        .replace("->", "→")
        .replace("<-", "←");

    let characters = arrows.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(arrows.len());
    let mut cursor = 0;
    while cursor < characters.len() {
        if characters[cursor].is_ascii_digit()
            && cursor > 0
            && matches!(characters[cursor - 1], 'A'..='Z' | 'a'..='z' | ')' | ']')
        {
            let start = cursor;
            while characters.get(cursor).is_some_and(char::is_ascii_digit) {
                cursor += 1;
            }
            let digits = characters[start..cursor].iter().collect::<String>();
            output.push_str(&positional_unicode_text(&digits, false).unwrap_or(digits));
            continue;
        }
        output.push(characters[cursor]);
        cursor += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_accepts_structural_latex_without_string_rewrites() {
        for source in [
            r"\frac{a}{b}",
            r"\sqrt[3]{x}",
            r"\left\langle x,y\right\rangle",
            r"\sum_{i=1}^{n}x_i",
            r"\overleftrightarrow{AB}",
            r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
            r"\begin{cases}x&x>0\\-x&x<0\end{cases}",
            r"\newcommand{\sqr}[1]{#1^2}\sqr{x}",
        ] {
            assert!(parse(source, true).is_some(), "failed to parse {source}");
        }
    }

    #[test]
    fn environment_alignment_creates_exactly_one_cell_boundary() {
        let node =
            parse(r"\begin{pmatrix}a&b\\c&d\end{pmatrix}", true).expect("matrix should parse");
        let EqNode::Delimited { content, .. } = node else {
            panic!("matrix delimiters should remain semantic");
        };
        let EqNode::Matrix { rows, .. } = *content else {
            panic!("matrix body should remain a grid");
        };
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.len() == 2));
    }

    #[test]
    fn unknown_commands_fail_instead_of_losing_source() {
        assert!(parse(r"\definitelyunsupported{x}", true).is_none());
    }

    #[test]
    fn compatibility_layer_is_limited_to_explicit_extensions() {
        let source = compatibility_source(
            r"\substack{a\\b}\quad\ce{H2O}\quad\llbracket x\rrbracket\label{ignored}",
        )
        .expect("balanced compatibility input");
        assert!(source.contains(r"\begin{subarray}{c}"));
        assert!(source.contains("H₂O"));
        assert!(source.contains("⟦ x⟧"));
        assert!(!source.contains("label"));
    }
}
