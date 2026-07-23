//! Snapshot tool adapter.
//!
//! Concrete Git/Rift snapshot operations are runtime-owned. This module only
//! connects tool payloads, permission checks, and the session registry.

use crate::message::{EnterSnapshotToolInput, ExitSnapshotToolInput};
use agena_runtime::{SnapshotRegistry, SnapshotSession};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

pub fn registry_for_executor() -> SnapshotRegistry {
    agena_runtime::snapshot_registry()
}

pub(super) fn execute_enter(
    executor: &ToolExecutor,
    input: &EnterSnapshotToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
    let session_id = session_id.ok_or_else(|| {
        ToolError::Plugin("snapshot.enter: no session in execution context".to_string())
    })?;
    let registry = executor
        .snapshot_registry()
        .ok_or_else(|| ToolError::Plugin("snapshot.enter: registry not configured".to_string()))?;
    if registry.read().contains_key(&session_id) {
        return Err(ToolError::Plugin(
            "snapshot.enter: session is already in a snapshot; call `snapshot exit` first"
                .to_string(),
        ));
    }
    if input.name.is_some() && input.path.is_some() {
        return Err(ToolError::Plugin(
            "snapshot.enter: provide either `name` or `path`, not both".to_string(),
        ));
    }

    let workspace = executor.workspace_root();
    let creation = if let Some(path) = input.path.as_deref() {
        agena_runtime::SnapshotCreation {
            session: agena_runtime::attach_existing_snapshot(workspace, path)
                .map_err(|error| ToolError::Plugin(error.to_string()))?,
            note: None,
        }
    } else {
        agena_runtime::create_managed_snapshot(workspace, input.name.as_deref())
            .map_err(|error| ToolError::Plugin(error.to_string()))?
    };
    let agena_runtime::SnapshotCreation { session, note } = creation;
    let note_line = note
        .as_deref()
        .map(|note| format!("  note:    {note}\n"))
        .unwrap_or_default();
    let view = ToolExecutionView::simple(
        format!("Snapshot → {}", session.path.display()),
        format!(
            "Switched to managed snapshot:\n  backend: {}\n  path:    {}\n  branch:  {}\n{}",
            session.backend,
            session.path.display(),
            session.branch,
            note_line,
        ),
    );
    let output = ToolPayloadOutput::EnterSnapshot {
        path: session.path.to_string_lossy().to_string(),
        branch: session.branch.clone(),
        backend: Some(session.backend.to_string()),
        note,
    };
    registry.write().insert(session_id, session);
    Ok(ToolPayloadExecution::new(output, view))
}

pub(super) fn execute_exit(
    executor: &ToolExecutor,
    input: &ExitSnapshotToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
    let session_id = session_id.ok_or_else(|| {
        ToolError::Plugin("snapshot.exit: no session in execution context".to_string())
    })?;
    let registry = executor
        .snapshot_registry()
        .ok_or_else(|| ToolError::Plugin("snapshot.exit: registry not configured".to_string()))?;
    let session = registry
        .write()
        .remove(&session_id)
        .ok_or_else(|| ToolError::Plugin("snapshot.exit: not in a snapshot".to_string()))?;

    let action = input.action.trim();
    if action == "remove"
        && session.created_here
        && let Err(error) = remove_created_workspace(executor, &session, input.discard_changes)
    {
        registry.write().insert(session_id, session.clone());
        return Err(error);
    }
    let view = ToolExecutionView::simple(
        format!("Snapshot exited ({action})"),
        format!(
            "Snapshot at {} (backend {}, branch {}) — action: {action}",
            session.path.display(),
            session.backend,
            session.branch,
        ),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::ExitSnapshot {
            action: action.to_string(),
            path: session.path.to_string_lossy().to_string(),
        },
        view,
    ))
}

fn remove_created_workspace(
    executor: &ToolExecutor,
    session: &SnapshotSession,
    discard_changes: bool,
) -> Result<(), ToolError> {
    executor.ensure_read_permission(&session.path)?;
    executor.ensure_edit_permission(&session.path)?;
    if !discard_changes && agena_runtime::snapshot_has_local_changes(&session.path) {
        return Err(ToolError::Plugin(
            "snapshot.exit: snapshot has local changes; re-call with `discard_changes: true` to force removal"
                .to_string(),
        ));
    }
    agena_runtime::remove_managed_snapshot(session)
        .map_err(|error| ToolError::Plugin(error.to_string()))
}
