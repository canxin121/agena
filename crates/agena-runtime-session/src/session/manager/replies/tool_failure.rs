use super::{
    AppError, Arc, ExecutionStatus, OperationPart, PartContent, PersistedPermissionRule,
    SessionManager, SessionManagerState, SessionPendingTool, ToolError, completed_lifecycle,
    resolve_pending_tool, text_result_blocks, tool_name, update_resolved_tool_message,
};
use crate::session::Session;
use agena_domain::ToolOutput;
use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, ModelFeedback,
    RecoveryDirective, RetryDirective, UserPresentation,
};

fn internal_tool_failure() -> Failure {
    Failure::new(
        FailureCode::new("tool.internal"),
        FailureCategory::Internal,
        FailureResponsibility::System,
        RetryDirective::UseAlternative,
        RecoveryDirective::ChooseAlternative,
        FailureImpact::OperationFailed,
        UserPresentation::new("tool-internal-failure", "The tool failed unexpectedly."),
    )
    .with_model_feedback(ModelFeedback::internal_tool_failure())
}

fn tool_error_failure(error: &ToolError) -> Failure {
    // Cancellation is an execution/tool outcome and is handled by
    // `apply_tool_cancellation`. Reaching this mapper is an invariant failure,
    // never a public "cancelled error".
    if matches!(
        error,
        ToolError::Cancelled | ToolError::Shell(agena_tool::ShellError::Cancelled)
    ) {
        return internal_tool_failure();
    }
    let (code, category, responsibility, retry, recovery, user, model) = match error {
        ToolError::InvalidPatch(_) => (
            "tool.invalid_input",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            UserPresentation::new("tool-invalid-input", "The tool input is invalid."),
            ModelFeedback::invalid_input(),
        ),
        ToolError::InvalidInput { fields, .. } => (
            "tool.invalid_input",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            UserPresentation::new("tool-invalid-input", "The tool input is invalid."),
            ModelFeedback::invalid_input_with_fields(fields.iter().cloned()),
        ),
        ToolError::InvalidGlobPattern(_) => (
            "tool.invalid_pattern",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            UserPresentation::new("tool-invalid-pattern", "The search pattern is invalid."),
            ModelFeedback::invalid_pattern(),
        ),
        ToolError::InvalidRegexPattern(_) => (
            "tool.invalid_pattern",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            UserPresentation::new("tool-invalid-pattern", "The search pattern is invalid."),
            ModelFeedback::invalid_pattern(),
        ),
        ToolError::ToolUnavailable(_) => (
            "tool.not_found",
            FailureCategory::NotFound,
            FailureResponsibility::Caller,
            RetryDirective::UseAlternative,
            RecoveryDirective::ChooseAlternative,
            UserPresentation::new("tool-not-found", "The requested tool is unavailable."),
            ModelFeedback::tool_unavailable(),
        ),
        ToolError::ToolUnavailable(_) => (
            "tool.not_found",
            FailureCategory::NotFound,
            FailureResponsibility::Caller,
            RetryDirective::UseAlternative,
            RecoveryDirective::ChooseAlternative,
            UserPresentation::new("tool-not-found", "The requested tool is unavailable."),
            ModelFeedback::tool_unavailable(),
        ),
        ToolError::UserDeclined(_) => (
            "tool.user_declined",
            FailureCategory::PermissionDenied,
            FailureResponsibility::Policy,
            RetryDirective::AfterUserAction,
            RecoveryDirective::RequestPermission,
            UserPresentation::new("tool-user-declined", "The user declined the tool."),
            ModelFeedback::permission_denied(),
        ),
        ToolError::InvalidExecutionGrant(_) => (
            "tool.invalid_grant",
            FailureCategory::Conflict,
            FailureResponsibility::Caller,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
            UserPresentation::new("tool-invalid-grant", "The execution grant is invalid or stale."),
            ModelFeedback::stale_tool_call(),
        ),
        ToolError::StaleToolCall { .. } => (
            "tool.stale_call",
            FailureCategory::Conflict,
            FailureResponsibility::Caller,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
            UserPresentation::new("tool-stale-call", "The tool call is no longer current."),
            ModelFeedback::stale_tool_call(),
        ),
        ToolError::PolicyDenied(_) => permission_denied_failure(),
        ToolError::CapabilityUnavailable(_) => (
            "tool.capability_unavailable",
            FailureCategory::DependencyUnavailable,
            FailureResponsibility::Caller,
            RetryDirective::UseAlternative,
            RecoveryDirective::ChooseAlternative,
            UserPresentation::new("tool-capability-unavailable", "The tool capability is unavailable."),
            ModelFeedback::tool_unavailable(),
        ),
        ToolError::UserInputRequired(_) => (
            "tool.user_input_required",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::AfterUserAction,
            RecoveryDirective::AskUser,
            UserPresentation::new(
                "tool-user-input-required",
                "The tool needs more information.",
            ),
            ModelFeedback::user_input_required(),
        ),
        ToolError::Plugin(plugin_str) => {
            return Failure::new(
                FailureCode::new("tool.plugin"),
                FailureCategory::ProtocolFailure,
                FailureResponsibility::System,
                RetryDirective::UseAlternative,
                RecoveryDirective::ChooseAlternative,
                FailureImpact::OperationFailed,
                UserPresentation::new(plugin_str.clone(), "The plugin reported an error."),
            )
            .with_model_feedback(ModelFeedback::plugin_failure());
        }
        ToolError::Shell(_) | ToolError::Io(_) | ToolError::Cancelled => {
            return internal_tool_failure();
        }
    };
    Failure::new(
        FailureCode::new(code),
        category,
        responsibility,
        retry,
        recovery,
        FailureImpact::OperationFailed,
        user,
    )
    .with_model_feedback(model)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::tool_error_failure;
    use crate::tool::ToolError;
    use agena_failure::{FailureCategory, RetryDirective};

    #[test]
    fn plugin_diagnostics_never_enter_user_or_model_channels() {
        let diagnostic = "database error: token=secret response 123 is missing or already terminal";
        let failure = tool_error_failure(&ToolError::plugin(diagnostic));
        let encoded = serde_json::to_string(&failure).expect("serialize safe failure");

        assert_eq!(failure.category, FailureCategory::Internal);
        assert_eq!(failure.retry, RetryDirective::UseAlternative);
        assert!(failure.model.is_some());
        assert!(!encoded.contains(diagnostic));
        assert!(!encoded.contains("token=secret"));
    }

    #[test]
    fn legacy_invalid_input_text_is_diagnostic_only() {
        let diagnostic = "invalid path /private/tmp/secret-project: parser backtrace";
        let failure = tool_error_failure(&ToolError::invalid_input(diagnostic));
        let encoded = serde_json::to_string(&failure).expect("serialize safe failure");

        assert_eq!(failure.category, FailureCategory::InvalidInput);
        assert_eq!(failure.retry, RetryDirective::CorrectInput);
        assert!(!encoded.contains(diagnostic));
        assert!(!encoded.contains("/private/tmp"));
    }

    #[test]
    fn structured_field_issue_guides_model_without_parser_diagnostic() {
        let diagnostic = "offset parser failed at /private/project token=secret";
        let failure = tool_error_failure(&ToolError::invalid_field(
            "offset",
            agena_failure::FieldIssueKind::OutOfRange,
            diagnostic,
        ));
        let model_message = failure.model.as_ref().expect("model feedback").message();
        let encoded = serde_json::to_string(&failure).expect("serialize failure");
        assert!(model_message.contains("offset (out of range)"));
        assert!(!model_message.contains("/private/project"));
        assert!(!encoded.contains("token=secret"));
    }
}

