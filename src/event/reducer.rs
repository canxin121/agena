use chrono::{DateTime, Utc};

use crate::message::{
    AttachmentItem, CommandExecutionPart, ExecutionStatus, Message, MessageMetadata, MessagePart,
    MessageSource, MessageStateStore, MessageStateStoreError, MessageStatus, MessageUpdate,
    MessageUsage, PartContent, TextPart, TimeRange, ToolAttachment, ToolExecutionPart,
    ToolInvocation, ToolOutput, ToolResultBlock,
};
use crate::role::Role;

#[derive(Debug, Clone)]
pub enum AiStreamEvent {
    MessageStarted {
        message_id: i64,
        role: Role,
        created_at: DateTime<Utc>,
        metadata: Option<MessageMetadata>,
    },
    MessageCompleted {
        message_id: i64,
        finish: Option<String>,
        usage: Option<MessageUsage>,
    },
    MessageFailed {
        message_id: i64,
        finish: Option<String>,
    },

    TextPartStarted {
        message_id: i64,
        part_id: i64,
        created_at: DateTime<Utc>,
        synthetic: bool,
        ignored: bool,
    },
    TextDelta {
        part_id: i64,
        delta: String,
    },
    TextCompleted {
        part_id: i64,
    },

    ReasoningPartStarted {
        message_id: i64,
        part_id: i64,
        created_at: DateTime<Utc>,
    },
    ReasoningSummaryDelta {
        part_id: i64,
        delta: String,
    },
    ReasoningRawDelta {
        part_id: i64,
        delta: String,
    },
    ReasoningCompleted {
        part_id: i64,
    },

    AttachmentPartAdded {
        message_id: i64,
        part_id: i64,
        created_at: DateTime<Utc>,
        attachments: Vec<AttachmentItem>,
    },

    ToolExecutionStarted {
        message_id: i64,
        part_id: i64,
        created_at: DateTime<Utc>,
        call_id: i64,
        invocation: ToolInvocation,
        title: String,
    },
    ToolOutputDelta {
        part_id: i64,
        delta: String,
    },
    ToolExecutionCompleted {
        part_id: i64,
        call_id: i64,
        invocation: ToolInvocation,
        output_text: String,
        blocks: Vec<ToolResultBlock>,
        attachments: Vec<ToolAttachment>,
        details: ToolOutput,
        lifecycle: TimeRange,
    },
    ToolExecutionFailed {
        part_id: i64,
        call_id: i64,
        invocation: ToolInvocation,
        error_message: String,
        output_text: String,
        blocks: Vec<ToolResultBlock>,
        attachments: Vec<ToolAttachment>,
        details: ToolOutput,
        lifecycle: TimeRange,
    },

