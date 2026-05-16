use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use super::{
    AttachmentItem, AttachmentPart, ErrorPart, OperationPart, PartKind, PermissionRequestPart,
    ReasoningPart, RequestPart, TextPart, UserInputRequestPart,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PartContent {
    Text(TextPart),
    Reasoning(ReasoningPart),
    Operation(OperationPart),
    Attachment(AttachmentPart),
    Request(RequestPart),
    Error(ErrorPart),
}

impl PartContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextPart {
            text: text.into(),
            synthetic: false,
            ignored: false,
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

    pub fn permission_request(part: PermissionRequestPart) -> Self {
        Self::Request(RequestPart::Permission(part))
    }

    pub fn user_input_request(part: UserInputRequestPart) -> Self {
        Self::Request(RequestPart::UserInput(part))
    }

    pub const fn kind(&self) -> PartKind {
        match self {
            Self::Text(_) => PartKind::Text,
            Self::Reasoning(_) => PartKind::Reasoning,
            Self::Operation(_) => PartKind::Operation,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{AttachmentKind, AttachmentSource};

    #[test]
    fn attachments_constructor_sets_attachment_kind() {
        let content = PartContent::attachments(vec![AttachmentItem {
            kind: AttachmentKind::File,
            mime: "application/octet-stream".to_owned(),
            source: AttachmentSource::FileId {
                file_id: "f_1".to_owned(),
            },
            filename: Some("blob.bin".to_owned()),
            title: None,
            size_bytes: None,
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        }]);

        assert_eq!(content.kind(), PartKind::Attachment);
    }
}
