use std::fmt;

use agena_macros::ToolInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("name", "path"))]
#[serde(tag = "target", rename_all = "snake_case")]
pub(crate) enum EnterSnapshotCommandInput {
    /// Create a new managed snapshot under the managed `snapshots` directory.
    #[input(non_empty_if_present("name"))]
    New {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Attach to an already-existing snapshot at the provided path.
    #[input(non_empty("path"))]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
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
            let snapshots_dir = agena_runtime::project_state_dir(workspace_root).join("snapshots");
            Ok(vec![PathRequest::write(
                snapshots_dir.to_string_lossy().to_string(),
            )])
        }
    }
}
use super::{Deserialize, JsonSchema, Path, PathRequest, SdkResult, Serialize};
