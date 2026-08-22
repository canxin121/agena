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
        self.apply_after_hooks_async(invocation, session_id, call_id, &mut execution)
            .await?;
        execution.view.normalize_presentation();
        if execution.view.summary.is_empty() {
            return Err(ToolError::plugin(format!(
                "tool `{model_tool_name}` returned an empty activity summary; every tool result must provide a concise outcome summary"
            )));
        }

        // Complete the call-time action/input title with a compact fact from
        // the full raw result so operator callers and session completion use
        // the same final headline as the read-time transcript renderer.
        let raw_output = agena_domain::RawOutput::from_parts(
            execution.output.to_json_payload(),
            execution.view.output_text.clone(),
            execution.view.attachments.clone(),
            execution.output.managed_outputs.clone(),
            execution
                .view
                .metadata
                .iter()
                .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
                .collect(),
            execution.output.truncated,
        );
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

        let model = rendered
            .model
            .get_or_insert_with(|| raw_model_fallback(output));
        project_model_at_read_time(
            model,
            registered
                .as_ref()
                .map(|tool| &tool.definition.runtime.result_policy),
        );
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
                if output.text.is_empty() {
                    "[tool result payload could not be serialized]".to_owned()
                } else {
                    output.text.clone()
                }
            }
        },
        None => output.text.clone(),
    }
}

fn project_model_at_read_time(model: &mut String, policy: Option<&SdkToolResultPolicy>) {
    if let Some(policy) = policy {
        let mut truncated_by_policy = false;
        if let Some(max_lines) = policy.preview_lines
            && max_lines > 0
        {
            let mut lines = model.lines();
            let selected = lines.by_ref().take(max_lines).collect::<Vec<_>>();
            if lines.next().is_some() {
                *model = selected.join("\n");
                truncated_by_policy = true;
            }
        }
        if let Some(max_chars) = policy.max_model_chars
            && max_chars > 0
            && model.chars().count() > max_chars
        {
            *model = truncate_to_char_count(model, max_chars);
            truncated_by_policy = true;
        }
        if truncated_by_policy {
            model.push_str(
                "\n\n[model projection truncated by tool result policy; raw output retained]",
            );
        }
    }

    if model.trim().is_empty()
        || !model_output_exceeds_boundary(
            model,
            TOOL_MODEL_OUTPUT_MAX_LINES,
            TOOL_MODEL_OUTPUT_MAX_BYTES,
        )
    {
        return;
    }
    let marker = format!(
        "... model projection truncated ({} lines, {} bytes); raw output retained ...",
        line_count(model),
        model.len(),
    );
    *model = bounded_model_output_preview(
        model,
        marker.as_str(),
        TOOL_MODEL_OUTPUT_MAX_LINES,
        TOOL_MODEL_OUTPUT_MAX_BYTES,
    );
}
use super::{
    Path, PluginShellEnvInput, PluginToolAfterInput, PluginToolFailureInput, SdkToolResultPolicy,
    TOOL_MODEL_OUTPUT_MAX_BYTES, TOOL_MODEL_OUTPUT_MAX_LINES, ToolError, ToolExecutor,
    ToolInvocation, ToolInvocationExecution, ToolOutput, bounded_model_output_preview,
    invocation_input_json, invocation_name, line_count, model_output_exceeds_boundary,
    truncate_to_char_count,
};

#[cfg(test)]
mod tests {
    use super::{SdkToolResultPolicy, TOOL_MODEL_OUTPUT_MAX_BYTES, project_model_at_read_time};

    #[test]
    fn model_boundary_is_an_ephemeral_projection_of_unchanged_raw_text() {
        let raw = (0..2_000)
            .map(|index| format!("raw-line-{index:04}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut model = raw.clone();

        project_model_at_read_time(&mut model, None);

        assert_eq!(raw.lines().count(), 2_000);
        assert_ne!(model, raw);
        assert!(model.len() <= TOOL_MODEL_OUTPUT_MAX_BYTES);
        assert!(model.contains("raw output retained"));
    }

    #[test]
    fn tool_result_policy_changes_only_the_runtime_model_projection() {
        let raw = "one\ntwo\nthree\nfour".to_owned();
        let mut model = raw.clone();
        let policy = SdkToolResultPolicy {
            preview_lines: Some(2),
            ..Default::default()
        };

        project_model_at_read_time(&mut model, Some(&policy));

        assert_eq!(raw, "one\ntwo\nthree\nfour");
        assert!(model.starts_with("one\ntwo"));
        assert!(model.contains("raw output retained"));
    }
}
