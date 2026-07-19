use agena_tui_components::ThemePalette;
use ratatui::{layout::Rect, style::Style, text::Line};
use std::time::Instant;

use crate::math_render::{MathLinePlacement, TranscriptMathPlacement};

use super::{RenderedTranscriptNode, TranscriptBlockCursor};

#[derive(Debug, Clone)]
pub(crate) struct RenderedTranscript {
    pub(crate) width: u16,
    pub(crate) palette: ThemePalette,
    pub(crate) remote_image_generation: u64,
    pub(crate) lines: Vec<RenderedLine>,
    pub(crate) search_matches: Vec<usize>,
    pub(crate) message_line_starts: Vec<(i64, usize)>,
    pub(crate) nodes: Vec<RenderedTranscriptNode>,
    pub(crate) line_nodes: Vec<Option<usize>>,
    pub(crate) math: Vec<TranscriptMathPlacement>,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedLine {
    pub(crate) text: String,
    /// Clipboard-facing text for this rendered row. This deliberately omits
    /// layout-only prefixes such as transcript indentation, quote rails, and
    /// card chrome. Rich semantic nodes can still override row copying with
    /// their node-level `copy_text`.
    pub(crate) copy_text: String,
    /// Display-cell column at which `copy_text` begins in `text`. Mouse
    /// selections use this to translate terminal coordinates into the clean
    /// clipboard projection without copying layout prefixes.
    pub(crate) copy_column: usize,
    /// Optional non-contiguous clipboard projection for layouts whose visual
    /// chrome cannot be described by one column offset (notably tables and
    /// mixed inline graphics). Empty means the simple `copy_text` projection
    /// above remains authoritative.
    pub(crate) copy_segments: Vec<RenderedCopySegment>,
    /// Optional semantic row shared by one or more terminal rows. Code lines,
    /// table rows, and inline graphic line boxes use this when one logical row
    /// wraps or occupies several display rows. UI-only borders leave it unset.
    pub(crate) navigation_unit: Option<usize>,
    /// Clipboard text for the complete semantic row. This is deliberately
    /// separate from `copy_text`, which remains the projection for just this
    /// terminal row during a free-form pointer selection.
    pub(crate) navigation_copy_text: String,
    /// Pointer selection policy is deliberately independent from keyboard
    /// navigation. Code and table rows have a `navigation_unit` but remain
    /// character-selectable; formulas and other graphical line boxes opt into
    /// semantic-unit selection because terminal cells cannot represent a
    /// meaningful partial image selection.
    pub(crate) pointer_selection: TranscriptPointerSelection,
    pub(crate) style: Style,
    pub(crate) rich_line: Option<Line<'static>>,
    pub(crate) math: Vec<MathLinePlacement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedCopySegment {
    pub(crate) display_column: usize,
    pub(crate) text: String,
    pub(crate) separator_before: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TranscriptPointerSelection {
    #[default]
    Character,
    SemanticUnit,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TranscriptDetailDefaults {
    pub(crate) activity_expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TranscriptMoveDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LayoutCache {
    pub(crate) transcript_body: Rect,
    pub(crate) transcript_scrollbar: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranscriptScrollbarDrag {
    pub(crate) grab_offset: usize,
}

/// Gesture metadata only: the selected line/block still lives exclusively in
/// `TranscriptInteraction`. Terminals do not report click counts, so the app
/// remembers the last completed click long enough to recognize a double click.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TranscriptClick {
    pub(crate) line: usize,
    pub(crate) at: Instant,
}

/// A terminal-cell position in the fully rendered transcript. The line is a
/// logical transcript row (not a viewport-relative row), while `column` is a
/// zero-based display-cell column. Pointer selection, click hit-testing, and
/// keyboard resumption all use this same coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TranscriptTextPosition {
    pub(crate) line: usize,
    pub(crate) column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranscriptTextSelection {
    pub(crate) anchor: TranscriptTextPosition,
    pub(crate) head: TranscriptTextPosition,
}

/// Ephemeral mouse intent owned by the input layer. It is kept separate from
/// both the permanent navigation cursor and the committed text range so a
/// press can become either a click or a drag without mutating either state in
/// advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranscriptPointerGesture {
    pub(crate) anchor: TranscriptTextPosition,
    pub(crate) head: TranscriptTextPosition,
    pub(crate) dragged: bool,
}

impl TranscriptPointerGesture {
    pub(crate) fn new(anchor: TranscriptTextPosition) -> Self {
        Self {
            anchor,
            head: anchor,
            dragged: false,
        }
    }

    pub(crate) fn update(&mut self, head: TranscriptTextPosition, drag_event: bool) {
        self.dragged |= drag_event || head != self.anchor;
        self.head = head;
    }

    pub(crate) fn selection(self) -> TranscriptTextSelection {
        TranscriptTextSelection {
            anchor: self.anchor,
            head: self.head,
        }
    }
}

impl TranscriptTextSelection {
    /// Return the selected display-cell interval for one logical row. Mouse
    /// endpoints are inclusive, matching terminal selection behavior. The
    /// input layer creates this type only after a real drag, so equal endpoints
    /// mean one selected cell rather than an uncommitted click.
    pub(crate) fn cell_range_for_line(self, line: usize) -> Option<std::ops::Range<usize>> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranscriptViewport {
    pub(crate) top: usize,
    pub(crate) follow_tail: bool,
}

impl Default for TranscriptViewport {
    fn default() -> Self {
        Self {
            top: 0,
            follow_tail: true,
        }
    }
}

/// Stable semantic attachment for a rendered-line cursor. `line` remains a
/// fast layout cache, while this anchor lets resize/reflow keep the cursor on
/// the same transcript node instead of an unrelated absolute row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptCursorAnchor {
    pub(crate) key: super::TranscriptNodeKey,
    pub(crate) line_offset: usize,
}

/// The cursor is the transcript's primary navigation state. The viewport is a
/// projection that keeps this target visible; it is never an independent
/// browsing cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptCursor {
    pub(crate) line: usize,
    pub(crate) anchor: Option<TranscriptCursorAnchor>,
    pub(crate) block_cursor: Option<TranscriptBlockCursor>,
    pub(crate) preferred_screen_row: usize,
}

/// The navigation cursor and committed pointer text range are independent.
/// Gesture recognition lives in the app input layer; neither dragging nor
/// committing a range changes this cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TranscriptInteraction {
    pub(crate) cursor: Option<TranscriptCursor>,
    pub(crate) text_selection: Option<TranscriptTextSelection>,
}
