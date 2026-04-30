use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use strum::Display;

use crate::{
    message::{MessageMetadata, MessageUsage},
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
    pub content: TranscriptContent,
    #[serde(default)]
    pub metadata: MessageMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessageCompleted {
    pub message_id: MessageId,
    pub turn_id: TurnId,
    pub created_at: DateTime<Utc>,
    pub content: TranscriptContent,
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
    pub call_id: ToolCallId,
    pub turn_id: TurnId,
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
    Other,
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
