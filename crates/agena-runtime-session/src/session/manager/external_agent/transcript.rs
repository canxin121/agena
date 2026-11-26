use std::collections::BTreeMap;

use super::*;
use crate::session::kernel::media::{MediaBinding, uri};
use crate::session::manager::{results::append_text_chunks, results_text::truncate};
use agena_domain::{
    AttachmentItem, AttachmentKind, AttachmentSource, DocumentMutation, MAX_DOCUMENT_BYTES,
    RawOutput, StructuredObject, ToolInvocation, ViewBlock,
};
use agena_runtime_tools::{media_input::PreparedMedia, tool::human_view::BuiltinHumanRenderer};
use agena_storage::kernel::DocumentOutput;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

pub(super) mod message;
use message::{MessageTarget, MessageUpdate, NativeMessages};

const TITLE_BYTES: usize = 512;
const TOOL_OUTPUT_BYTES: usize = 32 * 1024;
pub(super) const TOOL_DIFF_BYTES: usize = 96 * 1024;
const MEDIA_BASE64_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONTENT_ITEMS: usize = 32;

#[derive(Default)]
pub(super) struct Transcript {
    output: Option<(bool, TextOutput)>,
    messages: NativeMessages,
    link: Option<ExternalAgentLink>,
    state: ExternalAgentState,
    // Keep completed IDs until the turn ends so repeated updates cannot open
    // another row. The MCP callback records forwarded Agena tools separately.
    tools: BTreeMap<(String, String), Option<NativeTool>>,
}

impl Transcript {
    pub(super) async fn start_native_session(
        &mut self,
        turn: &ExternalTurn,
        link: ExternalAgentLink,
        previous: &ExternalAgentState,
        mut current: ExternalAgentState,
        replacements: Vec<Value>,
    ) -> Result<(), AppError> {
        // A pending user message makes any suggestions from the prior native
        // prompt stale, even if the peer can't withdraw them after reconnecting.
        current.details.suggestions.clear();
        self.state = current;
        for update in replacements {
            if let Some(message) = MessageUpdate::parse(&update)
                && !self
                    .messages
                    .apply(MessageTarget::Round(turn, &link), message, false)
                    .await?
            {
                super::state::record_idle_message(&mut self.state, &update);
            }
        }
        if &self.state != previous {
            self.persist_state(&turn.round).await?;
        }
        self.link = Some(link);
        Ok(())
    }

