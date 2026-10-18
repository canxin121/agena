//! Explicit full-history reads for clipboard, export, and the external pager.
//! These reads never hydrate source bodies into durable Part resources.

use std::{collections::BTreeMap, path::PathBuf};

use agena_api::{live::ToolDetailSection, part::PartResource, resource::SessionExecutionResource};
use agena_domain::{ContentCursor, ContentKind, ContentPayload, ContentRef};
use agena_tui::i18n::I18n;
use agena_tui_transcript::{
    TranscriptActivityContent, TranscriptContentId, TranscriptDetailDefaults, TranscriptEntryPart,
    TranscriptPartContent, parts_entries, render_entry_part_export, ui_text,
};
use anyhow::{Context, Result, ensure};
use chrono::Local;
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;

use super::TuiBackend;

pub(crate) struct TranscriptExportRequest {
    pub session_id: i64,
    pub session_title: String,
    pub execution: Option<SessionExecutionResource>,
    pub i18n: I18n,
    pub markdown: bool,
    pub destination: Option<PathBuf>,
    pub pager_context: String,
}

impl TuiBackend {
    pub(crate) async fn last_assistant_message_text(
        &self,
        session_id: i64,
    ) -> Result<Option<String>> {
        let parts = self.client().session_all_parts(session_id).await?;
        let Some(marker) = parts.iter().rev().find(|part| {
            part.is_message_boundary()
                && part.role == "assistant"
                && part
                    .content
                    .get("run_kind")
                    .and_then(serde_json::Value::as_str)
                    != Some("compaction")
        }) else {
            return Ok(None);
        };
        let mut text = Vec::new();
        for (index, part) in parts
            .iter()
            .filter(|part| part.run_id == Some(marker.part_id) && part.kind == "text")
            .enumerate()
        {
            if index > 0 {
                text.push(b'\n');
            }
            self.client()
                .write_part_text(session_id, part, &mut text)
                .await?;
        }
        let text = String::from_utf8(text).context("assistant text is not valid UTF-8")?;
        Ok((!text.trim().is_empty()).then_some(text))
    }

    pub(crate) async fn prepare_transcript_export(
        &self,
        request: TranscriptExportRequest,
    ) -> Result<Option<NamedTempFile>> {
        let parts = self.client().session_all_parts(request.session_id).await?;
        let entries = parts_entries(&parts);
        if entries.is_empty() {
            return Ok(None);
        }
        let by_id = parts
            .iter()
            .map(|part| (part.part_id, part))
            .collect::<BTreeMap<_, _>>();
        // Staging beside an explicit destination makes the final persist an
        // atomic rename. Dropping this guard removes every incomplete export.
        let staged = if let Some(parent) = request
            .destination
            .as_deref()
            .and_then(std::path::Path::parent)
        {
            let parent = if parent.as_os_str().is_empty() {
                std::path::Path::new(".")
            } else {
                parent
            };
            tokio::fs::create_dir_all(parent).await?;
            NamedTempFile::new_in(parent)?
        } else {
            NamedTempFile::new()?
        };
        let mut output = tokio::fs::File::from_std(staged.reopen()?);
        let title = if request.session_title.trim().is_empty() {
            ui_text::session_fallback_title(&request.i18n, request.session_id)
        } else {
            request.session_title.clone()
        };
        let mut header = vec![
            if request.markdown {
                format!("# {title}")
            } else {
                title
            },
            String::new(),
            ui_text::transcript_export_session_id_line(&request.i18n, request.session_id),
            ui_text::transcript_export_exported_at_line(&request.i18n, Local::now()),
            ui_text::transcript_export_messages_loaded_line(&request.i18n, entries.len()),
        ];
        if let Some(execution) = &request.execution {
            if let Some(parent_id) = execution.session.parent_id {
                header.push(ui_text::transcript_export_parent_session_line(
                    &request.i18n,
                    parent_id,
                ));
            }
            header.push(ui_text::transcript_export_child_sessions_line(
                &request.i18n,
                execution.session.child_session_count,
            ));
        }
        if !request.markdown && !request.pager_context.is_empty() {
            header.push(request.pager_context);
        }
        output.write_all(header.join("\n").as_bytes()).await?;
        output.write_all(b"\n\n").await?;
        let defaults = TranscriptDetailDefaults {
            activity_default_expanded: true,
            kind_defaults: BTreeMap::new(),
        };
        for mut entry in entries {
            let role = entry
                .role
                .map(|role| ui_text::role_label(&request.i18n, role))
                .unwrap_or_else(|| ui_text::t(&request.i18n, "transcript-node-kind-activity"));
            let timestamp = entry
                .created_at
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S");
            let prefix = if request.markdown { "## " } else { "" };
            output
                .write_all(
                    format!(
                        "{prefix}{role} · {} · {timestamp}\n\n",
                        ui_text::message_state_label(&request.i18n, entry.state),
                    )
                    .as_bytes(),
                )
                .await?;
            for index in 0..entry.parts.len() {
                let source_part = match entry.parts[index].id {
                    TranscriptContentId::StoredPart(id) => by_id.get(&id).copied(),
                    _ => None,
                };
                self.load_export_tool_details(
                    request.session_id,
                    source_part,
                    &mut entry.parts[index],
                )
                .await?;
                let mut projected = entry.parts[index].clone();
                let body = take_export_body(&mut projected);
                let rendered =
                    render_entry_part_export(&entry, &projected, &request.i18n, &defaults);
                if request.markdown {
                    output.write_all(b"~~~~text\n").await?;
                }
                for line in rendered {
                    output.write_all(line.text.as_bytes()).await?;
                    output.write_all(b"\n").await?;
                }
                let resources = export_resources(source_part, &entry.parts[index])?;
                for source in &resources {
                    self.write_export_resource(
                        request.session_id,
                        source,
                        &mut output,
                        &request.i18n,
                    )
                    .await?;
                }
                if let Some(body) = body {
                    output.write_all(body.as_bytes()).await?;
                }
                output.write_all(b"\n").await?;
                if request.markdown {
                    output.write_all(b"~~~~\n\n").await?;
                }
            }
            output.write_all(b"\n").await?;
        }
        output.flush().await?;
        output.sync_all().await?;
        drop(output);
        Ok(Some(staged))
    }

