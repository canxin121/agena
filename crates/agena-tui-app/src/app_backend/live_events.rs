//! Live session event presentation: the [`LiveEvent`] value the TUI consumes
//! and the subscription adapter that pumps server-stream events into a typed
//! channel.

use tokio::sync::mpsc;

/// Session subscription signal used by the TUI to converge from snapshots and
/// force a persisted-state refresh after lag or transport loss.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    /// Snapshot captured after a live subscription was established. Remote
    /// reconnect uses this to close the subscribe/read race.
    pub snapshot: Option<agena_api::resource::SessionExecutionResource>,
    /// True when the UI should ignore incremental assumptions and force a
    /// replay from persisted state (for example after bus lag).
    pub force_refresh: bool,
}

/// Subscribe through the server's session event stream into a typed
/// channel. Generic transport events remain available separately for timeline
/// consumers.
pub(crate) fn subscribe_session_events(
    application: &super::TuiBackend,
    session_id: i64,
) -> Option<mpsc::Receiver<LiveEvent>> {
    Some(application.subscribe_session_events(session_id))
}