    pub(super) async fn update(
        &mut self,
        turn: &ExternalTurn,
        root: &str,
        source: &str,
        update: &Value,
    ) -> Result<(), AppError> {
        if !self.accepts_session(root, source) {
            return Ok(());
        }
        if super::state::apply_native_update(&mut self.state, root, source, update) {
            self.persist_state(&turn.round).await?;
            if matches!(
                update["sessionUpdate"].as_str(),
                Some("terminal_update" | "terminal_output_chunk")
            ) && let Some(id) = update["terminalId"].as_str()
            {
                for tool in self.tools.values_mut().flatten().filter(|tool| {
                    tool.source == source
                        && tool
                            .native
                            .items
                            .iter()
                            .any(|item| item["terminal_id"] == id)
                }) {
                    tool.record_snapshot(turn, &self.state.details).await?;
                }
            }
            return Ok(());
        }
        if matches!(
            update["sessionUpdate"].as_str(),
            Some("tool_call" | "tool_call_update" | "tool_call_content_chunk")
        ) {
            return self.update_tool(turn, root, source, update).await;
        }
        if root != source {
            return Ok(());
        }
        let reasoning = match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk" | "agent_message") => false,
            Some("agent_thought_chunk" | "agent_thought") => true,
            _ => return Ok(()),
        };
        let content = &update["content"];
        if let Some(update) = MessageUpdate::parse(update) {
            self.finish_text().await?;
            let link = self.link.as_ref().ok_or_else(|| {
                AppError::Internal("missing native assistant document identity".into())
            })?;
            self.messages
                .apply(MessageTarget::Round(turn, link), update, true)
                .await?;
            return Ok(());
        }
        if let Some(blocks) = content.as_array() {
            for block in blocks {
                self.append_content(turn, reasoning, block).await?;
            }
        } else {
            self.append_content(turn, reasoning, content).await?;
        }
        Ok(())
    }

    async fn append_content(
        &mut self,
        turn: &ExternalTurn,
        reasoning: bool,
        content: &Value,
    ) -> Result<(), AppError> {
        let text;
        let value = match content["type"].as_str() {
            Some("text") => content["text"].as_str().unwrap_or_default(),
            Some("resource_link") => {
                text = resource_label(content);
                &text
            }
            Some("resource") if content["resource"]["text"].is_string() => {
                let resource = &content["resource"];
                text = format!(
                    "{}\n{}",
                    resource_label(resource),
                    bounded(
                        resource["text"].as_str().unwrap_or_default(),
                        TOOL_OUTPUT_BYTES
                    )
                );
                &text
            }
            Some("image" | "audio" | "resource") if !reasoning => {
                if let Some(media) = NativeMedia::from_content(content) {
                    self.finish_text().await?;
                    return publish_media_reply(turn, media).await;
                }
                "[Unsupported ACP attachment]"
            }
            _ => return Ok(()),
        };
        if value.is_empty() {
            return Ok(());
        }
        if !self
            .output
            .as_ref()
            .is_some_and(|(kind, _)| *kind == reasoning)
        {
            self.finish_text().await?;
            let command = RequestId::new();
            let output = persist(|| turn.round.begin_text(command, reasoning))
                .await
                .map_err(kernel_error)?
                .value;
            self.output = Some((reasoning, output));
        }
        if let Some((_, output)) = &self.output {
            append_text_chunks(output, value)
                .await
                .map_err(kernel_error)?;
        }
        Ok(())
    }

    async fn persist_state(&self, round: &RoundHandle) -> Result<(), AppError> {
        let command = RequestId::new();
        let snapshot = self
            .state
            .bounded_snapshot(super::state::MAX_STATE_SNAPSHOT_BYTES);
        persist(|| round.record_external_agent_state(command, snapshot.clone()))
            .await
            .map_err(kernel_error)
    }

    async fn update_tool(
        &mut self,
        turn: &ExternalTurn,
        root: &str,
        source: &str,
        update: &Value,
    ) -> Result<(), AppError> {
        let Some(id) = update["toolCallId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 4092)
        else {
            return Ok(());
        };
        let child = (root != source).then_some(source);
        let key = (source.to_owned(), id.to_owned());
        if let Some(Some(tool)) = self.tools.get_mut(&key) {
            tool.native.patch(update);
        } else if !self.tools.contains_key(&key) {
            if child.is_none() {
                self.finish_text().await?;
            }
            let name = update["name"].as_str().or_else(|| {
                update
                    .pointer("/_meta/claudeCode/toolName")
                    .and_then(Value::as_str)
            });
            if name.is_some_and(|name| name.starts_with("mcp__agena__")) {
                self.tools.insert(key, None);
                return Ok(());
            }
            let title = update["title"]
                .as_str()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or("Tool call");
            let child_label = child.map(|child| self.child_label(child).unwrap_or(child));
            let label = match child_label {
                Some(child) => format!("{} · {child}", turn.agent.label()),
                None => turn.agent.label().to_owned(),
            };
            let call_id = match child {
                Some(child) => {
                    let mut hash = Sha256::new();
                    hash.update(child.len().to_be_bytes());
                    hash.update(child);
                    hash.update(id);
                    format!("acp-child:{}", hex::encode(hash.finalize()))
                }
                None => format!("acp:{id}"),
            };
            let native = NativeToolBody::new(turn.agent, &label, title, child, update);
            let command = RequestId::new();
            let call = ToolStart {
                call_id,
                name: native.title.clone(),
                input: native.input.clone(),
            };
            let handle = persist(|| turn.round.begin_provider_tool(command, call.clone()))
                .await
                .map_err(kernel_error)?
                .value;
            self.tools.insert(
                key.clone(),
                Some(NativeTool {
                    handle,
                    native,
                    source: source.into(),
                    preview: None,
                }),
            );
        }
        let Some(slot) = self.tools.get_mut(&key) else {
            return Ok(());
        };
        let phase = match update["status"].as_str() {
            Some("completed") => Some(ToolPhase::Succeeded),
            Some("failed") => Some(ToolPhase::Failed),
            Some("cancelled") => Some(ToolPhase::Cancelled),
            _ => None,
        };
        if let Some(phase) = phase {
            if let Some(tool) = slot.take() {
                tool.finish(turn, &self.state.details, phase).await?;
            }
        } else if let Some(tool) = slot.as_mut()
            && [
                "content",
                "title",
                "kind",
                "locations",
                "rawInput",
                "rawOutput",
            ]
            .into_iter()
            .any(|key| update.get(key).is_some())
        {
            tool.record_snapshot(turn, &self.state.details).await?;
        }
        Ok(())
    }

    pub(super) async fn finish_text(&mut self) -> Result<(), AppError> {
        if let Some((_, output)) = self.output.take() {
            let command = RequestId::new();
            persist(|| output.finish(command))
                .await
                .map_err(kernel_error)?;
        }
        Ok(())
    }

    pub(super) async fn finish_turn(&mut self, turn: &ExternalTurn) -> Result<(), AppError> {
        self.finish_text().await?;
        for tool in std::mem::take(&mut self.tools).into_values().flatten() {
            tool.finish(turn, &self.state.details, ToolPhase::Interrupted)
                .await?;
        }
        Ok(())
    }

    pub(super) fn accepts_session(&self, root: &str, source: &str) -> bool {
        super::state::accepts_session(&self.state, root, source)
    }

    pub(super) fn snapshot_state(&self) -> ExternalAgentState {
        self.state.clone()
    }

    pub(super) fn child_label(&self, source: &str) -> Option<&str> {
        self.state.details.subagents.get(source)?.title.as_deref()
    }

    pub(super) fn has_native_background(&self) -> bool {
        self.state
            .details
            .subagents
            .values()
            .any(|child| matches!(child.state.as_deref(), Some("running" | "requires_action")))
    }
}

