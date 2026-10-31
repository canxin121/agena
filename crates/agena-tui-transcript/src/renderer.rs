//! Transcript renderer driving the terminal frame.

use agena_api::resource::{RunRole, RunStatus, SessionExecutionResource};
use agena_tui::i18n::I18n;
use agena_tui_components::trim_empty_line_edges;
use chrono::{DateTime, Local};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use crate::layout::BODY_PREFIX;
use crate::ui_text;
use crate::{
    RenderedCopySegment, RenderedLine, RenderedTranscriptNode, ToolOutputPreview,
    TranscriptDetailDefaults, TranscriptEntry, TranscriptEntryPart, TranscriptNodeKey,
    TranscriptNodeKind, TranscriptPartContent, TranscriptPointerSelection,
    transcript_spinner_placeholder,
};

mod transcript_ast;
mod transcript_aux;
mod transcript_diff;
mod transcript_render;
mod transcript_text;
mod transcript_tool_summary;

/// Render a workspace patch with the same gutters, colors and wrapping as a
/// file-edit operation in the transcript.
pub fn render_diff_document(text: &str, width: u16) -> Vec<RenderedLine> {
    render_diff_document_with_i18n(text, width, &I18n::english())
}

/// Render a workspace patch with one locale's catalog wording for the labels
/// the renderer generates itself — currently the header of a patch that carries
/// no file path.
pub fn render_diff_document_with_i18n(text: &str, width: u16, i18n: &I18n) -> Vec<RenderedLine> {
    let mut out = Vec::new();
    transcript_aux::push_expanded_diff_text(&mut out, "", text, width, i18n);
    out
}

pub(super) use self::transcript_aux::*;
pub(super) use self::transcript_diff::*;
pub use self::transcript_render::*;
pub use self::transcript_render::{render_entry_export, rewind_message_preview};
pub use self::transcript_text::*;
pub use self::transcript_tool_summary::*;
pub(super) use crate::{
    TableColumnAlignment, fit_table_column_widths, is_markdown_table_header,
    markdown_fence_delimiter, push_multiline, push_table_border, push_wrapped_line,
    push_wrapped_rich_line, sanitize_terminal_text, truncate_display_width, wrap_rich_line,
};

pub(crate) const TOOL_CARD_PREVIEW_LINES: usize = 8;
pub(crate) const TOOL_CARD_PREVIEW_CHARS: usize = 2_500;

