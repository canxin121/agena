use super::shell_tools::{
    ExitInterpretation, analyze_command, inherited_environment, prepare_detected_launch,
    prepare_posix_launch, resolve_workdir_in_folder, shell_settings,
    validate_declared_filesystem_effects,
};
use agena_tool::{
    ShellOutput, ShellRequest,
    shell::{DEFAULT_SHELL_TIMEOUT_MS, shell_command_for_platform, truncate_shell_output_budget},
    shell_analysis::{CommandAnalysis, interpret_exit_code},
};

use crate::part::ShellCommandInput;
use agena_domain::{ProcessShell, ProcessStatus};
use agena_plugin_host::{CommandAfterInput, CommandBeforeInput, CommandBeforeOutcome};

use super::imported_shell::DirectoryCapture;
use super::{
    PreparedShellCommand, ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution,
    ToolPayloadOutput, ToolRuntimeContext,
};

pub(super) async fn prepare_command_async(
    executor: &ToolExecutor,
    input: &ShellCommandInput,
    session_id: i64,
    call_id: i64,
    tool_name: &str,
    bash: Option<&agena_process::shell::DetectedShell>,
) -> Result<Option<PreparedShellCommand>, ToolError> {
    let cwd =
        resolve_workdir_in_folder(executor, input.workdir.as_deref(), input.folder.as_deref())?;
    let mut env = inherited_environment();
    // Resolved before plugin overrides so the precedence is
    // inherited < snapshot < plugin `shell.env`/`command.before` values.
    let launch = match bash {
        Some(bash) => prepare_detected_launch(shell_settings(), bash, &mut env).await?,
        None => prepare_posix_launch(shell_settings(), &mut env).await?,
    };
    env.extend(
        executor
            .shell_env_overrides_async(&cwd, Some(session_id), Some(call_id))
            .await?,
    );
    add_agent_plugin_bins(executor, &mut env);
    let fallback_env = env.clone();
    let hook_input = CommandBeforeInput {
        session_id: Some(session_id),
        call_id: Some(call_id),
        workspace_root: (!executor.is_global())
            .then(|| executor.workspace_root().to_string_lossy().to_string()),
        command: "sh".to_string(),
        args: vec!["-c".to_string(), input.command.clone()],
        cwd: cwd.clone(),
        env: env.into_iter().collect(),
    };
    match executor
        .plugin_manager()
        .dispatch_command_before(hook_input)
        .await
    {
        Ok(CommandBeforeOutcome::Continue(updated)) => {
            if executor.is_global() && !updated.cwd.is_absolute() {
                return Err(ToolError::invalid_input(
                    "global shell execution requires an explicit absolute workdir",
                ));
            }
            let command =
                if updated.args.len() >= 2 && updated.args[0] == "-c" && updated.command == "sh" {
                    updated.args[1].clone()
                } else {
                    input.command.clone()
                };
            Ok(Some(PreparedShellCommand {
                command,
                cwd: updated.cwd,
                env: updated.env.into_iter().collect(),
                launch: Some(launch),
            }))
        }
        Ok(CommandBeforeOutcome::Abort(reason)) => {
            let action = agena_domain::PermissionAction::Tool {
                tool_name: tool_name.to_string(),
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
                "shell.before hook failed (continuing): {err}"
            );
            Ok(Some(PreparedShellCommand {
                command: input.command.clone(),
                cwd,
                env: fallback_env,
                launch: Some(launch),
            }))
        }
    }
}