struct NativeTool {
    handle: ToolHandle,
    native: NativeToolBody,
    source: String,
    preview: Option<NativeToolPreview>,
}

impl NativeTool {
    async fn raw_output(
        &self,
        turn: &ExternalTurn,
        details: &agena_domain::ExternalAgentDetails,
        attachments: Vec<AttachmentItem>,
    ) -> RawOutput {
        let mut content = self.native.items.clone();
        let mut remaining = TOOL_OUTPUT_BYTES;
        for item in &mut content {
            if item["type"] != "terminal" || remaining == 0 {
                continue;
            }
            let Some(id) = item["terminal_id"].as_str() else {
                continue;
            };
            if let Some(display) = details
                .terminals
                .iter()
                .find(|terminal| terminal.session_id == self.source && terminal.id == id)
            {
                let output = bounded(&display.output, remaining);
                remaining -= output.len();
                item["output"] = json!(output);
                item["exit_code"] = json!(display.exit_code);
                item["command"] = json!(display.command);
            } else if let Some((output, exit_code)) = turn.native_terminal_output(id).await {
                let output = bounded(&output, remaining);
                remaining -= output.len();
                item["output"] = json!(output);
                item["exit_code"] = json!(exit_code);
            }
        }
        RawOutput {
            payload: Some(json!({
                "text":self.native.text,
                "external_agent":{
                    "content": content,
                    "locations":self.native.input["locations"],
                    "output": self.native.output,
                    "preview_id": self.preview.as_ref().map(|preview| preview.stream.id()),
                }
            })),
            attachments,
            ..Default::default()
        }
    }

