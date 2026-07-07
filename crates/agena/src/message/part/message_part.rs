use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    AttachmentPart, ExecutionStatus, ExecutionStatusTransitionError, PartContent, PartKind,
    RequestPart, ToolInvocation,
};

/// Alias kept for ergonomic reuse at MessagePart call sites; the underlying
/// type is the unified [`ExecutionStatusTransitionError`] from
/// [`super::common`].
pub type PartStateTransitionError = ExecutionStatusTransitionError;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagePart {
    pub id: i64,
    pub message_id: i64,
    pub part_index: i32,
    #[serde(default)]
    pub status: ExecutionStatus,
    pub kind: PartKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_detail: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<PartContent>,
}

impl MessagePart {
    pub fn from_content(
        id: i64,
        message_id: i64,
        created_at: DateTime<Utc>,
        status: ExecutionStatus,
        content: PartContent,
    ) -> Self {
        Self::from_content_with_index(id, message_id, 0, created_at, status, content)
    }

    pub fn from_content_with_index(
        id: i64,
        message_id: i64,
        part_index: i32,
        created_at: DateTime<Utc>,
        status: ExecutionStatus,
        content: PartContent,
    ) -> Self {
        let kind = content.kind();
        let name = name_from_content(&content);
        let summary = summary_from_content(&content);
        Self {
            id,
            message_id,
            part_index,
            status,
            kind,
            name,
            summary,
            has_detail: true,
            operation_id: None,
            created_at,
            content: Some(content),
        }
    }

    pub fn without_detail(&self) -> Self {
        let mut part = self.clone();
        part.content = None;
        part
    }

    pub const fn kind(&self) -> PartKind {
        self.kind
    }

    pub fn text(&self) -> Option<&str> {
        self.content.as_ref().and_then(PartContent::text_value)
    }

    pub fn reasoning_summary(&self) -> Option<&[String]> {
        self.content
            .as_ref()
            .and_then(PartContent::reasoning_summary_value)
    }

    pub fn set_content(&mut self, content: PartContent) {
        self.content = Some(content);
        self.refresh_summary_from_content();
    }

    pub fn append_text_delta(&mut self, delta: &str) -> bool {
        let updated = self
            .content
            .as_mut()
            .is_some_and(|content| content.append_text_delta(delta));
        if updated {
            self.refresh_summary_from_content();
        }
        updated
    }

    pub fn append_reasoning_summary_delta(&mut self, delta: String) -> bool {
        let updated = self
            .content
            .as_mut()
            .is_some_and(|content| content.append_reasoning_summary_delta(delta));
        if updated {
            self.refresh_summary_from_content();
        }
        updated
    }

    pub fn append_reasoning_raw_delta(&mut self, delta: String) -> bool {
        let updated = self
            .content
            .as_mut()
            .is_some_and(|content| content.append_reasoning_raw_delta(delta));
        if updated {
            self.refresh_summary_from_content();
        }
        updated
    }

    pub fn append_command_output_delta(&mut self, delta: &str) -> bool {
        let updated = self
            .content
            .as_mut()
            .is_some_and(|content| content.append_command_output_delta(delta));
        if updated {
            self.refresh_summary_from_content();
        }
        updated
    }

    pub fn append_tool_output_delta(&mut self, delta: &str) -> bool {
        let updated = self
            .content
            .as_mut()
            .is_some_and(|content| content.append_tool_output_delta(delta));
        if updated {
            self.refresh_summary_from_content();
        }
        updated
    }

    pub fn transition_status(
        &mut self,
        to: ExecutionStatus,
    ) -> Result<(), PartStateTransitionError> {
        let from = self.status;
        if !from.can_transition(to) {
            return Err(PartStateTransitionError { from, to });
        }
        self.status = to;
        Ok(())
    }

    fn refresh_summary_from_content(&mut self) {
        if let Some(content) = self.content.as_ref() {
            self.kind = content.kind();
            self.name = name_from_content(content);
            self.summary = summary_from_content(content);
            self.has_detail = true;
        }
    }
}

fn name_from_content(content: &PartContent) -> Option<String> {
    match content {
        PartContent::Text(_) => Some("text".to_string()),
        PartContent::Reasoning(_) => Some("reasoning".to_string()),
        PartContent::Operation(operation) => Some(tool_name(operation.invocation())),
        PartContent::Error(error) => {
            let code = error.code.trim();
            if code.is_empty() {
                Some("error".to_string())
            } else {
                Some(code.to_string())
            }
        }
        PartContent::Attachment(_) => Some("attachment".to_string()),
        PartContent::Request(RequestPart::Permission(_)) => Some("permission".to_string()),
        PartContent::Request(RequestPart::UserInput(_)) => Some("user_input".to_string()),
    }
}

fn summary_from_content(content: &PartContent) -> Option<String> {
    match content {
        PartContent::Text(text) => truncate_summary(&text.text),
        PartContent::Reasoning(reasoning) => truncate_summary(&reasoning.preferred_text()),
        PartContent::Operation(operation) => {
            let invocation = operation.invocation();
            let candidate = operation
                .error_message()
                .or_else(|| operation.output_text())
                .or_else(|| operation.title())
                .or_else(|| (!operation.summary.is_empty()).then_some(operation.summary.as_str()));
            candidate
                .and_then(truncate_summary)
                .or_else(|| truncate_summary(&tool_name(invocation)))
        }
        PartContent::Error(error) => {
            truncate_summary(&format!("{}: {}", error.code.trim(), error.message.trim()))
        }
        PartContent::Attachment(attachment) => attachment_part_summary(attachment),
        PartContent::Request(request) => truncate_summary(request.summary_text().as_str()),
    }
}

fn attachment_part_summary(part: &AttachmentPart) -> Option<String> {
    let count = part.attachments.len();
    if count == 0 {
        return Some("0 attachment(s)".to_string());
    }

    let first = part
        .attachments
        .first()
        .map(|item| item.summary_label())
        .unwrap_or_else(|| "attachment".to_string());

    if count == 1 {
        truncate_summary(&format!("1 attachment: {first}"))
    } else {
        truncate_summary(&format!("{count} attachments (first: {first})"))
    }
}

fn tool_name(invocation: &ToolInvocation) -> String {
    let ToolInvocation { name, .. } = invocation;
    name.clone()
}

fn truncate_summary(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    const LIMIT: usize = 240;
    let mut chars = trimmed.chars();
    let mut summary = String::new();
    for _ in 0..LIMIT {
        if let Some(ch) = chars.next() {
            summary.push(ch);
        } else {
            return Some(summary);
        }
    }

    if chars.next().is_some() {
        summary.push('…');
    }

    Some(summary)
}
