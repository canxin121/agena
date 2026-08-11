//! Session refresh presentation: whether anything changed since the last
//! observed event sequence, and how many events were missed.

use agena_api::resource::SessionExecutionResource;
use agena_application::Application;
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
    application: &Application,
    session_id: i64,
    after_seq: Option<i64>,
    force: bool,
) -> Result<SessionRefresh> {
    let queries = application
        .session_query_service()
        .map_err(anyhow::Error::new)?;
    let latest_event_seq = queries
        .latest_event_seq(session_id)
        .await
        .map_err(anyhow::Error::new)?;
    let changed = force
        || match (after_seq, latest_event_seq) {
            (None, Some(_)) => true,
            (Some(after), Some(current)) => current > after,
            _ => false,
        };

    if !changed {
        return Ok(SessionRefresh {
            latest_event_seq,
            event_count: 0,
            execution: None,
        });
    }

    let event_count = after_seq
        .zip(latest_event_seq)
        .map(|(after, current)| current.saturating_sub(after).clamp(0, 256) as usize)
        .unwrap_or(0);

    let execution =
        crate::app_backend::operations::get_session_state(application, session_id).await?;
    Ok(SessionRefresh {
        latest_event_seq,
        event_count,
        execution: Some(execution),
    })
}
