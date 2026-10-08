static BUILTIN_BLOCKING_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);

impl ToolExecutor {
    /// Native async shell preparation, including environment and command
    /// policy hooks.
    pub async fn prepare_shell_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<(ToolInvocation, Option<PreparedShellCommand>), ToolError> {
        let Some(resolution) = self.plugin_resolution_for_invocation(invocation) else {
            return Ok((invocation.clone(), None));
        };
        let Some(payload) =
            ToolPayloadInput::from_executor_backed_invocation(&resolution, invocation)
        else {
            return Ok((invocation.clone(), None));
        };
        let payload = payload.map_err(|error| ToolError::invalid_input_error(&error))?;
        let process_input = match payload {
            ToolPayloadInput::Shell(input) => input
                .launch_command()
                .filter(|(shell, _)| *shell == agena_domain::ProcessShell::Bash)
                .map(|(_, command)| command.clone()),
            _ => None,
        };
        let Some(process_input) = process_input else {
            return Ok((invocation.clone(), None));
        };
        let prepared_shell = bash::prepare_command_async(
            self,
            &process_input,
            session_id,
            call_id,
            &resolution.canonical_name(),
        )
        .await?;
        let Some(prepared_shell) = prepared_shell.clone() else {
            return Ok((invocation.clone(), None));
        };
        if prepared_shell.command == process_input.command
            && prepared_shell.cwd
                == crate::tool::shell_tools::resolve_workdir(
                    self,
                    process_input.workdir.as_deref(),
                )?
        {
            return Ok((invocation.clone(), Some(prepared_shell)));
        }
        // Rewrite only the command, preserving the launch tool's exact schema
        // and all terminal options. Preparation and permission hooks run once.
        let mut input_value = serde_json::Value::from(invocation.input.clone());
        input_value["command"] = serde_json::Value::String(prepared_shell.command.clone());
        input_value["workdir"] =
            serde_json::Value::String(prepared_shell.cwd.to_string_lossy().into_owned());
        let input = StructuredObject::try_from(input_value).map_err(ToolError::invalid_input)?;
        Ok((
            ToolInvocation {
                tool_api_call: invocation.tool_api_call.clone(),
                name: invocation.name.clone(),
                plugin_name: invocation.plugin_name.clone(),
                input,
            },
            Some(prepared_shell),
        ))
    }

