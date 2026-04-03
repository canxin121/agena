use serde::Serialize;

use crate::{
    message::{
        ExecutionStatus, Message, MessagePart, PartContent, PermissionRequestPart, TimeRange,
        ToolExecutionPart, ToolInvocation, UserInputRequest, UserInputRequestPart,
    },
    permission::PermissionRequest,
    role::Role,
};

use super::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRuntimeCacheSource {
    Fresh,
    Memory,
    Database,
    Restored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionRuntimeCache {
    pub source: SessionRuntimeCacheSource,
    pub approx_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionPendingTool {
    #[serde(skip_serializing)]
    pub message_index: usize,
    #[serde(skip_serializing)]
    pub part_index: usize,
    pub message_id: i64,
    pub part_id: i64,
    pub operation_id: String,
    pub call_id: i64,
    pub invocation: ToolInvocation,
    pub lifecycle: TimeRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionPendingPermission {
    #[serde(skip_serializing)]
    pub permission_message_index: usize,
    #[serde(skip_serializing)]
    pub permission_part_index: usize,
    pub permission_message_id: i64,
    pub permission_part_id: i64,
    pub request: PermissionRequest,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionPendingUserInput {
    #[serde(skip_serializing)]
    pub input_message_index: usize,
    #[serde(skip_serializing)]
    pub input_part_index: usize,
    pub input_message_id: i64,
    pub input_part_id: i64,
    pub request: UserInputRequest,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionRuntimeStatus {
    Idle,
    AwaitingModel,
    AwaitingTool { tool: SessionPendingTool },
    AwaitingPermission { pending: SessionPendingPermission },
    AwaitingUserInput { pending: SessionPendingUserInput },
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRuntime {
    pub session: Session,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_session_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    pub cache: SessionRuntimeCache,
    pub status: SessionRuntimeStatus,
}

impl SessionRuntime {
    pub fn new(
        session: Session,
        child_session_ids: Vec<i64>,
        messages: Vec<Message>,
        cache_source: SessionRuntimeCacheSource,
    ) -> Self {
        let mut runtime = Self {
            session,
            child_session_ids,
            messages,
            cache: SessionRuntimeCache {
                source: cache_source,
                approx_bytes: 0,
            },
            status: SessionRuntimeStatus::Idle,
        };
        runtime.refresh_derived();
        runtime
    }

    pub fn set_cache_source(&mut self, source: SessionRuntimeCacheSource) {
        self.cache.source = source;
    }

    pub fn refresh_derived(&mut self) {
        self.cache.approx_bytes = self.compute_approx_bytes();
        self.status = self.derive_status();
    }

    pub fn blocked(&self) -> bool {
        matches!(
            self.status,
            SessionRuntimeStatus::AwaitingPermission { .. }
                | SessionRuntimeStatus::AwaitingUserInput { .. }
        )
    }

    pub fn next_call_id(&self) -> i64 {
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

    pub fn find_pending_permission_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingPermission> {
        self.find_any_pending_permission()
            .filter(|pending| pending.request.request_id == request_id)
    }

    pub fn find_pending_user_input_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingUserInput> {
        self.find_any_pending_user_input()
            .filter(|pending| pending.request.request_id == request_id)
    }

    pub fn append_child_session_id(&mut self, child_session_id: i64) {
        if !self.child_session_ids.contains(&child_session_id) {
            self.child_session_ids.push(child_session_id);
            self.child_session_ids.sort_unstable();
            self.refresh_derived();
        }
    }

    fn compute_approx_bytes(&self) -> usize {
        serde_json::to_vec(&(
            &self.session,
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

    fn derive_status(&self) -> SessionRuntimeStatus {
        if let Some(pending) = self.find_any_pending_permission() {
            return SessionRuntimeStatus::AwaitingPermission { pending };
        }

        if let Some(pending) = self.find_any_pending_user_input() {
            return SessionRuntimeStatus::AwaitingUserInput { pending };
        }

        if let Some(tool) = self.find_next_pending_tool() {
            return SessionRuntimeStatus::AwaitingTool { tool };
        }

        if self.should_run_model() {
            SessionRuntimeStatus::AwaitingModel
        } else {
            SessionRuntimeStatus::Idle
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

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::message::{MessageMetadata, MessageSource, MessageStatus};

    #[test]
    fn runtime_derives_awaiting_model_when_last_message_is_user() {
        let session = Session::new(1, 1, "Test", Utc::now());
        let message = Message {
            id: 10,
            role: Role::User,
            state: MessageStatus::Completed,
            parts: Vec::new(),
            created_at: Utc::now(),
            metadata: MessageMetadata {
                source: MessageSource::User,
                parent_message_id: None,
                generated_by_call_id: None,
                model_provider_id: "openai".to_owned(),
                model_id: "gpt-5".to_owned(),
                tags: Vec::new(),
            },
            usage: None,
            finish: None,
        };

        let runtime = SessionRuntime::new(
            session,
            Vec::new(),
            vec![message],
            SessionRuntimeCacheSource::Fresh,
        );

        assert!(matches!(
            runtime.status,
            SessionRuntimeStatus::AwaitingModel
        ));
    }
}
