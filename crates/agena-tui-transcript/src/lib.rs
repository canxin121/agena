//! Transport-neutral transcript viewport state.
//!
//! The application owns transcript data and rendering, while this small state
//! value belongs to the TUI layer: it describes how a rendered transcript is
//! positioned without depending on API, domain, or core message types.

use std::ops::Range;

pub use agena_api::message_part::{
    ActivityKindResource, ActivityPartResource, MessagePartDetailResource, MessagePartResource,
    MessageRequestPartResource, OperationBlockResource, OperationPartResource,
    PartExecutionStatusResource, ToolInvocationResource,
};
pub use agena_api::resource::{MessageResource, MessageStatus, SessionExecutionResource};
use ratatui::layout::Rect;

pub mod markdown;
pub mod math;
pub mod navigation;
pub mod render_model;
pub mod renderer;
pub mod selection;
pub mod text;

pub use markdown::*;
pub use math::*;
pub use navigation::*;
pub use render_model::*;
pub use renderer::{
    is_terminal_user_execution_activity, render_message_detailed, render_message_export,
    render_transcript_export_markdown, rewind_message_preview,
};
pub use selection::{normalize_transcript_text_selection, transcript_text_selection_text};
pub use text as ui_text;

#[cfg(test)]
pub(crate) use test_fixtures::TranscriptFixture;

#[cfg(test)]
mod test_fixtures {
    use agena_domain::ExecutionStatus;
    use chrono::{DateTime, Utc};

    use super::{MessagePartResource, MessageRequestPartResource, OperationPartResource};

    pub(crate) struct TranscriptFixture;

    impl TranscriptFixture {
        pub(crate) fn text_part(
            id: i64,
            message_id: i64,
            created_at: DateTime<Utc>,
            status: ExecutionStatus,
            text: impl Into<String>,
        ) -> MessagePartResource {
            Self::text_part_with_flags(id, message_id, created_at, status, text, false)
        }

        pub(crate) fn text_part_with_flags(
            id: i64,
            message_id: i64,
            created_at: DateTime<Utc>,
            status: ExecutionStatus,
            text: impl Into<String>,
            synthetic: bool,
        ) -> MessagePartResource {
            MessagePartResource {
                id,
                message_id,
                part_index: 0,
                status: fixture_part_status(status),
                kind: agena_api::message_part::MessagePartKindResource::Text,
                name: None,
                summary: None,
                has_detail: true,
                operation_id: None,
                created_at,
                content: Some(agena_api::message_part::MessagePartDetailResource::Text(
                    agena_api::message_part::MessageTextPartResource {
                        text: text.into(),
                        synthetic,
                    },
                )),
            }
        }

        pub(crate) fn reasoning_part(
            id: i64,
            message_id: i64,
            created_at: DateTime<Utc>,
            status: ExecutionStatus,
            reasoning: agena_domain::ReasoningPart,
        ) -> MessagePartResource {
            MessagePartResource {
                id,
                message_id,
                part_index: 0,
                status: fixture_part_status(status),
                kind: agena_api::message_part::MessagePartKindResource::Reasoning,
                name: None,
                summary: None,
                has_detail: true,
                operation_id: None,
                created_at,
                content: Some(
                    agena_api::message_part::MessagePartDetailResource::Reasoning(
                        agena_api::message_part::MessageReasoningPartResource {
                            summary: reasoning.summary,
                            raw_content: reasoning.raw_content,
                            encrypted_content: reasoning.encrypted_content,
                        },
                    ),
                ),
            }
        }

        pub(crate) fn operation_part(
            id: i64,
            message_id: i64,
            created_at: DateTime<Utc>,
            status: ExecutionStatus,
            operation: OperationPartResource,
        ) -> MessagePartResource {
            MessagePartResource {
                id,
                message_id,
                part_index: 0,
                status: fixture_part_status(status),
                kind: agena_api::message_part::MessagePartKindResource::Operation,
                name: None,
                summary: None,
                has_detail: true,
                operation_id: None,
                created_at,
                content: Some(
                    agena_api::message_part::MessagePartDetailResource::Operation(Box::new(
                        operation,
                    )),
                ),
            }
        }

        pub(crate) fn activity_part(
            id: i64,
            message_id: i64,
            created_at: DateTime<Utc>,
            status: ExecutionStatus,
            activity: agena_api::message_part::ActivityPartResource,
        ) -> MessagePartResource {
            MessagePartResource {
                id,
                message_id,
                part_index: 0,
                status: fixture_part_status(status),
                kind: agena_api::message_part::MessagePartKindResource::Activity,
                name: Some("activity".to_owned()),
                summary: Some(activity.title.clone()),
                has_detail: true,
                operation_id: Some(activity.activity_id.clone()),
                created_at,
                content: Some(
                    agena_api::message_part::MessagePartDetailResource::Activity(Box::new(
                        activity,
                    )),
                ),
            }
        }

