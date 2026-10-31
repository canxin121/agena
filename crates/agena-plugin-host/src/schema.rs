//! Compiled JSON Schema contracts shared by registry and interactive callers.

pub(crate) use agena_plugin_sdk::schema_validation::CompiledSchema;
pub use agena_plugin_sdk::schema_validation::validate_json_schema_value;

/// A fixed semantic marker; peer-provided failure policies are never trusted.
pub const OUTPUT_CONTRACT_VIOLATION_MARKER: &str = "output_contract_violation";

pub fn output_contract_failure() -> agena_failure::Failure {
    use agena_failure::*;
    Failure::new(
        FailureCode::new("tool.output_contract_violation"),
        FailureCategory::ProtocolFailure,
        FailureResponsibility::Dependency,
        RetryDirective::Never,
        RecoveryDirective::None,
        FailureImpact::PartialSuccess,
        UserPresentation::new(
            "tool.output_contract_violation",
            "The tool ran but its output is invalid. Effects may be committed; do not repeat automatically.",
        ),
    )
}

pub fn output_contract_error(diagnostic: impl std::fmt::Display) -> crate::sdk::PluginError {
    let mut error = crate::sdk::PluginError::internal(diagnostic)
        .with_public_problem(OUTPUT_CONTRACT_VIOLATION_MARKER);
    let mut failure = output_contract_failure();
    let detail =
        agena_failure::diagnostic::user_message_with_context(&error.diagnostic.message, 128);
    if !detail.is_empty() {
        failure.user = agena_failure::UserPresentation::validated(
            "tool.output_contract_violation-detail",
            format!("{} Validation: {detail}", failure.user.fallback),
        );
    }
    error.failure = Box::new(failure);
    error
}
