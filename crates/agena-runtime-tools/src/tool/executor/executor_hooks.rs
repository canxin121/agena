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
        original_payload: Option<&serde_json::Value>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let definition = self
            .invocation_definition(invocation)
            .ok_or_else(|| self.unknown_tool_error(invocation.name.as_str()))?;
        // The domain projection normalizes missing/null payloads to an empty
        // object. Validate the producer's value before that distinction is
        // lost, using the same compiled contract as the host boundary.
        definition
            .validate_output(original_payload)
            .map_err(|error| {
                ToolError::from_plugin_error(agena_plugin_host::schema::output_contract_error(
                    error,
                ))
            })?;
        execution.view.normalize_presentation();
        let raw_execution = execution.clone();
        if let Err(error) = self
            .apply_after_hooks_async(
                &definition,
                session_id,
                call_id,
                &mut execution,
                original_payload,
            )
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
        definition: &agena_plugin_host::registry::RegisteredTool,
        session_id: i64,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
        original_payload: Option<&serde_json::Value>,
    ) -> Result<(), ToolError> {
        let summary = execution.summary();
        let after_in = PluginToolAfterInput {
            tool: definition.tool_key().clone(),
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            title: summary.title,
            summary: summary.summary,
            output_text: summary.output_text,
            payload: original_payload.cloned(),
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
            definition
                .validate_output(Some(&payload_value))
                .map_err(|error| {
                    ToolError::from_plugin_error(agena_plugin_host::schema::output_contract_error(
                        error,
                    ))
                })?;
            execution.output =
                ToolOutput::from_json_payload(Some(&payload_value)).map_err(|error| {
                    ToolError::from_plugin_error(agena_plugin_host::schema::output_contract_error(
                        error,
                    ))
                })?;
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
        let Some(hook_tool) = self
            .invocation_definition(invocation)
            .map(|entry| entry.tool_key().clone())
            .or_else(|| invocation.name.parse().ok())
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
        let registered = self.invocation_definition(invocation);
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

        rendered.model.get_or_insert_with(|| {
            crate::tool::human_view::BuiltinHumanRenderer::inline_reply_model(
                &invocation.name,
                output,
            )
            .unwrap_or_else(|| raw_model_fallback(output))
        });
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
        if invocation.name == "image_generation"
            && let Some(human) = rendered.human.as_mut()
        {
            let title = human.title.trim();
            let summary = human.summary.trim();
            // The hosted image title already carries its compact result fact.
            // Clear only an exact repeated segment, preserving independent
            // plugin summaries and every body block.
            if !summary.is_empty()
                && (title == summary
                    || title
                        .strip_suffix(summary)
                        .is_some_and(|prefix| prefix.trim_end().ends_with('·')))
            {
                human.summary.clear();
            }
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
};

#[cfg(test)]
mod output_contract_tests {
    use super::*;
    use crate::authorization::ExecutionPrincipal;
    use crate::permission::{PermissionPolicy, ToolPermissionPolicy};
    use agena_plugin_host::sdk::{ToolContract, ToolDefinition};
    use agena_plugin_host::{PluginHost, registry::RegisteredTool};
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct PresentationHook(Arc<AtomicUsize>);

    #[agena_plugin_host::sdk::async_trait]
    impl agena_plugin_host::sdk::Plugin for PresentationHook {
        fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
            let mut manifest =
                agena_plugin_host::sdk::PluginManifest::new("test", "after", "1.0.0");
            manifest.hooks = agena_plugin_host::sdk::HookSubscription::TOOL_AFTER;
            manifest
        }

        async fn tool_execute_after(
            &self,
            input: agena_plugin_host::sdk::ToolAfterInput,
        ) -> agena_plugin_host::sdk::Result<Option<agena_plugin_host::sdk::ToolAfterPatch>>
        {
            assert!(input.payload.is_none());
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(Some(agena_plugin_host::sdk::ToolAfterPatch {
                summary: Some("Presentation updated".into()),
                ..Default::default()
            }))
        }
    }

    fn executor(output_schema: serde_json::Value) -> (ToolExecutor, ToolInvocation) {
        let tool = RegisteredTool::new(
            "test.output".parse().unwrap(),
            ToolDefinition {
                name: "run".into(),
                contract: ToolContract {
                    input_schema: json!({"type":"object"}),
                    output_schema,
                },
                docs: Default::default(),
                runtime: Default::default(),
                tags: vec![],
            },
        )
        .unwrap();
        let invocation = ToolInvocation::new(tool.canonical_name(), Default::default());
        let mut executor = ToolExecutor::new(
            std::env::temp_dir(),
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            PluginHost::new_empty(),
            None,
            None,
        );
        executor.definition_catalog = Some(Arc::new(vec![tool]));
        (executor, invocation)
    }

    fn execution() -> ToolInvocationExecution {
        ToolInvocationExecution::new(
            ToolOutput::default(),
            crate::tool::ToolExecutionView::simple("Run", "Completed", ""),
        )
    }

    #[tokio::test]
    async fn hosted_image_projection_has_one_caption_and_no_repeated_headline_summary() {
        let (executor, _) = executor(json!({"type": "object"}));
        let invocation = ToolInvocation::new(
            "image_generation",
            agena_domain::StructuredObject::try_from(json!({"prompt": "Draw a rose"})).unwrap(),
        );
        let output: agena_domain::RawOutput = serde_json::from_value(json!({
            "payload": {
                "text": "A rose in a vase, lit from the left",
                "warning": "The provider adjusted the requested dimensions",
            },
            "attachments": [{
                "kind": "image", "mime": "image/png", "filename": "rose.png",
                "source": {"source": "local_path", "path": "/generated/rose.png"},
            }],
        }))
        .unwrap();
        let original = output.clone();
        let rendered = executor.render_tool_result(&invocation, &output).await;
        let human = rendered.human.unwrap();
        assert_eq!(human.title, "Generate image · 1 image");
        assert!(human.summary.is_empty());
        let body = human
            .blocks
            .iter()
            .filter_map(agena_domain::ViewBlock::text_value)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            body.matches("A rose in a vase, lit from the left").count(),
            1
        );
        assert!(body.contains("The provider adjusted the requested dimensions"));
        assert_eq!(
            human
                .blocks
                .iter()
                .filter(|block| matches!(block, agena_domain::ViewBlock::Media { .. }))
                .count(),
            1
        );
        assert!(
            rendered
                .model
                .unwrap()
                .contains("A rose in a vase, lit from the left")
        );
        assert_eq!(output, original);
    }

    #[tokio::test]
    async fn finalization_distinguishes_absent_payload_from_an_empty_object() {
        let (null_executor, invocation) = executor(json!({"type":"null"}));
        assert!(
            null_executor
                .finalize_execution_async(&invocation, 1, &invocation.name, 1, execution(), None,)
                .await
                .is_ok()
        );
        assert!(
            null_executor
                .finalize_execution_async(
                    &invocation,
                    1,
                    &invocation.name,
                    1,
                    execution(),
                    Some(&json!({})),
                )
                .await
                .is_err()
        );
        let (object_executor, invocation) = executor(json!({"type":"object"}));
        assert!(
            object_executor
                .finalize_execution_async(
                    &invocation,
                    1,
                    &invocation.name,
                    1,
                    execution(),
                    Some(&json!({})),
                )
                .await
                .is_ok()
        );
        assert!(
            object_executor
                .finalize_execution_async(&invocation, 1, &invocation.name, 1, execution(), None,)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn absent_output_survives_a_hook_that_only_changes_presentation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let hook_key = "test.after".parse().unwrap();
        let host = PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
            static_plugins: vec![agena_plugin_host::StaticPluginRegistration::new(
                hook_key,
                PresentationHook(calls.clone()),
            )],
            config: agena_plugin_host::PluginsConfig {
                list: [("test.after".into(), Default::default())].into(),
                ..Default::default()
            },
            workspace_root: std::env::temp_dir(),
            agena_version: env!("CARGO_PKG_VERSION").into(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .unwrap();
        let (mut executor, invocation) = executor(json!({"type":"null"}));
        executor.plugins = host;
        let execution = executor
            .finalize_execution_async(&invocation, 1, &invocation.name, 1, execution(), None)
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(execution.view.summary, "Presentation updated");
        assert!(
            !execution
                .view
                .metadata
                .contains_key("postprocessing_warning")
        );
    }
}
