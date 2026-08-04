use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, RecoveryDirective,
    RetryDirective, UserPresentation,
};

pub(crate) fn unexpected_service_failure(
    code: &'static str,
    fallback: &'static str,
    diagnostic: impl std::fmt::Display,
) -> Failure {
    let diagnostic = diagnostic.to_string();
    // Surface the scrubbed root cause instead of the static fallback, so the
    // user sees what actually went wrong. `validated_with_context` degrades to
    // a generic invalid-request text only when nothing safe remains.
    let presentation = UserPresentation::validated_with_context(code, &diagnostic);
    let fallback =
        if presentation.fallback == "The request is invalid. Review the input and try again." {
            fallback.to_owned()
        } else {
            presentation.fallback
        };
    let failure = Failure::new(
        FailureCode::new(code),
        FailureCategory::Internal,
        FailureResponsibility::System,
        RetryDirective::Unknown,
        RecoveryDirective::Retry,
        FailureImpact::RequestRejected,
        UserPresentation {
            key: code.to_owned(),
            fallback,
            detail_key: None,
        },
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
    formatter.write_str(failure.user.fallback.as_str())
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
        assert!(!display.contains("token=secret"));
        assert!(!display.contains("Reference:"));
        // The user sees a real message, never a bare correlation id.
        assert!(!display.contains("Something went wrong."));
        assert!(!display.is_empty());
    }
}
