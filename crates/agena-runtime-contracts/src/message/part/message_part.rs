use agena_domain::{
    ActivityId, ExecutionStatus, ExecutionStatusTransitionError, PartKind, TextSegmentId,
};
#[cfg(test)]
use agena_domain::{ToolApiFunction, ToolInvocation};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::OperationCompletion;
use super::{AttachmentPart, PartContent, RequestPart, RuntimeActivity};

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
    pub activity_id: Option<ActivityId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<TextSegmentId>,
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
        let activity_id = matches!(&content, PartContent::Activity(_)).then(ActivityId::new);
        let segment_id = matches!(&content, PartContent::Text(_)).then(TextSegmentId::new);
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
            activity_id,
            segment_id,
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

    /// Bind a text-backed artifact to its Activity identity. A part is either
    /// primitive text or structured Activity content in the transcript; it
    /// can never carry both identities.
    pub fn bind_activity(&mut self, activity_id: ActivityId) {
        self.activity_id = Some(activity_id);
        self.segment_id = None;
    }

    /// Whether this part represents an unresolved interaction that survives
    /// the execution which requested it. Operation authorization and user-input Activities
    /// are conversation state; finishing the requesting execution must not
    /// terminalize them before the user replies.
    pub fn awaits_user_reply(&self) -> bool {
        matches!(
            self.content.as_ref(),
            Some(PartContent::Activity(RuntimeActivity::Interaction(request)))
                if request.pending_interactive_request().is_some()
        )
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
    ) -> Result<(), ExecutionStatusTransitionError> {
        let from = self.status;
        if !from.can_transition(to) {
            return Err(ExecutionStatusTransitionError { from, to });
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
        PartContent::Activity(RuntimeActivity::Reasoning(_)) => Some("reasoning".to_string()),
        PartContent::Activity(RuntimeActivity::Operation(operation)) => {
            Some(operation_header_title(operation))
        }
        PartContent::Activity(RuntimeActivity::SkillReference(_)) => {
            Some("skill_reference".to_string())
        }
        PartContent::Activity(RuntimeActivity::Error(error)) => {
            Some(error.problem.code.to_string())
        }
        PartContent::Activity(RuntimeActivity::Resource(_)) => Some("resource".to_string()),
        PartContent::Activity(RuntimeActivity::Interaction(RequestPart::UserInput(_))) => {
            Some("user_input".to_string())
        }
    }
}

fn summary_from_content(content: &PartContent) -> Option<String> {
    match content {
        PartContent::Text(text) => truncate_summary(&text.text),
        PartContent::Activity(RuntimeActivity::Reasoning(reasoning)) => {
            truncate_summary(&reasoning.preferred_text())
        }
        PartContent::Activity(RuntimeActivity::Operation(operation)) => operation
            .error_message()
            .or_else(|| (!operation.summary.is_empty()).then_some(operation.summary.as_str()))
            .and_then(truncate_summary)
            .or_else(|| truncate_summary(operation_header_title(operation).as_str())),
        PartContent::Activity(RuntimeActivity::Error(error)) => {
            truncate_summary(error.problem.user.fallback.as_str())
        }
        PartContent::Activity(RuntimeActivity::Resource(attachment)) => {
            attachment_part_summary(attachment)
        }
        PartContent::Activity(RuntimeActivity::SkillReference(skill_reference)) => {
            truncate_summary(skill_reference.summary().as_str())
        }
        PartContent::Activity(RuntimeActivity::Interaction(request)) => {
            truncate_summary(request.summary_text().as_str())
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

/// Human-readable, persisted Operation title. This becomes the `name` header
/// in `agena_model_message_parts`, so collapsed transcript queries need not load
/// the operation payload merely to explain the action.
fn operation_header_title(operation: &super::OperationPart) -> String {
    let title = operation.title.trim();
    if !title.is_empty() {
        return title.to_owned();
    }
    let display_title = operation.result.display.title.trim();
    if !display_title.is_empty() {
        return display_title.to_owned();
    }
    operation.invocation.name.clone()
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
    use super::*;
    use crate::{OperationPart, PartContent};
    use agena_domain::{StructuredObject, TimeRange, ToolOutput};

    #[test]
    fn operation_headers_name_the_real_tools_call_target_and_compact_input_output() {
        let arguments = StructuredObject::try_from(serde_json::json!({
            "tool": "fs.read",
            "input": {"path": "README.md"}
        }))
        .expect("structured tools_call input");
        let invocation = ToolInvocation {
            tool_api_call: Some(agena_domain::ToolApiCall {
                function: ToolApiFunction::Call,
                arguments,
            }),
            name: "fs.read".to_owned(),
            plugin_name: None,
            input: StructuredObject::try_from(serde_json::json!({"path": "README.md"}))
                .expect("structured target input"),
        };
        let operation = OperationPart::completed(
            1,
            invocation,
            OperationCompletion::new(
                "Read README.md",
                "42 lines",
                "Read README.md (42 lines)",
                Vec::new(),
                Vec::new(),
                ToolOutput::default(),
            ),
            TimeRange::default(),
        );
        let part = MessagePart::from_content(
            1,
            1,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::operation(operation),
        );

        assert_eq!(part.name.as_deref(), Some("Read README.md"));
        assert_eq!(part.summary.as_deref(), Some("42 lines"));
        assert!(part.has_detail);
    }

    #[test]
    fn discovery_completion_persists_its_page_title_without_structured_output() {
        let arguments = StructuredObject::try_from(serde_json::json!({"offset": 20}))
            .expect("structured tools_list input");
        let invocation = ToolInvocation {
            tool_api_call: Some(agena_domain::ToolApiCall {
                function: ToolApiFunction::List,
                arguments: arguments.clone(),
            }),
            name: "tools_list".to_owned(),
            plugin_name: None,
            input: arguments,
        };
        let operation = OperationPart::completed(
            1,
            invocation,
            OperationCompletion::new(
                "List tools · 20/133",
                "Returned 20 of 133 tools; continue at offset 40.",
                "Available tools: returned 20 of 133 starting at offset 20.",
                Vec::new(),
                Vec::new(),
                ToolOutput::default(),
            ),
            TimeRange::default(),
        );
        let part = MessagePart::from_content(
            1,
            1,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::operation(operation),
        );

        assert_eq!(part.name.as_deref(), Some("List tools · 20/133"));
        assert!(part.has_detail);
    }
}