    async fn write_export_resource(
        &self,
        session_id: i64,
        source: &ContentRef,
        output: &mut tokio::fs::File,
        i18n: &I18n,
    ) -> Result<()> {
        if matches!(source.kind, ContentKind::Text | ContentKind::Log) {
            return Ok(self
                .client()
                .write_content_text(session_id, source, output)
                .await?);
        }
        // Document/terminal mutations are state, not text events. Reuse the
        // production typed reducer; streamed text/log records bypass its
        // viewport window so an export retains their entire history.
        let mut page = self
            .client()
            .read_content(session_id, source.resource_id, None, 64 * 1024)
            .await?;
        let boundary = page.resource.cursor;
        let mut cursor = ContentCursor {
            sequence: 0,
            ..boundary
        };
        let mut view = agena_tui_transcript::content::ContentView::default();
        loop {
            ensure!(
                page.resource.resource_id == source.resource_id
                    && page.resource.owner_session_id == session_id
                    && page.resource.kind == source.kind,
                "transcript export received a different typed content source",
            );
            ensure!(
                page.resource.cursor.epoch == boundary.epoch
                    && page.next_cursor.epoch == boundary.epoch,
                "transcript export content epoch reset during transfer",
            );
            ensure!(
                !page.gap && page.resource.dropped_bytes == 0 && page.resource.dropped_packets == 0,
                "transcript export encountered a content retention gap",
            );
            ensure!(
                page.resource.cursor.sequence >= boundary.sequence,
                "transcript export source head moved backwards"
            );
            let received_end = page
                .chunks
                .last()
                .map_or(cursor.sequence, |chunk| chunk.cursor.sequence);
            ensure!(
                page.next_cursor.sequence == received_end
                    && received_end <= page.resource.cursor.sequence,
                "transcript export typed page has an invalid cursor",
            );
            let mut expected = cursor.sequence;
            for chunk in &page.chunks {
                ensure!(
                    chunk.cursor.epoch == boundary.epoch && chunk.cursor.sequence > expected,
                    "transcript export typed content cursor did not advance"
                );
                let starts_from_checkpoint = expected == 0
                    && matches!(source.kind, ContentKind::Structured | ContentKind::Terminal)
                    && matches!(
                        &chunk.payload,
                        ContentPayload::StructuredSnapshot { .. } | ContentPayload::Terminal { .. }
                    );
                ensure!(
                    starts_from_checkpoint || chunk.cursor.sequence == expected + 1,
                    "transcript export typed content is missing records"
                );
                expected = chunk.cursor.sequence;
            }
            ensure!(
                received_end > cursor.sequence || cursor.sequence == boundary.sequence,
                "transcript export typed content read made no progress"
            );
            ensure!(
                page.has_more || received_end >= boundary.sequence,
                "transcript export typed content ended before its initial source head"
            );
            page.chunks
                .retain(|chunk| chunk.cursor.sequence <= boundary.sequence);
            page.next_cursor = page.chunks.last().map_or(cursor, |chunk| chunk.cursor);
            let reached_boundary = page.next_cursor.sequence >= boundary.sequence;
            ensure!(
                view.apply(page.clone()),
                "transcript export typed content could not be projected without a gap or reset"
            );
            if source.kind == ContentKind::Document {
                for chunk in &page.chunks {
                    if let Some(text) = chunk.payload.text_content() {
                        output.write_all(text.as_bytes()).await?;
                    }
                }
            }
            if reached_boundary {
                break;
            }
            cursor = page.next_cursor;
            page = self
                .client()
                .read_content(session_id, source.resource_id, Some(cursor), 64 * 1024)
                .await?;
        }
        if let Some(frame) = view.frame {
            if let Some(document) = &frame.document {
                let mut lines = Vec::new();
                agena_tui_transcript::renderer::render_content_document(
                    document, &mut lines, 120, i18n,
                );
                for line in lines {
                    output.write_all(line.text.as_bytes()).await?;
                    output.write_all(b"\n").await?;
                }
            }
            if source.kind == ContentKind::Terminal {
                for (index, line) in frame.lines.iter().enumerate() {
                    for span in &line.spans {
                        output.write_all(span.content.as_bytes()).await?;
                    }
                    if index + 1 < frame.lines.len() {
                        output.write_all(b"\n").await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn load_export_tool_details(
        &self,
        session_id: i64,
        source: Option<&PartResource>,
        part: &mut TranscriptEntryPart<'_>,
    ) -> Result<()> {
        let Some(source) = source else {
            return Ok(());
        };
        let TranscriptPartContent::Activity(TranscriptActivityContent::Operation(tool)) =
            &mut part.content
        else {
            return Ok(());
        };
        for section in [
            ToolDetailSection::Input,
            ToolDetailSection::Metadata,
            ToolDetailSection::Output,
        ] {
            let detail = self
                .client()
                .session_tool_detail(session_id, source.part_id, section)
                .await?;
            ensure!(
                detail.part_id == source.part_id && detail.section == section,
                "tool export detail does not match its requested part and section"
            );
            match section {
                ToolDetailSection::Input => {
                    tool.operation.invocation.input =
                        detail.value.try_into().map_err(anyhow::Error::msg)?;
                    tool.input_loaded = true;
                }
                ToolDetailSection::Metadata => {
                    tool.operation.metadata = serde_json::from_value(detail.value)?;
                    tool.metadata_loaded = true;
                }
                ToolDetailSection::Output => {
                    tool.operation.output = serde_json::from_value(detail.value)?;
                    tool.output_loaded = true;
                }
                ToolDetailSection::Presentation => unreachable!(),
            }
        }
        // Source bodies are streamed below. Suppress the viewport-only
        // loading placeholders in this otherwise complete document.
        tool.presentation
            .blocks
            .retain(|block| !matches!(block, agena_domain::ViewBlock::Content { .. }));
        Ok(())
    }
}

fn take_export_body(part: &mut TranscriptEntryPart<'_>) -> Option<String> {
    match &mut part.content {
        TranscriptPartContent::UserDocument(document) => {
            let text = document.plain_text();
            document.nodes.clear();
            Some(text)
        }
        TranscriptPartContent::Text(body) => Some(std::mem::take(&mut body.text)),
        TranscriptPartContent::Activity(
            TranscriptActivityContent::TextSegment(body) | TranscriptActivityContent::Answer(body),
        ) => Some(std::mem::take(&mut body.text)),
        TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(body)) => {
            let text = body.preferred_text();
            body.summary.clear();
            body.raw_content.clear();
            Some(text)
        }
        _ => None,
    }
}

fn export_resources(
    source: Option<&PartResource>,
    part: &TranscriptEntryPart<'_>,
) -> Result<Vec<ContentRef>> {
    let mut resources = source
        .and_then(|source| source.content.get("resources"))
        .map(|value| serde_json::from_value::<Vec<ContentRef>>(value.clone()))
        .transpose()?
        .unwrap_or_default();
    if let TranscriptPartContent::Activity(TranscriptActivityContent::Operation(tool)) =
        &part.content
    {
        resources.extend(tool.operation.resources.iter().cloned());
        resources.extend(
            tool.operation
                .output
                .iter()
                .flat_map(|output| output.fields.iter().map(|field| field.resource.clone())),
        );
        if let Some(presentation) = source.and_then(|part| part.presentation.as_ref()) {
            resources.extend(presentation.blocks.iter().filter_map(|block| match block {
                agena_domain::ViewBlock::Content { resource, .. } => Some(resource.clone()),
                _ => None,
            }));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    resources.retain(|source| seen.insert(source.resource_id));
    Ok(resources)
}

#[cfg(test)]
mod tests;
