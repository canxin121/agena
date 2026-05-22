use std::collections::HashMap;

use crate::message::PowerShellToolInput;

use super::shell::ShellRequest;
use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    ToolRuntimeContext,
};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_OUTPUT_BYTES: usize = 50 * 1024;
const MAX_OUTPUT_LINES: usize = 2_000;

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &PowerShellToolInput,
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
    if input.filesystem_effects.is_empty()
        && let Some(reason) = super::bash::filesystem_command_reason(input.command.as_str())
    {
        return Err(ToolError::InvalidInput(format!(
            "powershell filesystem_effects must declare every accessed path because the command appears to touch the filesystem: {reason}"
        )));
    }
    let cwd = input
        .workdir
        .as_deref()
        .map(|workdir| executor.resolve_target_path(workdir))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    executor.ensure_read_permission(&cwd)?;
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

    let output = ToolPayloadOutput::PowerShell {
        output: Some(display_output.clone()),
        description: if input.description.trim().is_empty() {
            None
        } else {
            Some(input.description.clone())
        },
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

fn inherited_environment() -> HashMap<String, String> {
    std::env::vars().collect::<HashMap<_, _>>()
}

fn powershell_command_for_windows(command: &str) -> Vec<String> {
    vec![
        "powershell.exe".to_string(),
        "-NoLogo".to_string(),
        "-NoProfile".to_string(),
        "-NonInteractive".to_string(),
        "-Command".to_string(),
        command.to_string(),
    ]
}

fn truncate_output(output: &str) -> (String, bool) {
    let mut lines = output.lines().collect::<Vec<_>>();
    let line_truncated = lines.len() > MAX_OUTPUT_LINES;
    if line_truncated {
        lines.truncate(MAX_OUTPUT_LINES);
    }

    let joined = lines.join("\n");
    let byte_truncated = joined.len() > MAX_OUTPUT_BYTES;
    let clipped = if byte_truncated {
        let bytes = joined.as_bytes();
        String::from_utf8_lossy(&bytes[..std::cmp::min(bytes.len(), MAX_OUTPUT_BYTES)]).to_string()
    } else {
        joined
    };

    let truncated = line_truncated || byte_truncated;
    if truncated {
        (
            format!(
                "{}\n\n[output truncated: max {} lines / {} bytes]",
                clipped, MAX_OUTPUT_LINES, MAX_OUTPUT_BYTES
            ),
            true,
        )
    } else {
        (clipped, false)
    }
}
