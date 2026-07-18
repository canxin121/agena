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
    pub(crate) style: Style,
    pub(crate) rich_line: Option<Line<'static>>,
    pub(crate) math: Vec<MathLinePlacement>,
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

impl TranscriptTextSelection {
    pub(crate) fn is_non_empty(self) -> bool {
        self.anchor != self.head
    }

    /// Return the selected display-cell interval for one logical row. Mouse
    /// endpoints are inclusive, matching terminal selection behavior.
    pub(crate) fn cell_range_for_line(self, line: usize) -> Option<std::ops::Range<usize>> {
        if !self.is_non_empty() {
            return None;
        }
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

/// The transcript has one interaction owner with three deliberately distinct
/// modes. Browsing moves only the viewport and has no hidden action target;
/// navigation owns the target used by `y` and `Enter`; text selection owns a
/// precise terminal-cell range and may span beyond the viewport.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum TranscriptInteraction {
    #[default]
    Browse,
    Navigate {
        cursor_line: usize,
        block_cursor: Option<TranscriptBlockCursor>,
    },
    TextSelect {
        selection: TranscriptTextSelection,
        dragging: bool,
    },
}
