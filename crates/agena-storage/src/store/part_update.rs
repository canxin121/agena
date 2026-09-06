//! Prepare complete part updates before either backend publishes them.

use serde_json::Value;

use super::{Part, PartDelta, PartState, RunOutcome, StoreError, apply_part_transition};

pub fn validate_run_content(state: PartState, content: &Value) -> Result<(), StoreError> {
    if content.get("run_kind").and_then(Value::as_str).is_none() {
        return Err(StoreError::InvalidState(
            "run marker requires a string run_kind in content".to_owned(),
        ));
    }
    let reason = content.get("abort_reason");
    let valid_reason = match (state, reason) {
        (PartState::Failed | PartState::Cancelled, Some(Value::String(_))) => true,
        (PartState::Failed | PartState::Cancelled, _) => false,
        (PartState::Completed, Some(Value::Null | Value::String(_))) => true,
        (PartState::Completed, _) => false,
        (_, None | Some(Value::Null | Value::String(_))) => true,
        _ => false,
    };
    if !valid_reason {
        return Err(StoreError::InvalidState(
            "run marker abort_reason must be a string or null; terminal markers require it, and failed/cancelled markers require a string".to_owned(),
        ));
    }
    Ok(())
}

/// Both backends publish only the returned candidate. The memory backend passes
/// a clone of its stored part; SQLite can pass ownership of its freshly read row.
pub fn prepare_part_update(
    mut part: Part,
    delta: PartDelta,
    now_ms: i64,
) -> Result<Part, StoreError> {
    let previous_updated_at_ms = part.updated_at_ms;
    if let Some(to) = delta.state {
        apply_part_transition(&mut part, to, now_ms, true)?;
    }
    if let Some(content) = delta.content {
        part.content = content;
    } else if let Some(delta_text) = delta.content_text_delta {
        append_text_delta(&mut part.content, &delta_text)?;
    }
    if let Some(summary) = delta.summary {
        part.summary = Some(summary);
    }
    if let Some(provider_state) = delta.provider_state {
        part.provider_state = Some(provider_state);
    }
    if let Some(finished) = delta.finished_at_ms {
        part.finished_at_ms = Some(finished);
    }
    if part.state.is_terminal() && part.finished_at_ms.is_none() {
        part.finished_at_ms = Some(now_ms);
    }
    if part.state == PartState::InProgress {
        part.finished_at_ms = None;
    }
    finish_update(previous_updated_at_ms, part, now_ms)
}

pub fn prepare_run_completion(
    mut part: Part,
    outcome: RunOutcome,
    now_ms: i64,
) -> Result<Part, StoreError> {
    if !part.is_run_marker() {
        return Err(StoreError::InvalidState(
            "part is not a run marker".to_owned(),
        ));
    }
    if !outcome.status.is_terminal() {
        return Err(StoreError::InvalidState(
            "complete_run requires a terminal outcome".to_owned(),
        ));
    }
    if matches!(outcome.status, PartState::Failed | PartState::Cancelled)
        && outcome.abort_reason.is_none()
    {
        return Err(StoreError::InvalidState(
            "terminal run markers require an abort_reason".to_owned(),
        ));
    }
    let previous_updated_at_ms = part.updated_at_ms;
    if let Some(content) = outcome.content {
        part.content = content;
    }
    let map = part.content.as_object_mut().ok_or_else(|| {
        StoreError::InvalidState("run marker content must be a JSON object".to_owned())
    })?;
    map.insert(
        "abort_reason".to_owned(),
        outcome.abort_reason.map_or(Value::Null, Value::String),
    );
    part.state = outcome.status;
    part.finished_at_ms = Some(now_ms);
    if let Some(provider_state) = outcome.provider_state {
        part.provider_state = Some(provider_state);
    }
    finish_update(previous_updated_at_ms, part, now_ms)
}

fn finish_update(
    previous_updated_at_ms: i64,
    mut part: Part,
    now_ms: i64,
) -> Result<Part, StoreError> {
    if part.started_at_ms < 0
        || part.created_at_ms < 0
        || now_ms < part.created_at_ms
        || now_ms < previous_updated_at_ms
    {
        return Err(StoreError::InvalidState(
            "invalid part lifecycle timestamps".to_owned(),
        ));
    }
    if part.state.is_terminal() != part.finished_at_ms.is_some()
        || part
            .finished_at_ms
            .is_some_and(|finished| finished < part.started_at_ms)
    {
        return Err(StoreError::InvalidState(
            "invalid part finished timestamp".to_owned(),
        ));
    }
    if part.is_run_marker() {
        validate_run_content(part.state, &part.content)?;
    }
    part.revision = part
        .revision
        .checked_add(1)
        .ok_or_else(|| StoreError::InvalidState("part revision is exhausted".to_owned()))?;
    part.updated_at_ms = now_ms;
    Ok(part)
}

fn append_text_delta(content: &mut Value, delta: &str) -> Result<(), StoreError> {
    match content {
        Value::String(text) => text.push_str(delta),
        Value::Object(map) => match map.get_mut("text") {
            Some(Value::String(text)) => text.push_str(delta),
            _ => {
                return Err(StoreError::InvalidState(
                    "content_text_delta requires a text-shaped content".to_owned(),
                ));
            }
        },
        _ => {
            return Err(StoreError::InvalidState(
                "content_text_delta requires a text-shaped content".to_owned(),
            ));
        }
    }
    Ok(())
}
