use serde::{Deserialize, Serialize};

/// Structured error information attached to an operation or tool result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationError {
    pub failure: agena_failure::Failure,
}

impl OperationError {
    pub fn user_message(&self) -> &str {
        self.failure.user.fallback.as_str()
    }

    pub fn model_message(&self) -> Option<String> {
        self.failure
            .model
            .as_ref()
            .map(|feedback| feedback.message())
    }
}

#[cfg(test)]
mod tests {
    use super::OperationError;

    #[test]
    fn operation_error_round_trips_optional_code() {
        let value = OperationError {
            failure: agena_failure::Failure::new(
                agena_failure::FailureCode::new("tool.permission_denied"),
                agena_failure::FailureCategory::PermissionDenied,
                agena_failure::FailureResponsibility::Policy,
                agena_failure::RetryDirective::AfterUserAction,
                agena_failure::RecoveryDirective::RequestPermission,
                agena_failure::FailureImpact::OperationFailed,
                agena_failure::UserPresentation::new(
                    "tool-permission-denied",
                    "Tool access was denied.",
                ),
            ),
        };
        assert_eq!(
            serde_json::from_value::<OperationError>(serde_json::to_value(&value).unwrap())
                .unwrap(),
            value
        );
    }
}
