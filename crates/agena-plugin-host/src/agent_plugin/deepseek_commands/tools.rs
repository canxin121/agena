use agena_domain::{AttachmentItem, AttachmentKind, AttachmentSource, safe_attachment_filename};
use agena_plugin_sdk::{
    ApplicableSessions, HostClient, PluginError, Result, SessionKind, ToolContract, ToolDefinition,
    ToolDocs, ToolInvokeInput, ToolInvokeOutput, ToolTag,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};

use super::{AgentBridge, Program, WorkflowCaller};

pub(super) const MODULE: &str = include_str!("tools.js");
const MAX_TOOL_BYTES: usize = 128 * 1024;
const MAX_MEDIA_BLOCKS: usize = 8;
const MAX_MEDIA_BLOCK_BYTES: usize = 4 * 1024 * 1024;
const MAX_MEDIA_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;

#[derive(Deserialize)]
pub(in crate::agent_plugin) struct Registration {
    pub(in crate::agent_plugin) name: String,
    description: String,
    parameters: Value,
    schema: Value,
    #[serde(rename = "timeoutMs")]
    timeout_ms: Option<f64>,
    #[serde(default, rename = "hasWhen")]
    pub(in crate::agent_plugin) has_when: bool,
    #[allow(dead_code)]
    #[serde(default, rename = "deferLoading")]
    pub(in crate::agent_plugin) defer_loading: bool,
    #[serde(skip)]
    pub(super) agent_coordination: bool,
}

#[derive(Deserialize)]
struct Reply {
    value: Value,
    text: String,
    #[serde(default)]
    media: Vec<RenderedMedia>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenderedMedia {
    kind: String,
    media_type: String,
    data: String,
    filename: Option<String>,
}

impl Program {
    pub(in crate::agent_plugin) fn has_conditional_tools(&self) -> bool {
        self.tools.iter().any(|tool| tool.has_when)
    }

    pub(in crate::agent_plugin) fn tool_has_when(&self, name: &str) -> bool {
        self.tools
            .iter()
            .find(|tool| tool.name == name)
            .is_some_and(|tool| tool.has_when)
    }

    pub(in crate::agent_plugin) async fn is_tool_active(
        self: &Arc<Self>,
        session: i64,
        host: &dyn HostClient,
        tool_name: &str,
    ) -> bool {
        Box::pin(self.evaluate_tool_when(session, host, tool_name)).await
    }

    pub(in crate::agent_plugin) async fn invoke_tool(
        self: &Arc<Self>,
        bridge: &AgentBridge,
        host: Arc<dyn HostClient>,
        input: ToolInvokeInput,
    ) -> Result<ToolInvokeOutput> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name == input.tool_name)
            .ok_or_else(|| PluginError::invalid_params("unknown Cordis tool"))?;
        tool.invoke(self, bridge, host, input).await
    }
}

