use crate::{Application, ApplicationError, dto::OperatorToolResource};

type DocumentKey = (i64, i64, (u64, u64));
type DocumentCell = std::sync::Arc<tokio::sync::OnceCell<Option<agena_domain::PartDocument>>>;

/// Hosted image results may already have a text resource containing the same
/// bytes as their image attachment. Render the attachment, keeping unrelated
/// text resources and the durable raw result available for explicit inspection.
fn tool_content_for_presentation(
    value: &serde_json::Value,
) -> Option<agena_runtime_contracts::part_content::ToolCallContent> {
    use agena_domain::{AttachmentKind, ContentFormat};
    use agena_runtime_contracts::part_content::ToolCallContent;

    let mut content = ToolCallContent::try_from(value).ok()?;
    // Provider diagnostics have a separate metadata reference. They are never
    // part of the human output, including the client's resource fallback.
    if let Some(trace) = content
        .metadata
        .get("agena.provider_trace")
        .and_then(|value| serde_json::from_value::<agena_domain::ContentRef>(value.clone()).ok())
    {
        content
            .resources
            .retain(|resource| resource.resource_id != trace.resource_id);
    }
    let Some(output) = content.output.as_mut() else {
        return Some(content);
    };
    let image_result = output
        .payload
        .as_ref()
        .and_then(|payload| payload.get("type"))
        .and_then(serde_json::Value::as_str)
        == Some("image_generation_call")
        && output
            .attachments
            .iter()
            .any(|attachment| attachment.kind == AttachmentKind::Image);
    if image_result {
        if let Some(payload) = output
            .payload
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
        {
            payload.remove("result");
        }
        let binary_resources = output
            .fields
            .iter()
            .filter(|field| field.pointer == "/result" && field.format == ContentFormat::Plain)
            .map(|field| field.resource.resource_id)
            .collect::<std::collections::BTreeSet<_>>();
        output
            .fields
            .retain(|field| !binary_resources.contains(&field.resource.resource_id));
        content
            .resources
            .retain(|resource| !binary_resources.contains(&resource.resource_id));
    }
    Some(content)
}

