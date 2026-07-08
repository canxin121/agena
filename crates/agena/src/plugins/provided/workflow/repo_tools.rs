use std::fmt;

use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape, ToolSuite};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "enter",
    summary = "Enter a managed repository snapshot.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_snapshot_enter,
    handle_field = args,
    permission_paths_handle = WorkflowPlugin::permission_snapshot_enter,
    display = brief,
    tags(ToolTag::Mutating, ToolTag::FilesystemWrite, ToolTag::Snapshot),
    capabilities(HostCapability::SnapshotRegistry, HostCapability::PluginStorage),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotEnterToolInput {
    #[serde(flatten)]
    #[tool(flatten_shape)]
    args: EnterSnapshotCommandInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "exit",
    summary = "Exit a managed repository snapshot.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_snapshot_exit,
    handle_field = args,
    permission_paths_handle = WorkflowPlugin::permission_snapshot_exit,
    display = brief,
    tags(ToolTag::Mutating, ToolTag::FilesystemWrite, ToolTag::Snapshot),
    capabilities(HostCapability::SnapshotRegistry, HostCapability::PluginStorage),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotExitToolInput {
    #[serde(flatten)]
    #[tool(flatten_shape)]
    args: ExitSnapshotCommandInput,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = WorkflowPlugin)]
pub(crate) enum SnapshotToolSuite {
    Enter(SnapshotEnterToolInput),
    Exit(SnapshotExitToolInput),
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

impl AsRef<str> for ExitSnapshotAction {
    fn as_ref(&self) -> &str {
        match self {
            Self::Keep => "keep",
            Self::Remove => "remove",
        }
    }
}

impl fmt::Display for ExitSnapshotAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
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
