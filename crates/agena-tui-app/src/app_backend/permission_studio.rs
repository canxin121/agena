//! Permission studio presentation state for a session.

use anyhow::{Context, Result};

/// Permission studio state of a session.
#[derive(Debug, Clone)]
pub struct SessionPermissionStudioState {
    pub session_id: i64,
    pub session_title: String,
    pub permission: agena_domain::PermissionConfig,
    pub effective_permission: agena_domain::PermissionConfig,
}

/// Builds the permission studio presentation from the current execution
/// snapshot and the session's selected permission.
pub(crate) async fn get_session_permission_studio_state(
    application: &super::TuiBackend,
    session_id: i64,
) -> Result<SessionPermissionStudioState> {
    let execution =
        crate::app_backend::operations::get_session_state(application, session_id).await?;
    let effective_permission: agena_domain::PermissionConfig = serde_json::from_value(
        serde_json::to_value(&execution.execution.effective_permission)
            .context("failed to serialize effective permission resource")?,
    )
    .context("failed to decode effective permission resource")?;
    // The current public execution resource exposes the effective policy, not
    // the pre-resolution selected policy. The TUI is a pure HTTP client, so
    // the permission studio opens read-only from the authoritative effective
    // projection.
    let permission = effective_permission.clone();
    Ok(SessionPermissionStudioState {
        session_id,
        session_title: execution.session.title.clone(),
        permission,
        effective_permission,
    })
}
