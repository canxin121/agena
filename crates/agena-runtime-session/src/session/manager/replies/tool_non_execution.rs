use super::{
    AppError, Arc, ExecutionStatus, OperationPart, PersistedPermissionRule, SessionManager,
    SessionManagerState, SessionPendingTool, completed_lifecycle, inherit_operation_context,
    operation_authorization, operation_from_part, resolve_pending_tool,
    update_resolved_tool_message,
};
use crate::session::Session;
use crate::session::store::{
    part_state_from_execution_status, tool_call_from_operation, typed_content_to_value,
};
use agena_domain::{
    CapabilityUnavailableResult, PolicyDeniedResult, ToolOutput, ToolUnavailableResult,
    UserDeclinedResult,
};
use agena_runtime_contracts::part_content::TypedContent;

impl SessionManager {
    pub(in crate::session::manager) async fn apply_tool_capability_unavailable(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        unavailable: CapabilityUnavailableResult,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let authorization = operation_authorization(&session, &resolved);
        let output_text = format!(
            "The operation was not executed because the current runtime does not provide the required capability: {}. User approval cannot enable this capability.",
            unavailable.reason
        );
        let payload = serde_json::json!({
            "status": "capability_unavailable",
            "code": "capability_unavailable",
            "retryable": unavailable.retryable,
            "unavailable": unavailable,
        });
        let details = ToolOutput::from_json_payload(Some(&payload)).map_err(AppError::Internal)?;
        let _assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::capability_unavailable(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    agena_domain::RawOutput::from_parts(
                        details.to_json_payload(),
                        output_text.clone(),
                        Vec::new(),
                        details.managed_outputs.clone(),
                        Default::default(),
                        details.truncated,
                    ),
                    lifecycle.clone(),
                );
                operation.authorization = authorization.clone();
                if let Some(existing) = operation_from_part(tool_part) {
                    inherit_operation_context(&mut operation, existing);
                }
                tool_part.content = typed_content_to_value(&TypedContent::ToolCall(Box::new(
                    tool_call_from_operation(&operation),
                )))
                .expect("tool content is always JSON serializable");
                tool_part.state =
                    part_state_from_execution_status(ExecutionStatus::CapabilityUnavailable);
            })?;
        self.persist_tool_completion(session, &resolved, Vec::new(), state)
            .await
    }

    pub(in crate::session::manager) async fn apply_tool_unavailable(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        unavailable: ToolUnavailableResult,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let authorization = operation_authorization(&session, &resolved);
        let output_text = format!(
            "The operation was not executed because the requested tool is unavailable: {}.",
            unavailable.reason
        );
        let payload = serde_json::json!({
            "status": "tool_unavailable",
            "code": "tool_unavailable",
            "retryable": unavailable.retryable,
            "unavailable": unavailable,
        });
        let details = ToolOutput::from_json_payload(Some(&payload)).map_err(AppError::Internal)?;
        let _assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::tool_unavailable(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    agena_domain::RawOutput::from_parts(
                        details.to_json_payload(),
                        output_text.clone(),
                        Vec::new(),
                        details.managed_outputs.clone(),
                        Default::default(),
                        details.truncated,
                    ),
                    lifecycle.clone(),
                );
                operation.authorization = authorization.clone();
                if let Some(existing) = operation_from_part(tool_part) {
                    inherit_operation_context(&mut operation, existing);
                }
                tool_part.content = typed_content_to_value(&TypedContent::ToolCall(Box::new(
                    tool_call_from_operation(&operation),
                )))
                .expect("tool content is always JSON serializable");
                tool_part.state =
                    part_state_from_execution_status(ExecutionStatus::ToolUnavailable);
            })?;
        self.persist_tool_completion(session, &resolved, Vec::new(), state)
            .await
    }

    pub(in crate::session::manager) async fn apply_tool_policy_denied(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        denial: PolicyDeniedResult,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let authorization = operation_authorization(&session, &resolved);
        let output_text = format!(
            "The operation was not executed because it is blocked by the effective permission policy: {}. Do not retry the same operation unless the permission rule changes.",
            denial.reason
        );
        let payload = serde_json::json!({
            "status": "policy_denied",
            "code": "permission_policy_denied",
            "retryable": false,
            "denial": denial,
        });
        let details = ToolOutput::from_json_payload(Some(&payload)).map_err(AppError::Internal)?;

        let _assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::policy_denied(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    agena_domain::RawOutput::from_parts(
                        details.to_json_payload(),
                        output_text.clone(),
                        Vec::new(),
                        details.managed_outputs.clone(),
                        Default::default(),
                        details.truncated,
                    ),
                    lifecycle.clone(),
                );
                operation.authorization = authorization.clone();
                if let Some(existing) = operation_from_part(tool_part) {
                    inherit_operation_context(&mut operation, existing);
                }
                tool_part.content = typed_content_to_value(&TypedContent::ToolCall(Box::new(
                    tool_call_from_operation(&operation),
                )))
                .expect("tool content is always JSON serializable");
                tool_part.state = part_state_from_execution_status(ExecutionStatus::PolicyDenied);
            })?;

        self.persist_tool_completion(session, &resolved, Vec::new(), state)
            .await
    }

    pub(in crate::session::manager) async fn apply_tool_user_declined(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        decline: UserDeclinedResult,
        persisted_rules: Vec<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let authorization = operation_authorization(&session, &resolved);
        let explanation = decline
            .reason
            .as_deref()
            .filter(|reason| !reason.trim().is_empty())
            .map(|reason| format!(": {reason}"))
            .unwrap_or_default();
        let output_text = format!(
            "The operation was not executed because the user declined the permission request{explanation}. Do not retry the same operation in this turn."
        );
        let payload = serde_json::json!({
            "status": "user_declined",
            "code": "permission_request_declined",
            "retryable": false,
            "decline": decline,
        });
        let details = ToolOutput::from_json_payload(Some(&payload)).map_err(AppError::Internal)?;

        let _assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::user_declined(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    agena_domain::RawOutput::from_parts(
                        details.to_json_payload(),
                        output_text.clone(),
                        Vec::new(),
                        details.managed_outputs.clone(),
                        Default::default(),
                        details.truncated,
                    ),
                    lifecycle.clone(),
                );
                operation.authorization = authorization.clone();
                if let Some(existing) = operation_from_part(tool_part) {
                    inherit_operation_context(&mut operation, existing);
                }
                tool_part.content = typed_content_to_value(&TypedContent::ToolCall(Box::new(
                    tool_call_from_operation(&operation),
                )))
                .expect("tool content is always JSON serializable");
                tool_part.state = part_state_from_execution_status(ExecutionStatus::UserDeclined);
            })?;

        self.persist_tool_completion(session, &resolved, persisted_rules, state)
            .await
    }
}
