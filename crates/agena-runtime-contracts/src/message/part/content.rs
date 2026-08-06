use agena_domain::{ErrorPart, PartKind, ReasoningPart, TextPart};
use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use super::{AttachmentItem, AttachmentPart, HookPart, OperationPart, RequestPart, SkillReferencePart};

/// Rich execution payload carried by the one structured-content envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "activity_type", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum RuntimeActivity {
    Reasoning(ReasoningPart),
    Operation(OperationPart),
    Resource(AttachmentPart),
    SkillReference(SkillReferencePart),
    Interaction(RequestPart),
    Hook(HookPart),
    Error(ErrorPart),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum PartContent {
    Text(TextPart),
    Activity(RuntimeActivity),
}

impl PartContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextPart {
            text: text.into(),
            synthetic: false,
        })
    }

    pub fn reasoning_summary(summary: impl Into<String>) -> Self {
        Self::Activity(RuntimeActivity::Reasoning(ReasoningPart {
            summary: vec![summary.into()],
            raw_content: Vec::new(),
            encrypted_content: None,
        }))
    }

    pub fn attachments(items: Vec<AttachmentItem>) -> Self {
        Self::Activity(RuntimeActivity::Resource(AttachmentPart {
            attachments: items,
        }))
    }

    pub fn request(part: RequestPart) -> Self {
        Self::Activity(RuntimeActivity::Interaction(part))
    }

    pub fn operation(part: OperationPart) -> Self {
        Self::Activity(RuntimeActivity::Operation(part))
    }

    pub fn skill_reference(part: SkillReferencePart) -> Self {
        Self::Activity(RuntimeActivity::SkillReference(part))
    }

    pub fn hook(part: HookPart) -> Self {
        Self::Activity(RuntimeActivity::Hook(part))
    }

    pub fn error(part: ErrorPart) -> Self {
        Self::Activity(RuntimeActivity::Error(part))
    }

    pub const fn kind(&self) -> PartKind {
        match self {
            Self::Text(_) => PartKind::Text,
            Self::Activity(_) => PartKind::Activity,
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
            Self::Activity(RuntimeActivity::Reasoning(part)) => Some(part.summary.as_slice()),
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
            Self::Activity(RuntimeActivity::Reasoning(part)) => {
                part.summary.push(delta.into());
                true
            }
            _ => false,
        }
    }

    pub fn append_reasoning_raw_delta(&mut self, delta: impl Into<String>) -> bool {
        match self {
            Self::Activity(RuntimeActivity::Reasoning(part)) => {
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
        match self {
            Self::Activity(RuntimeActivity::Operation(part)) => part.append_output_delta(delta),
            _ => false,
        }
    }

    pub fn append_operation_output_delta(&mut self, delta: &str) -> bool {
        match self {
            Self::Activity(RuntimeActivity::Operation(part)) => part.append_output_delta(delta),
            _ => false,
        }
    }
}
