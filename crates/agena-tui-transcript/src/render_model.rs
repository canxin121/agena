//! Transcript render model: lines, blocks, and layout.

use agena_api::{
    message_part::{
        MessageAttachmentPartResource, MessageErrorPartResource, MessageHookPartResource,
        MessagePartDetailResource, MessageReasoningPartResource, MessageRequestPartResource,
        MessageSkillReferencePartResource, MessageTextPartResource, OperationPartResource,
        PartExecutionStatusResource,
    },
    resource::{MessageResource, MessageRole, MessageStatus},
};
use agena_domain::{
    ActivityId, ActivityPayload, AssistantReplyId, TextSegmentActivity, TextSegmentId, TurnId,
};
use agena_tui_components::ThemePalette;
use chrono::{DateTime, Utc};
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
};
use std::time::Instant;

use agena_tui_media::{MathLinePlacement, TranscriptMathPlacement};

use crate::{
    RenderedTranscriptNode, TranscriptBlockCursor, TranscriptPointerSelection,
    TranscriptTextPosition, TranscriptTextSelection,
};

/// Stable identity of one top-level transcript entry.
///
/// User turns and assistant-reply anchors are the canonical conversation
/// identity. A stored message id appears only in views whose business object
/// is still a stored provider-history message (for example rewind previews);
/// it is never synthesized for a canonical snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TranscriptEntryId {
    TurnInput(TurnId),
    PendingTurn(u64),
    /// One user-visible canonical assistant reply.
    AssistantReply(AssistantReplyId),
    SessionActivity(ActivityId),
    StoredMessage(i64),
}

/// Stable identity of one ordered node inside a transcript entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TranscriptContentId {
    /// Presentation identity for one editor-like user document. Its nested
    /// nodes retain the canonical segment and Activity identities.
    TurnDocument(TurnId),
    PendingDocument(u64),
    Text(TextSegmentId),
    Activity(ActivityId),
    /// Stable presentation identity for the terminal outcome of a reply.
    /// The outcome is derived from reply state and is not a synthetic
    /// persisted Activity.
    AssistantReplyLifecycle(AssistantReplyId),
    StoredPart(i64),
}

#[derive(Debug, Clone)]
/// A part of a transcript entry.
pub struct TranscriptEntryPart<'a> {
    pub id: TranscriptContentId,
    pub status: PartExecutionStatusResource,
    pub content: TranscriptPartContent<'a>,
}

#[derive(Debug, Clone)]
/// Content of a transcript part.
pub enum TranscriptPartContent<'a> {
    UserDocument(TranscriptUserDocument),
    Text(MessageTextPartResource),
    Activity(TranscriptActivityContent<'a>),
}

