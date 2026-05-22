use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::message::metadata::MessageMetadata;
use crate::message::part::{
    ExecutionStatus, ExecutionStatusTransitionError, MessagePart, OperationPart, PartContent,
};
use crate::message::usage::MessageUsage;
use crate::role::Role;

/// Message state mirrors the lifecycle of any execution-tracked entity.
pub type MessageStatus = ExecutionStatus;

/// Alias preserved for callers that already import this name. The underlying
/// type is the unified [`ExecutionStatusTransitionError`].
pub type MessageStateTransitionError = ExecutionStatusTransitionError;

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
            Role::Assistant,
            vec![PartContent::Operation(OperationPart::completed(
                0,
                crate::message::ToolInvocation {
                    name: "tool".to_owned(),
                    plugin_name: None,
                    input: crate::message::StructuredObject::default(),
                },
                output.into(),
                Vec::new(),
                Vec::new(),
                crate::message::ToolOutput::default(),
                crate::message::TimeRange::default(),
            ))],
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
                            let text = reasoning.preferred_text();
                            (!text.is_empty()).then_some(text)
                        }
                        PartContent::Operation(tool) => tool_text_lossy(tool),
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

/// Best-effort textual rendering of an operation part for `as_text_lossy`.
fn tool_text_lossy(tool: &OperationPart) -> Option<String> {
    let candidates = [
        tool.output_text(),
        tool.error_message(),
        tool.title(),
        (!tool.summary.trim().is_empty()).then_some(tool.summary.as_str()),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|s| !s.trim().is_empty())
        .map(str::to_owned)
}
