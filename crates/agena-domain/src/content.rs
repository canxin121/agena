//! Client-neutral, independently readable content owned by a part or process.
//!
//! A cursor identifies an exact generation and record position. Neither a
//! database revision nor a timestamp is a substitute for this position.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{CommandOutputStream, ContentId};

mod document;
pub use document::{ContentDocument, DocumentMutation, MAX_DOCUMENT_BLOCKS, MAX_DOCUMENT_BYTES};

/// At most 40,000 cells, including highly fragmented standard attributes.
/// Frames have an independent atomic budget; pipe records remain small.
pub const MAX_TERMINAL_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Text,
    Log,
    Structured,
    /// A tool document may contain text, log and structured events together.
    Document,
    Terminal,
}

/// Source syntax and semantic representation, independent of client layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentFormat {
    #[default]
    Plain,
    Markdown,
    Code,
    Diff,
    Json,
}

/// A large result field lives in a content resource instead of the Part row.
/// The JSON pointer identifies its place in the original tool payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentField {
    pub pointer: String,
    pub resource: ContentRef,
    pub format: ContentFormat,
}

impl ContentKind {
    pub fn accepts(self, payload: ContentKind) -> bool {
        self == payload
            || (self == Self::Document
                && matches!(payload, Self::Text | Self::Log | Self::Structured))
            || (self == Self::Terminal && payload == Self::Log)
    }
}

