//! Session refresh presentation: whether anything changed since the last
//! observed event sequence, and how many events were missed.

use anyhow::Result;

use super::SessionStateWithTranscriptPage;

/// Refresh signal of a session.
#[derive(Debug, Clone)]
pub struct SessionRefresh {
    pub latest_event_seq: Option<i64>,
    pub event_count: usize,
    /// Keep the transcript's folds and history cursor beside its parts.
    pub(crate) snapshot: Option<SessionStateWithTranscriptPage>,
}

/// Load the latest execution together with its bounded transcript page,
/// including fold expansion cursors and the older-history boundary.
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
