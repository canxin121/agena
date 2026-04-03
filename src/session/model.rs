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
    permission::PermissionRequest,
    role::Role,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionCacheSource {
    #[default]
    Fresh,
    Memory,
    Database,
    Restored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct SessionCache {
    #[serde(default)]
    pub source: SessionCacheSource,
    #[serde(default)]
    pub approx_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingTool {
    #[serde(default, skip_serializing)]
    pub message_index: usize,
    #[serde(default, skip_serializing)]
    pub part_index: usize,
    pub message_id: i64,
    pub part_id: i64,
    pub operation_id: String,
    pub call_id: i64,
    pub invocation: ToolInvocation,
    pub lifecycle: TimeRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingPermission {
    #[serde(default, skip_serializing)]
    pub permission_message_index: usize,
    #[serde(default, skip_serializing)]
    pub permission_part_index: usize,
    pub permission_message_id: i64,
    pub permission_part_id: i64,
    pub request: PermissionRequest,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingUserInput {
    #[serde(default, skip_serializing)]
    pub input_message_index: usize,
    #[serde(default, skip_serializing)]
    pub input_part_index: usize,
    pub input_message_id: i64,
    pub input_part_id: i64,
    pub request: UserInputRequest,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SessionStatus {
    #[default]
    Idle,
    AwaitingModel,
    AwaitingTool {
        tool: SessionPendingTool,
    },
    AwaitingPermission {
        pending: SessionPendingPermission,
    },
    AwaitingUserInput {
        pending: SessionPendingUserInput,
    },
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
    child_session_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    #[serde(default)]
    cache: SessionCache,
    #[serde(default)]
    status: SessionStatus,
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
            child_session_ids: Vec::new(),
            messages: Vec::new(),
            cache: SessionCache::default(),
            status: SessionStatus::default(),
        };
        session.refresh_derived();
        session
    }

    pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self.refresh_derived();
        self
    }

    pub fn child_session_ids(&self) -> &[i64] {
        self.child_session_ids.as_slice()
    }

    pub(crate) fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.refresh_derived();
    }

    pub(crate) fn replace_child_session_ids(&mut self, child_session_ids: Vec<i64>) {
        self.child_session_ids = child_session_ids;
        self.refresh_derived();
    }

    pub(crate) fn set_cache_source(&mut self, source: SessionCacheSource) {
        self.cache.source = source;
    }

    pub(crate) fn refresh_derived(&mut self) {
        self.cache.approx_bytes = self.compute_approx_bytes();
        self.status = self.derive_status();
    }

    pub(crate) fn status(&self) -> &SessionStatus {
        &self.status
    }

    pub(crate) fn apply_persisted_metadata(&mut self, persisted: &Session) {
        self.id = persisted.id;
        self.parent_id = persisted.parent_id;
        self.workspace_id = persisted.workspace_id;
        self.title = persisted.title.clone();
        self.version = persisted.version;
        self.created_at = persisted.created_at;
        self.updated_at = persisted.updated_at;
    }

    pub fn blocked(&self) -> bool {
        matches!(
            self.status,
            SessionStatus::AwaitingPermission { .. } | SessionStatus::AwaitingUserInput { .. }
        )
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
        self.cache.approx_bytes
    }

    pub(crate) fn find_pending_permission_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingPermission> {
        self.find_any_pending_permission()
            .filter(|pending| pending.request.request_id == request_id)
    }

    pub(crate) fn find_pending_user_input_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingUserInput> {
        self.find_any_pending_user_input()
            .filter(|pending| pending.request.request_id == request_id)
    }

    pub(crate) fn append_child_session_id(&mut self, child_session_id: i64) {
        if !self.child_session_ids.contains(&child_session_id) {
            self.child_session_ids.push(child_session_id);
            self.child_session_ids.sort_unstable();
            self.refresh_derived();
        }
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
            &self.child_session_ids,
            &self.messages,
            &self.status,
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

    fn derive_status(&self) -> SessionStatus {
        if let Some(pending) = self.find_any_pending_permission() {
            return SessionStatus::AwaitingPermission { pending };
        }

        if let Some(pending) = self.find_any_pending_user_input() {
            return SessionStatus::AwaitingUserInput { pending };
        }

        if let Some(tool) = self.find_next_pending_tool() {
            return SessionStatus::AwaitingTool { tool };
        }

        if self.should_run_model() {
            SessionStatus::AwaitingModel
        } else {
            SessionStatus::Idle
        }
    }

    fn should_run_model(&self) -> bool {
        matches!(
            self.messages.last().map(|message| message.role),
            Some(Role::User | Role::Tool)
        )
    }

    fn find_any_pending_permission(&self) -> Option<SessionPendingPermission> {
        for message_index in 0..self.messages.len() {
            let message = &self.messages[message_index];
            if message.role != Role::Assistant {
                continue;
            }

            for part_index in 0..message.parts.len() {
                let part = &message.parts[part_index];
                if part.status != ExecutionStatus::Pending {
                    continue;
                }
                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };
                let Some(PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id,
                    invocation,
                    title: _,
                    lifecycle,
                })) = part.content.as_ref()
                else {
                    continue;
                };

                if let Some((permission_part_index, request)) =
                    self.find_permission_part(message_index, operation_id)
                {
                    let permission_part = &message.parts[permission_part_index];
                    return Some(SessionPendingPermission {
                        permission_message_index: message_index,
                        permission_part_index,
                        permission_message_id: message.id,
                        permission_part_id: permission_part.id,
                        request,
                        tool: SessionPendingTool {
                            message_index,
                            part_index,
                            message_id: message.id,
                            part_id: part.id,
                            operation_id: operation_id.to_string(),
                            call_id: *call_id,
                            invocation: invocation.clone(),
                            lifecycle: lifecycle.clone(),
                        },
                    });
                }
            }
        }

        None
    }

    fn find_any_pending_user_input(&self) -> Option<SessionPendingUserInput> {
        for message_index in 0..self.messages.len() {
            let message = &self.messages[message_index];
            if message.role != Role::Assistant {
                continue;
            }

            for part_index in 0..message.parts.len() {
                let part = &message.parts[part_index];
                if part.status != ExecutionStatus::Pending {
                    continue;
                }
                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };
                let Some(PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id,
                    invocation,
                    title: _,
                    lifecycle,
                })) = part.content.as_ref()
                else {
                    continue;
                };

                if let Some((input_part_index, request)) =
                    self.find_user_input_part(message_index, operation_id)
                {
                    let input_part = &message.parts[input_part_index];
                    return Some(SessionPendingUserInput {
                        input_message_index: message_index,
                        input_part_index,
                        input_message_id: message.id,
                        input_part_id: input_part.id,
                        request,
                        tool: SessionPendingTool {
                            message_index,
                            part_index,
                            message_id: message.id,
                            part_id: part.id,
                            operation_id: operation_id.to_string(),
                            call_id: *call_id,
                            invocation: invocation.clone(),
                            lifecycle: lifecycle.clone(),
                        },
                    });
                }
            }
        }

        None
    }

    fn find_next_pending_tool(&self) -> Option<SessionPendingTool> {
        for (message_index, message) in self.messages.iter().enumerate() {
            if message.role != Role::Assistant {
                continue;
            }

            for (part_index, part) in message.parts.iter().enumerate() {
                if part.status != ExecutionStatus::Pending {
                    continue;
                }
                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };
                let Some(PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id,
                    invocation,
                    title: _,
                    lifecycle,
                })) = part.content.as_ref()
                else {
                    continue;
                };

                if self
                    .find_permission_part(message_index, operation_id)
                    .is_some()
                {
                    continue;
                }
                if self
                    .find_user_input_part(message_index, operation_id)
                    .is_some()
                {
                    continue;
                }
                if self.has_tool_result(operation_id) {
                    continue;
                }

                return Some(SessionPendingTool {
                    message_index,
                    part_index,
                    message_id: message.id,
                    part_id: part.id,
                    operation_id: operation_id.to_string(),
                    call_id: *call_id,
                    invocation: invocation.clone(),
                    lifecycle: lifecycle.clone(),
                });
            }
        }

        None
    }

    fn has_tool_result(&self, operation_id: &str) -> bool {
        self.messages
            .iter()
            .filter(|message| message.role == Role::Tool)
            .any(|message| {
                message
                    .parts
                    .iter()
                    .any(|part| part.operation_id.as_deref() == Some(operation_id))
            })
    }

    fn find_permission_part(
        &self,
        message_index: usize,
        operation_id: &str,
    ) -> Option<(usize, PermissionRequest)> {
        let message = &self.messages[message_index];
        for (part_index, part) in message.parts.iter().enumerate() {
            if part.operation_id.as_deref() != Some(operation_id)
                || part.status != ExecutionStatus::Pending
            {
                continue;
            }
            let Some(PartContent::PermissionRequest(PermissionRequestPart { request, .. })) =
                part.content.as_ref()
            else {
                continue;
            };
            return Some((part_index, request.clone()));
        }
        None
    }

    fn find_user_input_part(
        &self,
        message_index: usize,
        operation_id: &str,
    ) -> Option<(usize, UserInputRequest)> {
        let message = &self.messages[message_index];
        for (part_index, part) in message.parts.iter().enumerate() {
            if part.operation_id.as_deref() != Some(operation_id)
                || part.status != ExecutionStatus::Pending
            {
                continue;
            }
            let Some(PartContent::UserInputRequest(UserInputRequestPart { request, .. })) =
                part.content.as_ref()
            else {
                continue;
            };
            return Some((part_index, request.clone()));
        }
        None
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
    #[sea_orm(num_value = 3)]
    SessionRestored,
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
            SessionEvent::SessionRestored(_) => Self::SessionRestored,
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

impl SessionEventRecord {
    pub fn new(
        session_id: i64,
        seq: i64,
        payload: SessionEvent,
        created_at: DateTime<Utc>,
    ) -> Self {
        let event_type = SessionEventType::from(&payload);
        Self {
            event_id: None,
            session_id,
            seq,
            event_type,
            payload,
            causation_id: None,
            correlation_id: None,
            created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionCheckpoint {
    pub id: i64,
    pub session_id: i64,
    pub upto_seq: i64,
    pub session: Session,
    pub state_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}