pub(super) async fn execute_async(
    executor: &ToolExecutor,
    input: &ShellCommandInput,
    context: ToolRuntimeContext,
) -> Result<(ToolPayloadExecution, Option<std::path::PathBuf>), ToolError> {
    if input.command.trim().is_empty() {
        return Err(ToolError::invalid_input(
            "bash command must not be empty".to_string(),
        ));
    }
    let analysis = analyze_command(input.command.as_str());
    let effects = input.filesystem_effects();
    validate_declared_filesystem_effects(executor, "bash", input.command.as_str(), &effects)?;
    let cwd =
        resolve_workdir_in_folder(executor, input.workdir.as_deref(), input.folder.as_deref())?;
    let prepared = match context.prepared_shell_command {
        Some(prepared) => Some(prepared),
        None => match (context.session_id, context.call_id) {
            (Some(session_id), Some(call_id)) => {
                prepare_command_async(
                    executor,
                    input,
                    session_id,
                    call_id,
                    "agena.shell.exec",
                    None,
                )
                .await?
            }
            _ => None,
        },
    };
    let (final_command, final_cwd, env, launch) = match prepared {
        Some(prepared) => (
            prepared.command,
            prepared.cwd,
            prepared.env,
            prepared.launch,
        ),
        None => {
            let mut env = inherited_environment();
            let launch = prepare_posix_launch(shell_settings(), &mut env).await?;
            env.extend(
                executor
                    .shell_env_overrides_async(&cwd, context.session_id, context.call_id)
                    .await?,
            );
            add_agent_plugin_bins(executor, &mut env);
            (input.command.clone(), cwd, env, Some(launch))
        }
    };
    let final_analysis = analyze_command(final_command.as_str());
    let command_rewritten = final_command != input.command;
    let files = executor
        .imported_shell_files(launch.as_ref().is_some_and(|spec| spec.dialect.is_posix()))
        .await;
    let sourced = files.source(&final_command);
    let script = sourced.as_deref().unwrap_or(&final_command);
    let tracked = executor.track_imported_shell(context.call_id);
    let capture = match (&tracked, launch.as_ref()) {
        (Some(_), Some(spec)) if spec.dialect.is_posix() => match DirectoryCapture::new() {
            Ok(capture) => capture.argv(spec, script).map(|argv| (capture, argv)),
            Err(error) => {
                tracing::warn!(%error, "could not prepare shell directory capture");
                None
            }
        },
        _ => None,
    };
    let (capture, command) = match capture {
        Some((capture, argv)) => (Some(capture), argv),
        None => (
            None,
            launch.as_ref().map_or_else(
                || shell_command_for_platform(script),
                |spec| spec.argv(script),
            ),
        ),
    };
    let mut private_files = files.readonly_mounts();
    if let Some(capture) = capture.as_ref() {
        private_files.push(capture.resource());
    }
    let mut request = ShellRequest {
        // The resolved launch owns the program and the login flag; callers that
        // never resolved a user shell keep the historical platform default.
        command,
        cwd: final_cwd,
        env,
        timeout_ms: Some(input.timeout_ms.unwrap_or(DEFAULT_SHELL_TIMEOUT_MS)),
    };
    (request.command, request.env) = crate::shell_sandbox::protect_async_with_files(
        executor,
        request.command,
        &request.cwd,
        input,
        request.env,
        private_files,
    )
    .await?;
    let _worker_permit = super::shell::acquire_worker_permit().await?;
    let execution = executor
        .execute_shell_command_with_live(&request, context.output.clone())
        .await?;
    drop(files);
    executor.ensure_not_cancelled()?;
    let instruction_target = if !executor.is_global()
        && !execution.timed_out
        && execution.exit_code == 0
        && !execution.stdout.is_empty()
        && launch.as_ref().is_none_or(|spec| spec.dialect.is_posix())
    {
        agena_tool::shell_analysis::single_file_read(&final_command)
            .map(|path| request.cwd.join(path))
    } else {
        None
    };
    if !execution.timed_out
        && let Some(shell) = tracked
        && let Some(cwd) = capture.and_then(DirectoryCapture::into_cwd)
    {
        executor
            .record_imported_shell_cwd(&shell, &request.cwd, cwd)
            .await;
    }

    let hook_input = CommandAfterInput {
        session_id: context.session_id,
        command: "sh".to_string(),
        args: vec!["-c".to_string(), final_command.clone()],
        cwd: request.cwd.clone(),
        exit_code: Some(execution.exit_code),
        stdout: execution.stdout.clone(),
        stderr: execution.stderr.clone(),
        timed_out: execution.timed_out,
    };
    let aggregated_for_display = match executor
        .plugin_manager()
        .dispatch_command_after(hook_input)
        .await
    {
        Ok(after) if after.stdout.is_empty() => after.stderr,
        Ok(after) if after.stderr.is_empty() => after.stdout,
        Ok(after) => format!("{}\n{}", after.stdout, after.stderr),
        Err(err) => {
            tracing::warn!(
                target: "agena_plugin_host::command_after",
                "shell.after hook failed (continuing): {err}"
            );
            execution.aggregated_output.clone()
        }
    };
    let output_resource = context
        .output
        .as_ref()
        .map(super::ToolOutputStreams::reference);
    let execution = render_execution(
        input,
        analysis,
        final_analysis,
        command_rewritten,
        final_command,
        &request,
        execution,
        aggregated_for_display,
        launch.as_ref(),
        output_resource,
    )?;
    Ok((execution, instruction_target))
}

