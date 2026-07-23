use serde::{Deserialize, Serialize};

/// Structured error information attached to an operation or tool result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::OperationError;

    #[test]
    fn operation_error_round_trips_optional_code() {
        let value = OperationError {
            message: "denied".into(),
            code: Some("permission_denied".into()),
        };
        assert_eq!(
            serde_json::from_value::<OperationError>(serde_json::to_value(&value).unwrap())
                .unwrap(),
            value
        );
    }
}
