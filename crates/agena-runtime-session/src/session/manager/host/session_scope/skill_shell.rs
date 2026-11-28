use std::path::Path;

use super::*;
use agena_domain::{ProcessShell, StructuredObject};
use agena_plugin_host::agent_plugin::{AgentPluginFormat, agent_plugin_data_path};
use agena_plugin_host::sdk::{
    ImportedSkillInvokedBy, ImportedSkillRender, ImportedSkillShellDecision,
    ImportedSkillShellHookInput, ImportedSkillShellRequest, ImportedSkillShellResult, PluginKey,
};
use agena_tool::PreparedShellCommand;
use serde_json::json;
use tokio_util::sync::CancellationToken;

enum Snippet {
    Run {
        call: i64,
        invocation: ToolInvocation,
        shell: Option<PreparedShellCommand>,
        command: String,
    },
    Rendered(String),
}

static SKILL_CONFIG_DIRECTORIES: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

struct SkillShellContext<'a> {
    workspace: &'a Path,
    cancel: &'a CancellationToken,
    tools: Option<&'a Arc<tokio::sync::Semaphore>>,
}

impl SessionManager {
    pub async fn run_imported_skill_shell(
        &self,
        session: i64,
        source: PluginKey,
        request: ImportedSkillShellRequest,
    ) -> Result<ImportedSkillShellResult, AppError> {
        let _admission = self.reserve_execution()?;
        let key = session_id(session)?;
        self.require_writable_bot_history(session).await?;
        let state = self.state_for_session(key).await?;
        let context = self.execution_context_with_state(key, &state).await?;
        if context.global || context.owner.is_some() {
            return Err(AppError::Config(
                "Claude skill shell expansion requires a project session".into(),
            ));
        }
        let cancel = sdk::current_cancellation_token()
            .as_ref()
            .unwrap_or(&self.shutdown)
            .child_token();
        let _cancel_on_drop = cancel.clone().drop_guard();
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(cancel.clone()));
        if !executor.plugin_is_enabled(&source)
            || !executor.plugin_manager().plugins().iter().any(|plugin| {
                plugin.key() == source
                    && plugin.agent_plugin().is_some_and(|package| {
                        package.format == AgentPluginFormat::ClaudeCode
                            && request
                                .skill_dir
                                .as_ref()
                                .is_none_or(|path| package.skill_dirs.contains(path))
                    })
            })
        {
            return Err(AppError::Config(
                "The Claude plugin is not enabled for this session".into(),
            ));
        }
        self.expand_imported_skill_shell(
            session,
            source,
            request,
            executor,
            SkillShellContext {
                workspace: state.executor.workspace_root(),
                cancel: &cancel,
                tools: Some(&state.tools),
            },
        )
        .await
    }

    pub async fn render_imported_skill(
        &self,
        session: i64,
        call: i64,
        path: &Path,
        document: &str,
        arguments: &str,
    ) -> Result<Option<ImportedSkillRender>, AppError> {
        if path.file_name() != Some("SKILL.md".as_ref()) {
            return Ok(None);
        }
        let Some(directory) = path.parent() else {
            return Ok(None);
        };
        let (_, control) = self.live_tool(session, call)?;
        let key = session_id(session)?;
        let state = &control.state;
        let context = self
            .execution_context_for_execution(key, state, Some(control.execution))
            .await?;
        if context.global || context.owner.is_some() {
            return Ok(None);
        }
        let cancel = control.cancel.child_token();
        let _cancel_on_drop = cancel.clone().drop_guard();
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(cancel.clone()));
        let input = sdk::CommandInvokeInput {
            command_id: String::new(),
            input: json!({"args": arguments}),
            session_id: Some(session),
            call_id: Some(call),
            workspace_root: Some(executor.workspace_root().to_string_lossy().into_owned()),
            slash: None,
            raw: String::new(),
            prompt_context: String::new(),
            cordis_attachments: None,
        };
        for plugin in executor.plugin_manager().plugins() {
            let source = plugin.key();
            let Some(package) = plugin.agent_plugin() else {
                continue;
            };
            if package.format != AgentPluginFormat::ClaudeCode
                || !package.skill_dirs.iter().any(|path| path == directory)
                || !executor.plugin_is_enabled(&source)
            {
                continue;
            }
            let Some(mut prompt) = package
                .prepare_claude_skill_prompt(&source, directory, document, &input)
                .map_err(|error| AppError::Config(error.diagnostic_message().to_owned()))?
            else {
                return Ok(Some(ImportedSkillRender::Inspection));
            };
            let fork = prompt.take_fork();
            let output = match prompt.take_shell() {
                Some(request) => {
                    self.expand_imported_skill_shell(
                        session,
                        source.clone(),
                        request,
                        executor.clone(),
                        SkillShellContext {
                            workspace: state.executor.workspace_root(),
                            cancel: &cancel,
                            tools: None,
                        },
                    )
                    .await?
                }
                None => Default::default(),
            };
            let text = prompt
                .finish(output)
                .map_err(|error| AppError::Config(error.diagnostic_message().to_owned()))?;
            if let Some(fork) = fork {
                let result = executor
                    .plugin_manager()
                    .invoke_imported_skill_fork(&source, input, fork, text)
                    .await
                    .map_err(|error| AppError::Config(error.diagnostic_message().to_owned()))?;
                let child_result = result
                    .output
                    .as_ref()
                    .and_then(|output| output["result"].as_str())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| result.detail.unwrap_or(result.summary));
                return Ok(Some(ImportedSkillRender::ForkResult(child_result)));
            }
            return Ok(Some(ImportedSkillRender::Prompt(text)));
        }
        Ok(None)
    }

    async fn expand_imported_skill_shell(
        &self,
        session: i64,
        source: PluginKey,
        request: ImportedSkillShellRequest,
        executor: ToolExecutor,
        scope: SkillShellContext<'_>,
    ) -> Result<ImportedSkillShellResult, AppError> {
        let SkillShellContext {
            workspace,
            cancel,
            tools,
        } = scope;
        let directory = agent_plugin_data_path(workspace, &source.to_string());
        let config_dir: Arc<str> = directory
            .to_str()
            .ok_or_else(|| AppError::Config("Claude plugin data path must be UTF-8".into()))?
            .into();
        SKILL_CONFIG_DIRECTORIES
            .run(move || agena_storage_layout::secure_directory(&directory))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .map_err(|error| {
                AppError::Config(format!("Cannot prepare Claude plugin data: {error}"))
            })?;
        let executor = self
            .with_session_scoped_path_authorizer(executor, session, cancel)
            .with_claude_skill_config_directory(config_dir.clone());
        let (shell, bash) =
            ToolExecutor::resolve_imported_skill_shell(request.shell).map_err(tool_error)?;
        let subagent = executor.session_relation_kind().is_subagent();
        let invoked_by = if tools.is_some() {
            ImportedSkillInvokedBy::User
        } else {
            ImportedSkillInvokedBy::Claude
        };
        let allow_deferred = !request.forked || !request.explicit_agent;
        let mut batch = Vec::with_capacity(request.commands.len());
        for command in &request.commands {
            ensure_skill_shell_command(command)?;
            let input = StructuredObject::try_from(json!({"shell": shell, "command": command}))
                .map_err(|error| AppError::Config(error.to_string()))?;
            let invocation = ToolInvocation::new("agena.shell.exec", input);
            if self
                .deferred_skill_shell_permission(session, &executor, &invocation, allow_deferred)
                .await?
            {
                batch.push(Snippet::Rendered(deferred_shell_instruction(
                    shell,
                    command,
                    &config_dir,
                )));
                continue;
            }
            let call = next_session_scoped_call();
            let (effective, prepared) = executor
                .prepare_imported_skill_shell_invocation(&invocation, session, call, bash.as_ref())
                .await
                .map_err(tool_error)?;
            let command = prepared
                .as_ref()
                .map_or(command.as_str(), |shell| &shell.command);
            ensure_skill_shell_command(command)?;
            if self
                .deferred_skill_shell_permission(session, &executor, &effective, allow_deferred)
                .await?
            {
                batch.push(Snippet::Rendered(deferred_shell_instruction(
                    shell,
                    command,
                    &config_dir,
                )));
            } else {
                batch.push(Snippet::Run {
                    call,
                    invocation: effective,
                    command: command.to_owned(),
                    shell: prepared,
                });
            }
        }

        let plugins = executor.plugin_manager();
        // A stop from any hook rejects the invocation before the first launch.
        for snippet in &mut batch {
            let Snippet::Run { call, command, .. } = snippet else {
                continue;
            };
            let decision = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                decision = plugins.dispatch_imported_skill_shell_before(
                    ImportedSkillShellHookInput {
                        session_id: session,
                        tool_use_id: format!("toolu_agena_skill_{session}_{}", call.unsigned_abs()),
                        command_name: request.command_name.clone(),
                        command: command.clone(),
                        shell,
                        invoked_by,
                        source_plugin: source.clone(),
                        forked: request.forked,
                        subagent,
                        skill_dir: request.skill_dir.clone(),
                    },
                    |plugin| executor.plugin_is_enabled(plugin),
                ) => decision,
            };
            match decision {
                Some(ImportedSkillShellDecision::Skip(reason)) => {
                    *snippet = Snippet::Rendered(format!("[not run: blocked by hook: {reason}]"));
                }
                Some(ImportedSkillShellDecision::Stop(reason)) => {
                    return Err(AppError::Config(format!(
                        "Operation stopped by hook: {reason}"
                    )));
                }
                None => {}
            }
        }

        let _permit = match tools {
            Some(tools) => Some(tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                permit = tools.acquire() => permit.map_err(|_| AppError::Cancelled)?,
            }),
            None => None,
        };
        let mut outputs = Vec::with_capacity(batch.len());
        for snippet in batch {
            let Snippet::Run {
                call,
                invocation,
                shell: prepared,
                command,
            } = snippet
            else {
                if let Snippet::Rendered(output) = snippet {
                    outputs.push(output);
                }
                continue;
            };
            if self
                .deferred_skill_shell_permission(session, &executor, &invocation, allow_deferred)
                .await?
            {
                outputs.push(deferred_shell_instruction(shell, &command, &config_dir));
                continue;
            }
            let execution_executor = self
                .sandbox_executor_for_session(&executor, session)
                .await
                .map_err(tool_error)?
                .without_tool_after_hooks();
            let execution = Box::pin(execution_executor.execute_invocation(
                &invocation,
                crate::tool::ToolRuntimeContext {
                    session_id: Some(session),
                    call_id: Some(call),
                    prepared_shell_command: prepared,
                    ..Default::default()
                },
            ))
            .await
            .map_err(|error| match error {
                ToolError::Cancelled => AppError::Cancelled,
                error => AppError::Config(format!(
                    "Shell command failed for pattern {command:?}: {error}"
                )),
            })?;
            outputs.push(skill_shell_output(execution, shell, &command)?);
        }
        Ok(ImportedSkillShellResult { outputs })
    }

    async fn deferred_skill_shell_permission(
        &self,
        session: i64,
        executor: &ToolExecutor,
        invocation: &ToolInvocation,
        allow_deferred: bool,
    ) -> Result<bool, AppError> {
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(invocation, Some(session))
            .await
            .map_err(tool_error)?;
        let mut deferred = false;
        for check in checks {
            match self
                .query_host_permission(Some(session), check.clone())
                .await?
            {
                PermissionDecision::Allow => {}
                PermissionDecision::Auto { .. } if allow_deferred => deferred = true,
                _ => {
                    self.authorize_session_scoped_checks(session, executor, vec![check])
                        .await?
                }
            }
        }
        Ok(deferred)
    }
}

