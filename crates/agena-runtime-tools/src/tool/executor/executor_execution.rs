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
        let payload = payload.map_err(|error| ToolError::invalid_input(error.to_string()))?;
        let ToolPayloadInput::Shell(crate::part::ShellToolInput::Run {
            shell: agena_domain::ProcessShell::Bash,
            command: process_input,
            run_in_background,
            monitor,
        }) = payload
        else {
            return Ok((invocation.clone(), None));
        };
        let prepared_shell =
            bash::prepare_command_async(self, process_input.as_ref(), session_id, call_id).await?;
        let Some(prepared_shell) = prepared_shell.clone() else {
            return Ok((invocation.clone(), None));
        };
        if prepared_shell.command == process_input.command {
            return Ok((invocation.clone(), Some(prepared_shell)));
        }
        let mut rewritten = *process_input;
        rewritten.command = prepared_shell.command.clone();
        let rewritten_invocation = ToolPayloadInput::Shell(crate::part::ShellToolInput::Run {
            shell: agena_domain::ProcessShell::Bash,
            command: Box::new(rewritten),
            run_in_background,
            monitor,
        })
        .into_invocation();
        let input_value = serde_json::Value::from(rewritten_invocation.input);
        let input = StructuredObject::try_from(input_value)
            .map_err(|err| ToolError::invalid_input(err.to_string()))?;
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
        let input_value: serde_json::Value = serde_json::from_str(&input_json)
            .map_err(|e| ToolError::invalid_input(e.to_string()))?;
        let effective_tags = definition
            .as_ref()
            .map(|definition| invocation_effective_tags(definition, invocation))
            .unwrap_or_default();
        let contract = definition
            .as_ref()
            .map(|definition| definition.definition.permissions.clone())
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
                    contract: contract.clone(),
                    input: input_value,
                    title_override: None,
                    metadata: Default::default(),
                },
                self.cancellation_token.clone(),
            )
            .await
            .map_err(|err| self.plugin_error_or_cancelled(err))?;

        let input_json = serde_json::to_string(&hooked.input)
            .map_err(|e| ToolError::invalid_input(e.to_string()))?;
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

    /// Async permission preflight, including plugin-provided dynamic path and
    /// network requests. It never blocks a Tokio worker waiting on transport
    /// I/O.
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
        let contract = self
            .invocation_definition(invocation)
            .map(|definition| definition.definition.permissions.clone())
            .unwrap_or_default();
        let action = crate::permission::tool_action(
            tool_name.as_str(),
            command.as_deref(),
            &contract,
            Some(&self.principal.tool_policy),
        );
        let mut checks = vec![ToolPermissionCheck {
            action,
            decision,
            contract: contract.clone(),
        }];

        if let Some(inspector) = self.permission_inspector.as_ref() {
            checks.extend(inspector.additional_checks(invocation, &self.principal)?);
        }

        if let Some(resolution) = self.plugin_resolution_for_invocation(invocation) {
            let input_value = resolved_tool_input_value(&resolution, invocation);
            if resolution.definition.permissions.shell {
                self.collect_declared_filesystem_effect_checks(
                    &mut checks,
                    tool_name.as_str(),
                    &input_value,
                )?;
            }
            self.collect_declared_path_checks(
                &mut checks,
                &input_value,
                &resolution.definition.permissions.input_paths,
                &resolution.definition.permissions.path_access,
            )?;
            self.collect_dynamic_path_checks_async(&mut checks, &resolution, &input_value)
                .await?;
            self.collect_declared_network_checks(
                &mut checks,
                &input_value,
                &resolution.definition.permissions.input_networks,
                &resolution.definition.permissions.network_access,
            )?;
            self.collect_dynamic_network_checks_async(&mut checks, &resolution, &input_value)
                .await?;
        }
        Ok(checks)
    }

    /// Execute a streaming tool. The executor is deliberately policy-free:
    /// model-call authorization is owned by the session permission state
    /// machine, while application and host invocations execute directly.
    pub async fn execute_invocation_streaming(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<Option<StreamingToolExecution>, ToolError> {
        self.ensure_not_cancelled()?;
        if !matches!(
            self.invocation_streaming_mode(invocation),
            Some(SdkToolStreamingMode::Streaming)
        ) {
            return Ok(None);
        }
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);

        let resolution = self
            .plugin_resolution_for_invocation(invocation)
            .ok_or_else(|| self.unknown_tool_error(plugin_invocation.tool_name.as_str()))?;
        let invoke_stream = self.plugins.invoke_tool_stream(
            &resolution,
            PluginToolInvokeInput {
                tool_name: resolution.tool_name().to_string(),
                session_id,
                call_id,
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: resolved_plugin_invocation_input_value(&resolution, &plugin_invocation),
            },
        );
        let stream = match self.cancellation_token() {
            Some(cancellation) => tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(ToolError::Cancelled),
                result = invoke_stream => result,
            },
            None => invoke_stream.await,
        }
        .map_err(|err| {
            if self
                .cancellation_token()
                .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
            {
                ToolError::Cancelled
            } else {
                ToolError::from_plugin_error(err)
            }
        })?;
        let stream_id = stream.stream_id;
        let chunks = stream.chunks;
        let end = stream.end;
        let result_policy = resolution.definition.runtime.result_policy.clone();
        let model_tool_name = resolution.canonical_name();
        let executor = self.clone();
        let cancellation = self.cancellation_token.clone();
        let invocation = invocation.clone();
        let (mut end_tx, end_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let lifecycle = async {
                match end.await {
                    Ok(Ok(end)) => {
                        let view = ToolExecutionView {
                            title: end.title,
                            summary: end.summary,
                            output_text: end.output_text,
                            sections: end.sections,
                            metadata: end.metadata.into_iter().collect(),
                            attachments: end.attachments,
                        };
                        let output = ToolOutput::from_json_payload(end.payload.as_ref())
                            .map_err(ToolError::invalid_input)?;
                        let execution = ToolInvocationExecution {
                            output: output.clone(),
                            view,
                            apply_patch: apply_patch_execution_from_tool_output(&output),
                        };
                        executor
                            .finalize_execution_async(
                                &invocation,
                                session_id,
                                model_tool_name.as_str(),
                                &result_policy,
                                call_id,
                                execution,
                            )
                            .await
                    }
                    Ok(Err(err)) => Err(ToolError::from_plugin_error(err)),
                    Err(_) => Err(ToolError::plugin(
                        "stream ended without a terminal frame".to_string(),
                    )),
                }
            };
            tokio::pin!(lifecycle);
            let result = match cancellation.as_ref() {
                Some(cancellation) => tokio::select! {
                    biased;
                    _ = end_tx.closed() => return,
                    _ = cancellation.cancelled() => Err(ToolError::Cancelled),
                    result = &mut lifecycle => result,
                },
                None => tokio::select! {
                    biased;
                    _ = end_tx.closed() => return,
                    result = &mut lifecycle => result,
                },
            };
            let _ = end_tx.send(result);
        });
        Ok(Some(StreamingToolExecution {
            stream_id,
            chunks,
            end: end_rx,
        }))
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
        self.ensure_not_cancelled()?;
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);
        let tool_name = plugin_invocation_name(&plugin_invocation);
        tracing::debug!(session_id, call_id, tool = tool_name.as_str(), "tool.call");
        let resolution = self
            .plugin_resolution_for_invocation(invocation)
            .ok_or_else(|| self.unknown_tool_error(plugin_invocation.tool_name.as_str()))?;

        if let Some(payload) =
            ToolPayloadInput::from_executor_backed_invocation(&resolution, invocation)
        {
            let payload = payload.map_err(|error| ToolError::invalid_input(error.to_string()))?;
            let context = crate::tool::ToolRuntimeContext {
                session_id: (session_id >= 0).then_some(session_id),
                call_id: (call_id >= 0).then_some(call_id),
                prepared_shell_command,
                launch_provenance,
            };
            let execution = match payload {
                ToolPayloadInput::Shell(input) => {
                    crate::tool::process_tool::execute_async(self, &input, context).await?
                }
                ToolPayloadInput::Monitor(input) => {
                    crate::tool::monitor_tool::execute_async(self, &input, context).await?
                }
                ToolPayloadInput::CronCreate(input) => {
                    crate::tool::cron::execute_create_async(self, &input, &context).await?
                }
                ToolPayloadInput::CronList(input) => {
                    crate::tool::cron::execute_list_async(self, &input).await?
                }
                ToolPayloadInput::CronDelete(input) => {
                    crate::tool::cron::execute_delete_async(self, &input).await?
                }
                ToolPayloadInput::CronUpdate(input) => {
                    crate::tool::cron::execute_update_async(self, &input).await?
                }
                ToolPayloadInput::CronPause(input) => {
                    crate::tool::cron::execute_pause_async(self, &input).await?
                }
                ToolPayloadInput::CronResume(input) => {
                    crate::tool::cron::execute_resume_async(self, &input).await?
                }
                ToolPayloadInput::CronHistory(input) => {
                    crate::tool::cron::execute_history_async(self, &input).await?
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
                        .map_err(|error| ToolError::invalid_input(error.to_string()))?;
                    if let Some(input) = input.as_object_mut() {
                        input.remove("tool");
                    }
                    let executor = self.clone();
                    let worker_permit = BUILTIN_BLOCKING_WORKERS.acquire().await.map_err(|_| {
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
            self.ensure_not_cancelled()?;
            return self
                .finalize_execution_async(
                    invocation,
                    session_id,
                    resolution.canonical_name().as_str(),
                    &resolution.definition.runtime.result_policy,
                    call_id,
                    execution.into(),
                )
                .await;
        }

        let response = self
            .plugins
            .invoke_tool(
                &resolution,
                PluginToolInvokeInput {
                    tool_name: resolution.tool_name().to_string(),
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    input: resolved_plugin_invocation_input_value(&resolution, &plugin_invocation),
                },
                self.cancellation_token.clone(),
            )
            .await
            .map_err(|err| self.plugin_error_or_cancelled(err))?;
        self.ensure_not_cancelled()?;

        let view = ToolExecutionView {
            title: response.title.clone(),
            summary: response.summary.clone(),
            output_text: response.output_text.clone(),
            sections: response.sections.clone(),
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
        self.finalize_execution_async(
            invocation,
            session_id,
            resolution.canonical_name().as_str(),
            &resolution.definition.runtime.result_policy,
            call_id,
            execution,
        )
        .await
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
    SdkToolStreamingMode, StreamingToolExecution, ToolError, ToolExecutionView, ToolExecutor,
    ToolInvocation, ToolInvocationExecution, ToolOutput, ToolPayloadInput, ToolPermissionCheck,
    apply_patch_execution_from_tool_output, bash, invocation_effective_tags, invocation_input_json,
    invocation_name, parse_invocation_from_json, plugin_invocation_name,
    resolved_plugin_invocation_input_value, resolved_tool_input_value,
    shell_command_from_invocation,
};
