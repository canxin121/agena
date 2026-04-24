use chrono::{DateTime, Utc};
use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use strum::Display;
use uuid::Uuid;

use crate::{
    event::SessionEvent,
    message::{ExecutionStatus, Message, MessageMetadata, MessageStatus, MessageUsage, PartContent},
    session::{ProviderPromptAnchor, PromptTokenRuntime, SessionRuntimeState},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryRecord {
    pub seq: i64,
    pub event_id: Uuid,
    pub session_id: i64,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<Uuid>,
    #[serde(flatten)]
    pub item: HistoryItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult, Display)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum HistoryItem {
    MessageStarted(MessageStarted),
    MessageSnapshotRecorded(MessageSnapshotRecorded),
    MessageStateChanged(MessageStateChanged),
    MessageUsageSet(MessageUsageSet),
    MessageFinishSet(MessageFinishSet),
    MessageTagsAdded(MessageTagsAdded),
    MessageTagsRemoved(MessageTagsRemoved),
    PartStarted(PartStarted),
    PartStatusChanged(PartStatusChanged),
    PartOperationIdSet(PartOperationIdSet),
    PartContentDelta(PartContentDelta),
    PartContentReplaced(PartContentReplaced),
    PromptCompactionApplied(PromptCompactionApplied),
    ToolResultPruned(ToolResultPruned),
    AttachmentPayloadStripped(AttachmentPayloadStripped),
    PromptWindowInvalidated(PromptWindowInvalidated),
    ProviderAnchorSet(ProviderAnchorSet),
    ProviderAnchorCleared(ProviderAnchorCleared),
    ProviderAnchorsCleared(ProviderAnchorsCleared),
    PromptTokensRecorded(PromptTokensRecorded),
    PromptTokensCleared(PromptTokensCleared),
    LoadedDeferredToolsRecorded(LoadedDeferredToolsRecorded),
    SessionRuntimeRecorded(SessionRuntimeRecorded),
    SessionRolledBack(SessionRolledBack),
    ClientEventRecorded(ClientEventRecorded),
    LegacySnapshotImported(LegacySnapshotImported),
}

impl HistoryItem {
    pub(crate) fn event_type(&self) -> &'static str {
        match self {
            Self::MessageStarted(_) => "message_started",
            Self::MessageSnapshotRecorded(_) => "message_snapshot_recorded",
            Self::MessageStateChanged(_) => "message_state_changed",
            Self::MessageUsageSet(_) => "message_usage_set",
            Self::MessageFinishSet(_) => "message_finish_set",
            Self::MessageTagsAdded(_) => "message_tags_added",
            Self::MessageTagsRemoved(_) => "message_tags_removed",
            Self::PartStarted(_) => "part_started",
            Self::PartStatusChanged(_) => "part_status_changed",
            Self::PartOperationIdSet(_) => "part_operation_id_set",
            Self::PartContentDelta(_) => "part_content_delta",
            Self::PartContentReplaced(_) => "part_content_replaced",
            Self::PromptCompactionApplied(_) => "prompt_compaction_applied",
            Self::ToolResultPruned(_) => "tool_result_pruned",
            Self::AttachmentPayloadStripped(_) => "attachment_payload_stripped",
            Self::PromptWindowInvalidated(_) => "prompt_window_invalidated",
            Self::ProviderAnchorSet(_) => "provider_anchor_set",
            Self::ProviderAnchorCleared(_) => "provider_anchor_cleared",
            Self::ProviderAnchorsCleared(_) => "provider_anchors_cleared",
            Self::PromptTokensRecorded(_) => "prompt_tokens_recorded",
            Self::PromptTokensCleared(_) => "prompt_tokens_cleared",
            Self::LoadedDeferredToolsRecorded(_) => "loaded_deferred_tools_recorded",
            Self::SessionRuntimeRecorded(_) => "session_runtime_recorded",
            Self::SessionRolledBack(_) => "session_rolled_back",
            Self::ClientEventRecorded(_) => "client_event_recorded",
            Self::LegacySnapshotImported(_) => "legacy_snapshot_imported",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageStarted {
    pub message_id: i64,
    pub role: crate::role::Role,
    pub state: MessageStatus,
    pub created_at: DateTime<Utc>,
    pub metadata: MessageMetadata,
}

impl From<&Message> for MessageStarted {
    fn from(message: &Message) -> Self {
        Self {
            message_id: message.id,
            role: message.role,
            state: MessageStatus::Pending,
            created_at: message.created_at,
            metadata: message.metadata.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageSnapshotRecorded {
    pub message: Message,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageStateChanged {
    pub message_id: i64,
    pub state: MessageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageUsageSet {
    pub message_id: i64,
    pub usage: MessageUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageFinishSet {
    pub message_id: i64,
    pub finish: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageTagsAdded {
    pub message_id: i64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageTagsRemoved {
    pub message_id: i64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartStarted {
    pub message_id: i64,
    pub part: crate::message::MessagePart,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartStatusChanged {
    pub part_id: i64,
    pub status: ExecutionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartOperationIdSet {
    pub part_id: i64,
    pub operation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum PartContentDelta {
    Text { part_id: i64, delta: String },
    ReasoningSummary { part_id: i64, delta: String },
    ReasoningRaw { part_id: i64, delta: String },
    CommandOutput { part_id: i64, delta: String },
    ToolOutput { part_id: i64, delta: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartContentReplaced {
    pub part_id: i64,
    pub content: PartContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptCompactionApplied {
    pub compacted_message_ids: Vec<i64>,
    pub summary_message_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolResultPruned {
    pub message_id: i64,
    pub part_id: i64,
    pub replacement_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentPayloadStripped {
    pub message_id: i64,
    pub part_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptWindowInvalidated {
    pub generation: u64,
    pub reason: PromptWindowInvalidationReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PromptWindowInvalidationReason {
    Compaction,
    ToolResultPruning,
    AttachmentPayloadStripping,
    Rewind,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderAnchorSet {
    pub anchor: ProviderPromptAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderAnchorCleared {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderAnchorsCleared;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptTokensRecorded {
    pub runtime: PromptTokenRuntime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptTokensCleared;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadedDeferredToolsRecorded {
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRuntimeRecorded {
    pub runtime: SessionRuntimeState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRolledBack {
    pub target_message_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClientEventRecorded {
    pub event: SessionEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacySnapshotImported {
    pub message_count: usize,
}
