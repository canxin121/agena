use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "snapshot",
    description = "Managed snapshot command. Use action `enter` or `exit`; `enter` uses `target = new|existing` to create or attach to a managed snapshot. Agena prefers Rift snapshots and falls back to git worktree when Rift cannot be used. `exit` uses enum `exit_action = keep|remove`.",
    summary = "Enter or exit a managed repository snapshot.",
    handler_receiver = WorkflowPlugin,
    display = brief,
    tags(ToolTag::Mutating, ToolTag::FilesystemWrite, ToolTag::Snapshot),
    capabilities(HostCapability::SnapshotRegistry, HostCapability::PluginStorage),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum SnapshotToolInput {
    #[tool(
        exec = "enter",
        handle = WorkflowPlugin::invoke_snapshot_enter,
        permission_paths_handle = WorkflowPlugin::permission_snapshot_enter
    )]
    Enter {
        #[serde(flatten)]
        #[tool(flatten_shape)]
        args: EnterSnapshotCommandInput,
    },
    #[tool(
        exec = "exit",
        handle = WorkflowPlugin::invoke_snapshot_exit,
        permission_paths_handle = WorkflowPlugin::permission_snapshot_exit
    )]
    Exit {
        #[serde(flatten)]
        #[tool(flatten_shape)]
        args: ExitSnapshotCommandInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("name", "path"))]
#[serde(tag = "target", rename_all = "snake_case")]
pub(crate) enum EnterSnapshotCommandInput {
    /// Create a new managed snapshot under the managed `snapshots` directory.
    #[tool_input(non_empty_if_present("name"))]
    New {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Attach to an already-existing snapshot at the provided path.
    #[tool_input(non_empty("path"))]
    Existing { path: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExitSnapshotAction {
    Keep,
    Remove,
}

impl ExitSnapshotAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Remove => "remove",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
pub(crate) struct ExitSnapshotCommandInput {
    #[serde(rename = "exit_action")]
    pub(crate) exit_action: ExitSnapshotAction,
    #[serde(default)]
    pub(crate) discard_changes: bool,
}

pub(crate) fn snapshot_enter_permission_paths(
    workspace_root: &Path,
    input: &EnterSnapshotCommandInput,
) -> SdkResult<Vec<PathRequest>> {
    match input {
        EnterSnapshotCommandInput::Existing { path } if !path.trim().is_empty() => Ok(vec![
            PathRequest::read(path.clone()),
            PathRequest::write(path.clone()),
        ]),
        EnterSnapshotCommandInput::Existing { .. } | EnterSnapshotCommandInput::New { .. } => {
            let snapshots_dir =
                crate::project_paths::project_state_dir(workspace_root).join("snapshots");
            Ok(vec![PathRequest::write(
                snapshots_dir.to_string_lossy().to_string(),
            )])
        }
    }
}
