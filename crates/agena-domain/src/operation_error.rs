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
