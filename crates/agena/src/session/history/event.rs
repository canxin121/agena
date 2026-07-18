use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use strum::Display;

use crate::{
    message::{
        ExecutionStatus, Message, MessageMetadata, MessagePart, MessageProviderState,
        MessageSource, MessageStatus, MessageUsage, OperationPart, PartContent, StructuredObject,
        TimeRange, ToolInvocation, ToolOutput,
    },
    role::Role,
    session::{
        ExecutionSource,
        history::transcript::TranscriptContent,
        ids::{MessageId, PartId, RunId, ToolCallId},
    },
};

// NOTE: the wrapper enum `HistoryItem` and its `HistoryRecord` envelope have
// been removed in favor of the unified `crate::event::EventKind` /
// `crate::event::DomainEvent`. The payload structs below stay in this
// module because they are referenced verbatim by the corresponding
// `EventKind` variants.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunStarted {
    pub execution_id: crate::session::ExecutionId,
    pub run_id: RunId,
    pub source: ExecutionSource,
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
    pub execution_id: crate::session::ExecutionId,
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
pub struct AssistantMessageFinished {
    pub execution_id: crate::session::ExecutionId,
    pub message_id: MessageId,
    pub run_id: RunId,
    pub created_at: DateTime<Utc>,
    pub content: TranscriptContent,
    /// Authoritative terminal message status. Terminal history never infers
    /// cancellation or failure from a separate, potentially missing event.
    pub status: MessageStatus,
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
    /// Stable name of the tool that produced this result.
    pub tool_name: SmolStr,
    /// Authoritative completed operation part, including attachments and
    /// provider-specific blocks.
    pub part: MessagePart,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemNoticeAppended {
    pub message_id: MessageId,
    pub part_id: PartId,
    pub created_at: DateTime<Utc>,
    pub kind: SystemNoticeKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<crate::session::PromptCompactionActivity>,
}

impl SystemNoticeAppended {
    /// Materialize the canonical transcript message represented by this
    /// history event. Writers use the same projection before caching the
    /// session, so an event-driven refresh cannot observe a stale cache entry.
    pub(crate) fn projected_message(&self) -> Message {
        Message {
            id: self.message_id.raw(),
            role: Role::System,
            state: ExecutionStatus::Completed,
            parts: vec![self.projected_part()],
            created_at: self.created_at,
            metadata: MessageMetadata {
                source: MessageSource::System,
                ..Default::default()
            },
            provider_state: None,
            usage: None,
        }
    }

    pub(crate) fn projected_part(&self) -> MessagePart {
        if let Some(compaction) = self.compaction.as_ref() {
            let structured = serde_json::to_value(compaction).unwrap_or_default();
            let invocation_input =
                StructuredObject::try_from(structured.clone()).unwrap_or_default();
            let mut operation = OperationPart::completed(
                self.message_id.raw(),
                ToolInvocation::new("session.compact", invocation_input),
                self.text.clone(),
                Vec::new(),
                Vec::new(),
                ToolOutput::default(),
                TimeRange {
                    start_ms: self.created_at.timestamp_millis(),
                    end_ms: Some(self.created_at.timestamp_millis()),
                },
            );
            operation.set_title("Context compacted");
            operation.set_ui_only(true);
            operation.structured = Some(structured);
            operation.metadata.insert(
                "agena.activity.kind".to_owned(),
                serde_json::Value::String("compaction".to_owned()),
            );
            let mut part = MessagePart::from_content(
                self.part_id.raw(),
                self.message_id.raw(),
                self.created_at,
                ExecutionStatus::Completed,
                PartContent::Operation(operation),
            );
            part.part_index = 0;
            part.operation_id = Some(format!("compaction:{}", compaction.checkpoint_id));
            return part;
        }

        let mut part = MessagePart::from_content(
            self.part_id.raw(),
            self.message_id.raw(),
            self.created_at,
            ExecutionStatus::Completed,
            PartContent::text(self.text.clone()),
        );
        part.part_index = 0;
        part
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SystemNoticeKind {
    ContextInjection,
    ToolPolicyHint,
    Compaction,
    Other,
}
