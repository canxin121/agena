use crate::{Application, ApplicationError, dto::OperatorToolResource};

type DocumentKey = (i64, i64, (u64, u64));
type DocumentCell = std::sync::Arc<tokio::sync::OnceCell<Option<agena_domain::PartDocument>>>;

/// Shared by list, detail and live reads. No client or transport dimension.
/// Entries share initialization and cache only bounded semantic documents.
#[derive(Default)]
pub(crate) struct PartDocumentCache {
    entries: std::sync::Mutex<std::collections::BTreeMap<DocumentKey, DocumentCell>>,
}

impl PartDocumentCache {
    fn cell(&self, key: DocumentKey) -> DocumentCell {
        let mut entries = self.entries.lock().expect("part document cache lock");
        if let Some(cell) = entries.get(&key) {
            return cell.clone();
        }
        while entries.len() >= 256 {
            entries.pop_first();
        }
        let cell = std::sync::Arc::new(tokio::sync::OnceCell::new());
        entries.insert(key, cell.clone());
        cell
    }

    fn remove_oversized(&self, key: DocumentKey, cell: &DocumentCell) {
        let mut entries = self.entries.lock().expect("part document cache lock");
        if entries
            .get(&key)
            .is_some_and(|entry| std::sync::Arc::ptr_eq(entry, cell))
        {
            entries.remove(&key);
        }
    }
}

impl Application {
    pub async fn part_document(
        &self,
        part: &agena_api::part::PartResource,
    ) -> Option<agena_domain::PartDocument> {
        let key = (
            part.part_id,
            part.revision,
            self.plugin_runtime().projection_generation(),
        );
        let cell = self.part_documents.cell(key);
        cell.get_or_init(|| async {
            let document = self
                .render_part_presentation(&part.kind, &part.content, part.summary.as_deref())
                .await;
            if document.as_ref().is_some_and(|document| {
                serde_json::to_vec(document).map_or(true, |bytes| bytes.len() > 64 * 1024)
            }) {
                self.part_documents.remove_oversized(key, &cell);
            }
            document
        })
        .await
        .clone()
    }

    /// Facts and semantic sections use one projection across all transports.
    pub async fn project_part(
        &self,
        fact: &agena_storage::store::Part,
        sections: &[agena_api::live::ToolDetailSection],
    ) -> agena_api::part::PartResource {
        let mut part = crate::session::part_resource_from_fact(fact);
        part.sections = sections
            .iter()
            .map(|section| agena_api::part::LoadedPartSection {
                section: *section,
                revision: fact.revision,
            })
            .collect();
        if sections.contains(&agena_api::live::ToolDetailSection::Presentation) {
            part.presentation = self.part_document(&part).await;
        }
        if fact.kind == agena_runtime_contracts::part_content::ToolCallContent::kind() {
            part.content = agena_api::live::project_tool_call_content(&part.content, sections);
        }
        part
    }
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

    /// The single semantic document projection used by snapshots, detail
    /// reads and live updates. Resource identity survives tool completion;
    /// layout and disclosure policy remain the consumer's responsibility.
    pub async fn render_part_presentation(
        &self,
        kind: &str,
        value: &serde_json::Value,
        summary: Option<&str>,
    ) -> Option<agena_domain::PartDocument> {
        use agena_domain::PartDocument;
        use agena_domain::{ContentKind, ViewBlock};
        use agena_runtime_contracts::part_content::ToolCallContent;

        if kind != ToolCallContent::kind() {
            return summary
                .filter(|summary| !summary.trim().is_empty())
                .map(|summary| PartDocument {
                    title: String::new(),
                    summary: summary.to_owned(),
                    blocks: Vec::new(),
                });
        }
        let content = ToolCallContent::try_from(value).ok()?;
        let input = agena_domain::StructuredObject::try_from(content.input.clone()).ok()?;
        let invocation = agena_domain::ToolInvocation {
            tool_api_call: content.tool_api_call.clone(),
            name: content.name.clone(),
            plugin_name: content.plugin.clone(),
            input,
        };
        let mut blocks = Vec::new();
        let shell = matches!(
            agena_domain::ToolPermissionConfig::canonical_shell_tool_name(&content.name),
            Some("agena.shell.exec" | "agena.shell.spawn" | "agena.shell.open")
        );
        if shell && !content.resources.is_empty() {
            let command = match content.input.get("command") {
                Some(serde_json::Value::String(command)) => command.clone(),
                Some(serde_json::Value::Array(args)) => args
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            if !command.is_empty() {
                blocks.push(ViewBlock::Command {
                    id: Some("command".to_owned()),
                    command,
                    cwd: content
                        .input
                        .get("workdir")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    exit_code: content
                        .output
                        .as_ref()
                        .and_then(|output| output.payload.as_ref())
                        .and_then(|payload| payload.get("exit_code"))
                        .and_then(serde_json::Value::as_i64)
                        .and_then(|code| i32::try_from(code).ok()),
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
        }
        let trace = content
            .metadata
            .get("agena.provider_trace")
            .and_then(|value| {
                serde_json::from_value::<agena_domain::ContentRef>(value.clone()).ok()
            });
        blocks.extend(
            content
                .resources
                .iter()
                .filter(|resource| {
                    !trace
                        .as_ref()
                        .is_some_and(|trace| trace.resource_id == resource.resource_id)
                })
                .map(|resource| ViewBlock::Content {
                    id: format!("output:{}", resource.resource_id),
                    resource: resource.clone(),
                    format: content
                        .output
                        .as_ref()
                        .and_then(|output| {
                            output
                                .fields
                                .iter()
                                .find(|field| field.resource.resource_id == resource.resource_id)
                        })
                        .map_or(agena_domain::ContentFormat::Plain, |field| field.format),
                }),
        );
        let Some(output) = content.output.as_ref() else {
            return Some(PartDocument {
                title: agena_tool::tool_title_for_state(&invocation, content.state),
                summary: summary.unwrap_or_default().to_owned(),
                blocks,
            });
        };
        let projection = self.render_tool_result(&invocation, output).await;
        if !content
            .resources
            .iter()
            .any(|resource| matches!(resource.kind, ContentKind::Log | ContentKind::Terminal))
        {
            blocks.extend(projection.human.blocks.into_iter().filter(|block| {
                content
                    .output
                    .as_ref()
                    .is_none_or(|output| output.fields.is_empty())
                    || block.text_value() != Some("No output.")
            }));
        }
        Some(PartDocument {
            title: agena_tool::completed_tool_title_for_state(&invocation, content.state, output),
            summary: projection.human.summary,
            blocks,
        })
    }

    pub(crate) async fn render_transcript_presentations(
        &self,
        parts: &mut [agena_api::part::PartResource],
    ) {
        for part in parts {
            part.presentation = self.part_document(part).await;
            part.sections = vec![agena_api::part::LoadedPartSection {
                section: agena_api::live::ToolDetailSection::Presentation,
                revision: part.revision,
            }];
            if part.kind == agena_runtime_contracts::part_content::ToolCallContent::kind() {
                part.content = agena_api::live::project_tool_call_content(
                    &part.content,
                    &[agena_api::live::ToolDetailSection::Presentation],
                );
            }
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
