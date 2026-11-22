use crate::Failure;

/// Prefix of a stored notice carrying a structured execution failure.
pub const FAILURE_NOTICE_PREFIX: &str = "\u{1f}agena.failure:";

pub fn failure_notice_text(failure: &Failure) -> serde_json::Result<String> {
    Ok(format!(
        "{FAILURE_NOTICE_PREFIX}{}",
        serde_json::to_string(failure)?
    ))
}

/// Ordinary notices have no failure marker. A malformed marker is kept
/// distinct so callers can report or recover from corrupted failure data.
pub fn decode_failure_notice(text: &str) -> Option<serde_json::Result<Failure>> {
    let payload = text.strip_prefix(FAILURE_NOTICE_PREFIX)?;
    Some(serde_json::from_str(payload))
}
