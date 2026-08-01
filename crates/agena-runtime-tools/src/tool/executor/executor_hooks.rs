impl ToolExecutor {
    pub fn shell_env_overrides(
        &self,
        cwd: &Path,
        session_id: Option<i64>,
        call_id: Option<i64>,
    ) -> Result<std::collections::HashMap<String, String>, ToolError> {
        self.ensure_not_cancelled()?;
        let patch = self
            .plugins
            .dispatch_shell_env_cancellable(
                PluginShellEnvInput {
                    cwd: cwd.to_path_buf(),
                    session_id,
                    call_id,
                },
                self.cancellation_token.clone(),
            )
            .map_err(|err| self.plugin_error_or_cancelled(err))?;
        Ok(patch.set.into_iter().collect())
    }

    pub(crate) fn finalize_execution(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        model_tool_name: &str,
        result_policy: &SdkToolResultPolicy,
        call_id: i64,
        mut execution: ToolInvocationExecution,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.apply_after_hooks(invocation, session_id, call_id, &mut execution)?;
        self.apply_result_policy(model_tool_name, result_policy, call_id, &mut execution)?;
        self.apply_model_output_boundary(model_tool_name, call_id, &mut execution)?;
        Ok(execution)
    }

    pub(crate) fn apply_after_hooks(
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
            output_text: summary.output_text,
            payload: summary.payload,
            metadata: summary.metadata.into_iter().collect(),
        };

        let hooked = self
            .plugins
            .dispatch_tool_after_cancellable(after_in, self.cancellation_token.clone())
            .map_err(|err| self.plugin_error_or_cancelled(err))?;

        execution.view.apply_neutral_fields(
            hooked.title,
            execution.view.summary.clone(),
            hooked.output_text,
            hooked.metadata,
        );

        if let Some(payload_value) = hooked.payload {
            execution.output = ToolOutput::from_json_payload(Some(&payload_value))
                .map_err(ToolError::invalid_input)?;
        }

        Ok(())
    }

    pub(crate) fn apply_result_policy(
        &self,
        model_tool_name: &str,
        policy: &SdkToolResultPolicy,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
    ) -> Result<(), ToolError> {
        if policy.is_default() {
            return Ok(());
        }

        execution.view.insert_neutral_metadata(
            "result_policy_ui_render_kind".to_string(),
            format!("{:?}", policy.ui_render_kind).to_ascii_lowercase(),
        );
        if let Some(preview_lines) = policy.preview_lines {
            execution.view.insert_neutral_metadata(
                "result_policy_preview_lines".to_string(),
                preview_lines.to_string(),
            );
        }

        let original = execution.view.output_text.clone();
        if original.is_empty() {
            return Ok(());
        }

        let mut preview = original.clone();
        let mut truncated = false;

        if let Some(max_lines) = policy.preview_lines
            && max_lines > 0
        {
            let mut lines = preview.lines();
            let selected = lines.by_ref().take(max_lines).collect::<Vec<_>>();
            if lines.next().is_some() {
                preview = selected.join("\n");
                truncated = true;
            }
        }

        if let Some(max_chars) = policy.max_model_chars
            && max_chars > 0
            && preview.chars().count() > max_chars
        {
            preview = truncate_to_char_count(preview.as_str(), max_chars);
            truncated = true;
        }

        if !truncated {
            return Ok(());
        }

        execution
            .view
            .insert_neutral_metadata("result_policy_truncated", "true");
        execution.view.insert_neutral_metadata(
            "result_policy_original_chars".to_string(),
            original.chars().count().to_string(),
        );
        execution.view.insert_neutral_metadata(
            "result_policy_model_chars".to_string(),
            preview.chars().count().to_string(),
        );

        if policy.persist_large_output {
            if let Some(path) = persist_tool_result_output(
                self.workspace_root(),
                model_tool_name,
                call_id,
                &original,
            )? {
                execution.view.insert_neutral_metadata(
                    "result_policy_persisted_path".to_string(),
                    path.display().to_string(),
                );
                preview.push_str("\n\n[output truncated; full output persisted at ");
                preview.push_str(path.display().to_string().as_str());
                preview.push(']');
            }
        } else {
            preview.push_str("\n\n[output truncated by tool result policy]");
        }

        execution.view.set_neutral_output(preview);
        Ok(())
    }

    pub(crate) fn apply_model_output_boundary(
        &self,
        model_tool_name: &str,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
    ) -> Result<(), ToolError> {
        let contextual = model_output_boundary_context(execution);
        if contextual.trim().is_empty()
            || !model_output_exceeds_boundary(
                contextual.as_str(),
                TOOL_MODEL_OUTPUT_MAX_LINES,
                TOOL_MODEL_OUTPUT_MAX_BYTES,
            )
        {
            return Ok(());
        }

        let Some(path) = persist_tool_result_output(
            self.workspace_root(),
            model_tool_name,
            call_id,
            contextual.as_str(),
        )?
        else {
            return Ok(());
        };

        let path_text = path.display().to_string();
        let marker = format!(
            "... output truncated ({} lines, {} bytes); full content saved to {path_text} ...",
            line_count(contextual.as_str()),
            contextual.len(),
        );
        let preview = bounded_model_output_preview(
            contextual.as_str(),
            marker.as_str(),
            TOOL_MODEL_OUTPUT_MAX_LINES,
            TOOL_MODEL_OUTPUT_MAX_BYTES,
        );

        if execution.view.output_text.trim().is_empty()
            || model_output_exceeds_boundary(
                execution.view.output_text.as_str(),
                TOOL_MODEL_OUTPUT_MAX_LINES,
                TOOL_MODEL_OUTPUT_MAX_BYTES,
            )
        {
            execution.view.set_neutral_output(preview);
        } else if !execution.view.output_text.contains(marker.as_str()) {
            let mut output = execution.view.output_text.clone();
            output.push_str("\n\n");
            output.push_str(marker.as_str());
            execution.view.set_neutral_output(output);
        }

        compact_tool_output_payload_for_model(
            &mut execution.output,
            path_text.as_str(),
            contextual.len(),
        )?;
        execution.output.mark_truncated(path_text.clone());
        execution
            .view
            .insert_neutral_metadata("model_output_truncated", "true");
        execution
            .view
            .insert_neutral_metadata("model_output_full_path", path_text);
        execution.view.insert_neutral_metadata(
            "model_output_original_bytes".to_string(),
            contextual.len().to_string(),
        );
        Ok(())
    }

    /// Fire-and-forget notification to plugins about a tool execution failure.
    pub fn broadcast_tool_failure(
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
        let input_value = invocation_input_json(invocation)
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .unwrap_or(serde_json::Value::Null);
        let failure_input = PluginToolFailureInput {
            tool: hook_tool,
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            input: input_value,
            failure: failure.into(),
        };
        let plugins = Arc::clone(&self.plugins);
        tokio::spawn(async move {
            plugins.broadcast_tool_failure(failure_input).await;
        });
    }

    pub fn broadcast_notification(
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
        let plugins = Arc::clone(&self.plugins);
        let input = agena_plugin_host::NotificationInput {
            kind: kind.into(),
            session_id,
            title: title.into(),
            message: message.into(),
            payload,
        };
        tokio::spawn(async move {
            plugins.broadcast_notification(input).await;
        });
    }
}
use super::{
    Arc, Path, PluginShellEnvInput, PluginToolAfterInput, PluginToolFailureInput,
    SdkToolResultPolicy, TOOL_MODEL_OUTPUT_MAX_BYTES, TOOL_MODEL_OUTPUT_MAX_LINES, ToolError,
    ToolExecutor, ToolInvocation, ToolInvocationExecution, ToolOutput,
    bounded_model_output_preview, compact_tool_output_payload_for_model, invocation_input_json,
    invocation_name, line_count, model_output_boundary_context, model_output_exceeds_boundary,
    persist_tool_result_output, truncate_to_char_count,
};
