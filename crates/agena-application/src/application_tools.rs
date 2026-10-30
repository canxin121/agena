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
    // A PTY's raw log and screen are two projections of the same output.
    // Keep the raw capture in storage, but display and subscribe to the screen
    // once. This also applies before a running tool has returned a result.
    if agena_domain::ToolPermissionConfig::canonical_shell_tool_name(&content.name).is_some()
        && content
            .resources
            .iter()
            .any(|resource| resource.kind == agena_domain::ContentKind::Terminal)
    {
        content
            .resources
            .retain(|resource| resource.kind != agena_domain::ContentKind::Log);
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

/// Rejected inputs may not form a renderable invocation. Keep their public
/// failure visible even when there is no tool document, without repeating the
/// ordinary error band in otherwise valid tool presentations.
pub(super) fn tool_document_with_failure(
    content: &agena_runtime_contracts::part_content::ToolCallContent,
    document: Option<agena_domain::PartDocument>,
) -> Option<agena_domain::PartDocument> {
    use agena_domain::{PartDocument, ViewBlock};

    if document.is_some() {
        return document;
    }
    let Some(error) = &content.error else {
        return document;
    };
    let message = error.user_message();
    if message.trim().is_empty() {
        return document;
    }
    Some(PartDocument {
        title: content.name.clone(),
        summary: String::new(),
        blocks: std::iter::once(ViewBlock::Text {
            id: Some("tool-error".into()),
            // Model feedback and private diagnostics are deliberately not
            // part of the human document.
            text: message.to_owned(),
        })
        .chain(content.resources.iter().map(|resource| ViewBlock::Content {
            id: format!("output:{}", resource.resource_id),
            resource: resource.clone(),
            format: agena_domain::ContentFormat::Plain,
        }))
        .collect(),
    })
}

/// The final state-aware title can already include the human summary as one
/// complete result suffix. Keep that fact once, without treating substrings,
/// truncated titles or independent plugin summaries as duplicates.
fn deduplicate_tool_document_summary(
    mut document: agena_domain::PartDocument,
) -> agena_domain::PartDocument {
    let title = document.title.trim();
    let summary = document.summary.trim();
    if !summary.is_empty()
        && (title == summary
            || title
                .strip_suffix(summary)
                .is_some_and(|prefix| prefix.ends_with(" · ")))
    {
        document.summary.clear();
    }
    document
}

#[cfg(test)]
mod tool_document_summary_tests {
    use super::*;
    use agena_domain::{
        ArtifactRef, PartDocument, RawOutput, StructuredObject, ToolInvocation, ToolResultState,
        ViewBlock,
    };
    use serde_json::json;

    fn document(title: &str, summary: &str) -> PartDocument {
        PartDocument {
            title: title.to_owned(),
            summary: summary.to_owned(),
            blocks: vec![ViewBlock::Text {
                id: Some("output".into()),
                text: "Independent output remains available.".into(),
            }],
        }
    }

    #[test]
    fn completed_read_media_count_is_displayed_once_and_keeps_its_media() {
        let invocation = ToolInvocation::new(
            "fs.read_media",
            StructuredObject::try_from(json!({"path":"ai-drawings/watercolor-cat.png"})).unwrap(),
        );
        let output = RawOutput {
            payload: Some(json!({"loaded_paths":["ai-drawings/watercolor-cat.png"]})),
            ..Default::default()
        };
        let title = agena_tool::completed_tool_title_for_state(
            &invocation,
            ToolResultState::Completed,
            &output,
        );
        let summary = agena_tool::result_title_fragment_for_tool(&invocation.name, &output);
        assert_eq!(
            title,
            "Read media · ai-drawings/watercolor-cat.png · 1 file loaded"
        );
        assert_eq!(summary, "1 file loaded");
        let mut before = document(&title, &summary);
        before.blocks.push(ViewBlock::Media {
            id: Some("media".into()),
            artifact: ArtifactRef {
                uri: "agena:media:v1:test-reference".into(),
                mime: "image/png".into(),
                name: Some("watercolor-cat.png".into()),
                size_bytes: Some(1690348),
                sha256: None,
            },
        });
        let after = deduplicate_tool_document_summary(before.clone());
        assert_eq!(after.title, before.title);
        assert!(after.summary.is_empty());
        assert_eq!(after.blocks, before.blocks);
    }

    #[test]
    fn summary_deduplication_requires_an_exact_complete_title_suffix() {
        for (title, summary) in [
            ("1 file loaded", "1 file loaded"),
            ("Read media · file.png · 1 file loaded", "1 file loaded"),
            (
                "Read · 1 file loaded · truncated",
                "1 file loaded · truncated",
            ),
            ("  Read · 1 file loaded  ", " 1 file loaded "),
            ("读取媒体 · 已载入 1 个文件", "已载入 1 个文件"),
        ] {
            let before = document(title, summary);
            let after = deduplicate_tool_document_summary(before.clone());
            assert!(after.summary.is_empty(), "{title:?} / {summary:?}");
            assert_eq!(after.title, before.title);
            assert_eq!(after.blocks, before.blocks);
        }
        for (title, summary) in [
            ("Read · 11 files loaded", "1 files loaded"),
            ("Read · cached1 file loaded", "1 file loaded"),
            ("Read ·1 file loaded", "1 file loaded"),
            ("Read · 1 file lo…", "1 file loaded"),
            ("Read · 1 file loaded", "1 File Loaded"),
            ("Read · 1 file loaded", "1 file loaded from cache"),
            ("Read · 1 file loaded", "1 file loaded · all bytes verified"),
            ("Read · 1 file loaded", ""),
        ] {
            let before = document(title, summary);
            assert_eq!(deduplicate_tool_document_summary(before.clone()), before);
        }
    }

    #[test]
    fn running_and_failed_titles_keep_independent_result_or_failure_summaries() {
        let invocation = ToolInvocation::new(
            "fs.read_media",
            StructuredObject::try_from(json!({"path":"ai-drawings/watercolor-cat.png"})).unwrap(),
        );
        let output = RawOutput {
            payload: Some(json!({"loaded_paths":["ai-drawings/watercolor-cat.png"]})),
            ..Default::default()
        };
        for state in [
            ToolResultState::Pending,
            ToolResultState::Running,
            ToolResultState::Failed,
        ] {
            let title = agena_tool::completed_tool_title_for_state(&invocation, state, &output);
            for summary in ["1 file loaded", "The remaining file could not be read."] {
                let before = document(&title, summary);
                assert_eq!(deduplicate_tool_document_summary(before.clone()), before);
            }
        }
    }
}

#[cfg(test)]
mod terminal_presentation_tests {
    use super::*;
    use agena_domain::{ContentId, ContentKind, ContentRef};
    use agena_runtime_contracts::part_content::ToolCallContent;

    #[test]
    fn pty_displays_one_projection_before_and_after_tool_completion() {
        let log = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Log,
        };
        let terminal = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Terminal,
        };
        let text = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Text,
        };
        let mut content = ToolCallContent {
            name: "shell.open".into(),
            input: serde_json::json!({"command": "seq 1 10"}),
            resources: vec![log.clone(), terminal.clone(), text.clone()],
            ..Default::default()
        };
        for output in [None, Some(agena_domain::RawOutput::default())] {
            content.output = output;
            let projected = tool_content_for_presentation(&content.as_value()).unwrap();
            assert_eq!(projected.resources, vec![terminal.clone(), text.clone()]);
            assert_eq!(content.resources[0], log);
        }
        content.resources = vec![log.clone()];
        assert_eq!(
            tool_content_for_presentation(&content.as_value())
                .unwrap()
                .resources,
            vec![log]
        );
    }
}