    CommandExecutionStarted {
        message_id: i64,
        part_id: i64,
        created_at: DateTime<Utc>,
        command: String,
    },
    CommandOutputDelta {
        part_id: i64,
        delta: String,
    },
    CommandExecutionCompleted {
        part_id: i64,
        command: String,
        exit_code: i32,
        output: String,
        lifecycle: TimeRange,
    },
    CommandExecutionFailed {
        part_id: i64,
        command: String,
        exit_code: Option<i32>,
        output: String,
        lifecycle: TimeRange,
    },
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MessageReducer;

impl MessageReducer {
    pub fn reduce(&self, event: AiStreamEvent) -> Vec<MessageUpdate> {
        match event {
            AiStreamEvent::MessageStarted {
                message_id,
                role,
                created_at,
                metadata,
            } => {
                let metadata = metadata.unwrap_or_else(|| default_metadata_for_role(role));
                vec![MessageUpdate::InsertMessage {
                    message: Message {
                        id: message_id,
                        role,
                        state: MessageStatus::Pending,
                        parts: Vec::new(),
                        created_at,
                        metadata,
                        usage: None,
                        finish: None,
                    },
                }]
            }
            AiStreamEvent::MessageCompleted {
                message_id,
                finish,
                usage,
            } => {
                let mut updates = vec![MessageUpdate::TransitionMessage {
                    message_id,
                    to: MessageStatus::Completed,
                }];
                if let Some(usage) = usage {
                    updates.push(MessageUpdate::SetMessageUsage { message_id, usage });
                }
                if let Some(finish) = finish {
                    updates.push(MessageUpdate::SetMessageFinish { message_id, finish });
                }
                updates
            }
            AiStreamEvent::MessageFailed { message_id, finish } => {
                let mut updates = vec![MessageUpdate::TransitionMessage {
                    message_id,
                    to: MessageStatus::Failed,
                }];
                if let Some(finish) = finish {
                    updates.push(MessageUpdate::SetMessageFinish { message_id, finish });
                }
                updates
            }

            AiStreamEvent::TextPartStarted {
                message_id,
                part_id,
                created_at,
                synthetic,
                ignored,
            } => {
                vec![MessageUpdate::InsertPart {
                    message_id,
                    part: MessagePart::with_content(
                        part_id,
                        message_id,
                        created_at,
                        ExecutionStatus::Pending,
                        PartContent::Text(TextPart {
                            text: String::new(),
                            synthetic,
                            ignored,
                        }),
                    ),
                }]
            }
            AiStreamEvent::TextDelta { part_id, delta } => {
                vec![MessageUpdate::AppendTextDelta { part_id, delta }]
            }
            AiStreamEvent::TextCompleted { part_id } => vec![MessageUpdate::TransitionPart {
                part_id,
                to: ExecutionStatus::Completed,
            }],

            AiStreamEvent::ReasoningPartStarted {
                message_id,
                part_id,
                created_at,
            } => {
                vec![MessageUpdate::InsertPart {
                    message_id,
                    part: MessagePart::with_content(
                        part_id,
                        message_id,
                        created_at,
                        ExecutionStatus::Pending,
                        PartContent::reasoning_summary(""),
                    ),
                }]
            }
            AiStreamEvent::ReasoningSummaryDelta { part_id, delta } => {
                vec![MessageUpdate::AppendReasoningSummaryDelta { part_id, delta }]
            }
            AiStreamEvent::ReasoningRawDelta { part_id, delta } => {
                vec![MessageUpdate::AppendReasoningRawDelta { part_id, delta }]
            }
            AiStreamEvent::ReasoningCompleted { part_id } => vec![MessageUpdate::TransitionPart {
                part_id,
                to: ExecutionStatus::Completed,
            }],

            AiStreamEvent::AttachmentPartAdded {
                message_id,
                part_id,
                created_at,
                attachments,
            } => {
                vec![MessageUpdate::InsertPart {
                    message_id,
                    part: MessagePart::with_content(
                        part_id,
                        message_id,
                        created_at,
                        ExecutionStatus::Completed,
                        PartContent::attachments(attachments),
                    ),
                }]
            }

            AiStreamEvent::ToolExecutionStarted {
                message_id,
                part_id,
                created_at,
                call_id,
                invocation,
                title,
            } => {
                vec![MessageUpdate::InsertPart {
                    message_id,
                    part: MessagePart::with_content(
                        part_id,
                        message_id,
                        created_at,
                        ExecutionStatus::Pending,
                        PartContent::ToolExecution(ToolExecutionPart::Pending {
                            call_id,
                            invocation,
                            title,
                            lifecycle: TimeRange {
                                start_ms: created_at.timestamp_millis(),
                                end_ms: None,
                            },
                        }),
                    ),
                }]
            }
            AiStreamEvent::ToolOutputDelta { part_id, delta } => {
                vec![MessageUpdate::AppendToolOutputDelta { part_id, delta }]
            }
            AiStreamEvent::ToolExecutionCompleted {
                part_id,
                call_id,
                invocation,
                output_text,
                blocks,
                attachments,
                details,
                lifecycle,
            } => vec![
                MessageUpdate::ReplacePartContent {
                    part_id,
                    content: PartContent::ToolExecution(ToolExecutionPart::Completed {
                        call_id,
                        invocation,
                        output_text,
                        blocks,
                        attachments,
                        details,
                        lifecycle,
                    }),
                },
                MessageUpdate::TransitionPart {
                    part_id,
                    to: ExecutionStatus::Completed,
                },
            ],
            AiStreamEvent::ToolExecutionFailed {
                part_id,
                call_id,
                invocation,
                error_message,
                output_text,
                blocks,
                attachments,
                details,
                lifecycle,
            } => vec![
                MessageUpdate::ReplacePartContent {
                    part_id,
                    content: PartContent::ToolExecution(ToolExecutionPart::Failed {
                        call_id,
                        invocation,
                        error_message,
                        output_text,
                        blocks,
                        attachments,
                        details,
                        lifecycle,
                    }),
                },
                MessageUpdate::TransitionPart {
                    part_id,
                    to: ExecutionStatus::Failed,
                },
            ],

            AiStreamEvent::CommandExecutionStarted {
                message_id,
                part_id,
                created_at,
                command,
            } => {
                vec![MessageUpdate::InsertPart {
                    message_id,
                    part: MessagePart::with_content(
                        part_id,
                        message_id,
                        created_at,
                        ExecutionStatus::Pending,
                        PartContent::CommandExecution(CommandExecutionPart {
                            command,
                            status: ExecutionStatus::Pending,
                            lifecycle: TimeRange {
                                start_ms: created_at.timestamp_millis(),
                                end_ms: None,
                            },
                            exit_code: None,
                            output: None,
                        }),
                    ),
                }]
            }
            AiStreamEvent::CommandOutputDelta { part_id, delta } => {
                vec![MessageUpdate::AppendCommandOutputDelta { part_id, delta }]
            }
            AiStreamEvent::CommandExecutionCompleted {
                part_id,
                command,
                exit_code,
                output,
                lifecycle,
            } => vec![
                MessageUpdate::ReplacePartContent {
                    part_id,
                    content: PartContent::CommandExecution(CommandExecutionPart {
                        command,
                        status: ExecutionStatus::Completed,
                        lifecycle,
                        exit_code: Some(exit_code),
                        output: if output.is_empty() {
                            None
                        } else {
                            Some(output)
                        },
                    }),
                },
                MessageUpdate::TransitionPart {
                    part_id,
                    to: ExecutionStatus::Completed,
                },
            ],
            AiStreamEvent::CommandExecutionFailed {
                part_id,
                command,
                exit_code,
                output,
                lifecycle,
            } => vec![
                MessageUpdate::ReplacePartContent {
                    part_id,
                    content: PartContent::CommandExecution(CommandExecutionPart {
                        command,
                        status: ExecutionStatus::Failed,
                        lifecycle,
                        exit_code,
                        output: if output.is_empty() {
                            None
                        } else {
                            Some(output)
                        },
                    }),
                },
                MessageUpdate::TransitionPart {
                    part_id,
                    to: ExecutionStatus::Failed,
                },
            ],
        }
    }

