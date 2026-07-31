use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, RecoveryDirective,
    RetryDirective, UserPresentation,
};

pub(crate) fn unexpected_service_failure(
    code: &'static str,
    fallback: &'static str,
    diagnostic: impl std::fmt::Display,
) -> Failure {
    let failure = Failure::new(
        FailureCode::new(code),
        FailureCategory::Internal,
        FailureResponsibility::System,
        RetryDirective::Unknown,
        RecoveryDirective::Retry,
        FailureImpact::RequestRejected,
        UserPresentation::new(code, fallback),
    );
    tracing::error!(
        failure_id = %failure.id,
        failure_code = %failure.code,
        diagnostic = %diagnostic,
        "runtime service boundary failure"
    );
    failure
}

pub(crate) fn display_service_failure(
    failure: &Failure,
    formatter: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    formatter.write_str(failure.user.fallback.as_str())?;
    if failure.is_unexpected() {
        write!(formatter, " Reference: {}", failure.id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn service_diagnostic_is_not_part_of_the_public_failure() {
        let error = crate::SessionQueryError::internal(
            "database error token=secret /private/session.sqlite",
        );
        let public = serde_json::to_string(&error.failure).expect("serialize failure");
        let display = error.to_string();
        assert!(!public.contains("token=secret"));
        assert!(!public.contains("/private/session.sqlite"));
        assert!(!display.contains("database error"));
        assert!(display.contains("Reference:"));
    }
}