    async fn record_snapshot(
        &mut self,
        turn: &ExternalTurn,
        details: &agena_domain::ExternalAgentDetails,
    ) -> Result<(), AppError> {
        let raw = self.raw_output(turn, details, Vec::new()).await;
        let mut blocks = BuiltinHumanRenderer::external_agent_blocks(&raw).unwrap_or_default();
        // The outgoing and incoming previews can both exist during a patch.
        let mut remaining = MAX_DOCUMENT_BYTES / 2 - 1024;
        blocks.retain(|block| {
            let size = serde_json::to_vec(block).map_or(usize::MAX, |bytes| bytes.len() + 1);
            if let Some(left) = remaining.checked_sub(size) {
                remaining = left;
                true
            } else {
                false
            }
        });
        if self.preview.is_none() && !blocks.is_empty() {
            let command = RequestId::new();
            let stream = persist(|| self.handle.begin_document(command))
                .await
                .map_err(kernel_error)?
                .value;
            self.preview = Some(NativeToolPreview {
                stream,
                blocks: Vec::new(),
            });
        }
        if let Some(preview) = self.preview.as_mut() {
            preview.update(blocks).await?;
        }
        Ok(())
    }

    async fn finish(
        mut self,
        turn: &ExternalTurn,
        details: &agena_domain::ExternalAgentDetails,
        phase: ToolPhase,
    ) -> Result<(), AppError> {
        let media = std::mem::take(&mut self.native.media);
        let (attachments, blobs) = materialize_media(turn, media).await?;
        let raw = self.raw_output(turn, details, attachments).await;
        let input = StructuredObject::try_from(self.native.input).map_err(AppError::Internal)?;
        let invocation = ToolInvocation::new(self.native.title, input);
        let outcome = ToolResult {
            phase,
            value: json!({"raw_output":raw, "invocation":invocation}),
        };
        let command = RequestId::new();
        persist(|| {
            self.handle
                .complete_with_blobs(command, outcome.clone(), &blobs)
        })
        .await
        .map_err(kernel_error)?;
        Ok(())
    }
}

struct NativeToolPreview {
    stream: DocumentOutput,
    blocks: Vec<ViewBlock>,
}

impl NativeToolPreview {
    async fn update(&mut self, next: Vec<ViewBlock>) -> Result<(), AppError> {
        for old in &self.blocks {
            let Some(id) = old.block_id() else { continue };
            if !next.iter().any(|block| block.block_id() == Some(id)) {
                let mutation = DocumentMutation::Remove {
                    block_id: id.to_owned(),
                };
                persist(|| self.stream.apply(mutation.clone()))
                    .await
                    .map_err(kernel_error)?;
            }
        }
        let mut after = None;
        for block in &next {
            let Some(id) = block.block_id() else { continue };
            let mutation = match self.blocks.iter().find(|old| old.block_id() == Some(id)) {
                Some(old) if old == block => None,
                Some(_) => Some(DocumentMutation::Replace {
                    block: block.clone(),
                }),
                None => Some(DocumentMutation::Insert {
                    after,
                    block: block.clone(),
                }),
            };
            if let Some(mutation) = mutation {
                persist(|| self.stream.apply(mutation.clone()))
                    .await
                    .map_err(kernel_error)?;
            }
            after = Some(id.to_owned());
        }
        self.blocks = next;
        Ok(())
    }
}

struct NativeToolBody {
    title: String,
    label: String,
    input: Value,
    output: Value,
    text: String,
    text_truncated: bool,
    last_was_text: bool,
    items: Vec<Value>,
    media: Vec<NativeMedia>,
    diff_remaining: usize,
    media_remaining: usize,
}

impl NativeToolBody {
    fn new(
        agent: ExternalAgent,
        label: &str,
        title: &str,
        child: Option<&str>,
        update: &Value,
    ) -> Self {
        let mut result = Self {
            title: format!("{label} · {}", bounded(title, TITLE_BYTES)),
            label: label.to_owned(),
            input: json!({"agent":agent}),
            output: Value::Null,
            text: String::new(),
            text_truncated: false,
            last_was_text: false,
            items: Vec::new(),
            media: Vec::new(),
            diff_remaining: TOOL_DIFF_BYTES,
            media_remaining: MEDIA_BASE64_BYTES,
        };
        if let Some(child) = child {
            result.input["session_id"] = json!(child);
        }
        result.patch(update);
        result
    }

