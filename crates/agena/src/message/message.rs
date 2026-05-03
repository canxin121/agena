use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};
use thiserror::Error;

use crate::message::metadata::MessageMetadata;
use crate::message::part::{ExecutionStatus, MessagePart, PartContent, ToolExecutionPart};
use crate::message::usage::MessageUsage;
use crate::role::Role;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: i64,
    pub role: Role,
    #[serde(default)]
    pub state: MessageStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MessagePart>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: MessageMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    Default,
    PartialEq,
    Eq,
    Hash,
    AsRefStr,
    Display,
    EnumString,
    EnumIter,
    DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[sea_orm(rs_type = "i8", db_type = "TinyInteger")]
pub enum MessageStatus {
    #[default]
    #[sea_orm(num_value = 1)]
    Pending,
    #[sea_orm(num_value = 2)]
    InProgress,
    #[sea_orm(num_value = 3)]
    Completed,
    #[sea_orm(num_value = 4)]
    Failed,
}

impl MessageStatus {
    pub fn can_transition(self, next: Self) -> bool {
        if self == next {
            return true;
        }

        matches!(
            (self, next),
            (Self::Pending, Self::InProgress | Self::Failed)
                | (Self::InProgress, Self::Completed | Self::Failed)
        )
    }
}

#[derive(Debug, Error)]
#[error("invalid message state transition: {from:?} -> {to:?}")]
pub struct MessageStateTransitionError {
    pub from: MessageStatus,
    pub to: MessageStatus,
}

impl Message {
    pub fn prompt_text(role: Role, content: impl Into<String>) -> Self {
        Self::prompt_parts(role, vec![PartContent::text(content)])
    }

    pub fn prompt_parts(role: Role, parts: Vec<PartContent>) -> Self {
        let created_at = Utc::now();
        let mut message = Self {
            id: 0,
            role,
            state: MessageStatus::Completed,
            parts: Vec::new(),
            created_at,
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
        };

        for (idx, content) in parts.into_iter().enumerate() {
            let mut part = MessagePart::with_content(
                idx as i64 + 1,
                message.id,
                created_at,
                ExecutionStatus::Completed,
                content,
            );
            part.part_index = idx as i32;
            message.parts.push(part);
        }

        message
    }

    pub fn prompt_tool_result(tool_call_id: impl Into<String>, output: impl Into<String>) -> Self {
        let mut message = Self::prompt_parts(
            Role::Tool,
            vec![PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: 0,
                invocation: crate::message::ToolInvocation::Custom {
                    name: "tool".to_owned(),
                    input: crate::message::StructuredObject::default(),
                },
                output_text: output.into(),
                blocks: Vec::new(),
                attachments: Vec::new(),
                details: crate::message::ToolOutput::default(),
                lifecycle: crate::message::TimeRange::default(),
            })],
        );
        if let Some(part) = message.parts.first_mut() {
            part.operation_id = Some(tool_call_id.into());
        }
        message
    }

    pub fn as_text_lossy(&self) -> String {
        self.parts
            .iter()
            .filter_map(|part| {
                if let Some(content) = part.content.as_ref() {
                    match content {
                        PartContent::Text(text) => Some(text.text.clone()),
                        PartContent::Reasoning(reasoning) => {
                            if !reasoning.summary.is_empty() {
                                Some(reasoning.summary.join(" "))
                            } else if !reasoning.raw_content.is_empty() {
                                Some(reasoning.raw_content.join(" "))
                            } else {
                                None
                            }
                        }
                        PartContent::ToolExecution(tool) => match tool {
                            ToolExecutionPart::Pending { title, .. } => {
                                if title.trim().is_empty() {
                                    None
                                } else {
                                    Some(title.clone())
                                }
                            }
                            ToolExecutionPart::InProgress {
                                title, output_text, ..
                            } => {
                                if !output_text.trim().is_empty() {
                                    Some(output_text.clone())
                                } else if !title.trim().is_empty() {
                                    Some(title.clone())
                                } else {
                                    None
                                }
                            }
                            ToolExecutionPart::Completed { output_text, .. } => {
                                if output_text.trim().is_empty() {
                                    None
                                } else {
                                    Some(output_text.clone())
                                }
                            }
                            ToolExecutionPart::Failed {
                                output_text,
                                error_message,
                                ..
                            } => {
                                if !output_text.trim().is_empty() {
                                    Some(output_text.clone())
                                } else if !error_message.trim().is_empty() {
                                    Some(error_message.clone())
                                } else {
                                    None
                                }
                            }
                        },
                        _ => part.summary.clone(),
                    }
                } else {
                    part.summary.clone()
                }
            })
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn push_part(&mut self, mut part: MessagePart) {
        part.message_id = self.id;
        part.part_index = self.parts.len() as i32;
        self.parts.push(part);
    }

    pub fn transition_state(
        &mut self,
        next: MessageStatus,
    ) -> Result<(), MessageStateTransitionError> {
        let from = self.state;
        if !from.can_transition(next) {
            return Err(MessageStateTransitionError { from, to: next });
        }
        self.state = next;
        Ok(())
    }
}
