impl ToolExecutor {
    pub fn issue_execution_grant(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<&PreparedShellCommand>,
        authorized_actions: Vec<agena_domain::PermissionAction>,
    ) -> Result<crate::tool::ExecutionGrant, ToolError> {
        let checks =
            self.collect_permission_checks_for_invocation_in_session(invocation, Some(session_id))?;
        let required_actions =
            unique_permission_actions(checks.into_iter().map(|check| check.action).collect());
        let authorized_actions = unique_permission_actions(authorized_actions);
        if required_actions.len() != authorized_actions.len()
            || required_actions
                .iter()
                .any(|action| !authorized_actions.contains(action))
        {
            return Err(ToolError::InvalidExecutionGrant(
                "authorization action set does not exactly match the prepared invocation"
                    .to_string(),
            ));
        }
        Ok(crate::tool::ExecutionGrant {
            session_id,
            call_id,
            invocation_digest: execution_invocation_digest(invocation)?,
            prepared_shell_digest: prepared_shell_command.map(prepared_shell_digest),
            authorized_actions,
        })
    }

    fn validate_execution_grant(
        &self,
        grant: &crate::tool::ExecutionGrant,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<&PreparedShellCommand>,
    ) -> Result<(), ToolError> {
        if grant.session_id != session_id || grant.call_id != call_id {
            return Err(ToolError::InvalidExecutionGrant(
                "session or call identity changed after authorization".to_string(),
            ));
        }
        if grant.invocation_digest != execution_invocation_digest(invocation)? {
            return Err(ToolError::InvalidExecutionGrant(
                "tool invocation changed after authorization".to_string(),
            ));
        }
        if grant.prepared_shell_digest != prepared_shell_command.map(prepared_shell_digest) {
            return Err(ToolError::InvalidExecutionGrant(
                "prepared shell command changed after authorization".to_string(),
            ));
        }
        let checks =
            self.collect_permission_checks_for_invocation_in_session(invocation, Some(session_id))?;
        let required_actions =
            unique_permission_actions(checks.into_iter().map(|check| check.action).collect());
        if required_actions.len() != grant.authorized_actions.len()
            || required_actions
                .iter()
                .any(|action| !grant.authorized_actions.contains(action))
        {
            return Err(ToolError::InvalidExecutionGrant(
                "protected action set changed after authorization".to_string(),
            ));
        }
        Ok(())
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
            shell: agena_domain::ProcessShell::Bash,
            command: process_input,
            background,
            monitor,
        })) = ToolPayloadInput::from_invocation(invocation)
        else {
            return Ok((invocation.clone(), None));
        };
        let prepared_shell =
            self.prepare_shell_command(process_input.as_ref(), session_id, call_id)?;
        let Some(prepared_shell) = prepared_shell.clone() else {
            return Ok((invocation.clone(), None));
        };
        if prepared_shell.command == process_input.command {
            return Ok((invocation.clone(), Some(prepared_shell)));
        }
        let mut rewritten = *process_input;
        rewritten.command = prepared_shell.command.clone();
        let rewritten_invocation = ToolPayloadInput::Shell(crate::message::ShellToolInput::Run {
            shell: agena_domain::ProcessShell::Bash,
            command: Box::new(rewritten),
            background,
            monitor,
        })
        .into_invocation();
        let input_value = serde_json::Value::from(rewritten_invocation.input);
        let input = StructuredObject::try_from(input_value)
            .map_err(|err| ToolError::invalid_input(err.to_string()))?;
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
            .ok_or_else(|| self.unknown_tool_error(hook_tool_name.as_str()))?;
        let input_json = invocation_input_json(invocation)?;
        let parsed_input_value: serde_json::Value = serde_json::from_str(&input_json)
            .map_err(|e| ToolError::invalid_input(e.to_string()))?;
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
            .map_err(|e| ToolError::invalid_input(e.to_string()))?;

        let mut prepared_invocation =
            parse_invocation_from_json(model_tool_name.as_str(), input_json.as_str())?;
        prepared_invocation.tool_api_function = invocation.tool_api_function;
        prepared_invocation.plugin_name = invocation
            .tool_api_function
            .is_none()
            .then_some(plugin_name);

        Ok(PreparedToolInvocation {
            invocation: prepared_invocation,
            title_override: hooked.title_override,
            metadata: hooked.metadata.into_iter().collect(),
        })
    }

    #[cfg(test)]
    pub(crate) fn collect_permission_checks_for_invocation(
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
        // These five functions are the provider-facing Tool API transport. They
        // only discover tools or route `tools_call` to an execution tool; they
        // are not themselves authority-bearing operations. In particular,
        // authorizing the outer `tools_call` would both ask about the
        // wrong tool and allow a persisted rule for that gateway to obscure
        // the permissions of the actual target. The host callback used by
        // `tools_call` re-enters this method with the resolved execution tool,
        // which is where tool/path/network permission checks belong.
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
            .map(|definition| invocation_effective_tags(&definition, invocation))
            .unwrap_or_default();
        let action = crate::permission::tool_action(
            tool_name.as_str(),
            command.as_deref(),
            tags.as_slice(),
            Some(&self.principal.tool_policy),
        );
        let mut checks = vec![ToolPermissionCheck { action, decision }];

        if let Some(inspector) = self.permission_inspector.as_ref() {
            checks.extend(inspector.additional_checks(invocation, &self.principal)?);
        }

        if let Some(resolution) = self.plugin_resolution_for_invocation(invocation) {
            let input_value = resolved_tool_input_value(&resolution, invocation);
            if resolution.has_tag(agena_plugin_host::sdk::ToolTag::Shell) {
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

    /// Run a streaming invocation after a trusted caller has resolved every
    /// permission check, including persisted rules and any user approval.
    ///
    /// This is an application-facing escape hatch for a trusted caller that
    /// owns the complete permission resolution flow.
    pub async fn execute_invocation_streaming_with_grant(
        &self,
        grant: &crate::tool::ExecutionGrant,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<&PreparedShellCommand>,
    ) -> Result<Option<StreamingToolExecution>, ToolError> {
        self.validate_execution_grant(
            grant,
            invocation,
            session_id,
            call_id,
            prepared_shell_command,
        )?;
        let mut authorized = self.clone();
        authorized.authorization_state = ExecutionAuthorizationState::GrantValidated;
        authorized
            .execute_invocation_streaming_inner(invocation, session_id, call_id)
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
            .plugin_resolution_for_invocation(invocation)
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
                        .map_err(ToolError::invalid_input)?;
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
                Ok(Err(err)) => Err(ToolError::from_plugin_error(err)),
                Err(_) => Err(ToolError::plugin(
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
            .plugin_resolution_for_invocation(invocation)
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
                    ToolError::from_plugin_error(err)
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
            .map_err(ToolError::invalid_input)?;
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

    pub fn execute_invocation_detailed_with_grant(
        &self,
        grant: &crate::tool::ExecutionGrant,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation_detailed_with_grant_and_prepared_shell(
            grant, invocation, session_id, call_id, None,
        )
    }

    pub fn execute_invocation_detailed_with_grant_and_prepared_shell(
        &self,
        grant: &crate::tool::ExecutionGrant,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.validate_execution_grant(
            grant,
            invocation,
            session_id,
            call_id,
            prepared_shell_command.as_ref(),
        )?;
        let mut trusted = self.clone();
        trusted.authorization_state = ExecutionAuthorizationState::GrantValidated;
        trusted.execute_invocation_detailed_inner(
            invocation,
            session_id,
            call_id,
            prepared_shell_command,
        )
    }
}

fn unique_permission_actions(
    actions: Vec<agena_domain::PermissionAction>,
) -> Vec<agena_domain::PermissionAction> {
    let mut unique = Vec::with_capacity(actions.len());
    for action in actions {
        if !unique.contains(&action) {
            unique.push(action);
        }
    }
    unique
}

fn execution_invocation_digest(invocation: &ToolInvocation) -> Result<[u8; 32], ToolError> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(invocation)
        .map_err(|error| ToolError::InvalidExecutionGrant(error.to_string()))?;
    Ok(Sha256::digest(bytes).into())
}

fn prepared_shell_digest(prepared: &PreparedShellCommand) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(prepared.command.as_bytes());
    digest.update([0]);
    digest.update(prepared.cwd.to_string_lossy().as_bytes());
    digest.finalize().into()
}
use agena_domain::{PluginInvocation, StructuredObject};

use super::{
    ExecutionAuthorizationState, PluginToolBeforeInput, PluginToolInvokeInput,
    PreparedShellCommand, PreparedToolInvocation, SdkToolStreamingMode, StreamingToolExecution,
    ToolError, ToolExecutionView, ToolExecutor, ToolInvocation, ToolInvocationExecution,
    ToolOutput, ToolPayloadInput, ToolPermissionCheck, apply_patch_execution_from_tool_output,
    bash, in_process_router, invocation_effective_tags, invocation_input_json, invocation_name,
    parse_invocation_from_json, plugin_invocation_name, resolved_plugin_invocation_input_value,
    resolved_tool_input_value, shell_command_from_invocation,
};