#[derive(Debug, Clone)]
/// Content of a transcript activity.
pub enum TranscriptActivityContent<'a> {
    Canonical(&'a ActivityPayload),
    /// Projected interstitial reply body text. The snapshot projection turns
    /// every non-final body segment into a collapsible TextSegment activity
    /// without persisting an ActivityNode, so the payload is owned here (it
    /// cannot borrow from a synthetic, function-local payload). Renderers
    /// treat it exactly like `Canonical` of a persisted TextSegment.
    TextSegment(Box<TextSegmentActivity>),
    Reasoning(MessageReasoningPartResource),
    Attachment(MessageAttachmentPartResource),
    SkillReference(MessageSkillReferencePartResource),
    Error(MessageErrorPartResource),
    Operation(Box<OperationPartResource>),
    Hook(Box<MessageHookPartResource>),
    AssistantReplyLifecycle(TranscriptAssistantReplyLifecycle),
    Request(Box<MessageRequestPartResource>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Lifecycle of an assistant reply in the transcript.
pub enum TranscriptAssistantReplyLifecycle {
    Running,
    Completed,
    /// Terminal failure. Carries the structured public failure projection
    /// (when the runtime persisted one) so renderers can show a readable
    /// summary in the collapsed row and a rich, expandable detail view.
    Failed {
        problem: Option<agena_failure::UserProblem>,
    },
    Cancelled,
}

#[derive(Debug, Clone)]
/// A user document in the transcript.
pub struct TranscriptUserDocument {
    pub nodes: Vec<TranscriptUserDocumentNode>,
}

impl TranscriptUserDocument {
    pub fn plain_text(&self) -> String {
        self.nodes
            .iter()
            .map(|node| match node {
                TranscriptUserDocumentNode::Text { text, .. } => text.as_str(),
                TranscriptUserDocumentNode::Activity { placeholder, .. } => placeholder.as_str(),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
/// A node of a transcript user document.
pub enum TranscriptUserDocumentNode {
    Text {
        id: Option<TextSegmentId>,
        text: String,
    },
    Activity {
        id: ActivityId,
        placeholder: String,
        style: TranscriptUserActivityStyle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Style of a user activity in the transcript.
pub enum TranscriptUserActivityStyle {
    Resource,
    Skill,
    TextArtifact,
    Other,
}

#[derive(Debug, Clone)]
/// Presentation of a transcript activity.
pub struct TranscriptActivityPresentation {
    pub title: String,
    pub summary: String,
    /// Structured, already-public failure projection. Renderers derive text
    /// from this value and cannot accept an arbitrary diagnostic string.
    pub problem: Option<agena_failure::UserProblem>,
}
impl From<MessagePartDetailResource> for TranscriptPartContent<'static> {
    fn from(content: MessagePartDetailResource) -> Self {
        match content {
            MessagePartDetailResource::Text(value) => Self::Text(value),
            MessagePartDetailResource::Reasoning(value) => {
                Self::Activity(TranscriptActivityContent::Reasoning(value))
            }
            MessagePartDetailResource::Attachment(value) => {
                Self::Activity(TranscriptActivityContent::Attachment(value))
            }
            MessagePartDetailResource::SkillReference(value) => {
                Self::Activity(TranscriptActivityContent::SkillReference(value))
            }
            MessagePartDetailResource::Error(value) => {
                Self::Activity(TranscriptActivityContent::Error(value))
            }
            MessagePartDetailResource::Operation(value) => {
                Self::Activity(TranscriptActivityContent::Operation(value))
            }
            MessagePartDetailResource::Hook(value) => {
                Self::Activity(TranscriptActivityContent::Hook(Box::new(value)))
            }
            MessagePartDetailResource::Request(value) => {
                Self::Activity(TranscriptActivityContent::Request(value))
            }
        }
    }
}

#[derive(Debug, Clone)]
/// A transcript entry.
pub struct TranscriptEntry<'a> {
    pub id: TranscriptEntryId,
    /// Message role when this entry is a Turn/Response. Session-owned
    /// activities are top-level Activity entries and therefore have no role.
    pub role: Option<MessageRole>,
    pub state: MessageStatus,
    pub created_at: DateTime<Utc>,
    pub parts: Vec<TranscriptEntryPart<'a>>,
}

impl<'a> From<&'a MessageResource> for TranscriptEntry<'a> {
    fn from(message: &MessageResource) -> Self {
        Self {
            id: TranscriptEntryId::StoredMessage(message.id),
            role: Some(message.role),
            state: message.state,
            created_at: message.created_at,
            parts: message
                .parts
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter_map(|part| {
                    let id = match (part.segment_id, part.activity_id) {
                        (Some(segment_id), None) => TranscriptContentId::Text(segment_id),
                        (None, Some(activity_id)) => TranscriptContentId::Activity(activity_id),
                        (None, None) => TranscriptContentId::StoredPart(part.id),
                        (Some(_), Some(_)) => return None,
                    };
                    Some(TranscriptEntryPart {
                        id,
                        status: part.status,
                        content: part.content.clone()?.into(),
                    })
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Preview of a tool output.
pub struct ToolOutputPreview {
    pub text: String,
    pub omitted_lines: usize,
}

#[derive(Debug, Clone)]
/// A fully rendered transcript.
pub struct RenderedTranscript {
    pub width: u16,
    pub palette: ThemePalette,
    pub remote_image_generation: u64,
    pub lines: Vec<RenderedLine>,
    pub search_matches: Vec<usize>,
    pub nodes: Vec<RenderedTranscriptNode>,
    pub line_nodes: Vec<Option<usize>>,
    pub math: Vec<TranscriptMathPlacement>,
}

#[derive(Debug, Clone)]
/// A rendered line of the transcript.
pub struct RenderedLine {
    pub text: String,
    /// Clipboard-facing text for this rendered row. This deliberately omits
    /// layout-only prefixes such as transcript indentation, quote rails, and
    /// card chrome. Rich semantic nodes can still override row copying with
    /// their node-level `copy_text`.
    pub copy_text: String,
    /// Display-cell column at which `copy_text` begins in `text`. Mouse
    /// selections use this to translate terminal coordinates into the clean
    /// clipboard projection without copying layout prefixes.
    pub copy_column: usize,
    /// Optional non-contiguous clipboard projection for layouts whose visual
    /// chrome cannot be described by one column offset (notably tables and
    /// mixed inline graphics). Empty means the simple `copy_text` projection
    /// above remains authoritative.
    pub copy_segments: Vec<RenderedCopySegment>,
    /// Optional semantic row shared by one or more terminal rows. Code lines,
    /// table rows, and inline graphic line boxes use this when one logical row
    /// wraps or occupies several display rows. UI-only borders leave it unset.
    pub navigation_unit: Option<usize>,
    /// Clipboard text for the complete semantic row. This is deliberately
    /// separate from `copy_text`, which remains the projection for just this
    /// terminal row during a free-form pointer selection.
    pub navigation_copy_text: String,
    /// Pointer selection policy is deliberately independent from keyboard
    /// navigation. Code and table rows have a `navigation_unit` but remain
    /// character-selectable; formulas and other graphical line boxes opt into
    /// semantic-unit selection because terminal cells cannot represent a
    /// meaningful partial image selection.
    pub pointer_selection: TranscriptPointerSelection,
    pub style: Style,
    pub rich_line: Option<Line<'static>>,
    pub math: Vec<MathLinePlacement>,
}

impl RenderedLine {
    pub fn plain(text: impl Into<String>, style: Style) -> Self {
        let text = text.into();
        Self {
            rich_line: Some(Line::from(Span::styled(text.clone(), style))),
            copy_text: text.clone(),
            text,
            copy_column: 0,
            copy_segments: Vec::new(),
            navigation_unit: None,
            navigation_copy_text: String::new(),
            pointer_selection: TranscriptPointerSelection::Character,
            style,
            math: Vec::new(),
        }
    }

    pub fn rich(line: Line<'static>) -> Self {
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let style = line.style;
        Self {
            copy_text: text.clone(),
            text,
            copy_column: 0,
            copy_segments: Vec::new(),
            navigation_unit: None,
            navigation_copy_text: String::new(),
            pointer_selection: TranscriptPointerSelection::Character,
            style,
            rich_line: Some(line),
            math: Vec::new(),
        }
    }

    pub fn with_copy_projection(
        mut self,
        copy_text: impl Into<String>,
        copy_column: usize,
    ) -> Self {
        self.copy_text = copy_text.into();
        self.copy_column = copy_column;
        self
    }

    pub fn with_copy_segments(mut self, segments: Vec<RenderedCopySegment>) -> Self {
        self.copy_segments = segments;
        self
    }

    pub fn with_navigation_unit(
        mut self,
        navigation_unit: usize,
        copy_text: impl Into<String>,
    ) -> Self {
        self.navigation_unit = Some(navigation_unit);
        self.navigation_copy_text = copy_text.into();
        self
    }

    pub fn replace_content_preserving_math(&mut self, mut replacement: Self) {
        replacement.math = std::mem::take(&mut self.math);
        *self = replacement;
    }

    pub fn dim(text: impl Into<String>) -> Self {
        Self::plain(
            text,
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A copyable segment of rendered text.
pub struct RenderedCopySegment {
    pub display_column: usize,
    pub text: String,
    pub separator_before: String,
}

#[derive(Debug, Clone, Copy)]
/// Default detail settings of the transcript.
pub struct TranscriptDetailDefaults {
    pub activity_expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Direction of a transcript move.
pub enum TranscriptMoveDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, Default)]
/// Cache of transcript layout.
pub struct LayoutCache {
    pub transcript_body: Rect,
    pub transcript_scrollbar: Rect,
    /// Full terminal area of the most recent frame. Route and overlay key
    /// handlers use it to compute height-aware page steps for their dialogs.
    pub overlay_area: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Scrollbar drag state of the transcript.
pub struct TranscriptScrollbarDrag {
    pub grab_offset: usize,
}

/// Gesture metadata only: the selected line/block still lives exclusively in
/// `TranscriptInteraction`. Terminals do not report click counts, so the app
/// remembers the last completed click long enough to recognize a double click.
#[derive(Debug, Clone, Copy)]
pub struct TranscriptClick {
    pub line: usize,
    pub at: Instant,
}

/// Stable semantic attachment for a rendered-line cursor. `line` remains a
/// fast layout cache, while this anchor lets resize/reflow keep the cursor on
/// the same transcript node instead of an unrelated absolute row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptCursorAnchor {
    pub key: crate::TranscriptNodeKey,
    pub line_offset: usize,
}

/// The cursor is the transcript's primary navigation state. The viewport is a
/// projection that keeps this target visible; it is never an independent
/// browsing cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptCursor {
    pub line: usize,
    /// Display-cell column of the grapheme under the cursor. This is a cell
    /// coordinate rather than a byte offset so CJK and emoji occupy one
    /// logical cursor target even when they consume two terminal columns.
    pub column: usize,
    /// The target display column preserved by vertical motions. Like Vim,
    /// moving through a short row temporarily clamps the visible cursor but
    /// returns to this column when a later row is wide enough.
    pub preferred_column: usize,
    pub anchor: Option<TranscriptCursorAnchor>,
    pub block_cursor: Option<TranscriptBlockCursor>,
    pub preferred_screen_row: usize,
}

/// Vim-style visual selection mode for the transcript's read-only surface.
/// The selected range remains independent from the cursor so keyboard motion
/// can extend it while pointer dragging keeps its existing behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptVisualSelectionMode {
    Character,
    Line,
    Block,
}

/// The geometry needed by Vim's `gv` command. It deliberately records the
/// keyboard anchor and live cursor endpoint rather than a normalized linear
/// range so rectangular Visual selections can be restored too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptVisualSelectionSnapshot {
    pub mode: TranscriptVisualSelectionMode,
    pub anchor: TranscriptTextPosition,
    pub head: TranscriptTextPosition,
}

/// The navigation cursor and committed pointer text range are independent.
/// Gesture recognition lives in the app input layer; neither dragging nor
/// committing a range changes this cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptInteraction {
    pub cursor: Option<TranscriptCursor>,
    pub text_selection: Option<TranscriptTextSelection>,
    pub visual_selection: Option<TranscriptVisualSelectionMode>,
    /// Stable start of a keyboard-driven visual range. Pointer ranges do not
    /// populate this field because they are complete at gesture end.
    pub visual_anchor: Option<TranscriptTextPosition>,
    /// Most recently left keyboard Visual range, restored by `gv`.
    pub last_visual_selection: Option<TranscriptVisualSelectionSnapshot>,
}