    /// Prepare an invocation while awaiting all plugin hooks on the owning
    /// Tokio runtime.
    pub async fn prepare_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<PreparedToolInvocation, ToolError> {
        self.ensure_not_cancelled()?;
        if invocation
            .tool_api_call
            .as_ref()
            .is_some_and(|call| call.function == ToolApiFunction::Call)
            && invocation.name == ToolApiFunction::Call.function_name()
        {
            let diagnostic = invocation
                .tool_api_call
                .as_ref()
                .and_then(|call| call.arguments.get(TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD))
                .and_then(StructuredValue::as_text);
            return Err(ToolError::invalid_field(
                "tool",
                agena_failure::FieldIssueKind::Required,
                diagnostic.unwrap_or(
                    "tools_call requires a string `tool` field naming an execution tool",
                ),
            ));
        }
        let model_tool_name = invocation_name(invocation).to_owned();
        let definition = self.invocation_definition(invocation);
        if self
            .conversation
            .as_ref()
            .is_some_and(|context| context.is_read_only())
        {
            // Reject the capability before tool-before hooks or shell
            // preparation can observe a mutation request.
            self.authorize_invocation(invocation)?;
        }
        let plugin_name = self.invocation_plugin_name_for(invocation);
        if definition.is_none() {
            let mut prepared_invocation = invocation.clone();
            prepared_invocation.plugin_name = Some(plugin_name);
            return Ok(PreparedToolInvocation {
                invocation: prepared_invocation,
                title_override: None,
                metadata: Default::default(),
            });
        }
        if let Some(definition) = definition.as_ref() {
            self.check_cloud_tool_adapter(&definition.canonical_name())?;
        }
        let hook_tool_name = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.tool_name().to_string())
            .unwrap_or_else(|| model_tool_name.clone());
        let hook_tool = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.tool_key().clone())
            .or_else(|| hook_tool_name.parse().ok())
            .ok_or_else(|| self.unknown_tool_error(hook_tool_name.as_str()))?;
        let input_json = invocation_input_json(invocation)?;
        let input_value: serde_json::Value =
            serde_json::from_str(&input_json).map_err(|e| ToolError::invalid_input_error(&e))?;
        let effective_tags = definition
            .as_ref()
            .map(|definition| invocation_effective_tags(definition, invocation))
            .unwrap_or_default();
        let hooked = self
            .plugins
            .dispatch_tool_before(
                PluginToolBeforeInput {
                    tool: hook_tool,
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    tags: effective_tags,
                    input: input_value,
                    title_override: None,
                    metadata: Default::default(),
                },
                self.cancellation_token.clone(),
            )
            .await
            .map_err(|err| self.plugin_error_or_cancelled(err))?;

        let input_json =
            serde_json::to_string(&hooked.input).map_err(|e| ToolError::invalid_input_error(&e))?;
        let mut prepared_invocation =
            parse_invocation_from_json(model_tool_name.as_str(), input_json.as_str())?;
        prepared_invocation.tool_api_call = invocation.tool_api_call.clone();
        let is_protocol_handler = invocation
            .tool_api_call
            .as_ref()
            .is_some_and(|call| call.function != agena_domain::ToolApiFunction::Call);
        prepared_invocation.plugin_name = (!is_protocol_handler).then_some(plugin_name);

        Ok(PreparedToolInvocation {
            invocation: prepared_invocation,
            title_override: hooked.title_override,
            metadata: hooked.metadata.into_iter().collect(),
        })
    }

    /// Permission preflight for one invocation.
    ///
    /// The host's own tools are authorized here, including their path and
    /// network arguments. A plugin that performs its own I/O is responsible for
    /// asking the host before it touches anything (see
    /// `HostClient::check_path_permission`); the host no longer extracts path
    /// or network effects from plugin declarations, because there are none.
    pub async fn collect_permission_checks_for_invocation_in_session(
        &self,
        invocation: &ToolInvocation,
        _session_id: Option<i64>,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.ensure_not_cancelled()?;
        if self
            .invocation_definition(invocation)
            .as_ref()
            .is_some_and(crate::tool::is_tool_api_handler)
        {
            return Ok(Vec::new());
        }
        let (tool_name, decision) = self.authorize_invocation(invocation)?;
        let command = shell_command_from_invocation(invocation);
        let tags = self
            .invocation_definition(invocation)
            .map(|definition| definition.effective_tags())
            .unwrap_or_default();
        let action = crate::permission::tool_action(
            tool_name.as_str(),
            command.as_deref(),
            &tags,
            Some(&self.principal.tool_policy),
        );
        let mut checks = vec![ToolPermissionCheck { action, decision }];

        if let Some(inspector) = self.permission_inspector.as_ref() {
            checks.extend(inspector.additional_checks(invocation, &self.principal)?);
        }

        if let Some(resolution) = self.plugin_resolution_for_invocation(invocation) {
            self.collect_builtin_effect_checks(&mut checks, &resolution, invocation)?;
        }

        Ok(checks)
    }

    /// Describe the source before starting execution, so its stable reference
    /// can be committed on the Part before a process produces its first byte.
    pub fn streaming_content_kind(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<Option<agena_domain::ContentKind>, ToolError> {
        let resolution = self
            .plugin_resolution_for_invocation(invocation)
            .ok_or_else(|| self.unknown_tool_error(invocation.name.as_str()))?;
        if let Some(payload) =
            ToolPayloadInput::from_executor_backed_invocation(&resolution, invocation)
        {
            let payload = payload.map_err(|error| ToolError::invalid_input_error(&error))?;
            return Ok(match payload {
                ToolPayloadInput::Shell(crate::part::ShellToolInput::Open { .. }) => {
                    Some(agena_domain::ContentKind::Terminal)
                }
                ToolPayloadInput::Shell(
                    crate::part::ShellToolInput::Exec { .. }
                    | crate::part::ShellToolInput::Spawn { .. },
                ) => Some(agena_domain::ContentKind::Log),
                ToolPayloadInput::Shell(crate::part::ShellToolInput::Watch { input })
                    if matches!(input.as_ref(), crate::part::ShellWatchInput::Launch { .. }) =>
                {
                    Some(agena_domain::ContentKind::Log)
                }
                _ => None,
            });
        }
        Ok(matches!(
            self.invocation_streaming_mode(invocation),
            Some(SdkToolStreamingMode::Streaming)
        )
        .then_some(agena_domain::ContentKind::Document))
    }

    pub async fn execute_invocation_detailed_with_prepared_shell(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation_detailed_with_launch_provenance(
            invocation,
            session_id,
            call_id,
            prepared_shell_command,
            None,
        )
        .await
    }

    /// Execute a tool with the exact assistant receipt that owns any durable
    /// child work it creates. Only model-originated tool paths provide this;
    /// host/application calls deliberately remain provenance-free.
    pub async fn execute_invocation_detailed_with_launch_provenance(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<PreparedShellCommand>,
        launch_provenance: Option<agena_scheduler::ScheduledJobLaunchProvenance>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation(
            invocation,
            crate::tool::ToolRuntimeContext {
                session_id: (session_id > 0).then_some(session_id),
                call_id: (call_id >= 0).then_some(call_id),
                prepared_shell_command,
                launch_provenance,
                output: None,
            },
        )
        .await
    }

    /// One execution lifecycle. A tool may emit zero or many intermediate
    /// events; both cases return one final outcome through this future.
    pub async fn execute_invocation(
        &self,
        invocation: &ToolInvocation,
        context: crate::tool::ToolRuntimeContext,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let writer = context.output.clone();
        let mut result = self.execute_invocation_inner(invocation, context).await;
        if let Some(writer) = writer.filter(|writer| !writer.lifecycle_is_delegated()) {
            let final_state = if matches!(result, Err(ToolError::Cancelled)) {
                agena_domain::ContentState::Interrupted
            } else {
                agena_domain::ContentState::Complete
            };
            let capture_error = match writer.finalize(final_state).await {
                Ok(resource) => resource.capture_error,
                Err(error) => {
                    tracing::error!(%error, "tool content capture did not commit its final descriptor");
                    writer
                        .resource()
                        .capture_error
                        .or_else(|| Some(error.to_string()))
                }
            };
            if let (Some(error), Ok(execution)) = (capture_error, &mut result) {
                execution
                    .view
                    .metadata
                    .insert("output_capture_error".into(), error);
                execution
                    .view
                    .metadata
                    .insert("output_capture_state".into(), "interrupted".into());
            }
        }
        result
    }

    async fn execute_invocation_inner(
        &self,
        invocation: &ToolInvocation,
        context: crate::tool::ToolRuntimeContext,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let session_id = context.session_id.unwrap_or(-1);
        let call_id = context.call_id.unwrap_or(-1);
        let output = context.output.clone();
        self.ensure_not_cancelled()?;
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);
        let tool_name = plugin_invocation_name(&plugin_invocation);
        tracing::debug!(session_id, call_id, tool = tool_name.as_str(), "tool.call");
        let resolution = self
            .plugin_resolution_for_invocation(invocation)
            .ok_or_else(|| self.unknown_tool_error(plugin_invocation.tool_name.as_str()))?;

        self.check_cloud_tool_adapter(&resolution.canonical_name())?;

        if resolution.namespace() == "agena"
            && resolution.plugin_name() == "fs"
            && resolution.tool_name() == "read_media"
        {
            let input = crate::part::ReadMediaInput::parse_input(
                resolved_plugin_invocation_input_value(&resolution, &plugin_invocation),
            )
            .map_err(|error| self.plugin_error_or_cancelled(error))?;
            let execution = crate::tool::read_media::execute(self, input).await?;
            return self
                .finalize_execution_async(
                    invocation,
                    session_id,
                    resolution.canonical_name().as_str(),
                    call_id,
                    execution.into(),
                )
                .await;
        }

        if resolution.namespace() == "agena"
            && resolution.plugin_name() == "content"
            && resolution.tool_name() == "read"
        {
            let execution = crate::tool::content::execute(self, invocation, session_id).await?;
            self.ensure_not_cancelled()?;
            return self
                .finalize_execution_async(
                    invocation,
                    session_id,
                    resolution.canonical_name().as_str(),
                    call_id,
                    execution,
                )
                .await;
        }

        if let Some(payload) =
            ToolPayloadInput::from_executor_backed_invocation(&resolution, invocation)
        {
            let payload = payload.map_err(|error| ToolError::invalid_input_error(&error))?;
            // Each built-in branch can carry a large async state machine. Keep
            // that state on the heap so an ordinary plugin call does not poll
            // through the combined stack frame of every built-in tool.
            let execution = Box::pin(async move {
                let execution = match payload {
                    ToolPayloadInput::Shell(input) => {
                        crate::tool::shell_tool::execute_async(self, &input, context).await?
                    }
                    ToolPayloadInput::Monitor(input) => {
                        crate::tool::monitor_tool::execute_async(self, &input, context).await?
                    }
                    ToolPayloadInput::CronCreate(input) => {
                        crate::tool::cron::execute_create_async(self, &input, &context).await?
                    }
                    ToolPayloadInput::CronList(input) => {
                        crate::tool::cron::execute_list_async(self, &input, &context).await?
                    }
                    ToolPayloadInput::CronDelete(input) => {
                        crate::tool::cron::execute_delete_async(self, &input, &context).await?
                    }
                    ToolPayloadInput::CronUpdate(input) => {
                        crate::tool::cron::execute_update_async(self, &input, &context).await?
                    }
                    ToolPayloadInput::CronPause(input) => {
                        crate::tool::cron::execute_pause_async(self, &input, &context).await?
                    }
                    ToolPayloadInput::CronResume(input) => {
                        crate::tool::cron::execute_resume_async(self, &input, &context).await?
                    }
                    ToolPayloadInput::CronHistory(input) => {
                        crate::tool::cron::execute_history_async(self, &input, &context).await?
                    }
                    ToolPayloadInput::LspDefinition(input) => {
                        crate::tool::lsp::execute_definition_async(self, &input).await?
                    }
                    ToolPayloadInput::LspReferences(input) => {
                        crate::tool::lsp::execute_references_async(self, &input).await?
                    }
                    ToolPayloadInput::LspHover(input) => {
                        crate::tool::lsp::execute_hover_async(self, &input).await?
                    }
                    ToolPayloadInput::LspDiagnostics(input) => {
                        crate::tool::lsp::execute_diagnostics_async(self, &input).await?
                    }
                    payload => {
                        let payload_name = payload.tool_name();
                        let mut input = serde_json::to_value(payload)
                            .map_err(|error| ToolError::invalid_input_error(&error))?;
                        if let Some(input) = input.as_object_mut() {
                            input.remove("tool");
                        }
                        let executor = self.clone();
                        let worker_permit =
                            BUILTIN_BLOCKING_WORKERS.acquire().await.map_err(|_| {
                                ToolError::plugin("builtin worker pool is unavailable".to_string())
                            })?;
                        tokio::task::spawn_blocking(move || {
                            let _worker_permit = worker_permit;
                            crate::tool::orchestrator::execute_tool(
                                &executor,
                                payload_name,
                                input,
                                context,
                            )
                        })
                        .await
                        .map_err(|error| {
                            ToolError::plugin(format!(
                                "builtin tool worker failed before completion: {error}"
                            ))
                        })??
                    }
                };
                Ok::<_, ToolError>(execution)
            })
            .await?;
            self.ensure_not_cancelled()?;
            return self
                .finalize_execution_async(
                    invocation,
                    session_id,
                    resolution.canonical_name().as_str(),
                    call_id,
                    execution.into(),
                )
                .await;
        }

        if matches!(
            self.invocation_streaming_mode(invocation),
            Some(SdkToolStreamingMode::Streaming)
        ) {
            let invoke = self.plugins.invoke_tool_stream(
                &resolution,
                PluginToolInvokeInput {
                    tool_name: resolution.tool_name().to_string(),
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    input: resolved_plugin_invocation_input_value(&resolution, &plugin_invocation),
                },
            );
            let mut stream = match self.cancellation_token.as_ref() {
                Some(token) => tokio::select! {
                    biased;
                    _ = token.cancelled() => return Err(ToolError::Cancelled),
                    result = invoke => result,
                },
                None => invoke.await,
            }
            .map_err(|error| self.plugin_error_or_cancelled(error))?;
            let mut open = true;
            let end = loop {
                tokio::select! {
                    biased;
                    _ = async {
                        match self.cancellation_token.as_ref() {
                            Some(token) => token.cancelled().await,
                            None => std::future::pending::<()>().await,
                        }
                    } => return Err(ToolError::Cancelled),
                    end = &mut stream.end => break end,
                    chunk = stream.chunks.recv(), if open => {
                        match chunk {
                            Some(chunk) => Self::capture_tool_chunk(output.as_ref(), chunk).await?,
                            None => open = false,
                        }
                    },
                }
            };
            // The end channel can win while earlier records are queued. The
            // host serializes delivery before end, so drain before sealing.
            stream.chunks.close();
            while let Ok(chunk) = stream.chunks.try_recv() {
                Self::capture_tool_chunk(output.as_ref(), chunk).await?;
            }
            let end = end
                .map_err(|error| {
                    ToolError::plugin(format!("tool stream lost its terminal result: {error}"))
                })?
                .map_err(|error| self.plugin_error_or_cancelled(error))?;
            let output = ToolOutput::from_json_payload(end.payload.as_ref())
                .map_err(ToolError::invalid_input)?;
            return self
                .finalize_execution_async(
                    invocation,
                    session_id,
                    resolution.canonical_name().as_str(),
                    call_id,
                    ToolInvocationExecution {
                        apply_patch: apply_patch_execution_from_tool_output(&output),
                        output,
                        view: ToolExecutionView {
                            title: end.title,
                            summary: end.summary,
                            output_text: end.output_text,
                            metadata: end.metadata.into_iter().collect(),
                            attachments: end.attachments,
                        },
                    },
                )
                .await;
        }

        let response = Box::pin(self.plugins.invoke_tool(
            &resolution,
            PluginToolInvokeInput {
                tool_name: resolution.tool_name().to_string(),
                session_id,
                call_id,
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: resolved_plugin_invocation_input_value(&resolution, &plugin_invocation),
            },
            self.cancellation_token.clone(),
        ))
        .await
        .map_err(|err| self.plugin_error_or_cancelled(err))?;
        self.ensure_not_cancelled()?;

        let view = ToolExecutionView {
            title: response.title.clone(),
            summary: response.summary.clone(),
            output_text: response.output_text.clone(),
            metadata: response.metadata.into_iter().collect(),
            attachments: response.attachments,
        };
        let output = ToolOutput::from_json_payload(response.payload.as_ref())
            .map_err(ToolError::invalid_input)?;
        let execution = ToolInvocationExecution {
            output: output.clone(),
            view,
            apply_patch: apply_patch_execution_from_tool_output(&output),
        };
        Box::pin(self.finalize_execution_async(
            invocation,
            session_id,
            resolution.canonical_name().as_str(),
            call_id,
            execution,
        ))
        .await
    }

    async fn capture_tool_chunk(
        output: Option<&agena_storage::content::ContentWriter>,
        chunk: agena_plugin_host::sdk::ToolStreamChunk,
    ) -> Result<(), ToolError> {
        if let Some(output) = output {
            let bytes = chunk.payload.byte_len();
            if let Err(error) = output.append(chunk.payload).await {
                output.record_loss(bytes, &error);
            }
        }
        Ok(())
    }

    pub async fn execute_invocation_detailed(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation_detailed_with_prepared_shell(invocation, session_id, call_id, None)
            .await
    }
}
use agena_domain::{
    PluginInvocation, StructuredObject, StructuredValue, TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD,
    ToolApiFunction,
};

use super::{
    PluginToolBeforeInput, PluginToolInvokeInput, PreparedShellCommand, PreparedToolInvocation,
    SdkToolStreamingMode, ToolError, ToolExecutionView, ToolExecutor, ToolInvocation,
    ToolInvocationExecution, ToolOutput, ToolPayloadInput, ToolPermissionCheck,
    apply_patch_execution_from_tool_output, bash, invocation_effective_tags, invocation_input_json,
    invocation_name, parse_invocation_from_json, plugin_invocation_name,
    resolved_plugin_invocation_input_value, shell_command_from_invocation,
};