impl Registration {
    pub(in crate::agent_plugin) fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            contract: ToolContract {
                input_schema: self.parameters.clone(),
                output_schema: self.schema.clone(),
            },
            docs: ToolDocs {
                summary: Some(self.description.clone()),
                ..Default::default()
            },
            runtime: Default::default(),
            // Cordis does not declare an Agena side-effect policy. Calls in
            // one retained VM serialize even when model requests run in parallel.
            tags: if self.agent_coordination {
                match self.name.as_str() {
                    "list_agents" | "list_subagent_models" =>
                        vec![ToolTag::Agent, ToolTag::Query, ToolTag::ReadOnly],
                    _ => vec![ToolTag::Agent],
                }
            } else {
                vec![ToolTag::Execute]
            },
            applies_to: ApplicableSessions::only(SessionKind::Project),
        }
    }

    async fn invoke(
        &self,
        program: &Arc<Program>,
        bridge: &AgentBridge,
        host: Arc<dyn HostClient>,
        input: ToolInvokeInput,
    ) -> Result<ToolInvokeOutput> {
        if input.context().is_global() || input.session_id <= 0 {
            return Err(PluginError::invalid_params(
                "Cordis tools require a project session",
            ));
        }
        // Waiting is host-side work. A retained JS VM held by job_output
        // would also serialize job_kill behind the very job it should cancel.
        if program.url.as_str() == "agena-cordis:tool-jobs" && self.name == "job_output" {
            return invoke_job_output(program, host.as_ref(), input.input).await;
        }
        if program.url.as_str() == "agena-cordis:tool-subagent-control" && self.name == "wait_agent" {
            let timeout_ms = input.input["timeout_ms"].as_u64().unwrap_or(30_000);
            let value = host.cordis_subagent_control(
                agena_plugin_sdk::host_api::CordisSubagentControl::Wait { timeout_ms },
                input.context().cancellation,
            ).await?;
            let mut output = ToolInvokeOutput::text(value.to_string());
            output.payload = Some(value);
            return Ok(output);
        }
        let invocation = serde_json::to_vec(&json!({
            "name": self.name, "id": input.call_id.to_string(), "args": input.input, "sessionId": input.session_id,
        }))
        .map_err(|error| PluginError::invalid_params_error(&error))?;
        if invocation.len() > MAX_TOOL_BYTES {
            return Err(PluginError::invalid_params(
                "Cordis tool input exceeds 128 KiB",
            ));
        }
        let caller = program.uses_workflow.then(|| WorkflowCaller {
            bridge: bridge.clone(),
            host: host.clone(),
            session: input.session_id,
            call: input.call_id,
        });
        let timeout = self
            .timeout_ms
            .map(|ms| Duration::from_secs_f64(ms.clamp(1., 600_000.) / 1000.));
        let reply = program
            .invoke::<Reply>(
                "invokeTool",
                Some(input.session_id),
                Some(host.as_ref()),
                invocation,
                caller,
                timeout,
            )
            .await?;
        let payload_bytes = serde_json::to_vec(&reply.value)
            .map_err(|error| PluginError::internal_error(&error))?
            .len();
        if payload_bytes.saturating_add(reply.text.len()) > MAX_TOOL_BYTES {
            return Err(PluginError::invalid_params(
                "Cordis tool output exceeds 128 KiB",
            ));
        }
        let attachments = decode_rendered_media(reply.media)?;
        let mut output = ToolInvokeOutput::text(reply.text);
        if output.summary.is_empty() && !attachments.is_empty() {
            output.summary = format!("Returned {} media attachment(s)", attachments.len());
        }
        output.payload = Some(reply.value);
        output.attachments = attachments;
        Ok(output)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct JobOutputInput {
    job_id: String,
    #[serde(default)]
    wait: bool,
    timeout_ms: Option<f64>,
}

async fn invoke_job_output(
    program: &Program,
    host: &dyn HostClient,
    input: Value,
) -> Result<ToolInvokeOutput> {
    let config = program.jobs_config.as_deref().ok_or_else(|| {
        PluginError::invalid_params("Cordis workflow job controller is unavailable")
    })?;
    let input: JobOutputInput =
        serde_json::from_value(input).map_err(|error| PluginError::invalid_params_error(&error))?;
    let wait_ms = if input.wait {
        let requested = input.timeout_ms.unwrap_or(config.wait_timeout_ms as f64);
        if !requested.is_finite() || requested < 1.0 {
            return Err(PluginError::invalid_params("timeout_ms must be positive"));
        }
        Some(requested.min(config.max_wait_timeout_ms.min(600_000) as f64) as u64)
    } else {
        None
    };
    let payload = host
        .cordis_workflow_job(
            agena_plugin_sdk::host_api::CordisWorkflowJobRequest::Output {
                job_id: input.job_id,
                wait_ms,
            },
            agena_plugin_sdk::current_cancellation_token().unwrap_or_default(),
        )
        .await?;
    let body = payload
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::internal("Cordis job output omitted its text"))?
        .trim_end();
    let status = payload
        .pointer("/job/status")
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::internal("Cordis job output omitted its status"))?;
    let detail = payload
        .pointer("/job/detail")
        .and_then(Value::as_str)
        .filter(|detail| !detail.is_empty());
    let status = match detail {
        Some(detail) => format!("[status: {status}, {detail}]"),
        None => format!("[status: {status}]"),
    };
    let body = if body.is_empty() {
        "(no new output)"
    } else {
        body
    };
    let rendered = if let Some(limit) = payload.pointer("/job/outputLimitBytes").and_then(Value::as_u64) {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        if let Some(room) = limit.checked_sub(status.len() + 1)
            && room > 0
        {
            format!("{}\n{status}", prefix(body, room))
        } else {
            prefix(&status, limit).into()
        }
    } else {
        format!("{body}\n{status}")
    };
    let mut output = ToolInvokeOutput::text(rendered);
    output.payload = Some(payload);
    Ok(output)
}

