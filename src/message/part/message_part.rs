use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    AttachmentPart, ExecutionStatus, FileChangeEntry, FileChangeKind, FileChangePart, PartContent,
    PartKind, ToolExecutionPart, ToolInvocation,
};

#[derive(Debug, Error)]
#[error("invalid part state transition: {from:?} -> {to:?}")]
pub struct PartStateTransitionError {
    pub from: ExecutionStatus,
    pub to: ExecutionStatus,
}

fn can_transition(from: ExecutionStatus, to: ExecutionStatus) -> bool {
    if from == to {
        return true;
    }

    match (from, to) {
        (ExecutionStatus::Pending, ExecutionStatus::InProgress | ExecutionStatus::Failed) => true,
        (ExecutionStatus::InProgress, ExecutionStatus::Completed | ExecutionStatus::Failed) => true,
        _ => false,
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePartSummary {
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
    pub fn with_content(
        id: i64,
        message_id: i64,
        created_at: DateTime<Utc>,
        status: ExecutionStatus,
        content: PartContent,
    ) -> Self {
        let mut part = Self {
            id,
            message_id,
            part_index: 0,
            status,
            kind: content.kind(),
            name: None,
            summary: None,
            has_detail: true,
            operation_id: None,
            created_at,
            content: Some(content),
        };
        part.refresh_summary_from_content();
        part
    }

    pub fn from_summary(summary: MessagePartSummary, content: Option<PartContent>) -> Self {
        let mut part = Self {
            id: summary.id,
            message_id: summary.message_id,
            part_index: summary.part_index,
            status: summary.status,
            kind: summary.kind,
            name: summary.name,
            summary: summary.summary,
            has_detail: summary.has_detail,
            operation_id: summary.operation_id,
            created_at: summary.created_at,
            content,
        };

        if part.content.is_some() {
            part.refresh_summary_from_content();
        }

        part
    }

    pub fn summary_view(&self) -> MessagePartSummary {
        MessagePartSummary {
            id: self.id,
            message_id: self.message_id,
            part_index: self.part_index,
            status: self.status,
            kind: self.kind,
            name: self.name.clone(),
            summary: self.summary.clone(),
            has_detail: self.has_detail,
            operation_id: self.operation_id.clone(),
            created_at: self.created_at,
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
        if !can_transition(from, to) {
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
        PartContent::ToolExecution(tool) => match tool {
            ToolExecutionPart::Pending { invocation, .. }
            | ToolExecutionPart::InProgress { invocation, .. }
            | ToolExecutionPart::Completed { invocation, .. }
            | ToolExecutionPart::Failed { invocation, .. } => Some(tool_name(invocation)),
        },
        PartContent::CommandExecution(_) => Some("command".to_string()),
        PartContent::FileChange(_) => Some("file_change".to_string()),
        PartContent::WebSearch(_) => Some("web_search".to_string()),
        PartContent::TodoList(_) => Some("todo_list".to_string()),
        PartContent::Error(error) => {
            let code = error.code.trim();
            if code.is_empty() {
                Some("error".to_string())
            } else {
                Some(code.to_string())
            }
        }
        PartContent::Attachment(_) => Some("attachment".to_string()),
        PartContent::PermissionRequest(_) => Some("permission_request".to_string()),
    }
}

fn summary_from_content(content: &PartContent) -> Option<String> {
    match content {
        PartContent::Text(text) => truncate_summary(&text.text),
        PartContent::Reasoning(reasoning) => {
            if !reasoning.summary.is_empty() {
                truncate_summary(&reasoning.summary.join(" "))
            } else {
                truncate_summary(&reasoning.raw_content.join(" "))
            }
        }
        PartContent::ToolExecution(tool) => match tool {
            ToolExecutionPart::Pending {
                invocation, title, ..
            }
            | ToolExecutionPart::InProgress {
                invocation, title, ..
            } => {
                if let Some(summary) = truncate_summary(title) {
                    Some(summary)
                } else {
                    truncate_summary(&tool_name(invocation))
                }
            }
            ToolExecutionPart::Completed {
                invocation,
                output_text,
                ..
            } => {
                if let Some(summary) = truncate_summary(output_text) {
                    Some(summary)
                } else {
                    truncate_summary(&tool_name(invocation))
                }
            }
            ToolExecutionPart::Failed {
                invocation,
                error_message,
                output_text,
                ..
            } => {
                if let Some(summary) = truncate_summary(error_message) {
                    Some(summary)
                } else if let Some(summary) = truncate_summary(output_text) {
                    Some(summary)
                } else {
                    truncate_summary(&tool_name(invocation))
                }
            }
        },
        PartContent::CommandExecution(command) => truncate_summary(&command.command),
        PartContent::FileChange(change) => file_change_part_summary(change),
        PartContent::WebSearch(search) => truncate_summary(&search.query),
        PartContent::TodoList(todo) => {
            truncate_summary(&format!("{} todo item(s)", todo.items.len()))
        }
        PartContent::Error(error) => {
            truncate_summary(&format!("{}: {}", error.code.trim(), error.message.trim()))
        }
        PartContent::Attachment(attachment) => attachment_part_summary(attachment),
        PartContent::PermissionRequest(permission) => {
            truncate_summary(permission.summary_text().as_str())
        }
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

fn file_change_part_summary(part: &FileChangePart) -> Option<String> {
    let count = part.changes.len();
    if count == 0 {
        return Some("0 file changes".to_string());
    }

    let first = part
        .changes
        .first()
        .map(file_change_entry_summary)
        .unwrap_or_else(|| "file change".to_string());

    if count == 1 {
        truncate_summary(&format!("1 file change: {first}"))
    } else {
        truncate_summary(&format!("{count} file changes (first: {first})"))
    }
}

fn file_change_entry_summary(change: &FileChangeEntry) -> String {
    format!("{} ({})", change.path, file_change_kind_label(change.kind))
}

const fn file_change_kind_label(kind: FileChangeKind) -> &'static str {
    match kind {
        FileChangeKind::Added => "added",
        FileChangeKind::Updated => "updated",
        FileChangeKind::Deleted => "deleted",
    }
}

fn tool_name(invocation: &ToolInvocation) -> String {
    match invocation {
        ToolInvocation::Builtin { input } => input.to_string(),
        ToolInvocation::Mcp { server, tool, .. } => format!("{server}:{tool}"),
        ToolInvocation::Custom { name, .. } => name.clone(),
    }
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

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::message::{AttachmentItem, AttachmentKind, AttachmentSource};
    use crate::permission::{PermissionAction, PermissionRequest};

    #[test]
    fn attachment_part_sets_kind_name_and_summary() {
        let part = MessagePart::with_content(
            10,
            1,
            Utc::now(),
            ExecutionStatus::Pending,
            PartContent::attachments(vec![AttachmentItem {
                kind: AttachmentKind::Pdf,
                mime: "application/pdf".to_owned(),
                source: AttachmentSource::FileId {
                    file_id: "file_123".to_owned(),
                },
                filename: Some("manual.pdf".to_owned()),
                title: None,
                size_bytes: Some(1024),
                sha256: None,
                width: None,
                height: None,
                duration_ms: None,
                page_count: Some(12),
            }]),
        );

        assert_eq!(part.kind, PartKind::Attachment);
        assert_eq!(part.name.as_deref(), Some("attachment"));
        assert_eq!(part.summary.as_deref(), Some("1 attachment: manual.pdf"));
    }

    #[test]
    fn attachment_summary_handles_multiple_items() {
        let summary = attachment_part_summary(&AttachmentPart {
            attachments: vec![
                AttachmentItem {
                    kind: AttachmentKind::Image,
                    mime: "image/png".to_owned(),
                    source: AttachmentSource::Url {
                        url: "https://example.com/a.png".to_owned(),
                    },
                    filename: Some("a.png".to_owned()),
                    title: None,
                    size_bytes: None,
                    sha256: None,
                    width: Some(100),
                    height: Some(100),
                    duration_ms: None,
                    page_count: None,
                },
                AttachmentItem {
                    kind: AttachmentKind::Audio,
                    mime: "audio/mpeg".to_owned(),
                    source: AttachmentSource::FileId {
                        file_id: "file_2".to_owned(),
                    },
                    filename: Some("voice.mp3".to_owned()),
                    title: None,
                    size_bytes: None,
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: Some(5000),
                    page_count: None,
                },
            ],
        });

        assert_eq!(summary.as_deref(), Some("2 attachments (first: a.png)"));
    }

    #[test]
    fn permission_request_part_sets_kind_name_and_summary() {
        let part = MessagePart::with_content(
            11,
            2,
            Utc::now(),
            ExecutionStatus::Pending,
            PartContent::PermissionRequest(crate::message::PermissionRequestPart::pending(
                PermissionRequest {
                    request_id: "req_1".to_string(),
                    session_id: Some(2),
                    action: PermissionAction::BuiltinTool {
                        tool_name: "apply_patch".to_string(),
                    },
                    reason: "tool 'apply_patch' requires confirmation by policy".to_string(),
                    created_at: Utc::now(),
                },
            )),
        );

        assert_eq!(part.kind, PartKind::PermissionRequest);
        assert_eq!(part.name.as_deref(), Some("permission_request"));
        assert_eq!(
            part.summary.as_deref(),
            Some("Awaiting permission: tool 'apply_patch' requires confirmation by policy")
        );
    }

    #[test]
    fn file_change_part_sets_summary_from_first_change() {
        let part = MessagePart::with_content(
            12,
            3,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::FileChange(FileChangePart {
                changes: vec![crate::message::FileChangeEntry {
                    path: "result.txt".to_string(),
                    kind: FileChangeKind::Added,
                }],
            }),
        );

        assert_eq!(part.kind, PartKind::FileChange);
        assert_eq!(part.name.as_deref(), Some("file_change"));
        assert_eq!(
            part.summary.as_deref(),
            Some("1 file change: result.txt (added)")
        );
    }
}