fn add_agent_plugin_bins(
    executor: &ToolExecutor,
    env: &mut std::collections::HashMap<String, String>,
) {
    if executor.is_global() || !executor.sandbox_enabled() {
        return;
    }
    let mut directories = executor.plugin_manager().agent_plugin_bin_dirs().peekable();
    if directories.peek().is_none() {
        return;
    }
    let policy = executor.sandbox_permission_snapshot();
    let authorized = directories.filter(|directory| {
        matches!(
            policy.authorize_path_access(agena_domain::AccessKind::Read, directory),
            agena_domain::PermissionDecision::Allow
        )
    });
    append_search_paths(env, authorized);
}

fn append_search_paths<'a>(
    env: &mut std::collections::HashMap<String, String>,
    directories: impl Iterator<Item = &'a std::path::Path>,
) {
    let key = if cfg!(windows) {
        env.keys()
            .find(|key| key.eq_ignore_ascii_case("PATH"))
            .cloned()
    } else {
        None
    }
    .unwrap_or_else(|| "PATH".to_owned());
    let mut paths = env
        .get(&key)
        .map(|value| std::env::split_paths(value).collect::<Vec<_>>())
        .unwrap_or_default();
    let original_len = paths.len();
    for directory in directories {
        if !paths.iter().any(|entry| entry == directory)
            && std::env::join_paths([directory]).is_ok()
        {
            paths.push(directory.to_path_buf());
        }
    }
    if paths.len() != original_len
        && let Ok(value) = std::env::join_paths(paths)
        && let Ok(value) = value.into_string()
    {
        env.insert(key, value);
    }
}

#[cfg(test)]
#[test]
fn plugin_bins_are_added_after_existing_shell_executables_once() {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
    };

    let before = PathBuf::from("host-bin");
    let plugin = Path::new("package/bin");
    let existing = std::env::join_paths([&before])
        .unwrap()
        .into_string()
        .unwrap();
    let mut env = HashMap::from([("PATH".to_owned(), existing)]);
    append_search_paths(&mut env, [plugin, plugin].into_iter());
    assert_eq!(
        std::env::split_paths(&env["PATH"]).collect::<Vec<_>>(),
        [before, plugin.to_path_buf()]
    );
}

#[allow(clippy::too_many_arguments)]
fn render_execution(
    input: &ShellCommandInput,
    analysis: CommandAnalysis,
    final_analysis: CommandAnalysis,
    command_rewritten: bool,
    final_command: String,
    request: &ShellRequest,
    execution: ShellOutput,
    aggregated_for_display: String,
    launch: Option<&agena_tool::shell::ShellLaunchSpec>,
    output_resource: Option<agena_domain::ContentRef>,
) -> Result<ToolPayloadExecution, ToolError> {
    let (mut trimmed_output, truncated) = truncate_shell_output_budget(
        &aggregated_for_display,
        crate::process_output::text_budget(input.max_output_bytes, false),
    );
    if let Some(resource) = &output_resource {
        trimmed_output.push_str(&format!("\n\nOutput resource: {}", resource.resource_id));
    }
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
        output_resource,
        terminal: None,
        dropped_bytes: 0,
        action: "exec".to_string(),
        shell: Some(ProcessShell::Bash),
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
    if let Some(spec) = launch {
        for (key, value) in spec.metadata() {
            view.metadata.insert(key, value);
        }
    }
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