fn prefix(text: &str, max_bytes: usize) -> &str {
    &text[..text.floor_char_boundary(max_bytes.min(text.len()))]
}

fn decode_rendered_media(blocks: Vec<RenderedMedia>) -> Result<Vec<AttachmentItem>> {
    decode_rendered_media_labeled(blocks, "Cordis tool", "cordis-tool-output")
}

pub fn decode_content_blocks(
    blocks: &[Value],
    label: &str,
) -> Result<(String, Vec<AttachmentItem>)> {
    let mut texts = Vec::new();
    let mut text_bytes = 0usize;
    let mut media = Vec::new();
    for block in blocks {
        let Some(object) = block.as_object() else {
            return Err(PluginError::invalid_params(format!(
                "{label} content entries must be objects"
            )));
        };
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                PluginError::invalid_params(format!("{label} content block requires a type"))
            })?;
        match kind {
            "text" => {
                let text = object.get("text").and_then(Value::as_str).ok_or_else(|| {
                    PluginError::invalid_params(format!("{label} text block requires a string"))
                })?;
                if !text.is_empty() && text_bytes < MAX_TOOL_BYTES {
                    let room = MAX_TOOL_BYTES.saturating_sub(text_bytes);
                    let slice = prefix(text, room);
                    text_bytes = text_bytes.saturating_add(slice.len());
                    texts.push(slice.to_owned());
                }
            }
            "image" | "audio" | "document" | "file" => {
                let (media_type, data) = extract_content_media(object, label)?;
                let filename = object
                    .get("filename")
                    .or_else(|| object.get("title"))
                    .or_else(|| object.get("name"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                media.push(RenderedMedia {
                    kind: kind.to_owned(),
                    media_type,
                    data,
                    filename,
                });
            }
            other => {
                return Err(PluginError::invalid_params(format!(
                    "{label} content block type {other:?} is not supported"
                )));
            }
        }
    }
    let attachments = decode_rendered_media_labeled(media, label, "tool-output")?;
    Ok((texts.join("\n"), attachments))
}

fn extract_content_media(
    object: &serde_json::Map<String, Value>,
    label: &str,
) -> Result<(String, String)> {
    let source = object.get("source").and_then(Value::as_object);
    if let Some(source) = source
        && let Some(source_type) = source.get("type").and_then(Value::as_str)
        && source_type != "base64"
    {
        return Err(PluginError::invalid_params(format!(
            "{label} media source type {source_type:?} is not supported; use base64"
        )));
    }
    let raw_data = object
        .get("data")
        .and_then(Value::as_str)
        .or_else(|| source.and_then(|s| s.get("data")).and_then(Value::as_str))
        .ok_or_else(|| {
            PluginError::invalid_params(format!("{label} media block requires base64 data"))
        })?;
    let explicit_mime = object
        .get("mimeType")
        .or_else(|| object.get("mediaType"))
        .or_else(|| object.get("media_type"))
        .and_then(Value::as_str)
        .or_else(|| {
            source.and_then(|s| {
                s.get("media_type")
                    .or_else(|| s.get("mediaType"))
                    .or_else(|| s.get("mimeType"))
                    .and_then(Value::as_str)
            })
        });
    if let Some(rest) = raw_data.strip_prefix("data:")
        && let Some((header, payload)) = rest.split_once(',')
    {
        let mime = header
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !header.split(';').any(|part| part.trim().eq_ignore_ascii_case("base64")) {
            return Err(PluginError::invalid_params(format!(
                "{label} media data URL must use base64 encoding"
            )));
        }
        return Ok((
            explicit_mime.unwrap_or(&mime).to_owned(),
            payload.to_owned(),
        ));
    }
    let mime = explicit_mime.ok_or_else(|| {
        PluginError::invalid_params(format!("{label} media block requires a MIME type"))
    })?;
    Ok((mime.to_owned(), raw_data.to_owned()))
}

