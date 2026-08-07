use super::shell_tools::{
    ExitInterpretation, analyze_command, inherited_environment, resolve_workdir,
    validate_declared_filesystem_effects,
};
use agena_tool::{
    ShellRequest,
    shell::{DEFAULT_SHELL_TIMEOUT_MS, shell_command_for_platform, truncate_shell_output},
    shell_analysis::interpret_exit_code,
};

use crate::message::ShellCommandInput;
use agena_domain::{ProcessShell, ProcessStatus};
use agena_plugin_host::{CommandAfterInput, CommandBeforeInput, CommandBeforeOutcome};

use super::{
    PreparedShellCommand, ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution,
    ToolPayloadOutput, ToolRuntimeContext,
};

pub(super) fn prepare_command(
    executor: &ToolExecutor,
    input: &ShellCommandInput,
    session_id: i64,
    call_id: i64,
) -> Result<Option<PreparedShellCommand>, ToolError> {
    let cwd = resolve_workdir(executor, input.workdir.as_deref())?;

    let mut env = inherited_environment();
    env.extend(executor.shell_env_overrides(&cwd, Some(session_id), Some(call_id))?);

    let env_btree = env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<std::collections::BTreeMap<String, String>>();
    let hook_input = CommandBeforeInput {
        session_id: Some(session_id),
        call_id: Some(call_id),
        workspace_root: Some(executor.workspace_root().to_string_lossy().to_string()),
        command: "sh".to_string(),
        args: vec!["-c".to_string(), input.command.clone()],
        cwd: cwd.clone(),
        env: env_btree,
    };
    match executor
        .plugin_manager()
        .dispatch_command_before_blocking(hook_input)
    {
        Ok(CommandBeforeOutcome::Continue(updated)) => {
            let command =
                if updated.args.len() >= 2 && updated.args[0] == "-c" && updated.command == "sh" {
                    updated.args[1].clone()
                } else {
                    input.command.clone()
                };
            Ok(Some(PreparedShellCommand {
                command,
                cwd: updated.cwd,
            }))
        }
        Ok(CommandBeforeOutcome::Abort(reason)) => {
            let action = agena_domain::PermissionAction::Tool {
                tool_name: "shell".to_string(),
                qualifier: Some(input.command.clone()),
            };
            Err(ToolError::PolicyDenied(Box::new(
                agena_domain::PolicyDeniedResult {
                    action: action.clone(),
                    related_actions: vec![action.clone()],
                    denied_actions: vec![action],
                    reason: format!("command aborted by plugin: {reason}"),
                    explanation: "a trusted command-before policy hook denied execution"
                        .to_string(),
                    source: Some("command_before_hook".to_string()),
                    scope: None,
                    operator: None,
                    authority: agena_domain::PermissionAuthorityKind::PluginPolicy,
                    rule_id: None,
                    rule_revision_ms: None,
                    trace: vec![agena_domain::DecisionTraceStep {
                        source_kind: agena_domain::PolicySourceKind::PluginAdvice,
                        summary: format!("command-before hook denied execution: {reason}"),
                        source: Some("command_before_hook".to_string()),
                        scope: None,
                        operator: None,
                    }],
                },
            )))
        }
        Err(err) => {
            tracing::warn!(
                target: "agena_plugin_host::command_before",
                "command.execute.before hook failed (continuing): {err}"
            );
            Ok(None)
        }
    }
}

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &ShellCommandInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    if input.command.trim().is_empty() {
        return Err(ToolError::invalid_input(
            "bash command must not be empty".to_string(),
        ));
    }

    let analysis = analyze_command(input.command.as_str());
    let effects = input.filesystem_effects();
    validate_declared_filesystem_effects("bash", input.command.as_str(), &effects)?;

    let cwd = resolve_workdir(executor, input.workdir.as_deref())?;

    let mut env = inherited_environment();
    env.extend(executor.shell_env_overrides(&cwd, context.session_id, context.call_id)?);

    let prepared = match context.prepared_shell_command {
        Some(prepared) => Some(prepared),
        None => match (context.session_id, context.call_id) {
            (Some(session_id), Some(call_id)) => {
                executor.prepare_shell_command(input, session_id, call_id)?
            }
            _ => None,
        },
    };
    let (final_command, final_cwd) = prepared
        .map(|prepared| (prepared.command, prepared.cwd))
        .unwrap_or_else(|| (input.command.clone(), cwd));
    let final_analysis = analyze_command(final_command.as_str());
    let command_rewritten = final_command != input.command;

    let request = ShellRequest {
        command: shell_command_for_platform(&final_command),
        cwd: final_cwd,
        env,
        timeout_ms: Some(input.timeout_ms.unwrap_or(DEFAULT_SHELL_TIMEOUT_MS)),
    };

    let execution = executor.execute_shell_command(
        &request,
        final_command.as_str(),
        context.session_id,
        context.call_id,
    )?;

    // Plugin chain: command.execute.after. Plugins can observe or rewrite
    // stdout/stderr; we use the (potentially patched) combined output.
    let patched_after = {
        let hook_input = CommandAfterInput {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), final_command.clone()],
            cwd: request.cwd.clone(),
            exit_code: Some(execution.exit_code),
            stdout: execution.stdout.clone(),
            stderr: execution.stderr.clone(),
            timed_out: execution.timed_out,
        };
        match executor
            .plugin_manager()
            .dispatch_command_after_blocking(hook_input)
        {
            Ok(after) => Some(after),
            Err(err) => {
                tracing::warn!(
                    target: "agena_plugin_host::command_after",
                    "command.execute.after hook failed (continuing): {err}"
                );
                None
            }
        }
    };
    let aggregated_for_display = patched_after
        .map(|a| {
            if a.stdout.is_empty() {
                a.stderr
            } else if a.stderr.is_empty() {
                a.stdout
            } else {
                format!("{}\n{}", a.stdout, a.stderr)
            }
        })
        .unwrap_or(execution.aggregated_output.clone());

    let (trimmed_output, truncated) = truncate_shell_output(&aggregated_for_display);
    let exit_interpretation =
        interpret_exit_code(&analysis, execution.exit_code, execution.timed_out);

    let status_text = if execution.timed_out {
        format!(
            "Command timed out after {} ms (exit_code={}).",
            request.timeout_ms.unwrap_or(DEFAULT_SHELL_TIMEOUT_MS),
            execution.exit_code
        )
    } else if matches!(exit_interpretation, ExitInterpretation::NoMatches) {
        format!(
            "Command completed with no matches (exit_code={}) in {} ms.",
            execution.exit_code,
            execution.duration.as_millis()
        )
    } else if matches!(exit_interpretation, ExitInterpretation::DifferencesFound) {
        format!(
            "Command completed and found differences (exit_code={}) in {} ms.",
            execution.exit_code,
            execution.duration.as_millis()
        )
    } else {
        format!(
            "Command exited with code {} in {} ms.",
            execution.exit_code,
            execution.duration.as_millis()
        )
    };
    let display_output = if trimmed_output.trim().is_empty() {
        status_text.clone()
    } else {
        trimmed_output.clone()
    };

    let output = ToolPayloadOutput::Shell {
        action: "run".to_string(),
        shell: Some(ProcessShell::Bash),
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

    let title = if input.description.trim().is_empty() {
        format!("Bash {}", input.command)
    } else {
        format!("Bash {}", input.description)
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
    let mut view = ToolExecutionView::simple(title, run_summary, display_output);
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
        "command_classification".to_string(),
        analysis.classification.label().to_string(),
    );
    view.metadata.insert(
        "final_command_classification".to_string(),
        final_analysis.classification.label().to_string(),
    );
    view.metadata.insert(
        "command_rewritten".to_string(),
        command_rewritten.to_string(),
    );
    view.metadata.insert(
        "exit_interpretation".to_string(),
        exit_interpretation.label().to_string(),
    );
    if let Some(primary_command) = analysis.primary_command.as_deref() {
        view.metadata
            .insert("primary_command".to_string(), primary_command.to_string());
    }
    if let Some(primary_command) = final_analysis.primary_command.as_deref() {
        view.metadata.insert(
            "final_primary_command".to_string(),
            primary_command.to_string(),
        );
    }
    if command_rewritten {
        view.metadata
            .insert("final_command".to_string(), final_command.clone());
    }
    if execution.timed_out {
        view.metadata.insert(
            "timeout_ms".to_string(),
            request
                .timeout_ms
                .unwrap_or(DEFAULT_SHELL_TIMEOUT_MS)
                .to_string(),
        );
    }

    Ok(ToolPayloadExecution::new(output, view))
}
