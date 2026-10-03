impl ToolExecutor {
    pub async fn shell_env_overrides_async(
        &self,
        cwd: &Path,
        session_id: Option<i64>,
        call_id: Option<i64>,
    ) -> Result<std::collections::HashMap<String, String>, ToolError> {
        self.ensure_not_cancelled()?;
        let patch = self
            .plugins
            .dispatch_shell_env(
                PluginShellEnvInput {
                    cwd: cwd.to_path_buf(),
                    session_id,
                    call_id,
                },
                self.cancellation_token.clone(),
            )
            .await
            .map_err(|err| self.plugin_error_or_cancelled(err))?;
        Ok(patch.set.into_iter().collect())
    }

    pub(crate) async fn finalize_execution_async(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        model_tool_name: &str,
        call_id: i64,
        mut execution: ToolInvocationExecution,
    ) -> Result<ToolInvocationExecution, ToolError> {
        execution.view.normalize_presentation();
        let raw_execution = execution.clone();
        if let Err(error) = self
            .apply_after_hooks_async(invocation, session_id, call_id, &mut execution)
            .await
        {
            // The tool has already returned. A post-hook failure cannot undo
            // its side effects and must not induce an automatic mutation retry.
            tracing::warn!(tool=%model_tool_name,diagnostic=%error,"post-execution hook failed after a tool returned; preserving raw receipt");
            execution = raw_execution;
            let warning = "Post-execution processing failed after the tool returned. Its side effects may already be committed. Inspect the recorded result; do not repeat the operation automatically.";
            execution
                .view
                .output_text
                .push_str(&format!("\n[{warning}]"));
            execution
                .view
                .metadata
                .insert("postprocessing_warning".into(), warning.into());
            execution
                .view
                .metadata
                .insert("postprocessing_state".into(), "failed".into());
            execution
                .view
                .metadata
                .insert("safe_to_reexecute_automatically".into(), "false".into());
            execution.view.summary = format!("{} · postprocessing warning", execution.view.summary);
        }
        execution.view.normalize_presentation();
        if execution.view.summary.is_empty() {
            return Err(ToolError::plugin(format!(
                "tool `{model_tool_name}` returned an empty activity summary; every tool result must provide a concise outcome summary"
            )));
        }

        crate::tool::post_edit::validate(self, invocation, session_id, &mut execution).await;

        if matches!(
            model_tool_name,
            "fs.read" | "agena.fs.read" | "fs.read_many" | "agena.fs.read_many"
        ) {
            let input = serde_json::Value::from(invocation.input.clone());
            let paths = input
                .get("paths")
                .and_then(serde_json::Value::as_array)
                .map(|paths| {
                    paths
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(|path| self.resolve_target_path(path))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| {
                    input
                        .get("file_path")
                        .and_then(serde_json::Value::as_str)
                        .map(|path| vec![self.resolve_target_path(path)])
                        .unwrap_or_default()
                });
            let instructions = self.project_instruction_section(&paths);
            if !instructions.is_empty() {
                execution
                    .view
                    .metadata
                    .insert("project_instructions".to_owned(), instructions.clone());
                execution
                    .view
                    .output_text
                    .push_str(&format!("\n\n{instructions}"));
            }
        }

        // Complete the call-time action/input title with a compact fact from
        // the full raw result so operator callers and session completion use
        // the same final headline as the read-time transcript renderer.
        let raw_output = execution.view.raw_output(&execution.output);
        execution.view.title = agena_tool::completed_tool_title(invocation, &raw_output);
        Ok(execution)
    }

    pub(crate) async fn apply_after_hooks_async(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
    ) -> Result<(), ToolError> {
        let model_tool_name = invocation_name(invocation).to_owned();
        let hook_tool_name = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.tool_name().to_string())
            .unwrap_or(model_tool_name);
        let hook_tool = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.tool_key().clone())
            .or_else(|| hook_tool_name.parse().ok())
            .ok_or_else(|| self.unknown_tool_error(hook_tool_name.as_str()))?;
        let summary = execution.summary();
        let after_in = PluginToolAfterInput {
            tool: hook_tool,
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            title: summary.title,
            summary: summary.summary,
            output_text: summary.output_text,
            payload: summary.payload,
            metadata: summary.metadata.into_iter().collect(),
        };

        let hooked = self
            .plugins
            .dispatch_tool_after(after_in, self.cancellation_token.clone())
            .await
            .map_err(|err| self.plugin_error_or_cancelled(err))?;

        execution.view.apply_neutral_fields(
            hooked.title,
            hooked.summary,
            hooked.output_text,
            hooked.metadata,
        );
        if let Some(payload_value) = hooked.payload {
            execution.output = ToolOutput::from_json_payload(Some(&payload_value))
                .map_err(ToolError::invalid_input)?;
        }
        Ok(())
    }

    /// Fire-and-forget notification to plugins about a tool execution failure.
    pub async fn broadcast_tool_failure(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        failure: &agena_failure::Failure,
    ) {
        if self.plugins.is_empty() {
            return;
        }
        let model_tool_name = invocation_name(invocation).to_owned();
        let hook_tool_name = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.tool_name().to_string())
            .unwrap_or(model_tool_name);
        let Some(hook_tool) = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.tool_key().clone())
            .or_else(|| hook_tool_name.parse().ok())
        else {
            return;
        };
        let input_value = match invocation_input_json(invocation) {
            Ok(json) => match serde_json::from_str::<serde_json::Value>(&json) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(
                        tool_name = %invocation.name,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "decode serialized tool input for a plugin failure hook",
                            &error,
                        ),
                        "plugin tool failure hook is receiving a null input projection"
                    );
                    serde_json::Value::Null
                }
            },
            Err(error) => {
                tracing::warn!(
                    tool_name = %invocation.name,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "serialize tool input for a plugin failure hook",
                        &error,
                    ),
                    "plugin tool failure hook is receiving a null input projection"
                );
                serde_json::Value::Null
            }
        };
        let failure_input = PluginToolFailureInput {
            tool: hook_tool,
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            input: input_value,
            failure: failure.into(),
        };
        self.plugins.broadcast_tool_failure(failure_input).await;
    }

    /// Render a durable raw result without writing the projection back to the
    /// session. The owning plugin gets first refusal; built-in tool rendering
    /// and the generic system projection fill only the sides it delegates.
    pub async fn render_tool_result(
        &self,
        invocation: &ToolInvocation,
        output: &agena_domain::RawOutput,
    ) -> agena_plugin_host::sdk::ToolRenderOutput {
        let input = agena_plugin_host::sdk::ToolRenderInput {
            tool_name: invocation.name.clone(),
            input: serde_json::Value::from(invocation.input.clone()),
            output: output.clone(),
        };
        let registered = self.plugin_resolution_for_invocation(invocation);
        let mut rendered = if let Some(registered) = registered.as_ref() {
            match self.plugins.render_tool(registered, input).await {
                Ok(Some(rendered)) => rendered,
                Ok(None) => Default::default(),
                Err(error) => {
                    tracing::warn!(
                        target: "agena::tool_render",
                        tool = %invocation.name,
                        "plugin tool renderer failed; using runtime fallback: {error}"
                    );
                    Default::default()
                }
            }
        } else {
            Default::default()
        };

        rendered
            .model
            .get_or_insert_with(|| raw_model_fallback(output));
        let plugin_human = rendered.human.take();
        let needs_runtime_human_fallback = plugin_human.as_ref().is_none_or(|human| {
            human.blocks.is_empty()
                || human
                    .blocks
                    .iter()
                    .all(|block| matches!(block, agena_domain::ViewBlock::Json { .. }))
        });
        if needs_runtime_human_fallback {
            let command = invocation
                .input
                .get("command")
                .and_then(|value| value.as_text())
                .map(ToOwned::to_owned);
            let cwd = invocation
                .input
                .get("workdir")
                .and_then(|value| value.as_text())
                .map(ToOwned::to_owned);
            let mut renderer =
                crate::tool::human_view::BuiltinHumanRenderer::new(invocation.name.as_str());
            if let Some(command) = command {
                renderer = renderer.with_command(command);
            }
            if let Some(cwd) = cwd {
                renderer = renderer.with_cwd(cwd);
            }
            let context = agena_tool::RenderContext {
                workspace_root: self.workspace_root.clone(),
                command: None,
            };
            let blocks = agena_tool::ToolHumanRenderer::render_human(&renderer, &context, output)
                .unwrap_or_default();
            rendered.human = Some(agena_plugin_host::sdk::ToolHumanPresentation {
                title: agena_tool::completed_tool_title(invocation, output),
                summary: plugin_human
                    .as_ref()
                    .map(|human| human.summary.clone())
                    .filter(|summary| !summary.trim().is_empty())
                    .unwrap_or_else(|| {
                        crate::tool::human_view::BuiltinHumanRenderer::human_summary_for_tool(
                            invocation.name.as_str(),
                            output,
                        )
                    }),
                blocks,
            });
        } else if let Some(mut human) = plugin_human {
            human.title = agena_tool::completed_tool_title(invocation, output);
            if human.summary.trim().is_empty() {
                human.summary =
                    crate::tool::human_view::BuiltinHumanRenderer::human_summary_for_tool(
                        invocation.name.as_str(),
                        output,
                    );
            }
            rendered.human = Some(human);
        }
        rendered
    }

    pub async fn broadcast_notification(
        &self,
        kind: impl Into<String>,
        session_id: Option<i64>,
        title: impl Into<String>,
        message: impl Into<String>,
        payload: serde_json::Value,
    ) {
        if self.plugins.is_empty() {
            return;
        }
        let input = agena_plugin_host::NotificationInput {
            kind: kind.into(),
            session_id,
            title: title.into(),
            message: message.into(),
            payload,
        };
        self.plugins.broadcast_notification(input).await;
    }
}

fn raw_model_fallback(output: &agena_domain::RawOutput) -> String {
    match output.payload.as_ref() {
        Some(payload)
            if payload.is_string()
                || (payload.as_object().is_some_and(|object| object.len() == 1)
                    && payload
                        .get("text")
                        .is_some_and(serde_json::Value::is_string)) =>
        {
            output.text_content().to_owned()
        }
        Some(payload) => match serde_json::to_string(payload) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "serialize a tool result payload for model projection",
                        &error,
                    ),
                    "tool result model projection fell back to its text representation"
                );
                if output.text_content().is_empty() {
                    "[tool result payload could not be serialized]".to_owned()
                } else {
                    output.text_content().to_owned()
                }
            }
        },
        None => output.text_content().to_owned(),
    }
}

use super::{
    Path, PluginShellEnvInput, PluginToolAfterInput, PluginToolFailureInput, ToolError,
    ToolExecutor, ToolInvocation, ToolInvocationExecution, ToolOutput, invocation_input_json,
    invocation_name,
};
