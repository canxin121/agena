use crate::message::{ProcessShell, ShellCommandInput};

use super::shell::ShellRequest;
use super::shell_tools::{
    DEFAULT_TIMEOUT_MS, powershell_command_for_windows, prepare_shell_execution,
    shell_execution_result, truncate_output,
};
use super::{ToolError, ToolExecutor, ToolPayloadExecution, ToolRuntimeContext};

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &ShellCommandInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    if !cfg!(windows) {
        return Err(ToolError::InvalidInput(
            "powershell tool is only available on Windows".to_string(),
        ));
    }
    let (cwd, env) = prepare_shell_execution(
        executor,
        input,
        &context,
        "powershell",
        "powershell command must not be empty",
    )?;

    let request = ShellRequest {
        command: powershell_command_for_windows(&input.command),
        cwd,
        env,
        timeout_ms: Some(input.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)),
    };
    let execution = executor.execute_shell_command(&request)?;
    let (trimmed_output, truncated) = truncate_output(&execution.aggregated_output);

    let status_text = if execution.timed_out {
        format!(
            "PowerShell command timed out after {} ms (exit_code={}).",
            request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            execution.exit_code
        )
    } else {
        format!(
            "PowerShell command exited with code {} in {} ms.",
            execution.exit_code,
            execution.duration.as_millis()
        )
    };
    Ok(shell_execution_result(
        ProcessShell::Powershell,
        "PowerShell",
        status_text,
        trimmed_output,
        truncated,
        execution.exit_code,
        execution.duration.as_millis(),
        execution.timed_out,
    ))
}