/// Standard VT cell attributes; they describe the source terminal, without
/// observer layout or theme preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TerminalAttributes {
    pub foreground: Option<TerminalColor>,
    pub background: Option<TerminalColor>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalColor {
    Indexed { index: u8 },
    Rgb { red: u8, green: u8, blue: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalCellRun {
    pub row: u16,
    pub col: u16,
    pub text: String,
    pub attributes: TerminalAttributes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub rows: u16,
    pub cols: u16,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub cursor_visible: bool,
    pub alternate_screen: bool,
    pub bracketed_paste: bool,
    pub application_cursor: bool,
    pub cells: Vec<TerminalCellRun>,
}

impl TerminalSnapshot {
    pub fn validate(&self, changed_rows: Option<&[u16]>) -> Result<(), &'static str> {
        use unicode_width::UnicodeWidthStr as _;
        if self.rows == 0
            || self.cols == 0
            || self.rows > 200
            || self.cols > 400
            || usize::from(self.rows) * usize::from(self.cols) > 40_000
            || self.cursor_row >= self.rows
            || self.cursor_col >= self.cols
        {
            return Err("invalid terminal geometry");
        }
        if let Some(rows) = changed_rows
            && (rows.iter().any(|row| *row >= self.rows)
                || rows.windows(2).any(|pair| pair[0] >= pair[1]))
        {
            return Err("invalid terminal patch rows");
        }
        let mut previous: Option<(u16, usize)> = None;
        for run in &self.cells {
            let width = run.text.as_str().width();
            if run.row >= self.rows
                || run.col >= self.cols
                || run.text.is_empty()
                || run.text.chars().any(char::is_control)
                || run.text.len() > usize::from(self.cols) * 64
                || width == 0
                || usize::from(run.col) + width > usize::from(self.cols)
                || changed_rows.is_some_and(|rows| rows.binary_search(&run.row).is_err())
                || previous.is_some_and(|(row, end)| {
                    run.row < row || (run.row == row && usize::from(run.col) < end)
                })
            {
                return Err("invalid terminal cell run");
            }
            previous = Some((run.row, usize::from(run.col) + width));
        }
        Ok(())
    }
    /// Re-encode a snapshot using standard VT operations. Embedders interpret
    /// this in their own emulator; it is never sent to an observer's outer tty.
    pub fn formatted(&self) -> String {
        self.format_rows(None)
    }

    pub fn formatted_patch(&self, rows: &[u16]) -> String {
        self.format_rows(Some(rows))
    }

    /// Cell-aligned plain observation for copy and bounded model projections.
    pub fn plain_text(&self) -> String {
        use unicode_width::UnicodeWidthStr as _;
        let mut text = String::new();
        for row in 0..self.rows {
            let mut col = 0;
            for run in self.cells.iter().filter(|run| run.row == row) {
                text.extend(std::iter::repeat_n(
                    ' ',
                    usize::from(run.col.saturating_sub(col)),
                ));
                text.push_str(&run.text);
                col = run.col.saturating_add(run.text.as_str().width() as u16);
            }
            if row + 1 < self.rows {
                text.push('\n');
            }
        }
        text
    }

    fn format_rows(&self, rows: Option<&[u16]>) -> String {
        use std::fmt::Write as _;
        let mut output = String::from("\x1b[0m");
        if let Some(rows) = rows {
            for row in rows {
                let _ = write!(output, "\x1b[{};1H\x1b[2K", row + 1);
            }
        } else {
            let _ = write!(
                output,
                "\x1b[?1049{}",
                if self.alternate_screen { 'h' } else { 'l' }
            );
            output.push_str("\x1b[2J\x1b[H");
        }
        for run in &self.cells {
            let _ = write!(output, "\x1b[{};{}H\x1b[0m", run.row + 1, run.col + 1);
            for (enabled, code) in [
                (run.attributes.bold, 1),
                (run.attributes.dim, 2),
                (run.attributes.italic, 3),
                (run.attributes.underline, 4),
                (run.attributes.inverse, 7),
            ] {
                if enabled {
                    let _ = write!(output, "\x1b[{code}m");
                }
            }
            for (color, channel) in [
                (&run.attributes.foreground, 38),
                (&run.attributes.background, 48),
            ] {
                match color {
                    Some(TerminalColor::Indexed { index }) => {
                        let _ = write!(output, "\x1b[{channel};5;{index}m");
                    }
                    Some(TerminalColor::Rgb { red, green, blue }) => {
                        let _ = write!(output, "\x1b[{channel};2;{red};{green};{blue}m");
                    }
                    None => {}
                }
            }
            output.push_str(&run.text);
        }
        let _ = write!(
            output,
            "\x1b[0m\x1b[{};{}H\x1b[?25{}",
            self.cursor_row + 1,
            self.cursor_col + 1,
            if self.cursor_visible { 'h' } else { 'l' }
        );
        let _ = write!(
            output,
            "\x1b[?2004{}\x1b[?1{}",
            if self.bracketed_paste { 'h' } else { 'l' },
            if self.application_cursor { 'h' } else { 'l' }
        );
        output
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentCursor {
    pub epoch: Uuid,
    pub sequence: u64,
}

/// Last fully consumed record plus UTF-8 bytes consumed in its successor.
/// An offset is always paired with its epoch and preceding record cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentTextPosition {
    pub after: ContentCursor,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentTextSlice {
    pub cursor: ContentCursor,
    pub captured_at_ms: i64,
    pub offset: usize,
    pub stream: Option<CommandOutputStream>,
    pub text: String,
}

/// A byte-bounded text representation of the same canonical record stream.
/// Non-text records advance the position without fabricating textual output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentTextPage {
    pub resource: ContentResource,
    pub slices: Vec<ContentTextSlice>,
    pub next_position: ContentTextPosition,
    pub has_more: bool,
    pub gap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentState {
    Active,
    Complete,
    Interrupted,
}

/// Inclusive range of retained record sequence numbers. Missing ranges are
/// unavailable output, never text that can be concatenated without a gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentRange {
    pub first: u64,
    pub last: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentRef {
    pub resource_id: ContentId,
    pub kind: ContentKind,
}

/// Small envelope; its size is independent of the amount of source content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentResource {
    pub resource_id: ContentId,
    pub owner_session_id: i64,
    pub part_id: i64,
    pub kind: ContentKind,
    pub state: ContentState,
    pub cursor: ContentCursor,
    pub committed_cursor: ContentCursor,
    pub total_bytes: u64,
    /// Observed source bytes that could not be retained. Independent of exit.
    pub dropped_bytes: u64,
    pub retained_ranges: Vec<ContentRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_error: Option<String>,
}

impl ContentResource {
    pub fn reference(&self) -> ContentRef {
        ContentRef {
            resource_id: self.resource_id,
            kind: self.kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentPayload {
    Text {
        text: String,
    },
    Log {
        stream: CommandOutputStream,
        text: String,
    },
    Structured {
        base_cursor: ContentCursor,
        event: DocumentMutation,
    },
    StructuredSnapshot {
        document: ContentDocument,
    },
    Terminal {
        screen: TerminalSnapshot,
    },
    TerminalPatch {
        base_cursor: ContentCursor,
        screen: TerminalSnapshot,
        rows_changed: Vec<u16>,
    },
}

impl ContentPayload {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Terminal { screen } => screen.validate(None),
            Self::Structured { base_cursor, event } => {
                if base_cursor.sequence == 0 {
                    return Err("document mutation requires a prior checkpoint");
                }
                event.validate()
            }
            Self::StructuredSnapshot { document } => document.validate(),
            Self::TerminalPatch {
                screen,
                rows_changed,
                base_cursor,
            } => {
                if base_cursor.sequence == 0 {
                    return Err("terminal patch requires a prior frame");
                }
                screen.validate(Some(rows_changed))
            }
            _ => Ok(()),
        }
    }
    pub fn kind(&self) -> ContentKind {
        match self {
            Self::Text { .. } => ContentKind::Text,
            Self::Log { .. } => ContentKind::Log,
            Self::Structured { .. } | Self::StructuredSnapshot { .. } => ContentKind::Structured,
            Self::Terminal { .. } | Self::TerminalPatch { .. } => ContentKind::Terminal,
        }
    }

    pub fn text_content(&self) -> Option<&str> {
        match self {
            Self::Text { text } | Self::Log { text, .. } => Some(text),
            _ => None,
        }
    }

    /// UTF-8 byte cost, not client string length or terminal display width.
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Text { text } | Self::Log { text, .. } => text.len(),
            Self::Structured { event, .. } => serde_json::to_vec(event)
                .expect("document event serializes")
                .len(),
            Self::StructuredSnapshot { document } => document.byte_len(),
            Self::Terminal { screen } | Self::TerminalPatch { screen, .. } => {
                serde_json::to_vec(screen)
                    .expect("terminal screen is serializable")
                    .len()
            }
        }
    }
}

/// A producer submits source data without choosing document cursors or
/// checkpoints. The owning writer publishes an exact, replayable payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentInput {
    Text {
        text: String,
    },
    Log {
        stream: CommandOutputStream,
        text: String,
    },
    Structured {
        event: DocumentMutation,
    },
    Terminal {
        screen: TerminalSnapshot,
    },
    TerminalPatch {
        base_cursor: ContentCursor,
        screen: TerminalSnapshot,
        rows_changed: Vec<u16>,
    },
}

