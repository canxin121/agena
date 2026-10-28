//! Native, cell-based inline replies. The content tree is preserved until layout;
//! Markdown is a copy/export projection, never the interactive terminal surface.
mod charts;
mod layout;
mod nodes;
#[cfg(test)]
mod tests;

use crate::{RenderedLine, RenderedTranscriptControl, TranscriptControlAction};
use agena_domain::{InlineBlock, InlineDocument, InlinePhase, InlineUiReferences, UiState};
use agena_tui::i18n::I18n;
use agena_tui_components::theme;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Default)]
pub struct InlinePresentation {
    /// Local disclosure/legend state. Never transmitted to the model or server.
    pub open: BTreeSet<String>,
    pub data: BTreeSet<String>,
    pub hidden_series: BTreeMap<String, BTreeSet<usize>>,
}
#[derive(Debug, Clone, Default)]
pub struct InlineReply {
    pub source: String,
    pub documents: InlineUiReferences,
    pub states: BTreeMap<String, BTreeMap<String, UiState>>,
    pub presentation: BTreeMap<String, BTreeMap<String, InlinePresentation>>,
    pub phases: BTreeMap<String, (u64, InlinePhase)>,
    pub streaming: bool,
    pub focused: bool,
}
impl InlineReply {
    pub fn copy_text(&self) -> String {
        agena_domain::expand_inline_references_with(
            &self.source,
            &self.documents,
            self.streaming,
            |reference, document| {
                document
                    .to_markdown_with_states(self.states.get(reference).unwrap_or(&BTreeMap::new()))
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineControl {
    pub reference: String,
    pub block: String,
    pub node: String,
    pub event: InlineEvent,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineEvent {
    Open,
    Reset,
    Activate,
    Choose(String),
    Task(String),
    Slider,
    Details,
    Data,
    Series(usize),
}

pub fn render_reply(reply: &InlineReply, width: u16, i18n: &I18n) -> Vec<RenderedLine> {
    crate::renderer::render_inline_reply_document(reply, width.max(1), i18n)
}

pub fn render_document(
    document: &InlineDocument,
    reference: &str,
    reply: &InlineReply,
    width: u16,
    i18n: &I18n,
) -> Vec<RenderedLine> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let title = if document.title.is_empty() {
        i18n.text("inline-ui-title")
    } else {
        document.title.clone()
    };
    let status = reply
        .phases
        .get(reference)
        .map(|(revision, phase)| {
            format!(
                "r{revision} · {}",
                i18n.text(match phase {
                    InlinePhase::Working => "inline-ui-working",
                    InlinePhase::Complete => "inline-ui-complete",
                    InlinePhase::Error => "inline-ui-error",
                })
            )
        })
        .unwrap_or_default();
    let header = layout::split_label(
        &format!("◆ {title}"),
        &status,
        width,
        Style::default().add_modifier(Modifier::BOLD),
        theme::muted_style(),
    );
    rows.push(header);
    let hint_start = rows.len();
    if !reply.focused {
        // A visible entry point is part of every embed, not a hidden slash command.
        let mut hint = layout::text(
            &format!(
                "[ {} ]  {}",
                i18n.text("inline-ui-open"),
                i18n.text("inline-ui-inline-help")
            ),
            width,
            theme::muted_style(),
        );
        let label_width = format!("[ {} ]", i18n.text("inline-ui-open"))
            .width()
            .min(usize::from(width));
        if let Some(line) = hint.first_mut() {
            line.controls.push(RenderedTranscriptControl {
                columns: 0..label_width,
                action: TranscriptControlAction::InlineUi(InlineControl {
                    reference: reference.into(),
                    block: String::new(),
                    node: String::new(),
                    event: InlineEvent::Open,
                }),
                enabled: true,
            });
        }
        rows.extend(hint);
    }
    let hint_end = rows.len();
    rows.push(layout::rule(width));
    for block in &document.blocks {
        let state = reply
            .states
            .get(reference)
            .and_then(|states| states.get(block.id()))
            .cloned()
            .unwrap_or_default();
        let presentation = reply
            .presentation
            .get(reference)
            .and_then(|states| states.get(block.id()))
            .cloned()
            .unwrap_or_default();
        let context = Context {
            reference,
            block: block.id(),
            presentation: &presentation,
            i18n,
        };
        let content = match block {
            InlineBlock::Panel { program, .. } => match program.render(&state) {
                Ok(view) => nodes::render_node(&view.root, width, &context),
                Err(error) => {
                    let mut rows =
                        layout::text(&error, width, Style::default().fg(theme::danger_color()));
                    rows.extend(layout::interactive(
                        layout::text(
                            &format!("[ {} ]", i18n.text("inline-ui-reset")),
                            width,
                            tone("primary"),
                        ),
                        context.control("", InlineEvent::Reset),
                        true,
                    ));
                    rows
                }
            },
            _ => nodes::render_static(block, width, &context),
        };
        if !rows.last().is_some_and(|line| line.text.trim().is_empty()) {
            rows.push(RenderedLine::plain("", Style::default()));
        }
        rows.extend(content);
    }
    if !rows.iter().flat_map(|row| &row.controls).any(|control| {
        matches!(&control.action,
        TranscriptControlAction::InlineUi(target) if target.event != InlineEvent::Open)
    }) {
        rows.drain(hint_start..hint_end);
    }
    rows
}

struct Context<'a> {
    reference: &'a str,
    block: &'a str,
    presentation: &'a InlinePresentation,
    i18n: &'a I18n,
}
impl Context<'_> {
    fn control(&self, node: &str, event: InlineEvent) -> TranscriptControlAction {
        TranscriptControlAction::InlineUi(InlineControl {
            reference: self.reference.into(),
            block: self.block.into(),
            node: node.into(),
            event,
        })
    }
}
fn display(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => {
            if number.is_f64() {
                number
                    .as_f64()
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            } else {
                number.to_string()
            }
        }
        Value::Bool(value) => value.to_string(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}
fn tone(value: &str) -> Style {
    Style::default().fg(match value {
        "success" => theme::success_color(),
        "warning" => theme::warning_color(),
        "danger" => theme::danger_color(),
        "info" => theme::info_color(),
        "primary" => theme::accent_color(),
        _ => theme::accent_color(),
    })
}

/// Prefix or grid composition must shift pointer geometry with the cells.
pub fn prefix_rows(rows: Vec<RenderedLine>, prefix: &str) -> Vec<RenderedLine> {
    rows.into_iter()
        .map(|mut row| {
            let offset = prefix.width();
            let mut spans = vec![Span::raw(prefix.to_owned())];
            spans.extend(
                row.rich_line
                    .take()
                    .unwrap_or_else(|| Line::styled(row.text.clone(), row.style))
                    .spans,
            );
            row.text = format!("{prefix}{}", row.text);
            row.rich_line = Some(Line::from(spans));
            row.copy_column += offset;
            for segment in &mut row.copy_segments {
                segment.display_column += offset;
            }
            for control in &mut row.controls {
                control.columns = (control.columns.start + offset)..(control.columns.end + offset);
            }
            for math in &mut row.math {
                math.column = math.column.saturating_add(offset as u16);
            }
            row
        })
        .collect()
}
