use crate::message::{ProcessShell, ProcessStatus, ShellCommandInput};

use super::shell::ShellRequest;
use super::shell_tools::{
    DEFAULT_TIMEOUT_MS, inherited_environment, powershell_command_for_windows, resolve_workdir,
    truncate_output, validate_declared_filesystem_effects,
};
use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    ToolRuntimeContext,
};

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
    if input.command.trim().is_empty() {
        return Err(ToolError::InvalidInput(
            "powershell command must not be empty".to_string(),
        ));
    }
    validate_declared_filesystem_effects(
        "powershell",
        input.command.as_str(),
        &input.filesystem_effects,
    )?;
    let cwd = resolve_workdir(executor, input.workdir.as_deref())?;
    executor.ensure_filesystem_effects_permission(&input.filesystem_effects, &cwd)?;
    executor.ensure_network_effects_permission(&input.network_effects)?;

    let mut env = inherited_environment();
    env.extend(executor.shell_env_overrides(&cwd, context.session_id, context.call_id)?);

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
    let display_output = if trimmed_output.trim().is_empty() {
        status_text.clone()
    } else {
        trimmed_output.clone()
    };

    let output = ToolPayloadOutput::Process {
        action: "run".to_string(),
        shell: Some(ProcessShell::Powershell),
        background: false,
        process_id: None,
        status: Some(if execution.timed_out {
            ProcessStatus::TimedOut
        } else {
            ProcessStatus::Exited
        }),
        output: Some(display_output.clone()),
        description: Some(status_text.clone()),
        events: Vec::new(),
        processes: Vec::new(),
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code: Some(execution.exit_code),
    };
    let mut view = ToolExecutionView::simple("PowerShell".to_string(), display_output);
    view.metadata
        .insert("exit_code".to_string(), execution.exit_code.to_string());
    view.metadata
        .insert("timed_out".to_string(), execution.timed_out.to_string());
    view.metadata.insert(
        "duration_ms".to_string(),
        execution.duration.as_millis().to_string(),
    );
    view.metadata
        .insert("truncated".to_string(), truncated.to_string());
    view.metadata.insert("status".to_string(), status_text);

    Ok(ToolPayloadExecution::new(output, view))
}