impl ContentInput {
    pub fn text_content(&self) -> Option<&str> {
        match self {
            Self::Text { text } | Self::Log { text, .. } => Some(text),
            _ => None,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Structured { event } => event.validate(),
            Self::Terminal { screen } => screen.validate(None),
            Self::TerminalPatch {
                base_cursor,
                screen,
                rows_changed,
            } => {
                if base_cursor.sequence == 0 {
                    return Err("terminal patch requires a prior frame");
                }
                screen.validate(Some(rows_changed))
            }
            _ => Ok(()),
        }
    }

    pub fn kind(&self) -> ContentKind {
        match self {
            Self::Text { .. } => ContentKind::Text,
            Self::Log { .. } => ContentKind::Log,
            Self::Structured { .. } => ContentKind::Structured,
            Self::Terminal { .. } | Self::TerminalPatch { .. } => ContentKind::Terminal,
        }
    }

    pub fn byte_len(&self) -> usize {
        match self {
            Self::Text { text } | Self::Log { text, .. } => text.len(),
            Self::Structured { event } => serde_json::to_vec(event)
                .expect("document event serializes")
                .len(),
            Self::Terminal { screen } | Self::TerminalPatch { screen, .. } => {
                serde_json::to_vec(screen)
                    .expect("terminal screen serializes")
                    .len()
            }
        }
    }

    /// Document mutations must be stamped under the source's admission lock.
    pub fn into_payload(
        self,
        document_cursor: Option<ContentCursor>,
    ) -> Result<ContentPayload, &'static str> {
        Ok(match self {
            Self::Text { text } => ContentPayload::Text { text },
            Self::Log { stream, text } => ContentPayload::Log { stream, text },
            Self::Structured { event } => ContentPayload::Structured {
                base_cursor: document_cursor.ok_or("document mutation requires a checkpoint")?,
                event,
            },
            Self::Terminal { screen } => ContentPayload::Terminal { screen },
            Self::TerminalPatch {
                base_cursor,
                screen,
                rows_changed,
            } => ContentPayload::TerminalPatch {
                base_cursor,
                screen,
                rows_changed,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentChunk {
    pub cursor: ContentCursor,
    /// Time observed by the source owner. Sequence defines collection order;
    /// independently drained stdout/stderr do not imply OS write order.
    pub captured_at_ms: i64,
    pub payload: ContentPayload,
}

/// A bounded chronological read. The cursor advances only over returned
/// records. `gap` tells the caller that requested records were not retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentPage {
    pub resource: ContentResource,
    pub chunks: Vec<ContentChunk>,
    pub next_cursor: ContentCursor,
    pub has_more: bool,
    pub gap: bool,
}
