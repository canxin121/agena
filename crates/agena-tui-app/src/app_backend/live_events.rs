//! Live session event presentation: the [`LiveEvent`] value the TUI consumes
//! and the subscription adapter that pumps server-stream events into a typed
//! channel.

use tokio::sync::mpsc;

/// Push notification emitted by the unified bus for the active session.
/// Indicates whether the change requires reloading messages.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    /// Snapshot captured after a live subscription was established. Remote
    /// reconnect uses this to close the subscribe/read race.
    pub snapshot: Option<agena_api::resource::SessionExecutionResource>,
    /// Concrete event payload when the subscriber kept up with the bus.
    /// `None` means the receiver lagged and the UI should force-refresh
    /// from persisted state instead of trying to apply an incremental patch.
    pub event: Option<agena_runtime::RuntimePresentationEvent>,
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