#[cfg(test)]
mod failure_presentation_tests {
    use super::*;
    use agena_domain::{ContentId, ContentKind, ContentRef, PartDocument, ViewBlock};
    use agena_runtime_contracts::part_content::ToolCallContent;

    fn failed_content(name: &str) -> ToolCallContent {
        use agena_failure::*;
        ToolCallContent {
            name: name.into(),
            state: agena_domain::ToolResultState::Failed,
            error: Some(agena_domain::OperationError {
                failure: Failure::new(
                    FailureCode::new("tool.invalid_input"),
                    FailureCategory::InvalidInput,
                    FailureResponsibility::Caller,
                    RetryDirective::CorrectInput,
                    RecoveryDirective::None,
                    FailureImpact::RequestRejected,
                    UserPresentation::new("tool.invalid_input", "file_path is required"),
                )
                .with_model_feedback(
                    ModelFeedback::invalid_input().with_text("private-model-sentinel"),
                ),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn failed_tools_without_output_have_public_reason_and_keep_resources() {
        for name in [
            "shell.exec",
            "fs.read",
            "fs.read_media",
            "chatgpt.image_understanding",
            "tools_list",
            "custom.plugin.tool",
        ] {
            let mut content = failed_content(name);
            // A rejected scalar is intentionally kept as input. It cannot be
            // made into a ToolInvocation merely to render the failure.
            content.input = serde_json::json!(17);
            let resource = ContentRef {
                resource_id: ContentId::new(),
                kind: ContentKind::Text,
            };
            content.resources.push(resource.clone());
            let document = tool_document_with_failure(&content, None).unwrap();
            assert_eq!(document.title, name);
            assert_eq!(
                document.blocks[0].text_value(),
                Some("file_path is required")
            );
            assert!(matches!(
                &document.blocks[1],
                ViewBlock::Content { resource: retained, .. } if retained == &resource
            ));
            assert!(
                !serde_json::to_string(&document)
                    .unwrap()
                    .contains("private-model-sentinel")
            );
            assert!(content.output.is_none());
        }
        assert!(tool_document_with_failure(&ToolCallContent::default(), None).is_none());
    }

    #[test]
    fn existing_tool_document_is_preserved_when_failure_is_available() {
        let content = failed_content("custom.plugin.tool");
        let distinct = ViewBlock::Text {
            id: Some("partial-output".into()),
            text: "Two files were already processed.".into(),
        };
        let base = PartDocument {
            title: "Custom tool".into(),
            summary: "Partial result".into(),
            blocks: vec![distinct.clone()],
        };
        assert_eq!(
            tool_document_with_failure(&content, Some(base.clone())),
            Some(base)
        );
        for same in [
            ViewBlock::Text {
                id: None,
                text: " file_path is required ".into(),
            },
            ViewBlock::Markdown {
                id: None,
                text: "file_path is required".into(),
            },
        ] {
            let base = PartDocument {
                title: "Custom tool".into(),
                summary: String::new(),
                blocks: vec![same, distinct.clone()],
            };
            assert_eq!(
                tool_document_with_failure(&content, Some(base.clone())),
                Some(base)
            );
        }
        let base = PartDocument {
            title: "Custom tool".into(),
            summary: "file_path is required".into(),
            blocks: vec![distinct],
        };
        assert_eq!(
            tool_document_with_failure(&content, Some(base.clone())),
            Some(base)
        );
    }
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
        let document = self.render_tool_part_presentation(&content, summary).await;
        tool_document_with_failure(&content, document).map(deduplicate_tool_document_summary)
    }

    async fn render_tool_part_presentation(
        &self,
        content: &agena_runtime_contracts::part_content::ToolCallContent,
        summary: Option<&str>,
    ) -> Option<agena_domain::PartDocument> {
        use agena_domain::{ContentKind, PartDocument, ViewBlock};

        if let Some(call) = &content.tool_api_call
            && call.function != agena_domain::ToolApiFunction::Call
        {
            // Protocol responses have no owning plugin and use a plain-text
            // presentation; they never enter plugin result rendering.
            let text = content
                .output
                .as_ref()
                .and_then(|output| output.payload.as_ref())
                .and_then(|payload| payload.get("text"))
                .and_then(serde_json::Value::as_str);
            return Some(PartDocument {
                title: call.function.function_name().to_owned(),
                summary: summary.unwrap_or_default().to_owned(),
                blocks: text
                    .into_iter()
                    .map(|text| ViewBlock::Text {
                        id: Some("tool-api-response".into()),
                        text: text.to_owned(),
                    })
                    .chain(content.resources.iter().map(|resource| ViewBlock::Content {
                        id: format!("tool-api-response:{}", resource.resource_id),
                        resource: resource.clone(),
                        format: agena_domain::ContentFormat::Plain,
                    }))
                    .collect(),
            });
        }
        let input = agena_domain::StructuredObject::try_from(content.input.clone()).ok()?;
        let invocation = agena_domain::ToolInvocation {
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
                agent: tool.agent,
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
