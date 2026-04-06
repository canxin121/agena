use std::collections::HashMap;

use thiserror::Error;

use super::{
    ExecutionStatus, Message, MessagePart, MessageStateTransitionError, MessageStatus,
    MessageUsage, PartContent, PartKind, PartStateTransitionError,
};

#[derive(Debug, Clone)]
struct MessageEntry {
    message: Message,
    part_order: Vec<i64>,
    parts: HashMap<i64, MessagePart>,
}

impl MessageEntry {
    fn snapshot(&self) -> Message {
        let mut message = self.message.clone();
        message.parts = self
            .part_order
            .iter()
            .filter_map(|part_id| self.parts.get(part_id).cloned())
            .collect();
        message
    }

    fn summary_snapshot(&self) -> Message {
        let mut message = self.message.clone();
        message.parts = self
            .part_order
            .iter()
            .filter_map(|part_id| self.parts.get(part_id).map(MessagePart::without_detail))
            .collect();
        message
    }
}

#[derive(Debug, Clone)]
pub enum MessageUpdate {
    InsertMessage {
        message: Message,
    },
    TransitionMessage {
        message_id: i64,
        to: MessageStatus,
    },
    SetMessageFinish {
        message_id: i64,
        finish: String,
    },
    SetMessageUsage {
        message_id: i64,
        usage: MessageUsage,
    },
    InsertPart {
        message_id: i64,
        part: MessagePart,
    },
    SetPartOperationId {
        part_id: i64,
        operation_id: String,
    },
    ReplacePartContent {
        part_id: i64,
        content: PartContent,
    },
    TransitionPart {
        part_id: i64,
        to: ExecutionStatus,
    },
    AppendTextDelta {
        part_id: i64,
        delta: String,
    },
    AppendReasoningSummaryDelta {
        part_id: i64,
        delta: String,
    },
    AppendReasoningRawDelta {
        part_id: i64,
        delta: String,
    },
    AppendCommandOutputDelta {
        part_id: i64,
        delta: String,
    },
    AppendToolOutputDelta {
        part_id: i64,
        delta: String,
    },
}

#[derive(Debug, Error)]
pub enum MessageStateStoreError {
    #[error("message not found: {0}")]
    MessageNotFound(i64),
    #[error("message already exists: {0}")]
    MessageAlreadyExists(i64),
    #[error("part not found: {0}")]
    PartNotFound(i64),
    #[error("part already exists: {0}")]
    PartAlreadyExists(i64),
    #[error("invalid message transition for {message_id}: {from:?} -> {to:?}")]
    InvalidMessageTransition {
        message_id: i64,
        from: MessageStatus,
        to: MessageStatus,
    },
    #[error("invalid part transition for {part_id}: {from:?} -> {to:?}")]
    InvalidPartTransition {
        part_id: i64,
        from: ExecutionStatus,
        to: ExecutionStatus,
    },
    #[error("part {part_id} cannot apply {operation}; current kind is {kind:?}")]
    PartKindMismatch {
        part_id: i64,
        operation: &'static str,
        kind: PartKind,
    },
    #[error("part {part_id} is not stream-updatable in status {status:?}")]
    PartNotStreaming {
        part_id: i64,
        status: ExecutionStatus,
    },
}

#[derive(Debug, Default)]
pub struct MessageStateStore {
    messages: HashMap<i64, MessageEntry>,
    part_owner: HashMap<i64, i64>,
}

impl MessageStateStore {
    pub fn insert_message(&mut self, mut message: Message) -> Result<(), MessageStateStoreError> {
        if self.messages.contains_key(&message.id) {
            return Err(MessageStateStoreError::MessageAlreadyExists(message.id));
        }

        let message_id = message.id;
        let initial_parts = std::mem::take(&mut message.parts);
        self.messages.insert(
            message_id,
            MessageEntry {
                message,
                part_order: Vec::new(),
                parts: HashMap::new(),
            },
        );

        for part in initial_parts {
            self.insert_part_inner(message_id, part)?;
        }

        Ok(())
    }

    pub fn get_message_snapshot(&self, message_id: i64) -> Option<Message> {
        self.messages.get(&message_id).map(MessageEntry::snapshot)
    }

    pub fn list_message_snapshots(&self) -> Vec<Message> {
        self.messages.values().map(MessageEntry::snapshot).collect()
    }

    pub fn get_message_summary_snapshot(&self, message_id: i64) -> Option<Message> {
        self.messages
            .get(&message_id)
            .map(MessageEntry::summary_snapshot)
    }

    pub fn list_message_summary_snapshots(&self) -> Vec<Message> {
        self.messages
            .values()
            .map(MessageEntry::summary_snapshot)
            .collect()
    }

