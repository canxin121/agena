use super::{
    AppError, Arc, ExecutionStatus, OperationPart, PartContent, PersistedPermissionRule,
    SessionManager, SessionManagerState, SessionPendingTool, completed_lifecycle,
    resolve_pending_tool, text_result_blocks, tool_name, update_resolved_tool_message,
};
use crate::session::Session;
use agena_domain::{
    CapabilityUnavailableResult, PolicyDeniedResult, ToolOutput, ToolUnavailableResult,
    UserDeclinedResult,
};
use chrono::Utc;

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
        let blocks = text_result_blocks(output_text.as_str());
        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::capability_unavailable(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    output_text.clone(),
                    blocks.clone(),
                    details.clone(),
                    lifecycle.clone(),
                );
                operation.set_title(format!("Tool {}", tool_name(&resolved.invocation)));
                tool_part.set_content(PartContent::operation(operation));
                tool_part.status = ExecutionStatus::CapabilityUnavailable;
            })?;
        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            Vec::new(),
            Vec::new(),
            state,
        )
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
        let blocks = text_result_blocks(output_text.as_str());
        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::tool_unavailable(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    output_text.clone(),
                    blocks.clone(),
                    details.clone(),
                    lifecycle.clone(),
                );
                operation.set_title(format!("Tool {}", tool_name(&resolved.invocation)));
                tool_part.set_content(PartContent::operation(operation));
                tool_part.status = ExecutionStatus::ToolUnavailable;
            })?;
        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            Vec::new(),
            Vec::new(),
            state,
        )
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
        let output_text = format!(
            "The operation was not executed because it is blocked by the effective permission policy: {}. Do not retry the same operation unless the permission rule changes.",
            denial.reason
        );
        let event_denial = denial.clone();
        let payload = serde_json::json!({
            "status": "policy_denied",
            "code": "permission_policy_denied",
            "retryable": false,
            "denial": denial,
        });
        let details = ToolOutput::from_json_payload(Some(&payload)).map_err(AppError::Internal)?;
        let blocks = text_result_blocks(output_text.as_str());
        let title = session
            .part(&resolved.pending.part)
            .and_then(|part| part.content.as_ref())
            .and_then(|content| match content {
                PartContent::Activity(crate::message::RuntimeActivity::Operation(operation)) => {
                    Some(operation.title.clone())
                }
                _ => None,
            })
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Tool {}", tool_name(&resolved.invocation)));

        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::policy_denied(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    output_text.clone(),
                    blocks.clone(),
                    details.clone(),
                    lifecycle.clone(),
                );
                operation.set_title(title.clone());
                tool_part.set_content(PartContent::operation(operation));
                tool_part.status = ExecutionStatus::PolicyDenied;
            })?;

        let terminal_events = vec![crate::event::EventKind::ToolPolicyDenied(
            agena_domain::ToolPolicyDeniedEvent {
                session_id: session.id,
                call_id: resolved.call_id,
                denial: event_denial,
                ts_ms: Utc::now().timestamp_millis(),
            },
        )];
        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            Vec::new(),
            terminal_events,
            state,
        )
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
        let explanation = decline
            .reason
            .as_deref()
            .filter(|reason| !reason.trim().is_empty())
            .map(|reason| format!(": {reason}"))
            .unwrap_or_default();
        let output_text = format!(
            "The operation was not executed because the user declined the permission request{explanation}. Do not retry the same operation in this turn."
        );
        let event_decline = decline.clone();
        let payload = serde_json::json!({
            "status": "user_declined",
            "code": "permission_request_declined",
            "retryable": false,
            "decline": decline,
        });
        let details = ToolOutput::from_json_payload(Some(&payload)).map_err(AppError::Internal)?;
        let blocks = text_result_blocks(output_text.as_str());
        let title = session
            .part(&resolved.pending.part)
            .and_then(|part| part.content.as_ref())
            .and_then(|content| match content {
                PartContent::Activity(crate::message::RuntimeActivity::Operation(operation)) => {
                    Some(operation.title.clone())
                }
                _ => None,
            })
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Tool {}", tool_name(&resolved.invocation)));

        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::user_declined(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    output_text.clone(),
                    blocks.clone(),
                    details.clone(),
                    lifecycle.clone(),
                );
                operation.set_title(title.clone());
                tool_part.set_content(PartContent::operation(operation));
                tool_part.status = ExecutionStatus::UserDeclined;
            })?;

        let terminal_events = vec![crate::event::EventKind::ToolUserDeclined(
            agena_domain::ToolUserDeclinedEvent {
                session_id: session.id,
                call_id: resolved.call_id,
                decline: event_decline,
                ts_ms: Utc::now().timestamp_millis(),
            },
        )];
        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            persisted_rules,
            terminal_events,
            state,
        )
        .await
    }
}