fn ensure_skill_shell_command(command: &str) -> Result<(), AppError> {
    if command.trim().is_empty() || command.contains('\0') {
        return Err(AppError::Config(format!(
            "Invalid injected shell command {command:?}"
        )));
    }
    Ok(())
}

fn deferred_shell_instruction(shell: ProcessShell, command: &str, config_dir: &str) -> String {
    let setup = match shell {
        ProcessShell::Bash => format!(
            "export CLAUDE_CONFIG_DIR='{}'",
            config_dir.replace('\'', "'\\''")
        ),
        ProcessShell::Powershell => format!(
            "$env:CLAUDE_CONFIG_DIR = '{}'",
            config_dir.replace('\'', "''")
        ),
    };
    format!(
        "[Run this command first with `agena.shell.exec` using shell `{shell}`, then use its output here:\n{setup}\n{command}]"
    )
}

fn skill_shell_output(
    execution: ToolInvocationExecution,
    shell: ProcessShell,
    command: &str,
) -> Result<String, AppError> {
    let payload = Value::from(execution.output.payload);
    let output = payload["output"]
        .as_str()
        .unwrap_or(&execution.view.output_text);
    let status = payload["status"].as_str();
    let code = payload["exit_code"].as_i64();
    if status != Some("exited")
        || !(code == Some(0) || code == Some(1) && benign_status_one(shell, command))
    {
        return Err(AppError::Config(format!(
            "Shell command failed for pattern {command:?}\n{output}"
        )));
    }
    // The shell tool presents a generated description when there are no bytes to insert.
    if payload["description"].as_str() == Some(output) {
        return Ok(String::new());
    }
    Ok(output.trim_end().to_owned())
}

fn benign_status_one(shell: ProcessShell, command: &str) -> bool {
    let analysis = agena_tool::shell_analysis::analyze_command(command);
    analysis.single_command
        && match (
            analysis.primary_command.as_deref(),
            analysis.subcommand.as_deref(),
            shell,
        ) {
            (Some("grep" | "rg" | "egrep" | "fgrep"), _, _)
            | (Some("git"), Some("grep" | "diff"), _) => true,
            (Some("find" | "diff" | "test" | "["), _, ProcessShell::Bash) => true,
            (Some("findstr"), _, ProcessShell::Powershell) => true,
            _ => false,
        }
}
