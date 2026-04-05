use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use sea_orm::FromJsonQueryResult;
use sea_orm::entity::prelude::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

use crate::{
    event::SessionEvent,
    message::{
        ExecutionStatus, Message, MessagePart, PartContent, PermissionRequestPart, TimeRange,
        ToolExecutionPart, ToolInvocation, UserInputRequest, UserInputRequestPart,
    },
    role::Role,
};

pub(crate) const MESSAGE_TAG_PROMPT_COMPACTED: &str = "prompt_compacted";
pub(crate) const MESSAGE_TAG_PROMPT_SUMMARY: &str = "prompt_summary";
pub(crate) const MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED: &str = "attachment_payload_stripped";
pub(crate) const MESSAGE_TAG_TOOL_RESULT_PRUNED: &str = "tool_result_pruned";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPartRef {
    #[serde(default, skip_serializing)]
    pub message_index: usize,
    #[serde(default, skip_serializing)]
    pub part_index: usize,
    pub message_id: i64,
    pub part_id: i64,
}

impl SessionPartRef {
    fn new(message_index: usize, message: &Message, part_index: usize, part: &MessagePart) -> Self {
        Self {
            message_index,
            part_index,
            message_id: message.id,
            part_id: part.id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingTool {
    pub part: SessionPartRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingPermission {
    pub request: SessionPartRef,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingUserInput {
    pub request: SessionPartRef,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SessionPendingOperation {
    Tool { tool: SessionPendingTool },
    Permission { pending: SessionPendingPermission },
    UserInput { pending: SessionPendingUserInput },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Idle,
    AwaitingModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct PromptWindowRuntime {
    #[serde(default)]
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct PromptTokenUsageSnapshot {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
}

impl PromptTokenUsageSnapshot {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
    }
}

impl From<&crate::message::MessageUsage> for PromptTokenUsageSnapshot {
    fn from(value: &crate::message::MessageUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            reasoning_tokens: value.reasoning_tokens,
            cache_write_tokens: value.cache_write_tokens,
            cache_read_tokens: value.cache_read_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct PromptTokenRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_usage: Option<PromptTokenUsageSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_assistant_message_id: Option<i64>,
    #[serde(default)]
    pub prompt_window_generation: u64,
    #[serde(default)]
    pub system_fingerprint: String,
    #[serde(default)]
    pub request_options_fingerprint: String,
    #[serde(default)]
    pub transcript_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_context_window_tokens: Option<u32>,
}

impl PromptTokenRuntime {
    pub fn is_empty(&self) -> bool {
        self.last_successful_usage.is_none()
            && self.last_successful_assistant_message_id.is_none()
            && self.prompt_window_generation == 0
            && self.system_fingerprint.is_empty()
            && self.request_options_fingerprint.is_empty()
            && self.transcript_digest.is_empty()
            && self.model_context_window_tokens.is_none()
    }

    pub fn total_tokens(&self) -> Option<u64> {
        self.last_successful_usage
            .as_ref()
            .map(PromptTokenUsageSnapshot::total_tokens)
    }

    pub fn matches_request(
        &self,
        prompt_window_generation: u64,
        system_fingerprint: &str,
        request_options_fingerprint: &str,
    ) -> bool {
        self.last_successful_usage.is_some()
            && self.last_successful_assistant_message_id.is_some()
            && self.prompt_window_generation == prompt_window_generation
            && self.system_fingerprint == system_fingerprint
            && self.request_options_fingerprint == request_options_fingerprint
    }

    pub fn record_success(
        &mut self,
        assistant_message_id: i64,
        usage: &crate::message::MessageUsage,
        prompt_window_generation: u64,
        model_context_window_tokens: Option<u32>,
        system_fingerprint: String,
        request_options_fingerprint: String,
        transcript_digest: String,
    ) {
        self.last_successful_usage = Some(PromptTokenUsageSnapshot::from(usage));
        self.last_successful_assistant_message_id = Some(assistant_message_id);
        self.prompt_window_generation = prompt_window_generation;
        self.model_context_window_tokens = model_context_window_tokens;
        self.system_fingerprint = system_fingerprint;
        self.request_options_fingerprint = request_options_fingerprint;
        self.transcript_digest = transcript_digest;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct ProviderPromptAnchor {
    pub provider_id: String,
    pub model_id: String,
    pub previous_response_id: String,
    pub assistant_message_id: i64,
    #[serde(default)]
    pub prompt_window_generation: u64,
    #[serde(default)]
    pub system_fingerprint: String,
    #[serde(default)]
    pub request_options_fingerprint: String,
    #[serde(default)]
    pub transcript_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct SessionRuntimeState {
    #[serde(default)]
    pub prompt_window: PromptWindowRuntime,
    #[serde(default, skip_serializing_if = "PromptTokenRuntime::is_empty")]
    pub prompt_tokens: PromptTokenRuntime,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_anchors: BTreeMap<String, ProviderPromptAnchor>,
}

impl SessionRuntimeState {
    pub fn provider_anchor_key(provider_id: &str, model_id: &str) -> String {
        format!("{provider_id}/{model_id}")
    }

    pub fn provider_anchor(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Option<&ProviderPromptAnchor> {
        self.provider_anchors
            .get(Self::provider_anchor_key(provider_id, model_id).as_str())
    }

    pub fn set_provider_anchor(&mut self, anchor: ProviderPromptAnchor) {
        self.provider_anchors.insert(
            Self::provider_anchor_key(anchor.provider_id.as_str(), anchor.model_id.as_str()),
            anchor,
        );
    }

    pub fn clear_provider_anchor(&mut self, provider_id: &str, model_id: &str) {
        self.provider_anchors
            .remove(Self::provider_anchor_key(provider_id, model_id).as_str());
    }

    pub fn clear_provider_anchors(&mut self) {
        self.provider_anchors.clear();
    }

    pub fn clear_prompt_tokens(&mut self) {
        self.prompt_tokens.clear();
    }

    pub fn record_prompt_tokens(
        &mut self,
        assistant_message_id: i64,
        usage: &crate::message::MessageUsage,
        prompt_window_generation: u64,
        model_context_window_tokens: Option<u32>,
        system_fingerprint: String,
        request_options_fingerprint: String,
        transcript_digest: String,
    ) {
        self.prompt_tokens.record_success(
            assistant_message_id,
            usage,
            prompt_window_generation,
            model_context_window_tokens,
            system_fingerprint,
            request_options_fingerprint,
            transcript_digest,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionListRequest {
    #[serde(default)]
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: u64,
    pub child_session_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
pub struct Session {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    #[serde(skip, default)]
    pub(crate) runtime: SessionRuntimeState,
    #[serde(skip, default)]
    approx_bytes: usize,
    #[serde(skip, default)]
    pending_operations: Vec<SessionPendingOperation>,
}

impl Session {
    pub fn new(id: i64, workspace_id: i64, title: impl Into<String>, now: DateTime<Utc>) -> Self {
        let mut session = Self {
            id,
            parent_id: None,
            workspace_id,
            title: title.into(),
            version: 1,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            runtime: SessionRuntimeState::default(),
            approx_bytes: 0,
            pending_operations: Vec::new(),
        };
        session.refresh_derived();
        session
    }

    pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self.refresh_derived();
        self
    }

    pub(crate) fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.refresh_derived();
    }

    pub(crate) fn refresh_derived(&mut self) {
        self.approx_bytes = self.compute_approx_bytes();
        self.pending_operations = self.derive_pending_operations();
    }

    pub fn status(&self) -> SessionStatus {
        if self.should_run_model() {
            SessionStatus::AwaitingModel
        } else {
            SessionStatus::Idle
        }
    }

    pub(crate) fn apply_persisted_metadata(&mut self, persisted: &Session) {
        self.id = persisted.id;
        self.parent_id = persisted.parent_id;
        self.workspace_id = persisted.workspace_id;
        self.title = persisted.title.clone();
        self.version = persisted.version;
        self.created_at = persisted.created_at;
        self.updated_at = persisted.updated_at;
        self.runtime = persisted.runtime.clone();
    }

    pub fn blocked(&self) -> bool {
        self.pending_operations.iter().any(|pending| {
            matches!(
                pending,
                SessionPendingOperation::Permission { .. }
                    | SessionPendingOperation::UserInput { .. }
            )
        })
    }

    pub(crate) fn next_call_id(&self) -> i64 {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter_map(extract_call_id)
            .max()
            .unwrap_or(0)
            + 1
    }

    pub fn approx_bytes(&self) -> usize {
        self.approx_bytes
    }

    pub(crate) fn find_pending_user_input_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingUserInput> {
        self.pending_operations.iter().find_map(|pending| {
            let SessionPendingOperation::UserInput { pending } = pending else {
                return None;
            };
            self.pending_user_input_request(&pending)
                .filter(|request| request.request_id == request_id)
                .map(|_| pending.clone())
        })
    }

    fn compute_approx_bytes(&self) -> usize {
        serde_json::to_vec(&(
            self.id,
            self.parent_id,
            self.workspace_id,
            &self.title,
            self.version,
            self.created_at,
            self.updated_at,
            &self.messages,
            &self.runtime,
        ))
        .map(|bytes| bytes.len())
        .unwrap_or_else(|_| {
            self.messages
                .iter()
                .map(Message::as_text_lossy)
                .map(|text| text.len())
                .sum()
        })
    }

    fn should_run_model(&self) -> bool {
        matches!(
            self.last_conversation_message().map(|message| message.role),
            Some(Role::User | Role::Tool)
        )
    }

    pub(crate) fn last_conversation_message(&self) -> Option<&Message> {
        self.messages
            .iter()
            .rev()
            .find(|message| !message.metadata.has_tag(MESSAGE_TAG_PROMPT_SUMMARY))
    }

    pub(crate) fn find_pending_permission_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingPermission> {
        self.pending_operations.iter().find_map(|pending| {
            let SessionPendingOperation::Permission { pending } = pending else {
                return None;
            };
            self.pending_permission_request(&pending)
                .filter(|request| request.request_id == request_id)
                .map(|_| pending.clone())
        })
    }

    #[cfg(test)]
    pub(crate) fn pending_operations(&self) -> &[SessionPendingOperation] {
        self.pending_operations.as_slice()
    }

    fn derive_pending_operations(&self) -> Vec<SessionPendingOperation> {
        let mut operations = Vec::new();
        let completed_tool_operations = self.completed_tool_operations();

        for (message_index, message) in self.messages.iter().enumerate() {
            if message.role != Role::Assistant {
                continue;
            }

            let mut permission_parts = HashMap::new();
            let mut user_input_parts = HashMap::new();

            for (part_index, part) in message.parts.iter().enumerate() {
                if part.status != ExecutionStatus::Pending {
                    continue;
                }

                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };

                match part.content.as_ref() {
                    Some(PartContent::PermissionRequest(_)) => {
                        permission_parts.insert(
                            operation_id,
                            SessionPartRef::new(message_index, message, part_index, part),
                        );
                    }
                    Some(PartContent::UserInputRequest(_)) => {
                        user_input_parts.insert(
                            operation_id,
                            SessionPartRef::new(message_index, message, part_index, part),
                        );
                    }
                    _ => {}
                }
            }

            for (part_index, part) in message.parts.iter().enumerate() {
                if part.status != ExecutionStatus::Pending {
                    continue;
                }

                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };
                let Some(PartContent::ToolExecution(ToolExecutionPart::Pending { .. })) =
                    part.content.as_ref()
                else {
                    continue;
                };
                if completed_tool_operations.contains(operation_id) {
                    continue;
                }

                let tool = SessionPendingTool {
                    part: SessionPartRef::new(message_index, message, part_index, part),
                };

                if let Some(request) = permission_parts.get(operation_id) {
                    operations.push(SessionPendingOperation::Permission {
                        pending: SessionPendingPermission {
                            request: request.clone(),
                            tool,
                        },
                    });
                    continue;
                }

                if let Some(request) = user_input_parts.get(operation_id) {
                    operations.push(SessionPendingOperation::UserInput {
                        pending: SessionPendingUserInput {
                            request: request.clone(),
                            tool,
                        },
                    });
                    continue;
                }

                operations.push(SessionPendingOperation::Tool { tool });
            }
        }

        operations
    }

    pub(crate) fn next_pending_tool(&self) -> Option<SessionPendingTool> {
        self.pending_operations.iter().find_map(|pending| {
            let SessionPendingOperation::Tool { tool } = pending else {
                return None;
            };
            Some(tool.clone())
        })
    }

    pub(crate) fn part(&self, part_ref: &SessionPartRef) -> Option<&MessagePart> {
        let message = self.messages.get(part_ref.message_index)?;
        if message.id != part_ref.message_id {
            return None;
        }

        let part = message.parts.get(part_ref.part_index)?;
        (part.id == part_ref.part_id).then_some(part)
    }

    pub(crate) fn part_mut(&mut self, part_ref: &SessionPartRef) -> Option<&mut MessagePart> {
        let message = self.messages.get_mut(part_ref.message_index)?;
        if message.id != part_ref.message_id {
            return None;
        }

        let part = message.parts.get_mut(part_ref.part_index)?;
        (part.id == part_ref.part_id).then_some(part)
    }

    pub(crate) fn pending_tool_execution(
        &self,
        pending: &SessionPendingTool,
    ) -> Option<(i64, &ToolInvocation, &TimeRange)> {
        let part = self.part(&pending.part)?;
        let PartContent::ToolExecution(ToolExecutionPart::Pending {
            call_id,
            invocation,
            lifecycle,
            ..
        }) = part.content.as_ref()?
        else {
            return None;
        };

        Some((*call_id, invocation, lifecycle))
    }

    pub(crate) fn pending_permission_request(
        &self,
        pending: &SessionPendingPermission,
    ) -> Option<&crate::permission::PermissionRequest> {
        let part = self.part(&pending.request)?;
        let PartContent::PermissionRequest(PermissionRequestPart { request, .. }) =
            part.content.as_ref()?
        else {
            return None;
        };

        Some(request)
    }

    pub(crate) fn pending_user_input_request(
        &self,
        pending: &SessionPendingUserInput,
    ) -> Option<&UserInputRequest> {
        let part = self.part(&pending.request)?;
        let PartContent::UserInputRequest(UserInputRequestPart { request, .. }) =
            part.content.as_ref()?
        else {
            return None;
        };

        Some(request)
    }

    fn completed_tool_operations(&self) -> HashSet<&str> {
        self.messages
            .iter()
            .filter(|message| message.role == Role::Tool)
            .flat_map(|message| message.parts.iter())
            .filter_map(|part| part.operation_id.as_deref())
            .collect()
    }
}

fn extract_call_id(part: &MessagePart) -> Option<i64> {
    part.content.as_ref().and_then(|content| match content {
        PartContent::ToolExecution(tool) => match tool {
            ToolExecutionPart::Pending { call_id, .. }
            | ToolExecutionPart::InProgress { call_id, .. }
            | ToolExecutionPart::Completed { call_id, .. }
            | ToolExecutionPart::Failed { call_id, .. } => Some(*call_id),
        },
        _ => None,
    })
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
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
pub enum SessionEventType {
    #[sea_orm(num_value = 1)]
    RunStarted,
    #[sea_orm(num_value = 2)]
    RunFailed,
    #[sea_orm(num_value = 4)]
    MessagePartUpdated,
    #[sea_orm(num_value = 5)]
    MessagePartDelta,
    #[sea_orm(num_value = 6)]
    CommandBegin,
    #[sea_orm(num_value = 7)]
    CommandOutputDelta,
    #[sea_orm(num_value = 8)]
    CommandEnd,
    #[sea_orm(num_value = 9)]
    StreamError,
}

impl From<&SessionEvent> for SessionEventType {
    fn from(value: &SessionEvent) -> Self {
        match value {
            SessionEvent::RunStarted(_) => Self::RunStarted,
            SessionEvent::RunFailed(_) => Self::RunFailed,
            SessionEvent::MessagePartUpdated(_) => Self::MessagePartUpdated,
            SessionEvent::MessagePartDelta(_) => Self::MessagePartDelta,
            SessionEvent::CommandBegin(_) => Self::CommandBegin,
            SessionEvent::CommandOutputDelta(_) => Self::CommandOutputDelta,
            SessionEvent::CommandEnd(_) => Self::CommandEnd,
            SessionEvent::StreamError(_) => Self::StreamError,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionEventRecord {
    pub event_id: Option<i64>,
    pub session_id: i64,
    pub seq: i64,
    pub event_type: SessionEventType,
    pub payload: SessionEvent,
    pub causation_id: Option<i64>,
    pub correlation_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::message::{
        BuiltinToolInput, ExecutionStatus, MessageMetadata, MessagePart, MessageStatus,
        PartContent, TimeRange, TodoWriteToolInput, ToolExecutionPart, ToolInvocation,
        UserInputQuestion,
    };
    use crate::permission::{PermissionAction, PermissionRequest};
    use crate::role::Role;

    use super::*;

    #[test]
    fn find_pending_permission_by_request_id_handles_multiple_requests() {
        let session = Session::new(1, 1, "multi-permission", Utc::now()).with_messages(vec![
            assistant_message(
                11,
                vec![
                    pending_tool_part(101, 11, "op-1", 1),
                    pending_permission_part(102, 11, "op-1", "perm-1"),
                    pending_tool_part(103, 11, "op-2", 2),
                    pending_permission_part(104, 11, "op-2", "perm-2"),
                ],
            ),
        ]);

        let pending = session
            .find_pending_permission_by_request_id("perm-2")
            .expect("second pending permission should resolve");

        assert_eq!(session.pending_operations().len(), 2);
        assert_eq!(pending.request.part_id, 104);
        assert_eq!(pending.tool.part.part_id, 103);
        assert!(session.blocked());
    }

    #[test]
    fn find_pending_user_input_by_request_id_handles_multiple_requests() {
        let session =
            Session::new(1, 1, "multi-input", Utc::now()).with_messages(vec![assistant_message(
                21,
                vec![
                    pending_tool_part(201, 21, "op-1", 1),
                    pending_user_input_part(202, 21, "op-1", "input-1"),
                    pending_tool_part(203, 21, "op-2", 2),
                    pending_user_input_part(204, 21, "op-2", "input-2"),
                ],
            )]);

        let pending = session
            .find_pending_user_input_by_request_id("input-2")
            .expect("second pending user input should resolve");

        assert_eq!(session.pending_operations().len(), 2);
        assert_eq!(pending.request.part_id, 204);
        assert_eq!(pending.tool.part.part_id, 203);
        assert!(session.blocked());
    }

    fn assistant_message(id: i64, mut parts: Vec<MessagePart>) -> Message {
        for (index, part) in parts.iter_mut().enumerate() {
            part.part_index = index as i32;
        }
        Message {
            id,
            role: Role::Assistant,
            state: MessageStatus::Completed,
            parts,
            created_at: Utc::now(),
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
        }
    }

    fn pending_tool_part(
        part_id: i64,
        message_id: i64,
        operation_id: &str,
        call_id: i64,
    ) -> MessagePart {
        let invocation = ToolInvocation::Builtin {
            input: BuiltinToolInput::TodoWrite(TodoWriteToolInput { items: Vec::new() }),
        };
        let mut part = MessagePart::with_content(
            part_id,
            message_id,
            Utc::now(),
            ExecutionStatus::Pending,
            PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id,
                invocation,
                title: format!("tool {operation_id}"),
                lifecycle: TimeRange::default(),
            }),
        );
        part.operation_id = Some(operation_id.to_string());
        part
    }

    fn pending_permission_part(
        part_id: i64,
        message_id: i64,
        operation_id: &str,
        request_id: &str,
    ) -> MessagePart {
        let mut part = MessagePart::with_content(
            part_id,
            message_id,
            Utc::now(),
            ExecutionStatus::Pending,
            PartContent::PermissionRequest(PermissionRequestPart::pending(PermissionRequest {
                request_id: request_id.to_string(),
                session_id: Some(1),
                action: PermissionAction::BuiltinTool {
                    tool_name: "todo_write".to_string(),
                },
                reason: format!("need permission for {operation_id}"),
                created_at: Utc::now(),
            })),
        );
        part.operation_id = Some(operation_id.to_string());
        part
    }

    fn pending_user_input_part(
        part_id: i64,
        message_id: i64,
        operation_id: &str,
        request_id: &str,
    ) -> MessagePart {
        let mut part = MessagePart::with_content(
            part_id,
            message_id,
            Utc::now(),
            ExecutionStatus::Pending,
            PartContent::UserInputRequest(UserInputRequestPart::pending(UserInputRequest {
                request_id: request_id.to_string(),
                session_id: Some(1),
                questions: vec![UserInputQuestion {
                    id: "answer".to_string(),
                    header: "Answer".to_string(),
                    question: format!("question for {operation_id}"),
                    options: Vec::new(),
                    multiple: false,
                    allow_custom: true,
                }],
                created_at: Utc::now(),
            })),
        );
        part.operation_id = Some(operation_id.to_string());
        part
    }
}
