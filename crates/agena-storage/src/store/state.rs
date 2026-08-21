//! The single derived session state (design section 17).
//!
//! `derive_session_state` is the ONE function that maps parts + leases to the
//! `SessionState` enum. It lives in the backend-neutral crate so every engine
//! and the facade derive identical state from identical rows — any process,
//! any backend, the same answer (17.1 principle 1).

use agena_domain::SessionLifecycleState;

use super::{
    InteractionRef, LeaseState, Part, SessionMeta, SessionPresentation, SessionState, StoreError,
};

/// A run marker that is still in flight (`pending` | `in_progress`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InFlightRun {
    pub part_id: i64,
    /// The last marker we know of; the session may have more than one only in
    /// transient crash states, and `most_recent` picks the newest.
    pub created_at_ms: i64,
}

/// A pending user-input request carried by a canonical `tool_call` part.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingInteraction {
    pub part_id: i64,
    pub created_at_ms: i64,
    pub content: serde_json::Value,
}

/// The raw inputs for deriving a session state. The engine and the facade both
/// assemble this from their rows; the derivation itself never touches storage.
#[derive(Debug, Clone, Default)]
pub struct StateInputs {
    pub meta: Option<SessionMeta>,
    /// In-flight run markers (state pending | in_progress), newest first.
    pub in_flight_runs: Vec<InFlightRun>,
    /// In-flight tool-call parts awaiting user input, newest first.
    pub pending_interactions: Vec<PendingInteraction>,
    /// The most recent error part content, if any (for `last_failure`).
    pub last_error: Option<serde_json::Value>,
}

/// Decode the first still-awaiting user-input request from the strict flat
/// `tool_call.user_input` control field. The ask lives inside the same durable
/// tool part; there is no nested operation or companion result row.
pub(crate) fn tool_call_first_awaiting_user_input(
    content: &serde_json::Value,
) -> Option<agena_domain::UserInputRequest> {
    let user_input = content.get("user_input")?;
    let user_input: agena_domain::OperationUserInput =
        serde_json::from_value(user_input.clone()).ok()?;
    user_input
        .awaiting()
        .next()
        .map(|record| record.request.clone())
}

impl StateInputs {
    /// Assemble from a loaded session view. Non-gating parts are ignored;
    /// run markers and in-flight `tool_call` parts whose operation is awaiting
    /// a user-input reply drive the state (17.3).
    pub fn from_view(view: &super::SessionView) -> Self {
        let mut pending_interactions = Vec::new();
        for part in &view.parts {
            if !part.state.is_in_flight() {
                continue;
            }
            if part.kind == "tool_call"
                && tool_call_first_awaiting_user_input(&part.content).is_some()
            {
                pending_interactions.push(PendingInteraction {
                    part_id: part.part_id,
                    created_at_ms: part.created_at_ms,
                    content: part.content.clone(),
                });
            }
        }
        Self {
            meta: Some(view.meta.clone()),
            in_flight_runs: view
                .parts
                .iter()
                .filter(|part| part.is_run_marker() && part.state.is_in_flight())
                .map(|part| InFlightRun {
                    part_id: part.part_id,
                    created_at_ms: part.created_at_ms,
                })
                .collect(),
            pending_interactions,
            last_error: view
                .parts
                .iter()
                .filter(|part| part.kind == "error")
                .max_by_key(|part| part.created_at_ms)
                .map(|part| part.content.clone()),
        }
    }
}

/// Whether a lease counts as "fresh" for state derivation (17.3).
pub fn lease_is_fresh(lease: Option<&LeaseState>, now_ms: i64) -> bool {
    match lease {
        Some(lease) => now_ms - lease.heartbeat_at_ms <= super::LEASE_STALENESS_MS,
        None => false,
    }
}

/// Stable staleness threshold shared by engines, facade, and state derivation.
/// Mirrors the storage-layer lease staleness (15s).
pub const LEASE_STALENESS_MS: i64 = 15_000;

