use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use strum::Display;

use crate::{
    message::{MessageMetadata, MessagePart, MessageUsage},
    session::{
        history::transcript::{TranscriptContent, TranscriptToolOutput},
        ids::{MessageId, ToolCallId, TurnId},
    },
};

// NOTE: the wrapper enum `HistoryItem` and its `HistoryRecord` envelope have
// been removed in favor of the unified `crate::event::EventKind` /
// `crate::event::DomainEvent`. The payload structs below stay in this
// module because they are referenced verbatim by the corresponding
// `EventKind` variants.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnStarted {
    pub turn_id: TurnId,
    pub model_id: SmolStr,
    pub provider_id: SmolStr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnCompleted {
    pub turn_id: TurnId,
    pub finish_reason: FinishReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnAborted {
    pub turn_id: TurnId,
    pub reason: TurnAbortReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TurnAbortReason {
    /// Detected on session load — an in-flight turn from a prior process.
    ProcessRestart,
    /// User cancelled the turn (e.g. via UI cancel button).
    UserCancelled,
    /// Provider returned an error before the turn could close.
    ProviderError,
    /// Internal scheduling error.
    Internal,
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
    pub turn_id: TurnId,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessageCompleted {
    pub message_id: MessageId,
    pub turn_id: TurnId,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallIssued {
    pub message_id: MessageId,
    pub turn_id: TurnId,
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
    pub turn_id: TurnId,
    /// Stable name of the tool that produced this result. Stored verbatim so
    /// the projection can reconstruct a faithful `ToolInvocation` rather than
    /// the placeholder `name: "tool"` it had to use when the field was
    /// missing.
    pub tool_name: SmolStr,
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
    CompactionSummary,
    ContextInjection,
    ToolPolicyHint,
    /// Audit marker emitted alongside a `rewind_to_message` operation.
    /// The notice text is a JSON [`RewindCheckpoint`] payload describing the
    /// messages that were dropped from the prompt window so a UI can show
    /// "you rewound past these N messages — undo?" without re-folding.
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
    /// The message id the user rewound *to* (inclusive — this and everything
    /// after it were compacted).
    pub target_message_id: i64,
    /// Per-message audit entries for every message that was compacted. Order
    /// matches the original transcript order.
    pub dropped: Vec<RewindCheckpointEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RewindCheckpointEntry {
    pub message_id: i64,
    pub role: String,
    /// Truncated preview of the message body (≤256 chars). Full content is
    /// still recoverable from the underlying event log if needed.
    pub preview: String,
}

/// Annotation that overlays a previously-appended message — used by prompt
/// window management. Rebuilds of the transcript apply revisions in seq order
/// to the matching `target_message_id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageRevised {
    pub target_message_id: i64,
    pub kind: RevisionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RevisionKind {
    /// The target message has been folded into a compaction summary and must
    /// be dropped from the transcript. The summary itself arrives as a
    /// `SystemNoticeAppended` with kind `CompactionSummary`.
    Compacted,
    /// Reverses a prior `Compacted` revision on the same target message,
    /// re-admitting it into the transcript. Used by the un-rewind flow so a
    /// rewind can be undone without losing the pre-existing event log.
    Uncompacted,
    /// A tool result on the target message has been pruned; the projection
    /// substitutes `replacement` for the original output.
    ToolResultPruned {
        call_id: ToolCallId,
        replacement: String,
    },
    /// An attachment payload on the target message was stripped to save
    /// tokens. The projection replaces the attachment with a placeholder.
    AttachmentStripped { part_id: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(value: &T) {
        let json = serde_json::to_string(value).expect("encode");
        let back: T = serde_json::from_str(&json).expect("decode");
        assert_eq!(&back, value);
    }

    #[test]
    fn message_revised_round_trip() {
        for kind in [
            RevisionKind::Compacted,
            RevisionKind::Uncompacted,
            RevisionKind::ToolResultPruned {
                call_id: ToolCallId::new("call_x"),
                replacement: "[pruned]".into(),
            },
            RevisionKind::AttachmentStripped { part_id: 7 },
        ] {
            let item = MessageRevised {
                target_message_id: 42,
                kind,
            };
            rt(&item);
        }
    }
}
