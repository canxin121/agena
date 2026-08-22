use crate::{Application, ApplicationError, dto::OperatorToolResource};

impl Application {
    /// Produce ephemeral model/human views from the invocation and the sole
    /// durable raw result. Callers may serialize the human side in a response,
    /// but must never persist either projection.
    pub async fn render_tool_result(
        &self,
        invocation: &agena_domain::ToolInvocation,
        output: &agena_domain::RawOutput,
    ) -> agena_runtime::RuntimeToolResultProjection {
        self.runtime_tools()
            .render_tool_result(invocation, output)
            .await
    }

    pub(crate) async fn render_transcript_tool_presentations(
        &self,
        parts: &mut [agena_api::resource::SessionTranscriptPart],
    ) {
        for part in parts {
            if part.kind != agena_runtime_contracts::part_content::ToolCallContent::kind() {
                continue;
            }
            let Ok(content) =
                agena_runtime_contracts::part_content::ToolCallContent::try_from(&part.content)
            else {
                part.content = agena_api::live::project_tool_call_content(
                    &part.content,
                    &[agena_api::live::ToolDetailSection::Presentation],
                );
                continue;
            };
            let Ok(input) = agena_domain::StructuredObject::try_from(content.input) else {
                part.content = agena_api::live::project_tool_call_content(
                    &part.content,
                    &[agena_api::live::ToolDetailSection::Presentation],
                );
                continue;
            };
            let invocation = agena_domain::ToolInvocation {
                tool_api_call: content.tool_api_call,
                name: content.name,
                plugin_name: content.plugin,
                input,
            };
            let human = if let Some(output) = content.output {
                self.render_tool_result(&invocation, &output).await.human
            } else {
                agena_runtime::RuntimeToolHumanPresentation {
                    title: invocation.name,
                    ..Default::default()
                }
            };
            part.presentation = Some(agena_api::live::ToolHumanPresentationResource {
                title: human.title,
                summary: human.summary,
                blocks: human.blocks,
            });
            part.content = agena_api::live::project_tool_call_content(
                &part.content,
                &[agena_api::live::ToolDetailSection::Presentation],
            );
        }
    }

    pub async fn list_operator_tools(&self) -> Vec<OperatorToolResource> {
        self.runtime_tools()
            .available_runtime_tools()
            .await
            .into_iter()
            .map(|tool| OperatorToolResource {
                name: tool.name,
                summary: tool.summary,
                before_help: tool.before_help,
                after_help: tool.after_help,
                input_schema: tool.input_schema,
                output_schema: tool.output_schema,
                interactive: tool.interactive,
                read_only: tool.read_only,
                destructive: tool.destructive,
                open_world: tool.open_world,
                task: tool.task,
                plugin_id: tool.plugin_id,
            })
            .collect()
    }

    pub async fn invoke_operator_tool(
        &self,
        workspace_id: i64,
        tool: &str,
        input: Option<serde_json::Value>,
        call_id: i64,
    ) -> Result<agena_tool::ToolExecutionSummary, ApplicationError> {
        self.ensure_operator_workspace(workspace_id).await?;
        let tool = tool.trim();
        if tool.is_empty() {
            return Err(ApplicationError::bad_request("tool name is required"));
        }
        let input = input.unwrap_or_else(|| serde_json::json!({}));
        let input = agena_domain::StructuredObject::try_from(input).map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("tool input must be an object", error)
        })?;
        let invocation = agena_domain::ToolInvocation::new(tool.to_owned(), input);
        self.runtime_tools()
            .execute_runtime_tool(&invocation, call_id)
            .await
            .map(agena_runtime::SessionToolExecutionOutcome::into_summary)
            .map_err(|error| ApplicationError::internal_error(&error))
    }

    async fn ensure_operator_workspace(&self, workspace_id: i64) -> Result<(), ApplicationError> {
        let workspace = self
            .service()
            .get_workspace(workspace_id)
            .await?
            .ok_or_else(|| {
                ApplicationError::not_found_with_diagnostic(
                    "The operator workspace was not found.",
                    format!("operator workspace not found: {workspace_id}"),
                )
            })?;
        let server_root = self.workspace_root().to_path_buf();
        let requested_root = std::path::PathBuf::from(workspace.path);
        let (server_root, requested_root) = tokio::task::spawn_blocking(move || {
            let server_root = std::fs::canonicalize(&server_root).map_err(|error| {
                format!(
                    "failed to canonicalize server workspace {}: {error}",
                    server_root.display()
                )
            })?;
            let requested_root = std::fs::canonicalize(&requested_root).map_err(|error| {
                format!(
                    "failed to canonicalize requested workspace {}: {error}",
                    requested_root.display()
                )
            })?;
            Ok::<_, String>((server_root, requested_root))
        })
        .await
        .map_err(|error| {
            ApplicationError::internal(format!(
                "operator workspace canonicalization task failed: {error}"
            ))
        })?
        .map_err(|diagnostic| {
            ApplicationError::bad_request_with_diagnostic(
                "The operator workspace cannot be resolved.",
                diagnostic,
            )
        })?;
        if server_root != requested_root {
            return Err(ApplicationError::conflict_with_diagnostic(
                "The operator workspace does not match this server.",
                format!(
                    "operator workspace mismatch: requested={}, server={}",
                    requested_root.display(),
                    server_root.display()
                ),
            ));
        }
        Ok(())
    }
}