    fn patch(&mut self, update: &Value) {
        if let Some(title) = update["title"].as_str() {
            self.title = format!("{} · {}", self.label, bounded(title, TITLE_BYTES));
        }
        if let Some(kind) = update["kind"].as_str() {
            self.input["kind"] = json!(bounded(kind, 80));
        }
        if let Some(value) = update.get("rawInput") {
            self.input["raw_input"] = bounded_json_value(value);
        }
        if let Some(locations) = update.get("locations") {
            self.input["locations"] = Value::Array(
                locations
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|item| {
                        let path = bounded(item["path"].as_str()?, 2048);
                        let line = item["line"].as_u64().filter(|line| *line > 0);
                        Some(json!({"path":path,"line":line}))
                    })
                    .take(MAX_CONTENT_ITEMS)
                    .collect(),
            );
        }
        if let Some(raw) = update.get("rawOutput") {
            self.output = bounded_json_value(raw);
        }
        if let Some(content) = update.get("content") {
            if update["sessionUpdate"] == "tool_call_content_chunk" {
                self.append_item(content, true);
            } else if content.is_array() || content.is_null() {
                self.text.clear();
                self.text_truncated = false;
                self.last_was_text = false;
                self.items.clear();
                self.media.clear();
                self.diff_remaining = TOOL_DIFF_BYTES;
                self.media_remaining = MEDIA_BASE64_BYTES;
                for part in content.as_array().into_iter().flatten() {
                    self.append_item(part, false);
                }
            }
        }
    }

    fn append_item(&mut self, part: &Value, streamed: bool) {
        let content = &part["content"];
        if part["type"] == "content" && content["type"] == "text" {
            if let Some(text) = content["text"].as_str().filter(|text| !text.is_empty()) {
                self.append_text(text, streamed && self.last_was_text);
                self.last_was_text = true;
            }
            return;
        }
        self.last_was_text = false;
        if self.items.len() + self.media.len() >= MAX_CONTENT_ITEMS {
            return;
        }

        let item =
            match part["type"].as_str() {
                Some("diff") => self.capture_diff(part),
                Some("terminal") => content_id(part, "terminalId")
                    .map(|id| json!({"type":"terminal","terminal_id":id})),
                Some("session") => content_id(part, "sessionId")
                    .map(|id| json!({"type":"session","session_id":id})),
                Some("content") => match content["type"].as_str() {
                    Some("resource_link") => {
                        Some(json!({"type":"resource","text":resource_label(content)}))
                    }
                    Some("resource") if content["resource"]["text"].is_string() => {
                        let resource = &content["resource"];
                        let text = resource["text"].as_str().unwrap_or_default();
                        let text = format!("{}\n{}", resource_label(resource), bounded(text, 4096));
                        Some(json!({"type":"resource","text":text}))
                    }
                    _ => {
                        if self.media.len() < 8
                            && let Some(size) = NativeMedia::encoded_size(content)
                            && size <= self.media_remaining
                            && let Some(media) = NativeMedia::from_content(content)
                        {
                            self.media_remaining -= size;
                            self.media.push(media);
                        }
                        None
                    }
                },
                _ => None,
            };
        if let Some(item) = item {
            self.items.push(item);
        }
    }

    fn append_text(&mut self, text: &str, adjacent: bool) {
        const MARKER: &str = "\n[Tool output truncated.]";
        if self.text_truncated {
            return;
        }
        let separator = !adjacent && !self.text.is_empty();
        let remaining = TOOL_OUTPUT_BYTES.saturating_sub(self.text.len() + usize::from(separator));
        let end = text.floor_char_boundary(remaining);
        if separator && remaining > 0 {
            self.text.push('\n');
        }
        self.text.push_str(&text[..end]);
        if end < text.len() {
            truncate(&mut self.text, TOOL_OUTPUT_BYTES - MARKER.len());
            self.text.push_str(MARKER);
            self.text_truncated = true;
        }
    }

    fn capture_diff(&mut self, part: &Value) -> Option<Value> {
        if part["changes"].is_array() {
            return capture_v2_diff(part, &mut self.diff_remaining);
        }
        let path = bounded(part["path"].as_str()?, 2048);
        let mut truncated = false;
        let mut capture = |side: &str| {
            part[side].as_str().map(|text| {
                let preview = bounded(text, self.diff_remaining);
                self.diff_remaining -= preview.len();
                truncated |= preview.len() != text.len();
                preview
            })
        };
        let old_text = capture("oldText");
        let new_text = capture("newText");
        (old_text.is_some() || new_text.is_some()).then(|| {
            json!({
                "type":"diff",
                "path": path,
                "old_text": old_text,
                "new_text": new_text,
                "truncated": truncated,
            })
        })
    }
}