fn project_tool_content(
    value: &serde_json::Value,
    sections: &[agena_api::live::ToolDetailSection],
) -> serde_json::Value {
    let mut projected = agena_api::live::project_tool_call_content(value, sections);
    if value.get("resources").is_some()
        && let Some(content) = tool_content_for_presentation(value)
    {
        projected["resources"] = serde_json::to_value(content.resources)
            .expect("content references are JSON serializable");
    }
    projected
}

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

    /// Read-time detail projection never grants write access to a transcript entry.
    pub(crate) async fn project_part_resource(
        &self,
        part: &mut agena_api::part::PartResource,
        sections: &[agena_api::live::ToolDetailSection],
    ) {
        part.sections = sections
            .iter()
            .map(|section| agena_api::part::LoadedPartSection {
                section: *section,
                revision: part.revision,
            })
            .collect();
        if sections.contains(&agena_api::live::ToolDetailSection::Presentation) {
            part.presentation = self.part_document(part).await;
        }
        if part.kind == agena_runtime_contracts::part_content::ToolCallContent::kind() {
            part.content = project_tool_content(&part.content, sections);
        }
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
        let content = tool_content_for_presentation(value)?;
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
        blocks.extend(content.resources.iter().map(|resource| {
            ViewBlock::Content {
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
            }
        }));
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
                part.content = project_tool_content(
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

#[cfg(test)]
mod media_presentation_tests {
    use super::*;
    use agena_domain::{
        ContentField, ContentFormat, ContentId, ContentKind, ContentRef, RawOutput,
    };
    use agena_runtime_contracts::part_content::ToolCallContent;
    use serde_json::json;

    fn image_result() -> ToolCallContent {
        let reference = || ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Text,
        };
        let binary = reference();
        let trace = reference();
        let caption = reference();
        ToolCallContent {
            name: "image_generation".into(),
            input: json!({}),
            resources: vec![binary.clone(), trace.clone(), caption.clone()],
            metadata: std::collections::BTreeMap::from([(
                "agena.provider_trace".into(),
                serde_json::to_value(trace).unwrap(),
            )]),
            output: Some(RawOutput {
                payload: Some(
                    json!({"type":"image_generation_call", "result":null, "status":"completed"}),
                ),
                attachments: vec![
                    serde_json::from_value(json!({
                        "kind":"image", "mime":"image/png", "filename":"generated-image.png",
                        "source":{"source":"data_url", "url":"data:image/png;base64,aW1hZ2U="}
                    }))
                    .unwrap(),
                ],
                fields: vec![
                    ContentField {
                        pointer: "/result".into(),
                        resource: binary,
                        format: ContentFormat::Plain,
                    },
                    ContentField {
                        pointer: "/caption".into(),
                        resource: caption,
                        format: ContentFormat::Markdown,
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn media_presentation_excludes_only_binary_result_and_diagnostic_resources() {
        let original = image_result().as_value();
        let human = tool_content_for_presentation(&original).unwrap();
        assert_eq!(human.resources.len(), 1);
        let output = human.output.unwrap();
        assert_eq!(output.fields.len(), 1);
        assert_eq!(output.fields[0].pointer, "/caption");
        assert_eq!(output.fields[0].resource, human.resources[0]);
        assert_eq!(output.attachments.len(), 1);
        assert!(output.payload.unwrap().get("result").is_none());

        for sections in [
            vec![agena_api::live::ToolDetailSection::Presentation],
            agena_api::live::ToolDetailSection::ALL.to_vec(),
        ] {
            let projected = project_tool_content(&original, &sections);
            assert_eq!(projected["resources"].as_array().unwrap().len(), 1);
            if sections.contains(&agena_api::live::ToolDetailSection::Output) {
                assert_eq!(
                    projected["output"], original["output"],
                    "explicit raw detail stays lossless"
                );
            }
        }
        assert_eq!(original["resources"].as_array().unwrap().len(), 3);
        assert_eq!(original["output"]["fields"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn media_presentation_preserves_result_text_without_an_image_attachment() {
        let mut content = image_result();
        content.output.as_mut().unwrap().attachments.clear();
        let human = tool_content_for_presentation(&content.as_value()).unwrap();
        assert_eq!(human.resources.len(), 2, "only the diagnostic is hidden");
        assert_eq!(human.output.unwrap().fields.len(), 2);

        let mut content = image_result();
        content.output.as_mut().unwrap().payload =
            Some(json!({"type":"other_tool", "result":"useful text"}));
        let human = tool_content_for_presentation(&content.as_value()).unwrap();
        assert_eq!(human.resources.len(), 2);
        assert_eq!(
            human.output.unwrap().payload.unwrap()["result"],
            "useful text"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn media_part_projection_keeps_images_and_captions_across_snapshot_paths() {
        use agena_api::live::ToolDetailSection;

        let workspace = tempfile::tempdir().unwrap();
        let runtime =
            agena_runtime::bootstrap_application_services(agena_runtime::RuntimeBootstrapRequest {
                workspace_root: Some(workspace.path().to_path_buf()),
                config_path: Some(workspace.path().join("global.json")),
                database_url: Some("sqlite::memory:".into()),
                scheduler_database_path: Some(workspace.path().join("scheduler.db")),
                ..Default::default()
            })
            .await
            .unwrap();
        let app =
            Application::from_composed_runtime_services(runtime.application_services()).unwrap();
        let mut content = image_result();
        content.state = agena_domain::ToolResultState::Completed;
        let mut projected = agena_api::part::PartResource {
            message_boundary: false,
            part_id: 45589,
            kind: "tool_call".into(),
            role: "assistant".into(),
            state: "completed".into(),
            content: content.as_value(),
            revision: 1,
            ..Default::default()
        };
        let raw = projected.clone();
        app.project_part_resource(&mut projected, &[ToolDetailSection::Presentation])
            .await;
        assert_eq!(projected.content["resources"].as_array().unwrap().len(), 1);
        let document = projected.presentation.as_ref().unwrap();
        assert_eq!(
            document
                .blocks
                .iter()
                .filter(|block| matches!(block, agena_domain::ViewBlock::Media { .. }))
                .count(),
            1
        );
        let output_resources = document
            .blocks
            .iter()
            .filter_map(|block| match block {
                agena_domain::ViewBlock::Content { resource, .. } => Some(resource),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            output_resources.len(),
            1,
            "only the caption is a text output"
        );
        assert_eq!(output_resources[0], &content.resources[2]);
        assert_eq!(
            serde_json::to_string(document)
                .unwrap()
                .matches("aW1hZ2U=")
                .count(),
            1
        );

        let mut transcript = vec![raw.clone()];
        app.render_transcript_presentations(&mut transcript).await;
        assert_eq!(transcript[0].content, projected.content);
        assert_eq!(transcript[0].presentation, projected.presentation);
        let mut detail = raw.clone();
        app.project_part_resource(&mut detail, &ToolDetailSection::ALL)
            .await;
        assert_eq!(detail.content["output"], raw.content["output"]);
        assert_eq!(detail.presentation, projected.presentation);
        assert_eq!(raw.content["resources"].as_array().unwrap().len(), 3);
    }
}
