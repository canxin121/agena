use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    path::{Component, Path},
};

use agena_domain::{
    ActivityId, ActivityPayload, AttachmentItem, AttachmentKind, AttachmentSource, BlobId,
    ComposerActivity, ComposerDocument, ComposerNode, ExternalAgent, RawOutput, ResourceActivity,
    ResourceDelivery, ResourceKind, ResourceReference, StructuredObject, ToolInvocation,
};
use agena_runtime_tools::media_input;
use agena_storage::kernel::{
    BlobDescriptor, ExternalAgentHistoryBlob, ExternalAgentHistoryPart, ExternalAgentHistoryTurn,
    ExternalAgentState, MAX_EXTERNAL_AGENT_HISTORY_MEDIA_BYTES,
    MAX_EXTERNAL_AGENT_HISTORY_METADATA_BYTES, ToolResult, ToolStart,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::super::super::state::{
    MAX_STATE_SNAPSHOT_BYTES, accepts_session, apply_native_update, apply_setup_response,
    command_name,
};
use super::super::super::transcript::{TOOL_DIFF_BYTES, bind_agent_attachment, capture_v2_diff};
use super::super::cursors;
use super::bounded_string;
use crate::AppError;
use crate::session::manager::media::MEDIA_WORKERS;
use agena_domain::session_kernel::ToolPhase;

const MAX_REPLAY_WIRE_BYTES: usize = 24 * 1024 * 1024;
const MAX_REPLAY_RECORDS: usize = 6_000;
const MAX_REPLAY_UPDATES: usize = 65_536;
const MAX_ENTRY_BLOBS: usize = 16;

#[derive(Default)]
pub(in super::super) struct Replay {
    entries: Vec<Record>,
    messages: HashMap<MessageKey, Vec<usize>>,
    tools: HashMap<String, usize>,
    cursors: cursors::Book,
    announced_entities: HashSet<cursors::Entity>,
    allow_rewind: bool,
    state: ExternalAgentState,
    wire: ReplayBudget<MAX_REPLAY_WIRE_BYTES>,
    updates: usize,
    pub(super) complete: bool,
    complete_children: HashSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Role {
    User,
    Assistant,
    Thought,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MessageKey {
    role: Role,
    id: Option<String>,
}

enum Record {
    Blocks { key: MessageKey, blocks: Vec<Value> },
    Tool(ReplayTool),
    Anchor(cursors::Entity),
    Discarded,
}

#[derive(Default)]
struct ReplayBudget<const LIMIT: usize>(usize);

impl<const LIMIT: usize> Write for ReplayBudget<LIMIT> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        if self.0 > LIMIT {
            return Err(io::Error::other("ACP replay payload limit"));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Replay {
    pub(super) fn new(allow_rewind: bool) -> Self {
        Self {
            allow_rewind,
            ..Self::default()
        }
    }

    pub(super) fn apply_setup_response(&mut self, response: &Value, v2: bool) {
        apply_setup_response(&mut self.state, response, v2);
    }

    pub(in super::super) fn update(
        &mut self,
        root: &str,
        source: &str,
        update: &Value,
        v2: bool,
    ) -> Result<(), AppError> {
        if !accepts_session(&self.state, root, source) {
            return Ok(());
        }
        let kind = update["sessionUpdate"].as_str().unwrap_or_default();
        if kind == "replay_complete" {
            if source == root {
                self.complete = true;
            } else {
                self.complete_children.insert(source.to_owned());
            }
            return Ok(());
        }
        if source != root
            || matches!(
                kind,
                "subagent_update" | "terminal_update" | "terminal_output_chunk"
            )
        {
            self.account(update)?;
            let historical = v2
                && if source == root {
                    !self.complete
                } else {
                    !self.complete_children.contains(source)
                };
            if historical && kind == "subagent_update" && update.get("state").is_some() {
                // The replay establishes the parent/child relationship. Its
                // live work state is reported after the parent's marker.
                let mut association = update.clone();
                association
                    .as_object_mut()
                    .expect("ACP update object")
                    .remove("state");
                apply_native_update(&mut self.state, root, source, &association);
            } else if !historical || kind != "state_update" {
                apply_native_update(&mut self.state, root, source, update);
            }
            return Ok(());
        }
        let message = match kind {
            "user_message_chunk" => Some((Role::User, false)),
            "agent_message_chunk" => Some((Role::Assistant, false)),
            "agent_thought_chunk" => Some((Role::Thought, false)),
            "user_message" => Some((Role::User, true)),
            "agent_message" => Some((Role::Assistant, true)),
            "agent_thought" => Some((Role::Thought, true)),
            _ => None,
        };
        let tool = matches!(
            kind,
            "tool_call" | "tool_call_update" | "tool_call_content_chunk"
        );
        let cursor = kind == "cursor_update";
        if message.is_none() && !tool && !cursor {
            apply_native_update(&mut self.state, root, source, update);
            if !self.complete
                && let Some(entity) = cursors::Entity::from_update(update)
                && self.announced_entities.insert(entity.clone())
            {
                self.account(update)?;
                self.push(Record::Anchor(entity))?;
            }
            return Ok(());
        }
        // Live transcript events can arrive before the response. The marker
        // ends the historical range, even if the peer keeps streaming.
        if self.complete {
            if cursor {
                self.account(update)?;
                self.cursors.observe(update, self.allow_rewind, 0);
            }
            return Ok(());
        }
        self.account(update)?;
        if let Some((role, replace)) = message {
            self.message(role, replace, update)?;
        } else if tool {
            self.tool(update)?;
        } else {
            self.cursors
                .observe(update, self.allow_rewind, MAX_REPLAY_RECORDS);
        }
        Ok(())
    }

    fn account(&mut self, update: &Value) -> Result<(), AppError> {
        self.updates += 1;
        if self.updates > MAX_REPLAY_UPDATES
            || serde_json::to_writer(&mut self.wire, update).is_err()
        {
            return Err(history_budget());
        }
        Ok(())
    }

    fn message(&mut self, role: Role, replace: bool, update: &Value) -> Result<(), AppError> {
        let key = MessageKey {
            role,
            id: bounded_string(&update["messageId"], 4096),
        };
        if replace && update.get("content").is_none() && self.messages.contains_key(&key) {
            return Ok(());
        }
        let blocks = match &update["content"] {
            Value::Array(blocks) => blocks.clone(),
            block if block.is_object() => vec![block.clone()],
            _ => Vec::new(),
        };
        if replace {
            if let Some(slots) = self.messages.get_mut(&key) {
                let keep = slots[0];
                // A snapshot supersedes every chunk of this message,
                // including segments on either side of a tool record.
                for slot in slots.drain(1..) {
                    self.entries[slot] = Record::Discarded;
                }
                self.entries[keep] = Record::Blocks { key, blocks };
                return Ok(());
            }
        } else {
            let slot = if key.role == Role::User {
                self.messages
                    .get(&key)
                    .and_then(|slots| slots.first().copied())
            } else {
                None
            };
            let adjacent = self.entries.len().checked_sub(1);
            if let Some(Record::Blocks {
                key: prior,
                blocks: saved,
            }) = slot
                .or(adjacent)
                .and_then(|index| self.entries.get_mut(index))
                && *prior == key
            {
                saved.extend(blocks);
                return Ok(());
            }
        }
        let index = self.entries.len();
        self.push(Record::Blocks {
            key: key.clone(),
            blocks,
        })?;
        if key.id.is_some() {
            self.messages.entry(key).or_default().push(index);
        }
        Ok(())
    }

    fn tool(&mut self, update: &Value) -> Result<(), AppError> {
        let Some(id) = bounded_string(&update["toolCallId"], 4092) else {
            return Ok(());
        };
        if let Some(&index) = self.tools.get(&id)
            && let Record::Tool(tool) = &mut self.entries[index]
        {
            tool.patch(update);
            return Ok(());
        }
        let mut tool = ReplayTool {
            id: id.clone(),
            ..Default::default()
        };
        tool.patch(update);
        let index = self.entries.len();
        self.push(Record::Tool(tool))?;
        self.tools.insert(id, index);
        Ok(())
    }

    fn push(&mut self, record: Record) -> Result<(), AppError> {
        if self.entries.len() >= MAX_REPLAY_RECORDS {
            return Err(history_budget());
        }
        self.entries.push(record);
        Ok(())
    }

    pub(super) async fn finish(
        mut self,
        agent: ExternalAgent,
        cwd: &Path,
    ) -> Result<(Vec<ExternalAgentHistoryTurn>, ExternalAgentState), AppError> {
        let mut projection = Projection {
            agent,
            cwd,
            media_bytes: 0,
            sequence: 0,
        };
        let mut turns = Vec::<ExternalAgentHistoryTurn>::new();
        let mut prelude = Vec::new();
        let mut preceding = None;
        for record in self.entries {
            match record {
                Record::Blocks { key, blocks } if key.role == Role::User => {
                    let (document, blobs) = projection.user(blocks).await?;
                    if !document.is_empty() {
                        let mut cursor =
                            self.cursors.for_user(key.id.as_deref(), preceding.as_ref());
                        if !self.complete
                            && let Some(cursor) = &mut cursor
                        {
                            cursor.fork = false;
                            cursor.rewind = false;
                            cursor.cursor_id = None;
                        }
                        turns.push(ExternalAgentHistoryTurn {
                            user: document,
                            cursor,
                            blobs,
                            prelude: std::mem::take(&mut prelude),
                            response: Vec::new(),
                        });
                    }
                    preceding = key
                        .id
                        .as_deref()
                        .map(|id| cursors::Entity::message("user_message", id));
                }
                Record::Blocks { key, blocks } => {
                    let kind = if key.role == Role::Thought {
                        "agent_thought"
                    } else {
                        "agent_message"
                    };
                    preceding = key
                        .id
                        .as_deref()
                        .map(|id| cursors::Entity::message(kind, id));
                    let output = turns
                        .last_mut()
                        .map_or(&mut prelude, |turn| &mut turn.response);
                    projection
                        .reply(blocks, key.role == Role::Thought, output)
                        .await?;
                }
                Record::Tool(tool) => {
                    preceding = Some(cursors::Entity::message("tool_call", &tool.id));
                    let part = tool.finish(&mut projection).await?;
                    turns
                        .last_mut()
                        .map_or(&mut prelude, |turn| &mut turn.response)
                        .push(part);
                }
                Record::Anchor(entity) => preceding = Some(entity),
                Record::Discarded => {}
            }
        }
        if !prelude.is_empty() {
            return Err(AppError::Config(
                "This native history has no recorded user message to associate with its assistant output.".into(),
            ));
        }
        self.state.cursor_tail = preceding.map(cursors::Entity::saved);
        retain_recent_turns(&mut turns)?;
        Ok((turns, self.state.bounded_snapshot(MAX_STATE_SNAPSHOT_BYTES)))
    }
}

fn retain_recent_turns(turns: &mut Vec<ExternalAgentHistoryTurn>) -> Result<(), AppError> {
    type ProjectionBudget = ReplayBudget<MAX_EXTERNAL_AGENT_HISTORY_METADATA_BYTES>;
    if serde_json::to_writer(ProjectionBudget::default(), &turns).is_ok() {
        return Ok(());
    }
    // The retained view must be a contiguous suffix ending at the real
    // conversation head. Reserve space for its array and projection notice.
    let mut remaining = MAX_EXTERNAL_AGENT_HISTORY_METADATA_BYTES - 1024;
    let available_turns = turns.len();
    let mut retained_turns = 0;
    for turn in turns.iter().rev() {
        let mut budget = ProjectionBudget::default();
        if serde_json::to_writer(&mut budget, turn).is_err() || budget.0 + 1 > remaining {
            break;
        }
        remaining -= budget.0 + 1;
        retained_turns += 1;
    }
    if retained_turns == 0 {
        return Err(AppError::Config(
            "The latest native turn exceeds the size limit for an Agena ACP history view.".into(),
        ));
    }
    turns.drain(..available_turns - retained_turns);
    let first = &mut turns[0];
    // There is no local predecessor for a native cut before this user input.
    first.cursor = None;
    first.prelude.insert(0, ExternalAgentHistoryPart::Notice {
        text: serde_json::to_string(&agena_runtime_contracts::NoticePart {
            kind: "external_agent_history_window".into(),
            summary: format!("Agena loaded the latest {retained_turns} of {available_turns} turns. The earlier turns remain in the native conversation."),
            detail: None,
            title: Some("Native history".into()),
        })?,
    });
    serde_json::to_writer(ProjectionBudget::default(), &turns).map_err(|_| history_budget())
}

#[derive(Default)]
struct ReplayTool {
    id: String,
    fields: Map<String, Value>,
    content: Vec<(bool, Value)>,
}

impl ReplayTool {
    fn patch(&mut self, update: &Value) {
        for key in [
            "title",
            "kind",
            "name",
            "status",
            "locations",
            "rawInput",
            "rawOutput",
        ] {
            if let Some(value) = update.get(key) {
                self.fields.insert(key.into(), value.clone());
            }
        }
        if let Some(content) = update.get("content") {
            if update["sessionUpdate"] == "tool_call_content_chunk" {
                self.content.push((true, content.clone()));
            } else if content.is_array() || content.is_null() {
                self.content = content
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|block| (false, block.clone()))
                    .collect();
            }
        }
    }

    async fn finish(
        self,
        projection: &mut Projection<'_>,
    ) -> Result<ExternalAgentHistoryPart, AppError> {
        let fields = Value::Object(self.fields);
        let title = bounded_string(&fields["title"], 512).unwrap_or_else(|| "Tool call".into());
        let mut input = json!({"agent": projection.agent});
        for (wire, stored) in [
            ("kind", "kind"),
            ("name", "name"),
            ("locations", "locations"),
            ("rawInput", "raw_input"),
        ] {
            if let Some(value) = fields.get(wire) {
                input[stored] = value.clone();
            }
        }
        let mut raw = RawOutput::default();
        let mut blobs = Vec::new();
        let mut text = String::new();
        let mut items = Vec::new();
        let mut diff_remaining = TOOL_DIFF_BYTES;
        let mut adjacent = false;
        for (chunk, mut item) in self.content {
            match item["type"].as_str() {
                Some("content") => match projection.block(item["content"].take()).await? {
                    Block::Text(value) => {
                        if !value.is_empty() {
                            if !text.is_empty() && !(chunk && adjacent) {
                                text.push('\n');
                            }
                            text.push_str(&value);
                            adjacent = true;
                            continue;
                        }
                    }
                    Block::Link(link) => items.push(link.tool_item()),
                    Block::Media(attachment, blob) => {
                        push_blob(&mut blobs, blob)?;
                        raw.attachments.push(attachment);
                    }
                },
                Some("diff") => {
                    if item["changes"].is_array() {
                        items.extend(capture_v2_diff(&item, &mut diff_remaining));
                    } else if let Some(path) = bounded_string(&item["path"], 8192) {
                        items.push(json!({
                            "type":"diff", "path":path,
                            "old_text":item["oldText"], "new_text":item["newText"]
                        }));
                    }
                }
                Some("terminal") => {
                    if let Some(id) = bounded_string(&item["terminalId"], 4096) {
                        items.push(json!({"type":"terminal", "terminal_id":id}));
                    }
                }
                Some("session") => {
                    if let Some(id) = bounded_string(&item["sessionId"], 4096) {
                        items.push(json!({"type":"session", "session_id":id}));
                    }
                }
                _ => {}
            }
            adjacent = false;
        }
        raw.payload = Some(json!({
            "text":text,
            "external_agent":{
                "content":items, "locations":fields["locations"], "output":fields["rawOutput"],
            }
        }));
        let phase = match fields["status"].as_str() {
            Some("completed") => ToolPhase::Succeeded,
            Some("failed") => ToolPhase::Failed,
            Some("cancelled") => ToolPhase::Cancelled,
            _ => ToolPhase::Interrupted,
        };
        tool_part(
            projection.agent,
            format!("acp:{}", self.id),
            &title,
            input,
            raw,
            blobs,
            phase,
        )
    }
}

struct Projection<'a> {
    agent: ExternalAgent,
    cwd: &'a Path,
    media_bytes: usize,
    sequence: usize,
}

enum Block {
    Text(String),
    Link(Link),
    Media(AttachmentItem, ExternalAgentHistoryBlob),
}

impl Projection<'_> {
    async fn user(
        &mut self,
        blocks: Vec<Value>,
    ) -> Result<(ComposerDocument, Vec<ExternalAgentHistoryBlob>), AppError> {
        let mut document = ComposerDocument::default();
        let mut blobs = Vec::new();
        for block in blocks {
            if block["type"] == "skill" {
                if let Some(name) = command_name(&block["name"]) {
                    document.0.push(ComposerNode::activity(ComposerActivity {
                        id: ActivityId::new(),
                        payload: ActivityPayload::SkillReference(
                            agena_domain::SkillReferenceActivity::external(
                                self.agent,
                                name.to_owned(),
                                String::new(),
                            ),
                        ),
                        provenance: Default::default(),
                    }));
                }
                continue;
            }
            let resource = match self.block(block).await? {
                Block::Text(text) => {
                    push_user_text(&mut document, text);
                    continue;
                }
                Block::Link(link) => {
                    let Some(reference) = link.reference(self.cwd) else {
                        push_user_text(&mut document, link.label());
                        continue;
                    };
                    ResourceActivity {
                        kind: if matches!(reference, ResourceReference::Url { .. }) {
                            ResourceKind::Url
                        } else {
                            ResourceKind::File
                        },
                        reference,
                        delivery: ResourceDelivery::Reference,
                        name: link.name,
                        media_type: link.mime,
                        size_bytes: link.size,
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: None,
                    }
                }
                Block::Media(item, blob) => {
                    let AttachmentSource::Url { url } = item.source else {
                        return Err(AppError::Internal(
                            "missing imported media reference".into(),
                        ));
                    };
                    push_blob(&mut blobs, blob)?;
                    ResourceActivity {
                        delivery: ResourceDelivery::ModelInput,
                        kind: match item.kind {
                            AttachmentKind::Image => ResourceKind::Image,
                            AttachmentKind::Audio => ResourceKind::Audio,
                            AttachmentKind::Video => ResourceKind::Video,
                            AttachmentKind::Pdf => ResourceKind::Pdf,
                            AttachmentKind::File => ResourceKind::File,
                        },
                        reference: ResourceReference::Artifact {
                            uri: url,
                            sha256: item.sha256.unwrap_or_default(),
                        },
                        name: item.filename.unwrap_or_else(|| "Agent attachment".into()),
                        media_type: Some(item.mime),
                        size_bytes: item.size_bytes,
                        width: item.width,
                        height: item.height,
                        duration_ms: item.duration_ms,
                        page_count: item.page_count,
                    }
                }
            };
            document.0.push(ComposerNode::activity(ComposerActivity {
                id: ActivityId::new(),
                payload: ActivityPayload::Resource(resource),
                provenance: Default::default(),
            }));
        }
        Ok((document, blobs))
    }

    async fn reply(
        &mut self,
        blocks: Vec<Value>,
        reasoning: bool,
        output: &mut Vec<ExternalAgentHistoryPart>,
    ) -> Result<(), AppError> {
        let mut text = String::new();
        for value in blocks {
            let (title, raw, blobs) = match self.block(value).await? {
                Block::Text(value) => {
                    text.push_str(&value);
                    continue;
                }
                Block::Link(link) => (
                    "Resource",
                    RawOutput {
                        payload: Some(json!({"external_agent":{"content":[link.tool_item()]}})),
                        ..Default::default()
                    },
                    vec![],
                ),
                Block::Media(item, blob) => (
                    "Attachment",
                    RawOutput {
                        attachments: vec![item],
                        ..Default::default()
                    },
                    vec![blob],
                ),
            };
            push_reply_text(output, std::mem::take(&mut text), reasoning);
            self.sequence += 1;
            output.push(tool_part(
                self.agent,
                format!("acp-replay-resource:{}", self.sequence),
                title,
                json!({"agent":self.agent}),
                raw,
                blobs,
                ToolPhase::Succeeded,
            )?);
        }
        push_reply_text(output, text, reasoning);
        Ok(())
    }

    async fn block(&mut self, value: Value) -> Result<Block, AppError> {
        match value["type"].as_str() {
            Some("text") => Ok(Block::Text(clean_text(
                value["text"].as_str().unwrap_or_default(),
            ))),
            Some("resource_link") => Ok(Link::parse(&value).map_or_else(
                || Block::Text("[Invalid ACP resource reference]".into()),
                Block::Link,
            )),
            Some("resource" | "image" | "audio") => self.media(value).await,
            _ => Ok(Block::Text(String::new())),
        }
    }

    async fn media(&mut self, value: Value) -> Result<Block, AppError> {
        let loaded = MEDIA_WORKERS
            .run(move || decode_media(value))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let prepared = match loaded {
            Ok(prepared) => prepared,
            Err(reason) => {
                return Ok(Block::Text(format!(
                    "[ACP attachment unavailable: {reason}]"
                )));
            }
        };
        self.media_bytes += prepared.bytes.len();
        if self.media_bytes > MAX_EXTERNAL_AGENT_HISTORY_MEDIA_BYTES {
            return Err(AppError::Config(
                "This native conversation exceeds the 8 MiB attachment import limit.".into(),
            ));
        }
        let id = BlobId::new();
        let mut item = prepared.attachment(None);
        bind_agent_attachment(self.agent, &mut item, id)?;
        let descriptor = BlobDescriptor {
            id,
            byte_len: prepared.bytes.len() as u64,
            sha256: Sha256::digest(&prepared.bytes).into(),
        };
        Ok(Block::Media(
            item,
            ExternalAgentHistoryBlob {
                descriptor,
                bytes: prepared.bytes.into(),
            },
        ))
    }
}

fn decode_media(value: Value) -> Result<media_input::PreparedMedia, String> {
    let kind = value["type"].as_str().unwrap_or_default();
    let resource = &value["resource"];
    let (expected, fields) = match kind {
        "image" => (Some(AttachmentKind::Image), &value),
        "audio" => (Some(AttachmentKind::Audio), &value),
        _ => (None, resource),
    };
    let name = bounded_string(&fields["name"], 512)
        .or_else(|| {
            let uri = fields["uri"].as_str()?;
            let segment = uri.rsplit('/').next()?;
            (!segment.is_empty() && segment.len() <= 512).then(|| segment.to_owned())
        })
        .unwrap_or_else(|| match kind {
            "image" => "Image".into(),
            "audio" => "Audio".into(),
            _ => "Resource".into(),
        });
    let mime = bounded_string(&fields["mimeType"], 128);
    let embedded_text = fields["text"].as_str();
    let bytes = if let Some(text) = embedded_text {
        text.as_bytes().to_vec()
    } else {
        let key = if kind == "resource" { "blob" } else { "data" };
        STANDARD
            .decode(fields[key].as_str().ok_or("missing resource bytes")?)
            .map_err(|_| "invalid base64 data")?
    };
    if bytes.is_empty() {
        return Err("resource has no bytes".into());
    }
    // ACP embedded resources may be arbitrary documents. Only recognized
    // media is admitted to the inline renderer. Other bytes remain an
    // attachment with a plain-text or download-only media type.
    if kind == "resource"
        && (embedded_text.is_some()
            || mime.as_deref().is_none_or(|mime| {
                !(mime.starts_with("image/")
                    || mime.starts_with("audio/")
                    || mime.starts_with("video/")
                    || mime == "application/pdf")
            }))
    {
        let filename = agena_domain::safe_attachment_filename(&name);
        return Ok(media_input::PreparedMedia {
            filename: if filename.is_empty() {
                "Resource".into()
            } else {
                filename
            },
            mime: if embedded_text.is_some() {
                "text/plain".into()
            } else {
                "application/octet-stream".into()
            },
            kind: AttachmentKind::File,
            size_bytes: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(&bytes)),
            width: None,
            height: None,
            bytes,
        });
    }
    let prepared = media_input::from_bytes(&name, bytes, mime.as_deref())?;
    if expected.is_some_and(|expected| expected != prepared.kind) {
        return Err("content does not match its reported type".into());
    }
    Ok(prepared)
}

