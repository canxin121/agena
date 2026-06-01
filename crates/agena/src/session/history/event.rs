use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use strum::Display;

use crate::{
    message::{MessageMetadata, MessagePart, MessageProviderState, MessageUsage},
    session::{
        history::transcript::{TranscriptContent, TranscriptToolOutput},
        ids::{MessageId, RunId, ToolCallId},
    },
};

// NOTE: the wrapper enum `HistoryItem` and its `HistoryRecord` envelope have
// been removed in favor of the unified `crate::event::EventKind` /
// `crate::event::DomainEvent`. The payload structs below stay in this
// module because they are referenced verbatim by the corresponding
// `EventKind` variants.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunStarted {
    pub run_id: RunId,
    #[serde(default)]
    pub source: RunSource,
    pub model_id: SmolStr,
    pub provider_id: SmolStr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunCompleted {
    pub run_id: RunId,
    pub finish_reason: FinishReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunAborted {
    pub run_id: RunId,
    pub reason: RunAbortReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RunAbortReason {
    /// Detected on session load — an in-flight run from a prior process.
    ProcessRestart,
    /// User cancelled the run (e.g. via UI cancel button).
    UserCancelled,
    /// Provider returned an error before the run could close.
    ProviderError,
    /// Internal scheduling error.
    Internal,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RunSource {
    #[default]
    User,
    Continue,
    Compaction,
    PermissionReply,
    UserInputReply,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FinishReason {
    #[default]
    Stop,
    ToolCalls,
    MaxTokens,
    ContentFilter,
    Error,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserMessageAppended {
    pub message_id: MessageId,
    pub run_id: RunId,
    pub created_at: DateTime<Utc>,
    /// Cache-stable transcript projection of the message body. Used to
    /// re-derive `ProviderTranscript::digest()` without re-folding the full
    /// part list.
    pub content: TranscriptContent,
    /// Authoritative copy of the message body. The projection rebuilds the
    /// in-memory `Message` from these parts directly so multi-modal fidelity
    /// (images, attachments, …) round-trips losslessly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MessagePart>,
    #[serde(default)]
    pub metadata: MessageMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<MessageProviderState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessageCompleted {
    pub message_id: MessageId,
    pub run_id: RunId,
    pub created_at: DateTime<Utc>,
    pub content: TranscriptContent,
    /// Authoritative copy of the assistant message body. See the
    /// [`UserMessageAppended::parts`] doc comment for rationale.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MessagePart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
    pub finish_reason: FinishReason,
    #[serde(default)]
    pub metadata: MessageMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<MessageProviderState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallIssued {
    pub message_id: MessageId,
    pub run_id: RunId,
    pub call_id: ToolCallId,
    pub name: SmolStr,
    pub arguments: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallCompleted {
    /// Assistant `MessageId` that owns the operation part for this tool call.
    /// Tool completion updates that part; it never creates a standalone tool
    /// message.
    pub message_id: MessageId,
    pub call_id: ToolCallId,
    pub run_id: RunId,
    /// Stable name of the tool that produced this result. Stored verbatim so
    /// the projection can reconstruct a faithful `ToolInvocation` rather than
    /// the placeholder `name: "tool"` it had to use when the field was
    /// missing.
    pub tool_name: SmolStr,
    /// Authoritative completed operation part for this tool call when
    /// available. New writers populate this so append-only history can
    /// reconstruct the exact completed tool payload, including attachments and
    /// provider-specific blocks, without relying on a prior mutable message
    /// rewrite. Older logs omit it and fall back to `output`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<MessagePart>,
    pub output: TranscriptToolOutput,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemNoticeAppended {
    pub message_id: MessageId,
    pub created_at: DateTime<Utc>,
    pub kind: SystemNoticeKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SystemNoticeKind {
    ContextInjection,
    ToolPolicyHint,
    /// Legacy audit marker emitted by older same-session rewind operations.
    /// The notice text is a JSON [`RewindCheckpoint`] payload describing the
    /// messages that were removed by the rewind so a UI can show "you rewound
    /// past these N messages — undo?" without re-folding.
    /// Projection drops these from the visible transcript.
    RewindCheckpoint,
    Other,
}

/// Payload carried as JSON inside a `SystemNoticeAppended` whose kind is
/// `RewindCheckpoint`. Stable wire format keyed on schema for forward
/// compatibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RewindCheckpoint {
    /// Format version. Increment on breaking changes.
    pub schema: u32,
    /// Millisecond UTC timestamp of when the rewind happened.
    pub at_ms: i64,
    /// The message id the user rewound *to*.
    pub target_message_id: i64,
    /// Per-message audit entries for every message skipped by the rewind.
    /// Order matches the original transcript order.
    pub dropped: Vec<RewindCheckpointRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RewindCheckpointRecord {
    pub message_id: i64,
    pub role: String,
    /// Truncated preview of the message body (≤256 chars). Full content is
    /// still recoverable from the underlying event log if needed.
    pub preview: String,
}
