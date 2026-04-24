use std::collections::BTreeSet;

use thiserror::Error;

use crate::{
    message::{
        AttachmentSource, Message, MessagePart, MessageStateStore, MessageStateStoreError,
        MessageStatus, MessageUpdate, PartContent,
    },
    provider::PRUNED_TOOL_RESULT_PLACEHOLDER,
    session::{
        MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED, MESSAGE_TAG_PROMPT_COMPACTED,
        MESSAGE_TAG_TOOL_RESULT_PRUNED, SessionRuntimeState,
    },
};

use super::{
    AttachmentPayloadStripped, HistoryItem, HistoryRecord, PartContentDelta,
    PromptWindowInvalidationReason, SessionRolledBack, ToolResultPruned,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionHistoryProjection {
    pub messages: Vec<Message>,
    pub runtime: SessionRuntimeState,
    pub compacted_message_ids: BTreeSet<i64>,
    pub pruned_part_ids: BTreeSet<i64>,
    pub stripped_attachment_part_ids: BTreeSet<i64>,
    pub last_seq: i64,
    pub rollback: Option<SessionRolledBack>,
}

#[derive(Debug, Error)]
pub(crate) enum HistoryReplayError {
    #[error(transparent)]
    MessageState(#[from] MessageStateStoreError),
}

#[derive(Debug, Default)]
struct HistoryReducer {
    store: MessageStateStore,
    projection: SessionHistoryProjection,
}

pub(crate) fn replay_history(
    records: &[HistoryRecord],
) -> Result<SessionHistoryProjection, HistoryReplayError> {
    let mut reducer = HistoryReducer::default();
    for record in records {
        reducer.apply(record)?;
    }
    let mut projection = reducer.projection;
    projection.messages = reducer.store.list_message_snapshots();
    projection.messages.sort_by_key(|message| (message.created_at, message.id));
    Ok(projection)
}

impl HistoryReducer {
    fn add_message_tag(
        &mut self,
        message_id: i64,
        tag: &'static str,
    ) -> Result<(), HistoryReplayError> {
        if let Some(mut message) = self.store.get_message_snapshot(message_id) {
            message.metadata.add_tag(tag);
            self.store.apply(MessageUpdate::SetMessageMetadata {
                message_id,
                metadata: message.metadata,
            })?;
        }
        Ok(())
    }

    fn rewind_to_message(&mut self, target_message_id: i64) -> Result<(), HistoryReplayError> {
        let messages = self.store.list_message_snapshots();
        let Some(target) = messages.iter().find(|message| message.id == target_message_id) else {
            return Ok(());
        };
        let target_key = (target.created_at, target.id);
        let mut next_store = MessageStateStore::default();
        for message in messages
            .into_iter()
            .filter(|message| (message.created_at, message.id) <= target_key)
        {
            next_store.apply(MessageUpdate::ReplaceMessageSnapshot { message })?;
        }
        self.store = next_store;
        Ok(())
    }

    fn apply(&mut self, record: &HistoryRecord) -> Result<(), HistoryReplayError> {
        self.projection.last_seq = record.seq;

        match &record.item {
            HistoryItem::MessageStarted(event) => {
                self.store.apply(MessageUpdate::InsertMessage {
                    message: Message {
                        id: event.message_id,
                        role: event.role,
                        state: MessageStatus::Pending,
                        parts: Vec::new(),
                        created_at: event.created_at,
                        metadata: event.metadata.clone(),
                        usage: None,
                        finish: None,
                    },
                })?;
                if event.state != MessageStatus::Pending {
                    self.store.apply(MessageUpdate::TransitionMessage {
                        message_id: event.message_id,
                        to: event.state,
                    })?;
                }
            }
            HistoryItem::MessageSnapshotRecorded(event) => {
                self.store.apply(MessageUpdate::ReplaceMessageSnapshot {
                    message: event.message.clone(),
                })?;
            }
            HistoryItem::MessageStateChanged(event) => {
                self.store.apply(MessageUpdate::TransitionMessage {
                    message_id: event.message_id,
                    to: event.state,
                })?;
            }
            HistoryItem::MessageUsageSet(event) => {
                self.store.apply(MessageUpdate::SetMessageUsage {
                    message_id: event.message_id,
                    usage: event.usage.clone(),
                })?;
            }
            HistoryItem::MessageFinishSet(event) => {
                self.store.apply(MessageUpdate::SetMessageFinish {
                    message_id: event.message_id,
                    finish: event.finish.clone(),
                })?;
            }
            HistoryItem::MessageTagsAdded(event) => {
                if let Some(mut message) = self.store.get_message_snapshot(event.message_id) {
                    for tag in &event.tags {
                        message.metadata.add_tag(tag.clone());
                    }
                    self.store.apply(MessageUpdate::SetMessageMetadata {
                        message_id: event.message_id,
                        metadata: message.metadata,
                    })?;
                }
            }
            HistoryItem::MessageTagsRemoved(event) => {
                if let Some(mut message) = self.store.get_message_snapshot(event.message_id) {
                    message.metadata.tags.retain(|tag| !event.tags.contains(tag));
                    self.store.apply(MessageUpdate::SetMessageMetadata {
                        message_id: event.message_id,
                        metadata: message.metadata,
                    })?;
                }
            }
            HistoryItem::PartStarted(event) => {
                self.store.apply(MessageUpdate::InsertPart {
                    message_id: event.message_id,
                    part: event.part.clone(),
                })?;
            }
            HistoryItem::PartStatusChanged(event) => {
                self.store.apply(MessageUpdate::TransitionPart {
                    part_id: event.part_id,
                    to: event.status,
                })?;
            }
            HistoryItem::PartOperationIdSet(event) => {
                self.store.apply(MessageUpdate::SetPartOperationId {
                    part_id: event.part_id,
                    operation_id: event.operation_id.clone(),
                })?;
            }
            HistoryItem::PartContentDelta(event) => apply_delta(&mut self.store, event)?,
            HistoryItem::PartContentReplaced(event) => {
                self.store.apply(MessageUpdate::ReplacePartContent {
                    part_id: event.part_id,
                    content: event.content.clone(),
                })?;
            }
            HistoryItem::PromptCompactionApplied(event) => {
                self.projection
                    .compacted_message_ids
                    .extend(event.compacted_message_ids.iter().copied());
                for message_id in &event.compacted_message_ids {
                    self.add_message_tag(*message_id, MESSAGE_TAG_PROMPT_COMPACTED)?;
                }
            }
            HistoryItem::ToolResultPruned(ToolResultPruned { message_id, part_id, .. }) => {
                self.projection.pruned_part_ids.insert(*part_id);
                self.add_message_tag(*message_id, MESSAGE_TAG_TOOL_RESULT_PRUNED)?;
                if let Some(mut message) = self.store.get_message_snapshot(*message_id) {
                    for part in &mut message.parts {
                        if part.id == *part_id {
                            part.set_content(crate::message::PartContent::text(
                                PRUNED_TOOL_RESULT_PLACEHOLDER,
                            ));
                        }
                    }
                    self.store.apply(MessageUpdate::ReplaceMessageSnapshot { message })?;
                }
            }
            HistoryItem::AttachmentPayloadStripped(AttachmentPayloadStripped { message_id, part_id, .. }) => {
                self.projection.stripped_attachment_part_ids.insert(*part_id);
                self.add_message_tag(*message_id, MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED)?;
                if let Some(mut message) = self.store.get_message_snapshot(*message_id) {
                    for part in &mut message.parts {
                        if part.id == *part_id {
                            strip_attachment_payload(part);
                        }
                    }
                    self.store.apply(MessageUpdate::ReplaceMessageSnapshot { message })?;
                }
            }
            HistoryItem::PromptWindowInvalidated(event) => {
                self.projection.runtime.prompt_window.generation = event.generation;
                match event.reason {
                    PromptWindowInvalidationReason::Compaction
                    | PromptWindowInvalidationReason::ToolResultPruning
                    | PromptWindowInvalidationReason::AttachmentPayloadStripping
                    | PromptWindowInvalidationReason::Rewind
                    | PromptWindowInvalidationReason::Manual => {
                        self.projection.runtime.clear_provider_anchors();
                        self.projection.runtime.clear_prompt_tokens();
                    }
                }
            }
            HistoryItem::ProviderAnchorSet(event) => {
                self.projection.runtime.set_provider_anchor(event.anchor.clone());
            }
            HistoryItem::ProviderAnchorCleared(event) => {
                self.projection
                    .runtime
                    .clear_provider_anchor(event.provider_id.as_str(), event.model_id.as_str());
            }
            HistoryItem::ProviderAnchorsCleared(_) => {
                self.projection.runtime.clear_provider_anchors();
            }
            HistoryItem::PromptTokensRecorded(event) => {
                self.projection.runtime.prompt_tokens = event.runtime.clone();
            }
            HistoryItem::PromptTokensCleared(_) => {
                self.projection.runtime.clear_prompt_tokens();
            }
            HistoryItem::LoadedDeferredToolsRecorded(event) => {
                self.projection.runtime.loaded_deferred_tools = event.tools.clone();
            }
            HistoryItem::SessionRuntimeRecorded(event) => {
                self.projection.runtime = event.runtime.clone();
            }
            HistoryItem::SessionRolledBack(event) => {
                self.rewind_to_message(event.target_message_id)?;
                self.projection.rollback = Some(event.clone());
            }
            HistoryItem::ClientEventRecorded(_) | HistoryItem::LegacySnapshotImported(_) => {}
        }

        Ok(())
    }
}

fn strip_attachment_payload(part: &mut MessagePart) {
    let Some(PartContent::Attachment(mut attachment)) = part.content.clone() else {
        return;
    };

    for item in &mut attachment.attachments {
        item.source = match &item.source {
            AttachmentSource::DataUrl { .. } | AttachmentSource::Base64 { .. } => {
                AttachmentSource::FileId {
                    file_id: item.summary_label(),
                }
            }
            AttachmentSource::Url { url } => AttachmentSource::Url { url: url.clone() },
            AttachmentSource::FileId { file_id } => AttachmentSource::FileId {
                file_id: file_id.clone(),
            },
            AttachmentSource::LocalPath { path } => AttachmentSource::LocalPath { path: path.clone() },
        };
    }

    part.set_content(PartContent::Attachment(attachment));
}

fn apply_delta(
    store: &mut MessageStateStore,
    event: &PartContentDelta,
) -> Result<(), MessageStateStoreError> {
    match event {
        PartContentDelta::Text { part_id, delta } => store.apply(MessageUpdate::AppendTextDelta {
            part_id: *part_id,
            delta: delta.clone(),
        }),
        PartContentDelta::ReasoningSummary { part_id, delta } => {
            store.apply(MessageUpdate::AppendReasoningSummaryDelta {
                part_id: *part_id,
                delta: delta.clone(),
            })
        }
        PartContentDelta::ReasoningRaw { part_id, delta } => {
            store.apply(MessageUpdate::AppendReasoningRawDelta {
                part_id: *part_id,
                delta: delta.clone(),
            })
        }
        PartContentDelta::CommandOutput { part_id, delta } => {
            store.apply(MessageUpdate::AppendCommandOutputDelta {
                part_id: *part_id,
                delta: delta.clone(),
            })
        }
        PartContentDelta::ToolOutput { part_id, delta } => {
            store.apply(MessageUpdate::AppendToolOutputDelta {
                part_id: *part_id,
                delta: delta.clone(),
            })
        }
    }
}