struct Link {
    uri: String,
    name: String,
    mime: Option<String>,
    size: Option<u64>,
}

impl Link {
    fn parse(value: &Value) -> Option<Self> {
        let uri = bounded_string(&value["uri"], 8192)?;
        let name = bounded_string(&value["name"], 512)
            .or_else(|| bounded_string(&value["title"], 512))
            .unwrap_or_else(|| "Resource".into());
        Some(Self {
            uri,
            name,
            mime: bounded_string(&value["mimeType"], 128),
            size: value["size"].as_u64(),
        })
    }

    fn label(&self) -> String {
        format!("{}: {}", self.name, self.uri)
    }

    fn reference(&self, cwd: &Path) -> Option<ResourceReference> {
        let url = url::Url::parse(&self.uri).ok()?;
        match url.scheme() {
            "http" | "https" if url.has_host() => Some(ResourceReference::Url {
                url: self.uri.clone(),
            }),
            "file" => {
                let path = url.to_file_path().ok()?;
                let relative = path.strip_prefix(cwd).ok()?;
                if relative.as_os_str().is_empty()
                    || relative
                        .components()
                        .any(|component| !matches!(component, Component::Normal(_)))
                {
                    return None;
                }
                let path = relative.to_str()?;
                (!path.chars().any(char::is_control)).then(|| ResourceReference::WorkspacePath {
                    path: path.to_owned(),
                })
            }
            _ => None,
        }
    }

