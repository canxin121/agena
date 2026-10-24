//! Transport-neutral transcript viewport state.
//!
//! The application owns transcript data and rendering, while this small state
//! value belongs to the TUI layer: it describes how a rendered transcript is
//! positioned without depending on API, domain, or core message types.

use std::ops::Range;

pub use agena_api::part::PartExecutionStatusResource;
use ratatui::layout::Rect;

mod activity_presentation;
pub mod content;
pub mod error_part;
pub mod interaction_view;
mod layout;
pub mod markdown;
pub mod math;
pub mod navigation;
mod part_headline;
pub mod parts;
pub mod render_model;
pub mod renderer;
pub mod selection;
pub mod text;

pub use interaction_view::*;
pub use markdown::*;
pub use math::*;
pub use navigation::*;
pub use part_headline::first_content_line as content_headline_preview;
pub use parts::{
    TranscriptFold, folds_from_run_window, part_state_is_terminal, parts_entries,
    parts_entries_with_folds, parts_have_non_terminal_runs,
};
pub use render_model::*;
pub use renderer::{
    COLLAPSED_ACTIVITY_VISIBLE_COUNT, RenderedMessageBlock, entry_activity_summary_keys,
    entry_hidden_part_count, render_diff_document, render_diff_document_with_i18n,
    render_entry_detailed, render_entry_detailed_with_interactions,
    render_entry_detailed_with_progressive_expansion, render_entry_export,
    render_entry_part_export, render_markdown_document, render_markdown_document_with_i18n,
    render_parts_export_markdown, rewind_message_preview,
};
pub use selection::{normalize_transcript_text_selection, transcript_text_selection_text};
pub use text as ui_text;

/// Private marker used while a live assistant/tool row is waiting for its
/// next render update. The app maps it to its spinner frame at the adapter.
pub const fn transcript_spinner_placeholder() -> &'static str {
    "\u{e000}"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Viewport of the transcript view.
pub struct TranscriptViewport {
    pub top: usize,
    pub follow_tail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Action performed on the transcript view.
pub enum TranscriptAction {
    Reset,
    ScrollTo(usize),
    FollowTail,
    MoveUp,
    MoveDown,
    Search { query: String },
    ToggleCurrentNode,
    CopyCurrentSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Effect produced by a transcript action.
pub struct TranscriptViewportEffect {
    pub top: usize,
    pub follow_tail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// The transcript view.
pub struct TranscriptView {
    pub visible: Range<usize>,
    pub follow_tail: bool,
}

/// Geometry for a terminal transcript scrollbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptScrollbarMetrics {
    pub max_scroll: usize,
    pub thumb_start: usize,
    pub thumb_len: usize,
    pub thumb_travel: usize,
}

/// Projects transcript line geometry into a scrollbar thumb.
pub fn scrollbar_metrics(
    total_lines: usize,
    viewport_lines: usize,
    track_lines: usize,
    scroll: usize,
) -> Option<TranscriptScrollbarMetrics> {
    let viewport_lines = viewport_lines.min(total_lines);
    let max_scroll = total_lines.saturating_sub(viewport_lines);
    if max_scroll == 0 || track_lines == 0 {
        return None;
    }

    let rounding_divide = |numerator: usize, denominator: usize| {
        numerator
            .saturating_add(denominator / 2)
            .checked_div(denominator)
            .unwrap_or(0)
    };
    let thumb_len = rounding_divide(viewport_lines.saturating_mul(track_lines), total_lines)
        .clamp(1, track_lines);
    let thumb_travel = track_lines.saturating_sub(thumb_len);
    let thumb_start = rounding_divide(
        scroll.min(max_scroll).saturating_mul(track_lines),
        total_lines,
    )
    .min(thumb_travel);

    Some(TranscriptScrollbarMetrics {
        max_scroll,
        thumb_start,
        thumb_len,
        thumb_travel,
    })
}

/// Converts a scrollbar pointer line back to a transcript scroll position.
pub fn scroll_for_thumb(
    metrics: TranscriptScrollbarMetrics,
    pointer_line: usize,
    grab_offset: usize,
) -> usize {
    if metrics.thumb_travel == 0 {
        return 0;
    }
    let thumb_start = pointer_line
        .saturating_sub(grab_offset)
        .min(metrics.thumb_travel);
    thumb_start
        .saturating_mul(metrics.max_scroll)
        .saturating_add(metrics.thumb_travel / 2)
        / metrics.thumb_travel
}

/// Reserves the host's rightmost column for a transcript scrollbar.
pub fn scrollbar_area(host: Rect, body: Rect) -> Rect {
    if host.width == 0 || body.height == 0 {
        return Rect::default();
    }
    Rect {
        x: host.x.saturating_add(host.width.saturating_sub(1)),
        y: body.y,
        width: 1,
        height: body.height,
    }
}

/// Whether a rendered transcript row accepts a character range or must be
/// selected as one semantic unit (for example, a formula/image line box).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TranscriptPointerSelection {
    #[default]
    Character,
    SemanticUnit,
}

/// A terminal-cell position in a fully rendered transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TranscriptTextPosition {
    pub line: usize,
    pub column: usize,
}