    pub fn apply(&mut self, update: MessageUpdate) -> Result<(), MessageStateStoreError> {
        match update {
            MessageUpdate::InsertMessage { message } => self.insert_message(message),
            MessageUpdate::TransitionMessage { message_id, to } => {
                let entry = self
                    .messages
                    .get_mut(&message_id)
                    .ok_or(MessageStateStoreError::MessageNotFound(message_id))?;
                entry.message.transition_state(to).map_err(
                    |MessageStateTransitionError { from, to }| {
                        MessageStateStoreError::InvalidMessageTransition {
                            message_id,
                            from,
                            to,
                        }
                    },
                )
            }
            MessageUpdate::SetMessageFinish { message_id, finish } => {
                let entry = self
                    .messages
                    .get_mut(&message_id)
                    .ok_or(MessageStateStoreError::MessageNotFound(message_id))?;
                entry.message.finish = Some(finish);
                Ok(())
            }
            MessageUpdate::SetMessageUsage { message_id, usage } => {
                let entry = self
                    .messages
                    .get_mut(&message_id)
                    .ok_or(MessageStateStoreError::MessageNotFound(message_id))?;
                entry.message.usage = Some(usage);
                Ok(())
            }
            MessageUpdate::InsertPart { message_id, part } => {
                self.insert_part_inner(message_id, part)
            }
            MessageUpdate::SetPartOperationId {
                part_id,
                operation_id,
            } => {
                let (_, part) = self.part_mut(part_id)?;
                part.operation_id = Some(operation_id);
                Ok(())
            }
            MessageUpdate::ReplacePartContent { part_id, content } => {
                let (message_id, part) = self.part_mut(part_id)?;
                part.set_content(content);
                self.touch_message_in_progress(message_id);
                Ok(())
            }
            MessageUpdate::TransitionPart { part_id, to } => {
                let (_, part) = self.part_mut(part_id)?;
                part.transition_status(to)
                    .map_err(|PartStateTransitionError { from, to }| {
                        MessageStateStoreError::InvalidPartTransition { part_id, from, to }
                    })
            }
            MessageUpdate::AppendTextDelta { part_id, delta } => {
                self.append_delta(part_id, "append_text_delta", move |part| {
                    part.append_text_delta(&delta)
                })
            }
            MessageUpdate::AppendReasoningSummaryDelta { part_id, delta } => {
                self.append_delta(part_id, "append_reasoning_summary_delta", move |part| {
                    part.append_reasoning_summary_delta(delta)
                })
            }
            MessageUpdate::AppendReasoningRawDelta { part_id, delta } => {
                self.append_delta(part_id, "append_reasoning_raw_delta", move |part| {
                    part.append_reasoning_raw_delta(delta)
                })
            }
            MessageUpdate::AppendCommandOutputDelta { part_id, delta } => {
                self.append_delta(part_id, "append_command_output_delta", move |part| {
                    part.append_command_output_delta(&delta)
                })
            }
            MessageUpdate::AppendToolOutputDelta { part_id, delta } => {
                self.append_delta(part_id, "append_tool_output_delta", move |part| {
                    part.append_tool_output_delta(&delta)
                })
            }
        }
    }

    fn insert_part_inner(
        &mut self,
        message_id: i64,
        mut part: MessagePart,
    ) -> Result<(), MessageStateStoreError> {
        let entry = self
            .messages
            .get_mut(&message_id)
            .ok_or(MessageStateStoreError::MessageNotFound(message_id))?;

        if self.part_owner.contains_key(&part.id) {
            return Err(MessageStateStoreError::PartAlreadyExists(part.id));
        }

        part.message_id = message_id;
        part.part_index = entry.part_order.len() as i32;
        self.part_owner.insert(part.id, message_id);
        entry.part_order.push(part.id);
        entry.parts.insert(part.id, part);
        self.touch_message_in_progress(message_id);
        Ok(())
    }

    fn part_mut(
        &mut self,
        part_id: i64,
    ) -> Result<(i64, &mut MessagePart), MessageStateStoreError> {
        let message_id = *self
            .part_owner
            .get(&part_id)
            .ok_or(MessageStateStoreError::PartNotFound(part_id))?;
        let entry = self
            .messages
            .get_mut(&message_id)
            .ok_or(MessageStateStoreError::MessageNotFound(message_id))?;
        let part = entry
            .parts
            .get_mut(&part_id)
            .ok_or(MessageStateStoreError::PartNotFound(part_id))?;
        Ok((message_id, part))
    }

    fn append_delta<F>(
        &mut self,
        part_id: i64,
        operation: &'static str,
        apply_fn: F,
    ) -> Result<(), MessageStateStoreError>
    where
        F: FnOnce(&mut MessagePart) -> bool,
    {
        let (message_id, part) = self.part_mut(part_id)?;
        match part.status {
            ExecutionStatus::Pending => {
                part.transition_status(ExecutionStatus::InProgress)
                    .map_err(|PartStateTransitionError { from, to }| {
                        MessageStateStoreError::InvalidPartTransition { part_id, from, to }
                    })?;
            }
            ExecutionStatus::InProgress => {}
            status => {
                return Err(MessageStateStoreError::PartNotStreaming { part_id, status });
            }
        }

        if !apply_fn(part) {
            return Err(MessageStateStoreError::PartKindMismatch {
                part_id,
                operation,
                kind: part.kind(),
            });
        }

        self.touch_message_in_progress(message_id);
        Ok(())
    }

    fn touch_message_in_progress(&mut self, message_id: i64) {
        if let Some(entry) = self.messages.get_mut(&message_id)
            && entry.message.state == MessageStatus::Pending
        {
            let _ = entry.message.transition_state(MessageStatus::InProgress);
        }
    }
}
