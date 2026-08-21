//! Session timeline presentation: the human-visible v2 part the terminal
//! consumes, plus the mapping from the runtime's `SessionPartView` projection
//! (the mapping stays in the TUI per the R7 brief).

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
    pub parent_part_id: Option<i64>,
    pub run_id: Option<i64>,
    pub revision: i64,
    pub created_at_ms: i64,
}

/// Loads one bounded visible timeline page through the server's cursor-paged
/// parts endpoint and maps it into the TUI's presentation value. This avoids
/// asking `/state` for the complete transcript merely to show the timeline
/// overlay.
pub(crate) async fn list_session_timeline(
    application: &super::TuiBackend,
    session_id: i64,
    limit: u64,
) -> Result<Vec<SessionTimelineEntry>> {
    let page = application
        .list_session_parts(session_id, limit, None)
        .await?;
    Ok(page
        .parts
        .into_iter()
        .map(|part| SessionTimelineEntry {
            part_id: part.part_id,
            kind: part.kind,
            role: part.role,
            state: part.state,
            summary: part.summary,
            content: part.content,
            parent_part_id: part.parent_part_id,
            run_id: part.run_id,
            revision: part.revision,
            created_at_ms: part.created_at_ms,
        })
        .collect())
}