fn decode_rendered_media_labeled(
    blocks: Vec<RenderedMedia>,
    label: &str,
    default_prefix: &str,
) -> Result<Vec<AttachmentItem>> {
    if blocks.len() > MAX_MEDIA_BLOCKS {
        return Err(PluginError::invalid_params(format!(
            "{label} output contains more than eight media attachments"
        )));
    }
    let mut attachments = Vec::with_capacity(blocks.len());
    let mut total_bytes = 0usize;
    let max_encoded = MAX_MEDIA_BLOCK_BYTES.div_ceil(3) * 4;
    for (index, block) in blocks.into_iter().enumerate() {
        let cleaned: Vec<u8> = block
            .data
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect();
        if cleaned.is_empty() || cleaned.len() > max_encoded {
            return Err(PluginError::invalid_params(format!(
                "{label} media payload is empty or exceeds the 4 MiB limit"
            )));
        }
        let bytes = STANDARD
            .decode(&cleaned)
            .map_err(|_| PluginError::invalid_params(format!("{label} media has invalid base64")))?;
        if bytes.is_empty() || bytes.len() > MAX_MEDIA_BLOCK_BYTES {
            return Err(PluginError::invalid_params(format!(
                "{label} media payload is empty or exceeds the 4 MiB limit"
            )));
        }
        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > MAX_MEDIA_TOTAL_BYTES {
            return Err(PluginError::invalid_params(format!(
                "{label} media attachments exceed the 8 MiB total budget"
            )));
        }
        let mime = block
            .media_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let (kind, ext, dimensions) = inspect_media(&block.kind, &mime, &bytes)?;
        let default_name = format!("{default_prefix}-{index}.{ext}");
        let filename = block
            .filename
            .as_deref()
            .map(safe_attachment_filename)
            .filter(|name| !name.is_empty() && name.len() <= 255)
            .unwrap_or(default_name);
        let sha256 = hex::encode(Sha256::digest(&bytes));
        attachments.push(AttachmentItem {
            kind,
            mime,
            source: AttachmentSource::Base64 {
                data: STANDARD.encode(&bytes),
            },
            filename: Some(filename),
            title: None,
            size_bytes: Some(bytes.len() as u64),
            sha256: Some(sha256),
            width: dimensions.map(|(w, _)| w),
            height: dimensions.map(|(_, h)| h),
            duration_ms: None,
            page_count: None,
        });
    }
    Ok(attachments)
}

