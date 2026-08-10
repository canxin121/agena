use super::{
    AppError, Arc, ExecutionStatus, OperationPart, PersistedPermissionRule, SessionManager,
    SessionManagerState, SessionPendingTool, ToolError, completed_lifecycle,
    operation_authorization, resolve_pending_tool, terminal_operation_title, text_result_blocks,
    update_resolved_tool_message,
};
use crate::session::Session;
use crate::session::store::{
    part_state_from_execution_status, tool_call_from_operation, typed_content_to_value,
};
use agena_domain::ToolOutput;
use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, ModelFeedback,
    RecoveryDirective, RetryDirective, UserPresentation,
};
use agena_runtime_contracts::part_content::TypedContent;

fn internal_tool_failure() -> Failure {
    Failure::new(
        FailureCode::new("tool.internal"),
        FailureCategory::Internal,
        FailureResponsibility::System,
        RetryDirective::UseAlternative,
        RecoveryDirective::ChooseAlternative,
        FailureImpact::OperationFailed,
        UserPresentation::new(
            "tool-internal-failure",
            "Tool execution failed without diagnostic details.",
        ),
    )
    .with_model_feedback(ModelFeedback::internal_tool_failure())
}

fn actionable_tool_presentation(
    error: &ToolError,
    key: &'static str,
    fallback: &'static str,
) -> UserPresentation {
    error
        .actionable_message()
        .filter(|message| !message.trim().is_empty())
        .map(|message| UserPresentation::validated(key, message))
        .unwrap_or_else(|| UserPresentation::new(key, fallback))
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
            actionable_tool_presentation(
                error,
                "tool-invalid-input-detail",
                "The tool input is invalid.",
            ),
            ModelFeedback::invalid_input(),
        ),
        ToolError::InvalidInput { fields, .. } => (
            "tool.invalid_input",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            actionable_tool_presentation(
                error,
                "tool-invalid-input-detail",
                "The tool input is invalid.",
            ),
            ModelFeedback::invalid_input_with_fields(fields.iter().cloned()),
        ),
        ToolError::InvalidGlobPattern(_) => (
            "tool.invalid_pattern",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            actionable_tool_presentation(
                error,
                "tool-invalid-pattern-detail",
                "The search pattern is invalid.",
            ),
            ModelFeedback::invalid_pattern(),
        ),
        ToolError::InvalidRegexPattern(_) => (
            "tool.invalid_pattern",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            actionable_tool_presentation(
                error,
                "tool-invalid-pattern-detail",
                "The search pattern is invalid.",
            ),
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
        ToolError::UserDeclined(_) => (
            "tool.user_declined",
            FailureCategory::PermissionDenied,
            FailureResponsibility::Policy,
            RetryDirective::AfterUserAction,
            RecoveryDirective::RequestPermission,
            UserPresentation::new("tool-user-declined", "The user declined the tool."),
            ModelFeedback::permission_denied(),
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
            UserPresentation::new(
                "tool-capability-unavailable",
                "The tool capability is unavailable.",
            ),
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
        ToolError::Plugin(problem) => return problem.public.clone(),
        ToolError::Shell(_) | ToolError::Io(_) => (
            "tool.execution_failed",
            FailureCategory::Internal,
            FailureResponsibility::System,
            RetryDirective::UseAlternative,
            RecoveryDirective::ChooseAlternative,
            actionable_tool_presentation(
                error,
                "tool-execution-failed-detail",
                "The tool could not complete the operation.",
            ),
            ModelFeedback::internal_tool_failure(),
        ),
        ToolError::Cancelled => return internal_tool_failure(),
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
    use super::super::is_authorization_phase_title;
    use super::{terminal_operation_title, tool_error_failure};
    use crate::tool::ToolError;
    use agena_domain::{StructuredObject, ToolInvocation};
    use agena_failure::{FailureCategory, RetryDirective};

    #[test]
    fn sensitive_plugin_diagnostics_are_redacted_from_user_and_model_channels() {
        let diagnostic = "database error: token=secret response 123 is missing or already terminal";
        let failure = tool_error_failure(&ToolError::plugin(diagnostic));
        let encoded = serde_json::to_string(&failure).expect("serialize safe failure");
        assert_eq!(failure.category, FailureCategory::Internal);
        assert_eq!(failure.retry, RetryDirective::UseAlternative);
        assert!(failure.model.is_some());
        // Secrets never cross the boundary, but the real root cause survives.
        assert!(!failure.user.fallback.contains("token=secret"));
        assert!(
            failure
                .user
                .fallback
                .contains("response 123 is missing or already terminal")
        );
        assert!(!encoded.contains(diagnostic));
        assert!(!encoded.contains("token=secret"));
    }

    #[test]
    fn actionable_plugin_error_is_preserved_verbatim() {
        let failure = tool_error_failure(&ToolError::plugin(
            "connection closed before the response body completed",
        ));
        assert_eq!(
            failure.user.fallback,
            "connection closed before the response body completed"
        );
        assert!(!failure.user.fallback.contains("failed unexpectedly"));
    }

    #[test]
    fn legacy_invalid_input_text_is_scrubbed_root_cause() {
        let diagnostic = "invalid path /private/tmp/secret-project: parser backtrace";
        let failure = tool_error_failure(&ToolError::invalid_input(diagnostic));
        let encoded = serde_json::to_string(&failure).expect("serialize safe failure");
        assert_eq!(failure.category, FailureCategory::InvalidInput);
        assert_eq!(failure.retry, RetryDirective::CorrectInput);
        assert!(!encoded.contains(diagnostic));
        assert!(!encoded.contains("/private/tmp"));
        // The user sees a bounded root cause rather than a wall of paths.
        assert!(failure.user.fallback.contains("invalid path"));
        assert!(!failure.user.fallback.contains("/private"));
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

    #[test]
    fn failed_tools_call_uses_the_execution_tool_as_its_title() {
        let arguments = StructuredObject::try_from(serde_json::json!({
            "tool": "shell.run",
            "input": {"command": "python3 calc_pi.py"}
        }))
        .expect("valid tools_call input");
        let invocation = ToolInvocation {
            tool_api_call: Some(agena_domain::ToolApiCall {
                function: agena_domain::ToolApiFunction::Call,
                arguments,
            }),
            name: "shell.run".to_owned(),
            plugin_name: None,
            input: StructuredObject::try_from(serde_json::json!({
                "command": "python3 calc_pi.py"
            }))
            .expect("valid target input"),
        };
        assert_eq!(terminal_operation_title(&invocation), "shell.run");
    }

    #[test]
    fn authorization_phase_titles_are_never_valid_terminal_titles() {
        assert!(is_authorization_phase_title(
            "Awaiting permission: shell.run"
        ));
        assert!(is_authorization_phase_title(
            "Awaiting approval · write access"
        ));
        assert!(is_authorization_phase_title("Permission request · network"));
        assert!(!is_authorization_phase_title("Run shell.run · exit 0"));
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
        let authorization = operation_authorization(&session, &resolved);
        let failure_title = terminal_operation_title(&resolved.invocation);

        // Notification delivery is bounded by the plugin host and remains
        // part of this lifecycle, so no detached hook task can outlive it.
        state
            .tool_executor
            .broadcast_tool_failure(&resolved.invocation, session.id, resolved.call_id, &failure)
            .await;

        let _assistant_message =
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
                operation.authorization = authorization.clone();
                operation.set_title(failure_title.clone());
                tool_part.content = typed_content_to_value(&TypedContent::ToolCall(
                    tool_call_from_operation(&operation),
                ))
                .expect("tool content is always JSON serializable");
                tool_part.state = part_state_from_execution_status(ExecutionStatus::Failed);
            })?;

        self.persist_tool_completion(session, &resolved, persisted_rules, state)
            .await
    }
}
