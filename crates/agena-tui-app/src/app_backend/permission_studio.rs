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
    let permission = if let Ok(embedded) = application.embedded_application() {
        embedded
            .session_execution_services()?
            .queries
            .execution_context(session_id)
            .await
            .map_err(anyhow::Error::new)
            .with_context(|| format!("failed to load execution context for session {session_id}"))?
            .selected_permission
    } else {
        // The current public execution resource exposes the effective policy,
        // not the pre-resolution selected policy. Remote mode therefore opens
        // this view read-only from the authoritative effective projection.
        effective_permission.clone()
    };
    Ok(SessionPermissionStudioState {
        session_id,
        session_title: execution.session.title.clone(),
        permission,
        effective_permission,
    })
}
