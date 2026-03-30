use std::cmp::min;
use std::collections::HashMap;

use procwarden::SandboxCommandRequest;

use crate::message::{BashToolInput, BuiltinToolOutput};

use super::{
    BuiltinExecution, BuiltinExecutionContext, ToolError, ToolExecutionView, ToolExecutor,
};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_OUTPUT_BYTES: usize = 50 * 1024;
const MAX_OUTPUT_LINES: usize = 2_000;

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &BashToolInput,
    context: BuiltinExecutionContext,
) -> Result<BuiltinExecution, ToolError> {
    if input.command.trim().is_empty() {
        return Err(ToolError::InvalidInput(
            "bash command must not be empty".to_string(),
        ));
    }

    let cwd = input
        .workdir
        .as_deref()
        .map(|workdir| executor.resolve_target_path(workdir))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    executor.ensure_read_permission(&cwd)?;

    let mut env = inherited_environment();
    env.extend(executor.shell_env_overrides(
        &cwd,
        context.session_id,
        context.call_id,
    )?);

    let request = SandboxCommandRequest {
        command: shell_command_for_platform(&input.command),
        cwd,
        env,
        timeout_ms: Some(input.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)),
    };

    let execution = executor.execute_sandboxed_command(&request)?;
    let (trimmed_output, truncated) = truncate_output(&execution.aggregated_output);

    let status_text = if execution.timed_out {
        format!(
            "Command timed out after {} ms (exit_code={}).",
            request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            execution.exit_code
        )
    } else {
        format!(
            "Command exited with code {} in {} ms.",
            execution.exit_code,
            execution.duration.as_millis()
        )
    };

    let output = BuiltinToolOutput::Bash {
        output: Some(trimmed_output.clone()),
        description: Some(status_text.clone()),
    };

    let title = if input.description.trim().is_empty() {
        format!("Bash {}", input.command)
    } else {
        format!("Bash {}", input.description)
    };

    let mut view = ToolExecutionView::simple(title, trimmed_output);
    view.metadata
        .insert("exit_code".to_string(), execution.exit_code.to_string());
    view.metadata.insert(
        "duration_ms".to_string(),
        execution.duration.as_millis().to_string(),
    );
    view.metadata
        .insert("timed_out".to_string(), execution.timed_out.to_string());
    view.metadata
        .insert("truncated".to_string(), truncated.to_string());
    view.metadata.insert(
        "sandbox_mode".to_string(),
        format!("{:?}", executor.sandbox_policy()),
    );
    if execution.timed_out {
        view.metadata.insert(
            "timeout_ms".to_string(),
            request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).to_string(),
        );
    }

    Ok(BuiltinExecution::new(output, view))
}

fn inherited_environment() -> HashMap<String, String> {
    std::env::vars().collect::<HashMap<_, _>>()
}

fn shell_command_for_platform(command: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            "cmd.exe".to_string(),
            "/d".to_string(),
            "/s".to_string(),
            "/c".to_string(),
            command.to_string(),
        ]
    } else {
        vec![
            "/bin/sh".to_string(),
            "-lc".to_string(),
            command.to_string(),
        ]
    }
}

fn truncate_output(output: &str) -> (String, bool) {
    let mut lines = output.lines().collect::<Vec<_>>();
    let line_truncated = lines.len() > MAX_OUTPUT_LINES;
    if line_truncated {
        lines.truncate(MAX_OUTPUT_LINES);
    }

    let joined = lines.join("\n");
    let byte_truncated = joined.as_bytes().len() > MAX_OUTPUT_BYTES;
    let clipped = if byte_truncated {
        let bytes = joined.as_bytes();
        String::from_utf8_lossy(&bytes[..min(bytes.len(), MAX_OUTPUT_BYTES)]).to_string()
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
