impl ToolExecutor {
    pub fn execute_tool_payload_detailed(
        &self,
        input: &ToolPayloadInput,
    ) -> Result<ToolPayloadExecution, ToolError> {
        self.execute_tool_payload_detailed_with_context(input, ToolRuntimeContext::default())
    }

    pub fn execute_tool_payload_output_for_session(
        &self,
        input: &ToolPayloadInput,
        session_id: i64,
    ) -> Result<ToolPayloadOutput, ToolError> {
        self.execute_tool_payload_detailed_with_context(
            input,
            ToolRuntimeContext {
                session_id: Some(session_id),
                call_id: None,
                session_context: None,
                prepared_shell_command: None,
            },
        )
        .map(|execution| execution.output)
    }

    pub fn execute_tool_payload_for_host(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
        call_id: Option<i64>,
        session_context: Option<&crate::session::SessionExecutionContext>,
    ) -> Result<crate::plugin::ToolInvokeOutput, ToolError> {
        let scoped_executor = session_context
            .map(|session_context| self.for_session_context(session_context))
            .unwrap_or_else(|| self.clone());
        let execution = orchestrator::execute_tool(
            &scoped_executor,
            tool_name,
            input,
            ToolRuntimeContext {
                session_id,
                call_id,
                session_context: None,
                prepared_shell_command: None,
            },
        )?;
        Ok(in_process_router::tool_execution_to_invoke_output(
            scoped_executor.truncator.apply(execution),
        ))
    }

    pub(crate) fn execute_tool_payload_detailed_with_context(
        &self,
        input: &ToolPayloadInput,
        context: ToolRuntimeContext,
    ) -> Result<ToolPayloadExecution, ToolError> {
        let scoped_executor = context
            .session_context
            .as_ref()
            .map(|session_context| self.for_session_context(session_context))
            .unwrap_or_else(|| self.clone());
        let invocation = input.clone().into_invocation();
        let tool_name = input.tool_name();
        let definition = scoped_executor
            .invocation_definition(&invocation)
            .ok_or_else(|| scoped_executor.unknown_tool_error(tool_name))?;
        if !scoped_executor
            .builtin_tool_set()
            .is_tool_enabled(&definition)
        {
            return Err(ToolError::UnsupportedInvocation(tool_name.to_string()));
        }
        let session_id = context.session_id.unwrap_or(-1);
        let call_id = context
            .call_id
            .unwrap_or_else(|| SYNTHETIC_TOOL_CALL_ID.fetch_sub(1, Ordering::Relaxed));
        let execution = scoped_executor.execute_invocation_detailed_inner(
            &invocation,
            session_id,
            call_id,
            context.prepared_shell_command,
        )?;
        let output =
            ToolPayloadOutput::from_tool_output(tool_name, &execution.output).ok_or_else(|| {
                ToolError::Plugin(format!(
                    "decode {tool_name} output: payload did not match tool payload schema"
                ))
            })?;
        Ok(scoped_executor.truncator.apply(ToolPayloadExecution {
            output,
            view: execution.view,
            apply_patch: execution.apply_patch,
        }))
    }

    pub fn collect_permission_checks(
        &self,
        input: &ToolPayloadInput,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.collect_permission_checks_for_invocation_in_session(
            &input.clone().into_invocation(),
            None,
        )
    }

    pub fn prepare_shell_command(
        &self,
        input: &crate::message::ShellCommandInput,
        session_id: i64,
        call_id: i64,
    ) -> Result<Option<PreparedShellCommand>, ToolError> {
        bash::prepare_command(self, input, session_id, call_id)
    }

    pub fn prepare_shell_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<(ToolInvocation, Option<PreparedShellCommand>), ToolError> {
        let Some(ToolPayloadInput::Shell(crate::message::ShellToolInput::Run {
            shell: crate::message::ProcessShell::Bash,
            command: process_input,
            background,
        })) = ToolPayloadInput::from_invocation(invocation)
        else {
            return Ok((invocation.clone(), None));
        };
        let prepared_shell = self.prepare_shell_command(&process_input, session_id, call_id)?;
        let Some(prepared_shell) = prepared_shell.clone() else {
            return Ok((invocation.clone(), None));
        };
        if prepared_shell.command == process_input.command {
            return Ok((invocation.clone(), Some(prepared_shell)));
        }
        let mut rewritten = process_input;
        rewritten.command = prepared_shell.command.clone();
        let rewritten_invocation = ToolPayloadInput::Shell(crate::message::ShellToolInput::Run {
            shell: crate::message::ProcessShell::Bash,
            command: rewritten,
            background,
        })
        .into_invocation();
        let input_value = serde_json::Value::from(rewritten_invocation.input);
        let input = StructuredObject::try_from(input_value)
            .map_err(|err| ToolError::InvalidInput(err.to_string()))?;
        Ok((
            ToolInvocation {
                tool_api_function: invocation.tool_api_function,
                name: invocation.name.clone(),
                plugin_name: invocation.plugin_name.clone(),
                input,
            },
            Some(prepared_shell),
        ))
    }

