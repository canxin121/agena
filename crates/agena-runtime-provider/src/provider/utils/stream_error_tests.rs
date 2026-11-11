//! Stream error classification covers the codes real gateways emit.

use super::*;
use agena_provider::ProviderErrorKind;
use serde_json::json;

fn classified(error: ProviderError) -> (ProviderErrorKind, bool) {
    match error {
        ProviderError::ProviderClassified {
            kind, retryable, ..
        } => (kind, retryable),
        other => panic!("expected a classified provider error, got {other:?}"),
    }
}

#[test]
fn gateway_internal_server_error_is_retryable() {
    // OpenAI-compatible gateways report their own failures with the OpenAI
    // style code. A corrupt terminal frame from one of them is a transient
    // transport/proxy condition, not a rejection of the request.
    let event = json!({
        "type": "response.failed",
        "response": {
            "status": "failed",
            "error": {
                "type": "server_error",
                "code": "internal_server_error",
                "message": "internal error"
            }
        }
    });
    let (kind, retryable) =
        classified(stream_error_from_event("local", &event).expect("failure event"));
    assert_eq!(kind, ProviderErrorKind::Unavailable);
    assert!(retryable, "internal_server_error must resample the request");
}

#[test]
fn request_scoped_stream_errors_stay_terminal() {
    // A malformed request is not a transport condition; retrying it only
    // repeats the rejection.
    let event = json!({
        "type": "error",
        "code": "invalid_prompt",
        "message": "unsupported instruction"
    });
    let (kind, retryable) =
        classified(stream_error_from_event("local", &event).expect("failure event"));
    assert_eq!(kind, ProviderErrorKind::InvalidRequest);
    assert!(!retryable);
}

#[test]
fn unknown_stream_error_codes_are_not_retried() {
    let event = json!({
        "type": "error",
        "code": "unrecognized_gateway_code",
        "message": "something the classifier does not know"
    });
    let (kind, retryable) =
        classified(stream_error_from_event("local", &event).expect("failure event"));
    assert_eq!(kind, ProviderErrorKind::Unavailable);
    assert!(!retryable);
}
