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
    let (permission, effective_permission) = permission_configs_from_resources(
        &execution.execution.selected_permission,
        &execution.execution.effective_permission,
    )?;
    Ok(SessionPermissionStudioState {
        session_id,
        session_title: execution.session.title.clone(),
        permission,
        effective_permission,
    })
}

fn permission_configs_from_resources(
    selected: &agena_api::resource::PermissionConfigResource,
    effective: &agena_api::resource::PermissionConfigResource,
) -> Result<(
    agena_domain::PermissionConfig,
    agena_domain::PermissionConfig,
)> {
    let selected = serde_json::from_value(
        serde_json::to_value(selected)
            .context("failed to serialize selected permission resource")?,
    )
    .context("failed to decode selected permission resource")?;
    let effective = serde_json::from_value(
        serde_json::to_value(effective)
            .context("failed to serialize effective permission resource")?,
    )
    .context("failed to decode effective permission resource")?;
    Ok((selected, effective))
}

#[cfg(test)]
mod tests {
    use super::permission_configs_from_resources;

    #[test]
    fn selected_and_effective_permission_remain_distinct() {
        let selected = serde_json::from_value(serde_json::json!({
            "tools": { "default": "ask" }
        }))
        .expect("selected permission resource");
        let effective = serde_json::from_value(serde_json::json!({
            "tools": { "default": "deny" }
        }))
        .expect("effective permission resource");

        let (selected, effective) =
            permission_configs_from_resources(&selected, &effective).expect("decode permissions");
        assert_eq!(
            selected.tools.and_then(|tools| tools.default),
            Some(agena_domain::PermissionMode::Ask)
        );
        assert_eq!(
            effective.tools.and_then(|tools| tools.default),
            Some(agena_domain::PermissionMode::Deny)
        );
    }
}