    pub fn prepare_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<PreparedToolInvocation, ToolError> {
        self.ensure_not_cancelled()?;
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
            .ok_or_else(|| ToolError::UnknownTool {
                tool: hook_tool_name.clone(),
            })?;
        let input_json = invocation_input_json(invocation)?;
        let parsed_input_value: serde_json::Value = serde_json::from_str(&input_json)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;
        let input_value = parsed_input_value;

        let effective_tags = definition
            .as_ref()
            .map(|definition| invocation_effective_tags(definition, invocation))
            .unwrap_or_default();

        let hooked = self
            .plugins
            .dispatch_tool_before_cancellable(
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
            .map_err(|err| self.plugin_error_or_cancelled(err))?;

        let input_json = serde_json::to_string(&hooked.input)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;

        let mut prepared_invocation =
            parse_invocation_from_json(model_tool_name.as_str(), input_json.as_str())?;
        prepared_invocation.tool_api_function = invocation.tool_api_function;
        prepared_invocation.plugin_name = Some(plugin_name);

        Ok(PreparedToolInvocation {
            invocation: prepared_invocation,
            title_override: hooked.title_override,
            metadata: hooked.metadata.into_iter().collect(),
        })
    }

    pub fn collect_permission_checks_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.collect_permission_checks_for_invocation_in_session(invocation, None)
    }

    pub fn collect_permission_checks_for_invocation_in_session(
        &self,
        invocation: &ToolInvocation,
        _session_id: Option<i64>,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.ensure_not_cancelled()?;
        let (tool_name, decision) = self.authorize_invocation(invocation)?;
        let command = shell_command_from_invocation(invocation);
        let tags = self
            .invocation_definition(invocation)
            .map(|definition| invocation_effective_tags(&definition, invocation))
            .unwrap_or_default();
        let action = crate::permission::tool_action(
            tool_name.as_str(),
            command.as_deref(),
            tags.as_slice(),
            Some(&self.agent.tool_policy),
        );
        let mut checks = vec![ToolPermissionCheck { action, decision }];

        if let Some(resolution) = self.plugin_resolution_for_invocation(invocation) {
            let input_value = resolved_tool_input_value(&resolution, invocation);
            if resolution.has_tag(crate::plugin::sdk::ToolTag::Shell) {
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
            self.collect_dynamic_path_checks(&mut checks, &resolution, &input_value)?;
            self.collect_declared_network_checks(
                &mut checks,
                &input_value,
                &resolution.definition.permissions.input_networks,
                &resolution.definition.permissions.network_access,
            )?;
            self.collect_dynamic_network_checks(&mut checks, &resolution, &input_value)?;
        }
        Ok(checks)
    }

    pub async fn execute_invocation_streaming(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<Option<StreamingToolExecution>, ToolError> {
        self.enforce_invocation_permissions(invocation, Some(session_id))?;
        self.execute_invocation_streaming_inner(invocation, session_id, call_id)
            .await
    }

    /// Run a streaming invocation after a trusted caller has resolved every
    /// permission check, including persisted rules and any user approval.
    ///
    /// This is crate-visible on purpose: entry points must use the normal
    /// method above unless they own the complete permission resolution flow.
    pub(crate) async fn execute_invocation_streaming_after_authorization(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<Option<StreamingToolExecution>, ToolError> {
        let mut authorized = self.clone();
        authorized.permission_mode = PermissionEnforcementMode::Bypassed;
        authorized
            .execute_invocation_streaming(invocation, session_id, call_id)
            .await
    }

    pub(crate) async fn execute_invocation_streaming_inner(
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
            .plugin_resolution_for_plugin_invocation(&plugin_invocation)
            .ok_or_else(|| self.unknown_tool_error(plugin_invocation.tool_name.as_str()))?;
        let executor_guard = in_process_router::install_executor_context(
            self,
            session_id,
            call_id,
            resolution.tool_name().to_string(),
        );
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
                ToolError::Plugin(err.message)
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
        let (end_tx, end_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let stream_end = match cancellation.as_ref() {
                Some(cancellation) => tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        let _ = end_tx.send(Err(ToolError::Cancelled));
                        return;
                    },
                    result = end => result,
                },
                None => end.await,
            };
            let result = match stream_end {
                Ok(Ok(end)) => (|| {
                    let view = ToolExecutionView {
                        title: end.title,
                        output_text: end.output_text,
                        metadata: end.metadata.into_iter().collect(),
                        attachments: end.attachments,
                    };
                    let output = ToolOutput::from_json_payload(end.payload.as_ref())
                        .map_err(ToolError::InvalidInput)?;
                    let execution = ToolInvocationExecution {
                        output: output.clone(),
                        view,
                        apply_patch: apply_patch_execution_from_tool_output(&output),
                    };
                    executor.finalize_execution(
                        &invocation,
                        session_id,
                        model_tool_name.as_str(),
                        &result_policy,
                        call_id,
                        execution,
                    )
                })(),
                Ok(Err(err)) => Err(ToolError::Plugin(err.message)),
                Err(_) => Err(ToolError::Plugin(
                    "stream ended without a terminal frame".to_string(),
                )),
            };
            let _ = end_tx.send(result);
        });
        Ok(Some(StreamingToolExecution {
            stream_id,
            chunks,
            end: end_rx,
            _executor_guard: Some(executor_guard),
        }))
    }