fn permission_denied_failure() -> (
    &'static str,
    FailureCategory,
    FailureResponsibility,
    RetryDirective,
    RecoveryDirective,
    UserPresentation,
    ModelFeedback,
) {
    (
        "tool.permission_denied",
        FailureCategory::PermissionDenied,
        FailureResponsibility::Policy,
        RetryDirective::AfterUserAction,
        RecoveryDirective::RequestPermission,
        UserPresentation::new("tool-permission-denied", "Tool access was denied."),
        ModelFeedback::permission_denied(),
    )
}

fn user_declined_failure() -> Failure {
    Failure::new(
        FailureCode::new("tool.user_declined"),
        FailureCategory::PermissionDenied,
        FailureResponsibility::Caller,
        RetryDirective::Never,
        RecoveryDirective::None,
        FailureImpact::OperationFailed,
        UserPresentation::new(
            "tool-user-declined",
            "The requested user input was declined.",
        ),
    )
    .with_model_feedback(ModelFeedback::user_declined())
}

impl SessionManager {
    pub(in crate::session::manager) async fn apply_tool_error(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        error: ToolError,
        persisted_rule: Option<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let diagnostic = error.to_string();
        let failure = tool_error_failure(&error);
        tracing::warn!(
            failure_id = %failure.id,
            session_id = session.id,
            diagnostic = %diagnostic,
            "tool execution failed"
        );
        self.persist_tool_failure(
            session,
            pending_tool,
            failure,
            persisted_rule.into_iter().collect(),
            state,
        )
        .await
    }

    pub(in crate::session::manager) async fn apply_permission_denied(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        diagnostic: String,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let (code, category, responsibility, retry, recovery, user, model) =
            permission_denied_failure();
        let failure = Failure::new(
            FailureCode::new(code),
            category,
            responsibility,
            retry,
            recovery,
            FailureImpact::OperationFailed,
            user,
        )
        .with_model_feedback(model);
        tracing::info!(
            failure_id = %failure.id,
            session_id = session.id,
            diagnostic = %diagnostic,
            "tool access denied"
        );
        self.persist_tool_failure(session, pending_tool, failure, Vec::new(), state)
            .await
    }

    pub(in crate::session::manager) async fn apply_user_declined(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        diagnostic: String,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let failure = user_declined_failure();
        tracing::info!(
            failure_id = %failure.id,
            session_id = session.id,
            diagnostic = %diagnostic,
            "user declined tool interaction"
        );
        self.persist_tool_failure(session, pending_tool, failure, Vec::new(), state)
            .await
    }

    async fn persist_tool_failure(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        failure: Failure,
        persisted_rules: Vec<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let blocks = text_result_blocks(failure.user.fallback.as_str());
        let failure_title = session
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

        // Notify plugins about the tool failure (fire-and-forget).
        state.tool_executor.broadcast_tool_failure(
            &resolved.invocation,
            session.id,
            resolved.call_id,
            &failure,
        );

        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::failed(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    failure.clone(),
                    blocks.clone(),
                    Vec::new(),
                    ToolOutput::default(),
                    lifecycle.clone(),
                );
                operation.set_title(failure_title.clone());
                tool_part.set_content(PartContent::operation(operation));
                tool_part.status = ExecutionStatus::Failed;
            })?;

        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            persisted_rules,
            Vec::new(),
            state,
        )
        .await
    }
}