fn inspect_media(
    block_kind: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<(AttachmentKind, &'static str, Option<(u32, u32)>)> {
    let check_dims = |width: u32, height: u32| -> Result<(u32, u32)> {
        if width == 0
            || height == 0
            || width > MAX_IMAGE_DIMENSION
            || height > MAX_IMAGE_DIMENSION
            || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
        {
            return Err(PluginError::invalid_params(
                "Cordis tool image dimensions exceed supported limits",
            ));
        }
        Ok((width, height))
    };
    let detected = match mime {
        "image/png" => {
            if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
                return Err(PluginError::invalid_params(
                    "Cordis tool image/png payload does not match PNG header",
                ));
            }
            let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
            let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            (AttachmentKind::Image, "png", Some(check_dims(width, height)?))
        }
        "image/jpeg" => {
            let (width, height) = jpeg_dimensions(bytes).ok_or_else(|| {
                PluginError::invalid_params(
                    "Cordis tool image/jpeg payload does not match JPEG format",
                )
            })?;
            (AttachmentKind::Image, "jpg", Some(check_dims(width, height)?))
        }
        "image/gif" => {
            if bytes.len() < 10 || (&bytes[..6] != b"GIF87a" && &bytes[..6] != b"GIF89a") {
                return Err(PluginError::invalid_params(
                    "Cordis tool image/gif payload does not match GIF header",
                ));
            }
            let width = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
            let height = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
            (AttachmentKind::Image, "gif", Some(check_dims(width, height)?))
        }
        "image/webp" => {
            let (width, height) = webp_dimensions(bytes).ok_or_else(|| {
                PluginError::invalid_params(
                    "Cordis tool image/webp payload does not match WebP format",
                )
            })?;
            (AttachmentKind::Image, "webp", Some(check_dims(width, height)?))
        }
        "audio/wav" => {
            if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
                return Err(PluginError::invalid_params(
                    "Cordis tool audio/wav payload does not match WAV header",
                ));
            }
            (AttachmentKind::Audio, "wav", None)
        }
        "audio/mpeg" => {
            let valid = (bytes.len() >= 3 && &bytes[..3] == b"ID3")
                || (bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0);
            if !valid {
                return Err(PluginError::invalid_params(
                    "Cordis tool audio/mpeg payload does not match MP3 header",
                ));
            }
            (AttachmentKind::Audio, "mp3", None)
        }
        "audio/ogg" => {
            if bytes.len() < 4 || &bytes[..4] != b"OggS" {
                return Err(PluginError::invalid_params(
                    "Cordis tool audio/ogg payload does not match Ogg header",
                ));
            }
            (AttachmentKind::Audio, "ogg", None)
        }
        "audio/flac" => {
            if bytes.len() < 4 || &bytes[..4] != b"fLaC" {
                return Err(PluginError::invalid_params(
                    "Cordis tool audio/flac payload does not match FLAC header",
                ));
            }
            (AttachmentKind::Audio, "flac", None)
        }
        "application/pdf" => {
            if bytes.len() < 5 || &bytes[..5] != b"%PDF-" {
                return Err(PluginError::invalid_params(
                    "Cordis tool application/pdf payload does not match PDF header",
                ));
            }
            (AttachmentKind::Pdf, "pdf", None)
        }
        _ => {
            return Err(PluginError::invalid_params(format!(
                "Cordis tool media type {mime:?} is not supported"
            )));
        }
    };
    let kind_matches = match block_kind {
        "image" => detected.0 == AttachmentKind::Image,
        "audio" => detected.0 == AttachmentKind::Audio,
        "document" | "file" => matches!(
            detected.0,
            AttachmentKind::Pdf | AttachmentKind::Image | AttachmentKind::Audio
        ),
        _ => false,
    };
    if !kind_matches {
        return Err(PluginError::invalid_params(format!(
            "Cordis tool {block_kind:?} block does not match media type {mime:?}"
        )));
    }
    Ok(detected)
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut index = 2;
    while index + 1 < bytes.len() {
        while index < bytes.len() && bytes[index] != 0xFF {
            index += 1;
        }
        while index < bytes.len() && bytes[index] == 0xFF {
            index += 1;
        }
        if index >= bytes.len() {
            return None;
        }
        let marker = bytes[index];
        index += 1;
        if matches!(marker, 0xD8 | 0xD9 | 0x01 | 0xD0..=0xD7) {
            continue;
        }
        if index + 1 >= bytes.len() {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([bytes[index], bytes[index + 1]]));
        if len < 2 || index + len > bytes.len() {
            return None;
        }
        if matches!(
            marker,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
        ) && len >= 7
        {
            let height = u32::from(u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]));
            let width = u32::from(u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]));
            return Some((width, height));
        }
        index += len;
    }
    None
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8X" => {
            let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
            let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
            Some((width, height))
        }
        b"VP8 " if bytes[23..26] == [0x9D, 0x01, 0x2A] => {
            let width = u32::from(u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3FFF);
            let height = u32::from(u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3FFF);
            Some((width, height))
        }
        b"VP8L" if bytes[20] == 0x2F => {
            let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
            let width = (bits & 0x3FFF) + 1;
            let height = ((bits >> 14) & 0x3FFF) + 1;
            Some((width, height))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_2X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DA8J8BAAf/Af8Bf4mnAAAAAElFTkSuQmCC";

    #[test]
    fn decode_rendered_media_validates_png_pdf_and_rejects_spoofed_mime() {
        let items = decode_rendered_media(vec![
            RenderedMedia {
                kind: "image".into(),
                media_type: "image/png".into(),
                data: PNG_2X1.into(),
                filename: Some("../preview.png".into()),
            },
            RenderedMedia {
                kind: "document".into(),
                media_type: "application/pdf".into(),
                data: STANDARD.encode(b"%PDF-1.7\n%%EOF\n"),
                filename: None,
            },
        ])
        .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, AttachmentKind::Image);
        assert_eq!(items[0].mime, "image/png");
        assert_eq!(items[0].width, Some(2));
        assert_eq!(items[0].height, Some(1));
        assert_eq!(items[0].filename.as_deref(), Some("preview.png"));
        assert_eq!(items[1].kind, AttachmentKind::Pdf);
        assert_eq!(items[1].filename.as_deref(), Some("cordis-tool-output-1.pdf"));

        let spoofed = decode_rendered_media(vec![RenderedMedia {
            kind: "image".into(),
            media_type: "image/png".into(),
            data: STANDARD.encode(b"not a png"),
            filename: None,
        }])
        .unwrap_err();
        assert!(
            spoofed.diagnostic_message().contains("does not match PNG"),
            "{spoofed}"
        );
    }
}