/// Derive the single `SessionState` from parts + leases (17.3).
///
/// Precedence: Creating → Failed → AwaitingInteraction (pending interaction wins over
/// an in-flight run) → Running (fresh lease) / Interrupted (stale or none).
pub fn derive_session_state(
    meta: Option<&SessionMeta>,
    in_flight_runs: &[InFlightRun],
    pending_interactions: &[PendingInteraction],
    lease: Option<&LeaseState>,
    now_ms: i64,
) -> SessionState {
    let Some(meta) = meta else {
        return SessionState::Ready;
    };
    if meta.lifecycle_state == SessionLifecycleState::Creating {
        return SessionState::Creating;
    }
    if meta.lifecycle_state == SessionLifecycleState::Failed {
        return SessionState::Failed;
    }
    if !pending_interactions.is_empty() {
        return SessionState::AwaitingInteraction;
    }
    if let Some(_marker) = in_flight_runs.first() {
        return if lease_is_fresh(lease, now_ms) {
            SessionState::Running
        } else {
            SessionState::Interrupted
        };
    }
    SessionState::Ready
}

/// Assemble the full [`SessionPresentation`] the UI reads (17.6).
pub fn presentation(
    meta: Option<&SessionMeta>,
    in_flight_runs: &[InFlightRun],
    pending_interactions: &[PendingInteraction],
    last_error: Option<&serde_json::Value>,
    lease: Option<&LeaseState>,
    now_ms: i64,
) -> Result<SessionPresentation, StoreError> {
    let state = derive_session_state(meta, in_flight_runs, pending_interactions, lease, now_ms);
    let mut presentation = SessionPresentation {
        state,
        pending_interaction: None,
        active_run_id: None,
        last_failure: last_error.cloned(),
    };
    if let Some(interaction) = pending_interactions.first() {
        let (kind, prompt) = tool_call_first_awaiting_user_input(&interaction.content)
            .map(|request| (request.kind.as_str().to_owned(), request.title))
            .unwrap_or_else(|| ("ask_user".to_owned(), String::new()));
        presentation.pending_interaction = Some(InteractionRef {
            part_id: interaction.part_id,
            kind,
            prompt,
            content: interaction.content.clone(),
        });
    }
    presentation.active_run_id = in_flight_runs.first().map(|run| run.part_id);
    Ok(presentation)
}

/// Part lifecycle transitions the engine enforces before touching storage.
/// Mirrors the SQLite triggers for the in-memory backend.
pub fn apply_part_transition(
    part: &mut Part,
    to: super::PartState,
    now_ms: i64,
    retry_allowed: bool,
) -> Result<(), StoreError> {
    if part.state == to {
        return Ok(());
    }
    let run_marker = part.is_run_marker();
    // A failed run marker is terminal: retrying a run is a new continue run
    // (18.2), so `failed -> in_progress` is forbidden for run markers.
    if part.state == super::PartState::Failed && to == super::PartState::InProgress && run_marker {
        return Err(StoreError::InvalidState(
            "a failed run marker is terminal; retry creates a new continue run".to_owned(),
        ));
    }
    if !part.state.can_transition(to) {
        return Err(StoreError::InvalidState(format!(
            "invalid part transition {} -> {}",
            part.state.as_str(),
            to.as_str()
        )));
    }
    if to == super::PartState::InProgress
        && part.state == super::PartState::Failed
        && !retry_allowed
    {
        return Err(StoreError::InvalidState(
            "retry (failed -> in_progress) requires an explicit retry".to_owned(),
        ));
    }
    part.state = to;
    part.updated_at_ms = now_ms;
    if to.is_terminal() {
        part.finished_at_ms = Some(now_ms);
    } else if part.state == super::PartState::InProgress {
        // Retry clears the finished timestamp.
        part.finished_at_ms = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PendingInteraction, presentation};

    #[test]
    fn flat_tool_call_user_input_drives_pending_presentation() {
        let pending = PendingInteraction {
            part_id: 7,
            created_at_ms: 1,
            content: serde_json::json!({
                "user_input": {
                    "requests": [{
                        "request": {
                            "request_id": "ask-1",
                            "session_id": 3,
                            "title": "Choose a path",
                            "kind": "review",
                            "source": "plugin",
                            "questions": [],
                            "created_at": "2026-08-20T00:00:00Z"
                        }
                    }]
                }
            }),
        };

        let projected = presentation(None, &[], &[pending], None, None, 1)
            .expect("project pending interaction");
        let interaction = projected
            .pending_interaction
            .expect("flat tool call should surface its ask");
        assert_eq!(interaction.part_id, 7);
        assert_eq!(interaction.kind, "review");
        assert_eq!(interaction.prompt, "Choose a path");
    }
}
