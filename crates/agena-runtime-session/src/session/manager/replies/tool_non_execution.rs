use super::{
    AppError, Arc, OperationPart, PersistedPermissionRule, SessionManager, SessionManagerState,
    SessionPendingTool, completed_lifecycle, inherit_operation_context, operation_authorization,
    required_operation, resolve_pending_tool,
};
use crate::session::Session;
use agena_domain::{
    CapabilityUnavailableResult, PolicyDeniedResult, ToolOutput, ToolUnavailableResult,
    UserDeclinedResult,
};

impl SessionManager {
    pub(in crate::session::manager) async fn apply_tool_capability_unavailable(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        unavailable: CapabilityUnavailableResult,
        _state: Arc<SessionManagerState>,
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
        let mut operation = OperationPart::capability_unavailable(
            resolved.call_id,
            resolved.invocation.clone(),
            agena_domain::RawOutput::from_parts(
                details.to_json_payload(),
                output_text.clone(),
                Vec::new(),
                details.managed_outputs.clone(),
                details.truncated,
            ),
            lifecycle.clone(),
        );
        operation.authorization = authorization.clone();
        inherit_operation_context(
            &mut operation,
            required_operation(&session, &resolved.pending.part)?,
        );
        self.persist_tool_completion(session, &resolved, operation, Vec::new(), Vec::new())
            .await
    }

    pub(in crate::session::manager) async fn apply_tool_unavailable(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        unavailable: ToolUnavailableResult,
        _state: Arc<SessionManagerState>,
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
        let mut operation = OperationPart::tool_unavailable(
            resolved.call_id,
            resolved.invocation.clone(),
            agena_domain::RawOutput::from_parts(
                details.to_json_payload(),
                output_text.clone(),
                Vec::new(),
                details.managed_outputs.clone(),
                details.truncated,
            ),
            lifecycle.clone(),
        );
        operation.authorization = authorization.clone();
        inherit_operation_context(
            &mut operation,
            required_operation(&session, &resolved.pending.part)?,
        );
        self.persist_tool_completion(session, &resolved, operation, Vec::new(), Vec::new())
            .await
    }

    pub(in crate::session::manager) async fn apply_tool_policy_denied(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        denial: PolicyDeniedResult,
        _state: Arc<SessionManagerState>,
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

        let mut operation = OperationPart::policy_denied(
            resolved.call_id,
            resolved.invocation.clone(),
            agena_domain::RawOutput::from_parts(
                details.to_json_payload(),
                output_text.clone(),
                Vec::new(),
                details.managed_outputs.clone(),
                details.truncated,
            ),
            lifecycle.clone(),
        );
        operation.authorization = authorization.clone();
        inherit_operation_context(
            &mut operation,
            required_operation(&session, &resolved.pending.part)?,
        );

        self.persist_tool_completion(session, &resolved, operation, Vec::new(), Vec::new())
            .await
    }

    pub(in crate::session::manager) async fn apply_tool_user_declined(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        decline: UserDeclinedResult,
        persisted_rules: Vec<PersistedPermissionRule>,
        _state: Arc<SessionManagerState>,
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

        let mut operation = OperationPart::user_declined(
            resolved.call_id,
            resolved.invocation.clone(),
            agena_domain::RawOutput::from_parts(
                details.to_json_payload(),
                output_text.clone(),
                Vec::new(),
                details.managed_outputs.clone(),
                details.truncated,
            ),
            lifecycle.clone(),
        );
        operation.authorization = authorization.clone();
        inherit_operation_context(
            &mut operation,
            required_operation(&session, &resolved.pending.part)?,
        );

        self.persist_tool_completion(session, &resolved, operation, persisted_rules, Vec::new())
            .await
    }
}
