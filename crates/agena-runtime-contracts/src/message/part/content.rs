use agena_domain::{ErrorPart, PartKind, ReasoningPart, TextPart};
use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use super::{ActivityPart, AttachmentItem, AttachmentPart, OperationPart, RequestPart};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum PartContent {
    Text(TextPart),
    Reasoning(ReasoningPart),
    Operation(OperationPart),
    Activity(ActivityPart),
    Attachment(AttachmentPart),
    Request(RequestPart),
    Error(ErrorPart),
}

impl PartContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextPart {
            text: text.into(),
            synthetic: false,
        })
    }

    pub fn reasoning_summary(summary: impl Into<String>) -> Self {
        Self::Reasoning(ReasoningPart {
            summary: vec![summary.into()],
            raw_content: Vec::new(),
            encrypted_content: None,
        })
    }

    pub fn attachments(items: Vec<AttachmentItem>) -> Self {
        Self::Attachment(AttachmentPart { attachments: items })
    }

    pub fn request(part: RequestPart) -> Self {
        Self::Request(part)
    }

    pub const fn kind(&self) -> PartKind {
        match self {
            Self::Text(_) => PartKind::Text,
            Self::Reasoning(_) => PartKind::Reasoning,
            Self::Operation(_) => PartKind::Operation,
            Self::Activity(_) => PartKind::Activity,
            Self::Attachment(_) => PartKind::Attachment,
            Self::Request(_) => PartKind::Request,
            Self::Error(_) => PartKind::Error,
        }
    }

    pub fn text_value(&self) -> Option<&str> {
        match self {
            Self::Text(part) => Some(part.text.as_str()),
            _ => None,
        }
    }

    pub fn reasoning_summary_value(&self) -> Option<&[String]> {
        match self {
            Self::Reasoning(part) => Some(part.summary.as_slice()),
            _ => None,
        }
    }

    pub fn append_text_delta(&mut self, delta: &str) -> bool {
        match self {
            Self::Text(part) => {
                part.text.push_str(delta);
                true
            }
            _ => false,
        }
    }

    pub fn append_reasoning_summary_delta(&mut self, delta: impl Into<String>) -> bool {
        match self {
            Self::Reasoning(part) => {
                part.summary.push(delta.into());
                true
            }
            _ => false,
        }
    }

    pub fn append_reasoning_raw_delta(&mut self, delta: impl Into<String>) -> bool {
        match self {
            Self::Reasoning(part) => {
                part.raw_content.push(delta.into());
                true
            }
            _ => false,
        }
    }

    pub fn append_command_output_delta(&mut self, delta: &str) -> bool {
        self.append_operation_output_delta(delta)
    }

    pub fn append_tool_output_delta(&mut self, delta: &str) -> bool {
        self.append_operation_output_delta(delta)
    }

    pub fn append_operation_output_delta(&mut self, delta: &str) -> bool {
        match self {
            Self::Operation(part) => part.append_output_delta(delta),
            _ => false,
        }
    }
}
