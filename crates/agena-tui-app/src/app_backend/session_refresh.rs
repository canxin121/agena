//! Session refresh presentation: whether anything changed since the last
//! observed event sequence, and how many events were missed.

use agena_api::resource::SessionExecutionResource;
use anyhow::Result;

/// Refresh signal of a session.
#[derive(Debug, Clone)]
pub struct SessionRefresh {
    pub latest_event_seq: Option<i64>,
    pub event_count: usize,
    pub execution: Option<SessionExecutionResource>,
}

/// Loads the latest event sequence and, when the session moved past
/// `after_seq`, the full session execution snapshot.
pub(crate) async fn refresh_session(
    application: &super::TuiBackend,
    session_id: i64,
    after_seq: Option<i64>,
    force: bool,
) -> Result<SessionRefresh> {
    application
        .refresh_session(session_id, after_seq, force)
        .await
}
