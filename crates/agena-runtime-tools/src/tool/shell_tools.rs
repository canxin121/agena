use std::{collections::HashMap, path::PathBuf};

use agena_domain::FilesystemEffects;
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
    effects: &FilesystemEffects,
) -> Result<(), ToolError> {
    // Only require declarations when the command provably mutates or reads
    // explicit files (write/redirect, input redirect, curl file ops).
    // Interpreters and build tools (node, python, uv, cargo, ...) may run with
    // an explicit empty list: their executable paths are not file effects.
    if effects.is_empty()
        && let Some(reason) =
            agena_tool::shell_analysis::filesystem_effects_required_reason(command)
    {
        return Err(ToolError::invalid_input(format!(
            "{tool_name} filesystem_effects must declare every accessed path because the command provably mutates or reads the filesystem: {reason}"
        )));
    }
    Ok(())
}