    pub fn execute_invocation_detailed(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation_detailed_with_prepared_shell(invocation, session_id, call_id, None)
    }

    pub fn execute_invocation_detailed_with_prepared_shell(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.enforce_invocation_permissions(invocation, Some(session_id))?;
        let result = self.execute_invocation_detailed_inner(
            invocation,
            session_id,
            call_id,
            prepared_shell_command,
        );
        crate::metrics::record_tool_execution(result.is_ok());
        result
    }

    pub(crate) fn execute_invocation_detailed_inner(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        _prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.ensure_not_cancelled()?;
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);
        let tool_name = plugin_invocation_name(&plugin_invocation);
        let _tool_span =
            tracing::info_span!("tool.call", session_id, call_id, tool = tool_name.as_str(),)
                .entered();
        let resolution = self
            .plugin_resolution_for_plugin_invocation(&plugin_invocation)
            .ok_or_else(|| self.unknown_tool_error(plugin_invocation.tool_name.as_str()))?;
        let _executor_guard = in_process_router::install_executor_context(
            self,
            session_id,
            call_id,
            resolution.tool_name().to_string(),
        );

        let response = self
            .plugins
            .invoke_tool_cancellable(
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
            .map_err(|err| {
                if self
                    .cancellation_token()
                    .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
                {
                    ToolError::Cancelled
                } else {
                    ToolError::Plugin(err.message)
                }
            })?;
        self.ensure_not_cancelled()?;

        let view = ToolExecutionView {
            title: response.title.clone(),
            output_text: response.output_text.clone(),
            metadata: response.metadata.into_iter().collect(),
            attachments: response.attachments,
        };
        let output = ToolOutput::from_json_payload(response.payload.as_ref())
            .map_err(ToolError::InvalidInput)?;
        let execution = ToolInvocationExecution {
            output: output.clone(),
            view,
            apply_patch: apply_patch_execution_from_tool_output(&output),
        };
        self.finalize_execution(
            invocation,
            session_id,
            resolution.canonical_name().as_str(),
            &resolution.definition.runtime.result_policy,
            call_id,
            execution,
        )
    }

    pub(crate) fn execute_invocation_detailed_bypassing_permissions(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
            invocation, session_id, call_id, None,
        )
    }

    pub(crate) fn execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let mut trusted = self.clone();
        trusted.permission_mode = PermissionEnforcementMode::Bypassed;
        trusted.execute_invocation_detailed_with_prepared_shell(
            invocation,
            session_id,
            call_id,
            prepared_shell_command,
        )
    }
}
use super::{
    Ordering, PermissionEnforcementMode, PluginInvocation, PluginToolBeforeInput,
    PluginToolInvokeInput, PreparedShellCommand, PreparedToolInvocation, SYNTHETIC_TOOL_CALL_ID,
    SdkToolStreamingMode, StreamingToolExecution, StructuredObject, ToolError, ToolExecutionView,
    ToolExecutor, ToolInvocation, ToolInvocationExecution, ToolOutput, ToolPayloadExecution,
    ToolPayloadInput, ToolPayloadOutput, ToolPermissionCheck, ToolRuntimeContext,
    apply_patch_execution_from_tool_output, bash, in_process_router, invocation_effective_tags,
    invocation_input_json, invocation_name, orchestrator, parse_invocation_from_json,
    plugin_invocation_name, resolved_plugin_invocation_input_value, resolved_tool_input_value,
    shell_command_from_invocation,
};