        pub(crate) fn permission_request_part(
            id: i64,
            message_id: i64,
            created_at: DateTime<Utc>,
            status: ExecutionStatus,
            request: agena_api::resource::PermissionRequest,
        ) -> MessagePartResource {
            MessagePartResource {
                id,
                message_id,
                part_index: 0,
                status: fixture_part_status(status),
                kind: agena_api::message_part::MessagePartKindResource::Request,
                name: None,
                summary: None,
                has_detail: true,
                operation_id: None,
                created_at,
                content: Some(agena_api::message_part::MessagePartDetailResource::Request(
                    Box::new(MessageRequestPartResource::Permission {
                        request,
                        reply: None,
                    }),
                )),
            }
        }
    }

    const fn fixture_part_status(
        status: ExecutionStatus,
    ) -> agena_api::message_part::PartExecutionStatusResource {
        match status {
            ExecutionStatus::Pending => {
                agena_api::message_part::PartExecutionStatusResource::Pending
            }
            ExecutionStatus::InProgress => {
                agena_api::message_part::PartExecutionStatusResource::InProgress
            }
            ExecutionStatus::Completed => {
                agena_api::message_part::PartExecutionStatusResource::Completed
            }
            ExecutionStatus::Failed => agena_api::message_part::PartExecutionStatusResource::Failed,
            ExecutionStatus::Cancelled => {
                agena_api::message_part::PartExecutionStatusResource::Cancelled
            }
        }
    }
}