pub(super) fn capture_v2_diff(part: &Value, remaining: &mut usize) -> Option<Value> {
    let changes = part["changes"].as_array()?;
    let mut truncated = changes.len() > MAX_CONTENT_ITEMS;
    let mut captured = Vec::new();
    for change in changes.iter().take(MAX_CONTENT_ITEMS) {
        let Some(path) = change["path"].as_str().filter(|path| !path.is_empty()) else {
            continue;
        };
        let Some(operation) = change["operation"]
            .as_str()
            .filter(|operation| !operation.is_empty())
        else {
            continue;
        };
        let mut capture = |text: &str, limit| {
            let preview = bounded(text, limit);
            truncated |= preview.len() != text.len();
            preview
        };
        let item = json!({
            "path": capture(path, 4096),
            "operation": capture(operation, 80),
            "old_path": change["oldPath"].as_str().map(|path| capture(path, 4096)),
            "file_type": change["fileType"].as_str().map(|kind| capture(kind, 80)),
            "mime_type": change["mimeType"].as_str().map(|mime| capture(mime, 128)),
        });
        let bytes = serde_json::to_vec(&item).ok()?.len();
        let Some(left) = remaining.checked_sub(bytes) else {
            truncated = true;
            break;
        };
        *remaining = left;
        captured.push(item);
    }
    if captured.is_empty() {
        return None;
    }
    let patch = if part["patch"]["format"] == "git_patch" {
        part["patch"]["text"].as_str().map(|text| {
            let preview = bounded(text, *remaining);
            *remaining -= preview.len();
            truncated |= preview.len() != text.len();
            preview
        })
    } else {
        None
    };
    Some(json!({"type":"diff", "changes": captured, "patch": patch, "truncated": truncated}))
}

fn content_id<'a>(part: &'a Value, key: &str) -> Option<&'a str> {
    part[key]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
}

struct NativeMedia {
    data: String,
    expected_kind: Option<AttachmentKind>,
    mime: Option<String>,
}

impl NativeMedia {
    fn fields(content: &Value) -> Option<(&str, Option<AttachmentKind>, Option<&str>)> {
        let (data, kind, mime) = match content["type"].as_str()? {
            "image" => (
                &content["data"],
                Some(AttachmentKind::Image),
                &content["mimeType"],
            ),
            "audio" => (
                &content["data"],
                Some(AttachmentKind::Audio),
                &content["mimeType"],
            ),
            "resource" => (
                &content["resource"]["blob"],
                None,
                &content["resource"]["mimeType"],
            ),
            _ => return None,
        };
        Some((data.as_str()?, kind, mime.as_str()))
    }

    fn encoded_size(content: &Value) -> Option<usize> {
        Some(Self::fields(content)?.0.len())
    }

    fn from_content(content: &Value) -> Option<Self> {
        let (data, expected_kind, mime) = Self::fields(content)?;
        (!data.is_empty() && data.len() <= MEDIA_BASE64_BYTES).then(|| Self {
            data: data.to_owned(),
            expected_kind,
            mime: mime.filter(|mime| mime.len() <= 128).map(str::to_owned),
        })
    }