    fn tool_item(&self) -> Value {
        json!({
            "type":"resource",
            "text":self.label(),
            "uri":self.uri,
            "name":self.name,
            "mime_type":self.mime,
            "size_bytes":self.size,
        })
    }
}

fn tool_part(
    agent: ExternalAgent,
    call_id: String,
    title: &str,
    input: Value,
    raw: RawOutput,
    blobs: Vec<ExternalAgentHistoryBlob>,
    phase: ToolPhase,
) -> Result<ExternalAgentHistoryPart, AppError> {
    let name = format!("{} · {title}", agent.label());
    let args = StructuredObject::try_from(input.clone()).map_err(AppError::Internal)?;
    let invocation = ToolInvocation::new(name.clone(), args);
    Ok(ExternalAgentHistoryPart::Tool {
        start: ToolStart {
            call_id,
            name,
            input,
        },
        result: ToolResult {
            phase,
            value: json!({"raw_output":raw, "invocation":invocation}),
        },
        blobs,
    })
}

fn push_blob(
    blobs: &mut Vec<ExternalAgentHistoryBlob>,
    blob: ExternalAgentHistoryBlob,
) -> Result<(), AppError> {
    if blobs.len() >= MAX_ENTRY_BLOBS {
        return Err(AppError::Config(
            "An imported message or tool result contains more than 16 attachments.".into(),
        ));
    }
    blobs.push(blob);
    Ok(())
}

fn push_user_text(document: &mut ComposerDocument, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(ComposerNode::Text { text: last }) = document.0.last_mut() {
        last.push_str(&text);
    } else {
        document.0.push(ComposerNode::Text { text });
    }
}

fn push_reply_text(output: &mut Vec<ExternalAgentHistoryPart>, text: String, reasoning: bool) {
    if !text.is_empty() {
        output.push(ExternalAgentHistoryPart::Text { text, reasoning });
    }
}

fn clean_text(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
        .collect()
}

fn history_budget() -> AppError {
    AppError::Config("This native conversation exceeds the history import budget.".into())
}
