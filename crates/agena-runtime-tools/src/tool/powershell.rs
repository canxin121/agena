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
    shell::{
        DEFAULT_SHELL_TIMEOUT_MS, powershell_command_for_windows, truncate_shell_output_budget,
    },
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
    let mut request = ShellRequest {
        command: powershell_command_for_windows(&input.command),
        cwd,
        env,
        timeout_ms: Some(input.timeout_ms.unwrap_or(DEFAULT_SHELL_TIMEOUT_MS)),
    };
    (request.command, request.env) =
        crate::shell_sandbox::protect_async(executor, request.command, input, request.env).await?;
    let _worker_permit = super::shell::acquire_worker_permit().await?;
    let execution = executor
        .execute_shell_command_with_live(&request, context.output.clone())
        .await?;
    executor.ensure_not_cancelled()?;
    let output_resource = context
        .output
        .as_ref()
        .map(|writer| writer.resource().reference());
    render_execution(&request, execution, output_resource, input.max_output_bytes)
}

fn render_execution(
    request: &ShellRequest,
    execution: agena_tool::ShellOutput,
    output_resource: Option<agena_domain::ContentRef>,
    max_output_bytes: Option<u32>,
) -> Result<ToolPayloadExecution, ToolError> {
    let (mut trimmed_output, truncated) = truncate_shell_output_budget(
        &execution.aggregated_output,
        crate::process_output::text_budget(max_output_bytes, false),
    );
    if let Some(resource) = &output_resource {
        trimmed_output.push_str(&format!("\n\nOutput resource: {}", resource.resource_id));
    }

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
        output_resource,
        terminal: None,
        dropped_bytes: 0,
        action: "exec".to_string(),
        shell: Some(ProcessShell::Powershell),
        background: false,
        ready: false,
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
        next_event_offset: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code: Some(execution.exit_code),
        completion_reason: Some(
            if execution.timed_out {
                "timeout"
            } else {
                "exit"
            }
            .to_owned(),
        ),
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
