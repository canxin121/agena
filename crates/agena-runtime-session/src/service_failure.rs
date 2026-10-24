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