    async fn prepare(self) -> Result<Option<PreparedMedia>, AppError> {
        crate::session::manager::media::MEDIA_WORKERS
            .run(move || {
                let bytes = STANDARD.decode(self.data).ok()?;
                let prepared = agena_runtime_tools::media_input::from_bytes(
                    "Agent attachment",
                    bytes,
                    self.mime.as_deref(),
                )
                .ok()?;
                self.expected_kind
                    .is_none_or(|kind| kind == prepared.kind)
                    .then_some(prepared)
            })
            .await
            .map_err(|error| AppError::Internal(error.to_string()))
    }
}

async fn materialize_media(
    turn: &ExternalTurn,
    audience_media: Vec<NativeMedia>,
) -> Result<(Vec<AttachmentItem>, Vec<PreparedBlob>), AppError> {
    let mut attachments = Vec::new();
    let mut blobs = Vec::new();
    for media in audience_media {
        let Some(prepared) = media.prepare().await? else {
            continue;
        };
        let blob = turn
            .manager
            .prepare_immutable_blob(RequestId::new(), turn.control.session, &prepared.bytes)
            .await?;
        attachments.push(agent_attachment(
            turn.agent,
            &prepared,
            blob.descriptor().id,
        )?);
        blobs.push(blob);
    }
    Ok((attachments, blobs))
}

fn agent_attachment(
    agent: ExternalAgent,
    prepared: &PreparedMedia,
    blob: agena_domain::BlobId,
) -> Result<AttachmentItem, AppError> {
    let mut attachment = prepared.attachment(None);
    match attachment.mime.as_str() {
        "image/png" => attachment.filename = Some("Agent attachment.png".into()),
        "image/jpeg" => attachment.filename = Some("Agent attachment.jpg".into()),
        "audio/wav" => attachment.filename = Some("Agent attachment.wav".into()),
        _ => {}
    }
    bind_agent_attachment(agent, &mut attachment, blob)?;
    Ok(attachment)
}

pub(super) fn bind_agent_attachment(
    agent: ExternalAgent,
    attachment: &mut AttachmentItem,
    blob: agena_domain::BlobId,
) -> Result<(), AppError> {
    attachment.source = AttachmentSource::Base64 {
        data: String::new(),
    };
    let binding = MediaBinding {
        version: 1,
        model: agent.run_identity(),
        route: format!("acp:{}", agent.id()),
        blob,
        attachment: attachment.clone(),
    };
    attachment.source = AttachmentSource::Url {
        url: uri(&binding).map_err(kernel_error)?,
    };
    Ok(())
}

async fn publish_media_reply(turn: &ExternalTurn, media: NativeMedia) -> Result<(), AppError> {
    let (attachments, blobs) = materialize_media(turn, vec![media]).await?;
    if attachments.is_empty() {
        return Ok(());
    }
    let command = RequestId::new();
    let call = ToolStart {
        call_id: format!("acp-attachment:{command}"),
        name: format!("{} · Attachment", turn.agent.label()),
        input: json!({"agent":turn.agent}),
    };
    let handle = persist(|| turn.round.begin_provider_tool(command, call.clone()))
        .await
        .map_err(kernel_error)?
        .value;
    let outcome = ToolResult {
        phase: ToolPhase::Succeeded,
        value: json!({"raw_output":RawOutput{ attachments, ..Default::default() }}),
    };
    let command = RequestId::new();
    persist(|| handle.complete_with_blobs(command, outcome.clone(), &blobs))
        .await
        .map_err(kernel_error)?;
    Ok(())
}

fn bounded_json_value(value: &Value) -> Value {
    if serde_json::to_vec(value).is_ok_and(|encoded| encoded.len() <= TOOL_OUTPUT_BYTES) {
        return value.clone();
    }
    json!({"truncated":true})
}

fn bounded(text: &str, max: usize) -> String {
    text[..text.floor_char_boundary(max)]
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
        .collect()
}

fn resource_label(content: &Value) -> String {
    let name = content["name"]
        .as_str()
        .or_else(|| content["title"].as_str())
        .unwrap_or("Resource");
    let uri = content["uri"].as_str().unwrap_or_default();
    format!("{}: {}", bounded(name, 512), bounded(uri, 2048))
}