/// Private marker used while a live assistant/tool row is waiting for its
/// next render update. The app maps it to its spinner frame at the adapter.
pub const fn transcript_spinner_placeholder() -> &'static str {
    "\u{e000}"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptViewport {
    pub top: usize,
    pub follow_tail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct TranscriptViewportEffect {
    pub top: usize,
    pub follow_tail: bool,
    pub request_older_messages: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptEffect {
    LoadOlderMessages {
        session_id: i64,
        cursor: Option<String>,
    },
    RefreshSession {
        session_id: i64,
    },
    CopyText {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePartCheckpoint {
    pub message_id: i64,
    pub part_id: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePartDelta {
    pub message_id: i64,
    pub part_id: i64,
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptLiveUpdateKind {
    UserMessageAppended { message_id: i64 },
    MessagePartCheckpointed(MessagePartCheckpoint),
    MessagePartDelta(MessagePartDelta),
    AssistantMessageFinished,
    RefreshRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptLiveUpdate {
    pub session_id: i64,
    pub sequence: Option<i64>,
    pub kind: TranscriptLiveUpdateKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSnapshot {
    pub session_id: Option<i64>,
    pub sequence: Option<i64>,
    pub follow_tail: bool,
    pub search_query: String,
    pub current_node_expanded: bool,
}

/// App-independent transcript state. Message materialization remains owned by
/// the app adapter; this state only records presentation and live-update
/// identity so it can be tested without Runtime or Backend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptState {
    pub session_id: Option<i64>,
    pub sequence: Option<i64>,
    pub viewport: TranscriptViewport,
    pub search_query: String,
    pub current_node_expanded: bool,
    pub selected_copy_text: Option<String>,
}

impl TranscriptState {
    pub fn snapshot(&self) -> TranscriptSnapshot {
        TranscriptSnapshot {
            session_id: self.session_id,
            sequence: self.sequence,
            follow_tail: self.viewport.follow_tail,
            search_query: self.search_query.clone(),
            current_node_expanded: self.current_node_expanded,
        }
    }

    pub fn reduce(&mut self, action: TranscriptAction) -> Option<TranscriptEffect> {
        match action {
            TranscriptAction::Reset => self.viewport.reset(),
            TranscriptAction::ScrollTo(top) => self.viewport.scroll_to(top),
            TranscriptAction::FollowTail => self.viewport.follow_tail(),
            TranscriptAction::MoveUp | TranscriptAction::MoveDown => {}
            TranscriptAction::Search { query } => self.search_query = query,
            TranscriptAction::ToggleCurrentNode => {
                self.current_node_expanded = !self.current_node_expanded
            }
            TranscriptAction::CopyCurrentSelection => {
                return self
                    .selected_copy_text
                    .clone()
                    .map(|text| TranscriptEffect::CopyText { text });
            }
        }
        None
    }

    pub fn set_copy_text(&mut self, text: Option<String>) {
        self.selected_copy_text = text;
    }

    pub fn apply_live_update(&mut self, update: &TranscriptLiveUpdate) -> bool {
        if self.session_id != Some(update.session_id) {
            return false;
        }
        if let (Some(current), Some(incoming)) = (self.sequence, update.sequence)
            && incoming <= current
        {
            return false;
        }
        self.sequence = update.sequence.or(self.sequence);
        if matches!(update.kind, TranscriptLiveUpdateKind::RefreshRequested) {
            self.viewport.follow_tail();
        }
        true
    }

    pub fn set_session(&mut self, session_id: Option<i64>) {
        self.session_id = session_id;
        self.sequence = None;
        self.viewport.reset();
        self.search_query.clear();
        self.current_node_expanded = false;
        self.selected_copy_text = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
            request_older_messages: false,
        }
    }

    /// Decide whether reaching the history edge should trigger an application
    /// request for older transcript data.
    pub fn history_effect(&self, has_more: bool, loading: bool) -> TranscriptViewportEffect {
        TranscriptViewportEffect {
            top: self.top,
            follow_tail: self.follow_tail,
            request_older_messages: has_more && !loading && self.top <= 2,
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

#[cfg(test)]
mod tests {
    use super::{
        TranscriptAction, TranscriptPointerGesture, TranscriptTextPosition, TranscriptViewport,
        project_view, scroll_for_thumb, scrollbar_area, scrollbar_metrics, visible_range,
    };
    use ratatui::layout::Rect;

    #[test]
    fn viewport_actions_are_transport_neutral() {
        let mut viewport = TranscriptViewport::default();
        let effect = viewport.reduce(TranscriptAction::ScrollTo(12));
        assert_eq!(viewport.top, 12);
        assert!(!viewport.follow_tail);
        assert_eq!(effect.top, 12);
        viewport.reduce(TranscriptAction::FollowTail);
        assert!(viewport.follow_tail);
        viewport.reduce(TranscriptAction::Reset);
        assert_eq!(viewport, TranscriptViewport::default());
        assert_eq!(visible_range(&viewport, 20, 5), 0..5);
        assert!(viewport.history_effect(true, false).request_older_messages);
        assert_eq!(
            project_view(&viewport, 20, 5),
            super::TranscriptView {
                visible: 0..5,
                follow_tail: true,
            }
        );
    }

    #[test]
    fn pointer_gesture_and_selection_are_presentation_only() {
        let anchor = TranscriptTextPosition { line: 3, column: 5 };
        let mut gesture = TranscriptPointerGesture::new(anchor);
        gesture.update(TranscriptTextPosition { line: 4, column: 2 }, true);
        assert!(gesture.dragged);
        let selection = gesture.selection();
        assert_eq!(selection.cell_range_for_line(2), None);
        assert_eq!(selection.cell_range_for_line(3), Some(5..usize::MAX));
        assert_eq!(selection.cell_range_for_line(4), Some(0..3));
    }

    #[test]
    fn scrollbar_geometry_reaches_both_ends_and_round_trips_the_middle() {
        let top = scrollbar_metrics(100, 20, 20, 0).expect("scrollable transcript");
        assert_eq!(top.max_scroll, 80);
        assert_eq!(top.thumb_start, 0);
        assert_eq!(top.thumb_len, 4);
        assert_eq!(top.thumb_travel, 16);

        let middle = scrollbar_metrics(100, 20, 20, 40).expect("scrollable transcript");
        assert_eq!(middle.thumb_start, 8);
        assert_eq!(scroll_for_thumb(middle, 8, 0), 40);

        let bottom = scrollbar_metrics(100, 20, 20, 80).expect("scrollable transcript");
        assert_eq!(bottom.thumb_start, bottom.thumb_travel);
        assert_eq!(
            scroll_for_thumb(bottom, bottom.thumb_travel, 0),
            bottom.max_scroll
        );
    }

    #[test]
    fn scrollbar_is_absent_when_every_line_fits_and_uses_the_host_margin() {
        assert!(scrollbar_metrics(20, 20, 20, 0).is_none());
        assert!(scrollbar_metrics(0, 20, 20, 0).is_none());
        assert!(scrollbar_metrics(100, 20, 0, 0).is_none());
        assert_eq!(
            scrollbar_area(Rect::new(0, 0, 80, 30), Rect::new(1, 3, 78, 20)),
            Rect::new(79, 3, 1, 20),
        );
    }
}
