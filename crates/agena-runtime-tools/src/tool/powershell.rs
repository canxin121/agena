use crate::part::ShellCommandInput;
use agena_domain::{ProcessShell, ProcessStatus};

use super::shell_tools::{
    inherited_environment, resolve_workdir, validate_declared_filesystem_effects,
};
use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    ToolRuntimeContext,
};
use agena_tool::{
    ShellRequest,
    shell::{DEFAULT_SHELL_TIMEOUT_MS, powershell_command_for_windows, truncate_shell_output},
};

pub(super) async fn execute_async(
    executor: &ToolExecutor,
    input: &ShellCommandInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    if !cfg!(windows) {
        return Err(ToolError::invalid_input(
            "powershell tool is only available on Windows".to_string(),
        ));
    }
    if input.command.trim().is_empty() {
        return Err(ToolError::invalid_input(
            "powershell command must not be empty".to_string(),
        ));
    }
    let effects = input.filesystem_effects();
    validate_declared_filesystem_effects("powershell", input.command.as_str(), &effects)?;
    let cwd = resolve_workdir(executor, input.workdir.as_deref())?;
    let mut env = inherited_environment();
    env.extend(
        executor
            .shell_env_overrides_async(&cwd, context.session_id, context.call_id)
            .await?,
    );
    let request = ShellRequest {
        command: powershell_command_for_windows(&input.command),
        cwd,
        env,
        timeout_ms: Some(input.timeout_ms.unwrap_or(DEFAULT_SHELL_TIMEOUT_MS)),
    };
    let worker_executor = executor.clone();
    let worker_request = request.clone();
    let command = input.command.clone();
    let session_id = context.session_id;
    let call_id = context.call_id;
    let worker_permit = super::shell::acquire_worker_permit()
        .await
        .ok_or_else(|| ToolError::plugin("shell worker pool is unavailable".to_string()))?;
    let execution = tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        worker_executor.execute_shell_command(
            &worker_request,
            command.as_str(),
            session_id,
            call_id,
        )
    })
    .await
    .map_err(|error| {
        ToolError::plugin(format!(
            "PowerShell process worker failed before completion: {error}"
        ))
    })??;
    executor.ensure_not_cancelled()?;
    render_execution(&request, execution)
}

fn render_execution(
    request: &ShellRequest,
    execution: agena_tool::ShellOutput,
) -> Result<ToolPayloadExecution, ToolError> {
    let (trimmed_output, truncated) = truncate_shell_output(&execution.aggregated_output);

    let status_text = if execution.timed_out {
        format!(
            "PowerShell command timed out after {} ms (exit_code={}).",
            request.timeout_ms.unwrap_or(DEFAULT_SHELL_TIMEOUT_MS),
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
        trimmed_output
    };

    let output = ToolPayloadOutput::Shell {
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
    let run_summary = if execution.timed_out {
        format!("Timed out · {} ms", execution.duration.as_millis())
    } else {
        format!(
            "Exit {} · {} ms",
            execution.exit_code,
            execution.duration.as_millis()
        )
    };
    let mut view = ToolExecutionView::simple("PowerShell", run_summary, display_output);
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
