use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    JsonSchema,
    Display,
    EnumString,
    Default,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Shell used to run a process.
pub enum ProcessShell {
    #[default]
    Bash,
    Powershell,
}

/// A bounded, plain-text projection of a terminal's current screen. Positions
/// are zero-based. Raw terminal output is returned separately and never executed
/// as control sequences by the human-facing renderer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TerminalScreen {
    pub rows: u16,
    pub cols: u16,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub cursor_visible: bool,
    pub alternate_screen: bool,
    pub bracketed_paste: bool,
    pub application_cursor: bool,
    pub text: String,
    pub truncated: bool,
}
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Output stream of a process event.
pub enum ProcessStream {
    Stdout,
    Stderr,
}
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Status of a monitored process.
pub enum ProcessStatus {
    Running,
    Exited,
    TimedOut,
    Stopped,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
/// One line or raw chunk of process output.
pub struct ProcessEvent {
    pub seq: u64,
    pub stream: ProcessStream,
    pub ts_ms: i64,
    pub line: String,
    /// Raw pipe/PTY bytes already contain their line endings.
    #[serde(default)]
    pub chunk: bool,
    /// Present only on deliberately selected notifications, not ordinary logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification: Option<String>,
    /// Monotonic notification identity, separate from the raw-log cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_seq: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Summary of a monitored process.
pub struct ProcessSummary {
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_archive: Option<ProcessOutputArchive>,
    /// True for a persistent pseudo-terminal; its events contain raw chunks,
    /// not newline-delimited log records.
    #[serde(default)]
    pub tty: bool,
    /// True for a WebSocket event subscription rather than a local command.
    #[serde(default)]
    pub websocket: bool,
    pub command: String,
    pub description: String,
    pub status: ProcessStatus,
    pub background: bool,
    /// True when the process was started with shell monitor conditions rather
    /// than as an unconstrained background process.
    #[serde(default)]
    pub monitored: bool,
    /// Readiness matched for the current watch configuration. The process may
    /// continue running; readiness is not a terminal state.
    #[serde(default)]
    pub ready: bool,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
    pub buffered_lines: u32,
    pub last_seq: u64,
    pub dropped_lines: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_reason: Option<String>,
    /// Session that owns the process when the runtime knows it. Background
    /// activities are scoped by session, so a synthesized shell or terminal
    /// record must carry the owner instead of nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<i64>,
}

/// Captured process output outside the model context. A running archive may
/// still have queued writes; only a complete archive contains all observed
/// output without storage loss. The file contains UTF-8 terminal/pipe text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ProcessOutputArchive {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Ordered retained files. Byte ranges refer to the original captured
    /// UTF-8 stream; each file starts at local byte offset 0. Gaps are omitted
    /// output, never contiguous text. path remains the stable startup prefix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<ProcessOutputSegment>,
    pub retained_bytes: u64,
    pub total_bytes: u64,
    pub pending: bool,
    pub complete: bool,
    pub truncated: bool,
    pub limit_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One retained, chronological piece of a rolling process output archive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ProcessOutputSegment {
    pub path: String,
    pub start_byte: u64,
    pub end_byte: u64,
}