/// A committed inclusive terminal-cell text range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptTextSelection {
    pub anchor: TranscriptTextPosition,
    pub head: TranscriptTextPosition,
}

/// Presentation-only gesture state. The application decides how a gesture
/// maps onto rendered transcript data and performs clipboard effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptPointerGesture {
    pub anchor: TranscriptTextPosition,
    pub head: TranscriptTextPosition,
    pub dragged: bool,
}

/// Project viewport state into the line range that a terminal view should
/// inspect. Rendering remains owned by the application, while this projection
/// keeps viewport arithmetic in the TUI contract.
pub fn visible_range(
    viewport: &TranscriptViewport,
    total_lines: usize,
    visible_lines: usize,
) -> Range<usize> {
    let start = viewport.top.min(total_lines);
    let end = start.saturating_add(visible_lines).min(total_lines);
    start..end
}

pub fn project_view(
    viewport: &TranscriptViewport,
    total_lines: usize,
    visible_lines: usize,
) -> TranscriptView {
    TranscriptView {
        visible: visible_range(viewport, total_lines, visible_lines),
        follow_tail: viewport.follow_tail,
    }
}

impl Default for TranscriptViewport {
    fn default() -> Self {
        Self {
            top: 0,
            follow_tail: true,
        }
    }
}

impl TranscriptViewport {
    /// Apply a transcript action and return the resulting presentation effect.
    pub fn reduce(&mut self, action: TranscriptAction) -> TranscriptViewportEffect {
        match action {
            TranscriptAction::Reset => self.reset(),
            TranscriptAction::ScrollTo(top) => self.scroll_to(top),
            TranscriptAction::FollowTail => self.follow_tail(),
            TranscriptAction::MoveUp
            | TranscriptAction::MoveDown
            | TranscriptAction::Search { .. }
            | TranscriptAction::ToggleCurrentNode
            | TranscriptAction::CopyCurrentSelection => {}
        }
        TranscriptViewportEffect {
            top: self.top,
            follow_tail: self.follow_tail,
        }
    }

    /// Reset the viewport when switching to a different transcript.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Mark a user-directed scroll position and stop following new content.
    pub fn scroll_to(&mut self, top: usize) {
        self.top = top;
        self.follow_tail = false;
    }

    /// Follow the rendered transcript tail on the next layout pass.
    pub fn follow_tail(&mut self) {
        self.follow_tail = true;
    }
}

impl TranscriptPointerGesture {
    pub fn new(anchor: TranscriptTextPosition) -> Self {
        Self {
            anchor,
            head: anchor,
            dragged: false,
        }
    }

    pub fn update(&mut self, head: TranscriptTextPosition, drag_event: bool) {
        self.dragged |= drag_event || head != self.anchor;
        self.head = head;
    }

    pub fn selection(self) -> TranscriptTextSelection {
        TranscriptTextSelection {
            anchor: self.anchor,
            head: self.head,
        }
    }
}

impl TranscriptTextSelection {
    /// Return the selected display-cell interval for one logical row. Endpoints
    /// are inclusive, matching terminal selection behavior.
    pub fn cell_range_for_line(self, line: usize) -> Option<Range<usize>> {
        let (start, end) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        if line < start.line || line > end.line {
            return None;
        }

        let range_start = if line == start.line { start.column } else { 0 };
        let range_end = if line == end.line {
            end.column.saturating_add(1)
        } else {
            usize::MAX
        };
        Some(range_start..range_end)
    }
}

pub mod display_types;
pub use display_types::TranscriptRun;