    pub fn apply_to_store(
        &self,
        store: &mut MessageStateStore,
        event: AiStreamEvent,
    ) -> Result<(), MessageStateStoreError> {
        for update in self.reduce(event) {
            store.apply(update)?;
        }
        Ok(())
    }
}

fn default_metadata_for_role(role: Role) -> MessageMetadata {
    let source = match role {
        Role::User => MessageSource::User,
        Role::Assistant => MessageSource::Assistant,
        Role::System => MessageSource::System,
        Role::Tool => MessageSource::Tool,
    };

    MessageMetadata {
        source,
        ..MessageMetadata::default()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::message::{AttachmentKind, AttachmentSource};

    #[test]
    fn attachment_part_added_event_creates_attachment_content() {
        let reducer = MessageReducer;
        let updates = reducer.reduce(AiStreamEvent::AttachmentPartAdded {
            message_id: 7,
            part_id: 9,
            created_at: Utc::now(),
            attachments: vec![AttachmentItem {
                kind: AttachmentKind::Image,
                mime: "image/png".to_owned(),
                source: AttachmentSource::Url {
                    url: "https://example.com/p.png".to_owned(),
                },
                filename: Some("p.png".to_owned()),
                title: None,
                size_bytes: Some(12),
                sha256: None,
                width: Some(10),
                height: Some(10),
                duration_ms: None,
                page_count: None,
            }],
        });

        assert_eq!(updates.len(), 1);
        let MessageUpdate::InsertPart { part, .. } = &updates[0] else {
            panic!("expected insert part update")
        };
        assert_eq!(part.kind, crate::message::PartKind::Attachment);
        assert!(matches!(part.content, Some(PartContent::Attachment(_))));
    }
}
