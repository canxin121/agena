//! Permission studio presentation state for a session.

use anyhow::Result;

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
    let execution = application.get_session_state(session_id).await?;
    Ok(SessionPermissionStudioState {
        session_id,
        session_title: execution.session.title.clone(),
        permission: execution.execution.selected_permission.into(),
        effective_permission: execution.execution.effective_permission.into(),
    })
}
