mod apply_patch;

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::agent::Agent;
use crate::message::{BuiltinToolInput, BuiltinToolOutput};
use crate::permission::{AccessKind, PermissionDecision};

pub use apply_patch::{AppliedFileChange, ApplyPatchExecution};

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("permission confirmation required: {0}")]
    PermissionAsk(String),
    #[error("invalid patch: {0}")]
    InvalidPatch(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported builtin tool in executor: {0}")]
    UnsupportedBuiltin(&'static str),
}

pub struct ToolExecutor {
    workspace_root: PathBuf,
    agent: Agent,
}

impl ToolExecutor {
    pub fn new(workspace_root: impl Into<PathBuf>, agent: Agent) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent,
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    pub fn execute_builtin(
        &self,
        input: &BuiltinToolInput,
    ) -> Result<(BuiltinToolOutput, Option<ApplyPatchExecution>), ToolError> {
        match self.agent.authorize_builtin_tool(input) {
            PermissionDecision::Allow => {}
            PermissionDecision::Ask { reason } => return Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => {
                return Err(ToolError::PermissionDenied(reason));
            }
        }

        match input {
            BuiltinToolInput::ApplyPatch(payload) => {
                let result = apply_patch::execute(self, payload)?;
                let output = BuiltinToolOutput::ApplyPatch {
                    operation_id: result.operation_id.clone(),
                    files: result
                        .files
                        .iter()
                        .map(|f| f.path.clone())
                        .collect::<Vec<_>>(),
                    before_hash: Some(result.before_hash.clone()),
                    after_hash: Some(result.after_hash.clone()),
                    inverse_patch: result.inverse_patch.clone(),
                };
                Ok((output, Some(result)))
            }
            BuiltinToolInput::Bash(_) => Err(ToolError::UnsupportedBuiltin("bash")),
            BuiltinToolInput::Read(_) => Err(ToolError::UnsupportedBuiltin("read")),
            BuiltinToolInput::Write(_) => Err(ToolError::UnsupportedBuiltin("write")),
            BuiltinToolInput::Edit(_) => Err(ToolError::UnsupportedBuiltin("edit")),
            BuiltinToolInput::Glob(_) => Err(ToolError::UnsupportedBuiltin("glob")),
            BuiltinToolInput::Grep(_) => Err(ToolError::UnsupportedBuiltin("grep")),
            BuiltinToolInput::Task(_) => Err(ToolError::UnsupportedBuiltin("task")),
        }
    }

    pub(crate) fn ensure_edit_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        match self.agent.authorize_path_access(
            AccessKind::Write,
            self.workspace_root(),
            target_path,
        ) {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask { reason } => Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
        }
    }
}
