use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "worktree",
    description = "Worktree command. Use action `enter` or `exit`; `enter` uses `target = new|existing` to create or attach to a git worktree and `exit` uses enum `exit_action = keep|remove`.",
    summary = "Enter or exit a git worktree.",
    handler_receiver = WorkflowPlugin,
    display = brief,
    tags(ToolTag::Mutating, ToolTag::FilesystemWrite, ToolTag::Worktree),
    host_capabilities(HostCapability::WorktreeRegistry, HostCapability::PluginStorage),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum WorktreeToolInput {
    #[tool(
        exec = "enter",
        handle = WorkflowPlugin::invoke_worktree_enter,
        permission_paths_handle = WorkflowPlugin::permission_worktree_enter
    )]
    Enter {
        #[serde(flatten)]
        #[tool(flatten_shape)]
        args: EnterWorktreeCommandInput,
    },
    #[tool(
        exec = "exit",
        handle = WorkflowPlugin::invoke_worktree_exit,
        permission_paths_handle = WorkflowPlugin::permission_worktree_exit
    )]
    Exit {
        #[serde(flatten)]
        #[tool(flatten_shape)]
        args: ExitWorktreeCommandInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("name", "path"))]
#[serde(tag = "target", rename_all = "snake_case")]
pub(crate) enum EnterWorktreeCommandInput {
    /// Create a new worktree under the managed `worktrees` directory.
    #[tool_input(non_empty_if_present("name"))]
    New {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Attach to an already-existing worktree at the provided path.
    #[tool_input(non_empty("path"))]
    Existing { path: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExitWorktreeAction {
    Keep,
    Remove,
}

impl ExitWorktreeAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Remove => "remove",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
pub(crate) struct ExitWorktreeCommandInput {
    #[serde(rename = "exit_action")]
    pub(crate) exit_action: ExitWorktreeAction,
    #[serde(default)]
    pub(crate) discard_changes: bool,
}

pub(crate) fn worktree_enter_permission_paths(
    workspace_root: &Path,
    input: &EnterWorktreeCommandInput,
) -> SdkResult<Vec<PathRequest>> {
    match input {
        EnterWorktreeCommandInput::Existing { path } if !path.trim().is_empty() => Ok(vec![
            PathRequest::read(path.clone()),
            PathRequest::write(path.clone()),
        ]),
        EnterWorktreeCommandInput::Existing { .. } | EnterWorktreeCommandInput::New { .. } => {
            let worktrees_dir =
                crate::project_paths::project_state_dir(workspace_root).join("worktrees");
            Ok(vec![PathRequest::write(
                worktrees_dir.to_string_lossy().to_string(),
            )])
        }
    }
}
