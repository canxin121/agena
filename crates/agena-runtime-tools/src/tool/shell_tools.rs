use std::{collections::HashMap, path::PathBuf};

use agena_domain::FilesystemEffect;
pub(crate) use agena_tool::shell_analysis::{ExitInterpretation, analyze_command};

use super::{ToolError, ToolExecutor};

pub(crate) fn inherited_environment() -> HashMap<String, String> {
    std::env::vars().collect::<HashMap<_, _>>()
}

pub(crate) fn resolve_workdir(
    executor: &ToolExecutor,
    workdir: Option<&str>,
) -> Result<PathBuf, ToolError> {
    let cwd = workdir
        .map(|value| executor.resolve_target_path(value))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    Ok(cwd)
}

pub(crate) fn validate_declared_filesystem_effects(
    tool_name: &str,
    command: &str,
    effects: &[FilesystemEffect],
) -> Result<(), ToolError> {
    if effects.is_empty()
        && let Some(reason) = agena_tool::shell_analysis::filesystem_command_reason(command)
    {
        return Err(ToolError::invalid_input(format!(
            "{tool_name} filesystem_effects must declare every accessed path because the command appears to touch the filesystem: {reason}"
        )));
    }
    Ok(())
}
