//! Session timeline presentation: the human-visible v2 part the terminal
//! consumes, plus the mapping from the runtime's `SessionPartView` projection
//! (the mapping stays in the TUI per the R7 brief).

use agena_application::Application;
use anyhow::Result;

/// One human-visible v2 part in the session timeline. The terminal consumes
/// this presentation value instead of a persisted runtime-event envelope.
#[derive(Debug, Clone)]
pub struct SessionTimelineEntry {
    pub part_id: i64,
    pub kind: String,
    pub role: String,
    pub state: String,
    pub summary: Option<String>,
    pub content: serde_json::Value,
    pub rendered_markdown: Option<String>,
    pub parent_part_id: Option<i64>,
    pub run_id: Option<i64>,
    pub revision: i64,
    pub created_at_ms: i64,
}

/// Loads the visible timeline parts through `Application` and maps them into
/// the TUI's presentation value.
pub(crate) async fn list_session_timeline(
    application: &Application,
    session_id: i64,
    limit: u64,
) -> Result<Vec<SessionTimelineEntry>> {
    let parts = application
        .list_session_timeline_parts(session_id, limit)
        .await
        .map_err(anyhow::Error::new)?;
    Ok(parts.into_iter().map(entry_from_part).collect())
}

fn entry_from_part(part: agena_storage::store::SessionPartView) -> SessionTimelineEntry {
    SessionTimelineEntry {
        part_id: part.part_id,
        kind: part.kind,
        role: part.role.as_str().to_owned(),
        state: part.state.as_str().to_owned(),
        summary: part.summary,
        content: part.content,
        rendered_markdown: part.rendered_markdown,
        parent_part_id: part.parent_part_id,
        run_id: part.run_id,
        revision: part.revision,
        created_at_ms: part.created_at_ms,
    }
}