pub(crate) fn format_timestamp(timestamp: DateTime<chrono::Utc>) -> String {
    DateTime::<Local>::from(timestamp)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Compact wall-clock time for one transcript event row, e.g. `14:32:05`.
pub(crate) fn format_occurred_time(occurred_at_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(occurred_at_ms)
        .map(|time| DateTime::<Local>::from(time).format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

pub fn style_for_role(role: RunRole) -> Style {
    match role {
        agena_api::resource::RunRole::User => {
            Style::default().fg(agena_tui_components::theme::success_color())
        }
        agena_api::resource::RunRole::Assistant => {
            Style::default().fg(agena_tui_components::theme::accent_color())
        }
        agena_api::resource::RunRole::System => {
            Style::default().fg(agena_tui_components::theme::special_color())
        }
        agena_api::resource::RunRole::Tool => {
            Style::default().fg(agena_tui_components::theme::warning_color())
        }
    }
}

pub fn render_entry_detailed(
    message: &TranscriptEntry,
    width: u16,
    i18n: &I18n,
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) -> RenderedMessageBlock {
    render_entry_detailed_with_interactions(
        message,
        width,
        i18n,
        defaults,
        expansions,
        &std::collections::BTreeMap::new(),
    )
}

/// Like [`render_entry_detailed`], but with inline interaction views for
/// pending user-input parts. The map is keyed by `request_id`; when an
/// expanded pending interaction part has an entry, the renderer draws the
/// native plan + decision rows from the wire request and the live selection
/// snapshot instead of the plain awaiting summary.
#[allow(clippy::too_many_arguments)]
pub fn render_entry_detailed_with_interactions(
    message: &TranscriptEntry,
    width: u16,
    i18n: &I18n,
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
    interactions: &std::collections::BTreeMap<
        String,
        crate::interaction_view::PendingInteractionView,
    >,
) -> RenderedMessageBlock {
    render_entry_detailed_with_progressive_expansion(
        message,
        width,
        i18n,
        defaults,
        expansions,
        &std::collections::BTreeMap::new(),
        interactions,
    )
}

/// Like [`render_entry_detailed_with_interactions`], but allows the caller to
/// reveal a bounded number of older activities from each folded run. A `true`
/// entry in `expansions` means "show all" for the ordinary per-part expansion
/// map.
#[allow(clippy::too_many_arguments)]
pub fn render_entry_detailed_with_progressive_expansion(
    message: &TranscriptEntry,
    width: u16,
    i18n: &I18n,
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
    summary_visible_counts: &std::collections::BTreeMap<TranscriptNodeKey, usize>,
    interactions: &std::collections::BTreeMap<
        String,
        crate::interaction_view::PendingInteractionView,
    >,
) -> RenderedMessageBlock {
    let mut lines = Vec::new();
    let mut nodes = Vec::new();
    if message.role.is_some() {
        push_message_header(&mut lines, message, width, i18n);
    }

    let parts = transcript_message_parts(message);
    if parts.is_empty() {
        let body_start = lines.len();
        lines.push(RenderedLine::dim(format!(
            "{BODY_PREFIX}{}",
            ui_text::t(i18n, "message-empty")
        )));
        nodes.push(RenderedTranscriptNode {
            key: TranscriptNodeKey::Content {
                entry_id: message.id,
                content_id: None,
            },
            kind: TranscriptNodeKind::Message,
            start_line: body_start,
            end_line: lines.len(),
            copy_text: "".into(),
            atomic: true,
            toggleable: false,
            expanded: true,
        });
    } else {
        // A reply entry renders exactly ONE collapsed-run marker row, like
        // Web. Two producers can supply one: server fold markers that omit a
        // page prefix (one per run window) and the client's visible budget
        // for the whole reply. Consecutive assistant runs merge into one
        // entry, so an entry can carry several gaps. The row precedes the
        // first hidden part and counts all hidden parts in the entry.
        let fold_markers = if message.role == Some(RunRole::User) {
            Vec::new()
        } else {
            fold_marker_plans(message, parts)
        };
        let activity_runs = if message.role == Some(RunRole::User) {
            Vec::new()
        } else {
            activity_run_plans(message, parts, expansions, summary_visible_counts)
        };
        let marker = entry_fold_marker(&activity_runs, &fold_markers, expansions);
        // The single informational row counts remote gaps and locally hidden
        // activities together. Reply shortcuts reveal the chronological window
        // without requiring selection of this row.
        let mut part_index = 0_usize;
        let mut run_index = 0_usize;
        while part_index < parts.len() {
            if let Some(run) = activity_runs
                .get(run_index)
                .filter(|run| run.start == part_index)
            {
                if let Some(marker) = marker
                    .as_ref()
                    .filter(|marker| marker.position == part_index)
                {
                    push_entry_fold_marker_row(&mut lines, &mut nodes, marker, i18n, width);
                }
                let mut activity_index = 0_usize;
                for part in &parts[run.start..run.end] {
                    // Invisible blank bridges group the run but never render.
                    if !is_activity_node(part) {
                        continue;
                    }
                    // Run folding is purely positional. Individual Activity
                    // expansion controls that Activity's body only and must not
                    // exempt an old Activity from the collapsed prefix.
                    let hidden = activity_index < run.hidden_prefix_len
                        && !is_pending_activity_interaction(part);
                    activity_index += 1;
                    if hidden {
                        continue;
                    }
                    append_rendered_part_node(
                        message,
                        part,
                        width,
                        &mut lines,
                        &mut nodes,
                        i18n,
                        defaults,
                        expansions,
                        interactions,
                    );
                }
                part_index = run.end;
                run_index += 1;
                continue;
            }

            // Server fold markers are presentation-only rows. The first one
            // owns the entry's merged row; the rest are already counted in it.
            if !fold_markers.is_empty()
                && matches!(
                    transcript_part_content(&parts[part_index]),
                    TranscriptPartContent::Activity(crate::TranscriptActivityContent::Fold { .. })
                )
            {
                if let Some(marker) = marker
                    .as_ref()
                    .filter(|marker| marker.position == part_index)
                {
                    push_entry_fold_marker_row(&mut lines, &mut nodes, marker, i18n, width);
                }
                part_index += 1;
                continue;
            }

            let part = &parts[part_index];
            if let TranscriptPartContent::Text(text) = transcript_part_content(part) {
                append_message_markdown_blocks(
                    message,
                    part,
                    text.text.as_str(),
                    width,
                    &mut lines,
                    &mut nodes,
                    i18n,
                );
                part_index += 1;
                continue;
            }

            append_rendered_part_node(
                message,
                part,
                width,
                &mut lines,
                &mut nodes,
                i18n,
                defaults,
                expansions,
                interactions,
            );
            part_index += 1;
        }
    }

    RenderedMessageBlock { lines, nodes }
}

/// One contiguous activity run of an entry with the client's visible-budget
/// decision for it. `hidden_prefix_len` leading activities collapse into the
/// entry's single marker row.
struct ActivityRunPlan {
    start: usize,
    end: usize,
    key: TranscriptNodeKey,
    hidden_prefix_len: usize,
    hidden_count: usize,
}

/// One server-projected fold marker part of an entry.
struct FoldMarkerPlan {
    position: usize,
    key: TranscriptNodeKey,
    hidden_count: usize,
}

/// The single collapsed-run row of one entry.
struct EntryFoldMarker {
    /// Index into the entry's parts where the row renders.
    position: usize,
    key: TranscriptNodeKey,
    hidden_count: usize,
    /// A server fold tracks its own expansion; the client budget summary is
    /// always collapsed and reveals through visible counts instead.
    expanded: bool,
    /// Kept per producer so the row's navigation semantics do not change.
    atomic: bool,
}

fn activity_run_plans(
    message: &TranscriptEntry,
    parts: &[TranscriptEntryPart],
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
    summary_visible_counts: &std::collections::BTreeMap<TranscriptNodeKey, usize>,
) -> Vec<ActivityRunPlan> {
    // One reply owns one recent-part budget. Server gaps consume positions in
    // that same window, so a loaded boundary before a gap cannot reappear as
    // an apparently recent part.
    let show_all = expansions.iter().any(|(key, expanded)| {
        matches!(key, TranscriptNodeKey::ActivitySummary { .. })
            && key.entry_id() == message.id
            && *expanded
    });
    let visible_count = if show_all {
        usize::MAX
    } else {
        summary_visible_counts
            .iter()
            .filter(|(key, _)| key.entry_id() == message.id)
            .map(|(_, count)| *count)
            .max()
            .unwrap_or(COLLAPSED_ACTIVITY_VISIBLE_COUNT)
    };
    let total = parts.iter().fold(0_usize, |total, part| {
        total.saturating_add(match transcript_part_content(part) {
            TranscriptPartContent::Activity(crate::TranscriptActivityContent::Fold {
                hidden_count,
            }) => *hidden_count,
            _ => usize::from(is_activity_node(part)),
        })
    });
    let mut remaining_hidden = total.saturating_sub(visible_count);
    let mut previous_end = 0;
    let mut plans = Vec::new();
    for (start, end) in activity_runs(parts) {
        for part in &parts[previous_end..start] {
            if let TranscriptPartContent::Activity(crate::TranscriptActivityContent::Fold {
                hidden_count,
            }) = transcript_part_content(part)
            {
                remaining_hidden = remaining_hidden.saturating_sub(*hidden_count);
            }
        }
        previous_end = end;
        let activities = parts[start..end]
            .iter()
            .filter(|part| is_activity_node(part))
            .collect::<Vec<_>>();
        let Some(first) = activities.first() else {
            continue;
        };
        // All activity runs share the reply's chronological window,
        // including session notices injected mid-reply.
        let key = TranscriptNodeKey::ActivitySummary {
            entry_id: message.id,
            first_content_id: first.id,
        };
        let hidden_prefix_len = remaining_hidden.min(activities.len());
        remaining_hidden = remaining_hidden.saturating_sub(activities.len());
        // Keep the pending request itself reachable without revealing all of
        // the unrelated old activities that follow it.
        let hidden_count = activities[..hidden_prefix_len]
            .iter()
            .filter(|part| !is_pending_activity_interaction(part))
            .count();
        plans.push(ActivityRunPlan {
            start,
            end,
            key,
            hidden_prefix_len,
            hidden_count,
        });
    }
    plans
}

fn is_pending_activity_interaction(part: &TranscriptEntryPart<'_>) -> bool {
    matches!(
        &part.content,
        TranscriptPartContent::Activity(crate::TranscriptActivityContent::Operation(tool))
            if !part.status.is_terminal() && tool.has_pending_interaction()
    )
}

fn fold_marker_plans(
    message: &TranscriptEntry,
    parts: &[TranscriptEntryPart],
) -> Vec<FoldMarkerPlan> {
    parts
        .iter()
        .enumerate()
        .filter_map(|(position, part)| match transcript_part_content(part) {
            TranscriptPartContent::Activity(crate::TranscriptActivityContent::Fold {
                hidden_count,
            }) => Some(FoldMarkerPlan {
                position,
                key: TranscriptNodeKey::Activity {
                    entry_id: message.id,
                    content_id: part.id,
                },
                hidden_count: *hidden_count,
            }),
            _ => None,
        })
        .collect()
}

/// The entry's single collapsed-run row: the first server fold when the entry
/// has one, otherwise the first activity run that hides older activities.
/// `None` when every window activity of the entry is already visible.
fn entry_fold_marker(
    runs: &[ActivityRunPlan],
    folds: &[FoldMarkerPlan],
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) -> Option<EntryFoldMarker> {
    if let Some(first) = folds.first() {
        return Some(EntryFoldMarker {
            position: runs
                .iter()
                .find(|run| run.hidden_count > 0)
                .map_or(first.position, |run| run.start.min(first.position)),
            key: first.key.clone(),
            hidden_count: folds.iter().map(|fold| fold.hidden_count).sum::<usize>()
                + runs.iter().map(|run| run.hidden_count).sum::<usize>(),
            expanded: expansions.get(&first.key).copied().unwrap_or(false),
            atomic: false,
        });
    }
    let first_hiding = runs.iter().find(|run| run.hidden_count > 0)?;
    Some(EntryFoldMarker {
        position: first_hiding.start,
        key: first_hiding.key.clone(),
        hidden_count: runs.iter().map(|run| run.hidden_count).sum(),
        // A summary row is progressive: it stays collapsed and reveals
        // through `summary_visible_counts`.
        expanded: false,
        atomic: true,
    })
}

/// Number represented by the entry's merged fold control, including both
/// unloaded server parts and locally hidden activities.
pub fn entry_hidden_part_count(
    entry: &TranscriptEntry<'_>,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
    summary_visible_counts: &std::collections::BTreeMap<TranscriptNodeKey, usize>,
) -> usize {
    if entry.role == Some(RunRole::User) {
        return 0;
    }
    let runs = activity_run_plans(entry, &entry.parts, expansions, summary_visible_counts);
    let folds = fold_marker_plans(entry, &entry.parts);
    entry_fold_marker(&runs, &folds, expansions).map_or(0, |marker| marker.hidden_count)
}

/// Emit the single collapsed-run row of an entry. The row is a UI marker,
/// never real content: copying the collapsed block must copy the visible
/// marker line, never the hidden activities' full text.
fn push_entry_fold_marker_row(
    lines: &mut Vec<RenderedLine>,
    nodes: &mut Vec<RenderedTranscriptNode>,
    marker: &EntryFoldMarker,
    i18n: &I18n,
    width: u16,
) {
    let start_line = lines.len();
    let summary = crate::renderer::transcript_tool_summary::push_fold_marker_row(
        lines,
        i18n,
        marker.hidden_count,
        width,
    );
    nodes.push(RenderedTranscriptNode {
        key: marker.key.clone(),
        kind: TranscriptNodeKind::Activity,
        start_line,
        end_line: lines.len(),
        copy_text: summary.into(),
        atomic: marker.atomic,
        toggleable: false,
        expanded: marker.expanded,
    });
}

/// Loaded activities in the recent window, excluding old pending requests
/// retained separately for interaction.
pub fn entry_visible_activity_count(
    entry: &TranscriptEntry<'_>,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
    summary_visible_counts: &std::collections::BTreeMap<TranscriptNodeKey, usize>,
) -> usize {
    let hidden = activity_run_plans(entry, &entry.parts, expansions, summary_visible_counts)
        .iter()
        .map(|run| run.hidden_prefix_len)
        .sum::<usize>();
    entry
        .parts
        .iter()
        .filter(|part| is_activity_node(part))
        .count()
        .saturating_sub(hidden)
}

/// Activity-summary node keys of one entry: one per contiguous activity run,
/// using the same identity the renderer uses. A single merged marker row
/// controls every run of its entry, so the app raises all of these keys at
/// once when the user reveals older activities.
pub fn entry_activity_summary_keys(entry: &TranscriptEntry<'_>) -> Vec<TranscriptNodeKey> {
    if entry.role == Some(RunRole::User) {
        return Vec::new();
    }
    activity_runs(entry.parts.as_slice())
        .into_iter()
        .filter_map(|(start, _end)| {
            let first = entry.parts.get(start)?;
            Some(TranscriptNodeKey::ActivitySummary {
                entry_id: entry.id,
                first_content_id: first.id,
            })
        })
        .collect()
}

use self::should_suppress_markdown_block;

/// Preserve one Markdown parse context around native embeds (links, footnotes,
/// code/quote boundaries and source semantics remain owned by the host parser).
pub(crate) fn render_inline_reply_document(
    reply: &crate::inline_ui::InlineReply,
    width: u16,
    i18n: &I18n,
) -> Vec<RenderedLine> {
    transcript_text::render_inline_reply_document(reply, width, i18n)
}
