use std::{
    collections::{HashMap, HashSet, VecDeque, hash_map::RandomState},
    hash::{BuildHasher, Hash},
    io::{self, Write},
    path::{Component, Path},
};

use agena_domain::{
    ActivityId, ActivityPayload, AttachmentItem, AttachmentKind, AttachmentSource, BlobId,
    ComposerActivity, ComposerDocument, ComposerNode, ExternalAgent, ExternalAgentHistoryPreview,
    ExternalAgentHistoryPreviewTurn, RawOutput, ResourceActivity, ResourceDelivery, ResourceKind,
    ResourceReference, StructuredObject, ToolInvocation,
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
const MAX_USER_VIEW_BYTES: usize = 224 * 1024;
const MAX_HISTORY_PREVIEW_TURNS: usize = 8;
const MAX_HISTORY_PREVIEW_TEXT_BYTES: usize = 32 * 1024;

#[derive(Default)]
pub(in super::super) struct Replay {
    entries: VecDeque<BufferedRecord>,
    first: usize,
    user_starts: VecDeque<usize>,
    messages: HashMap<MessageKey, Vec<usize>>,
    tools: HashMap<String, usize>,
    discarded: DiscardedIds,
    cursors: cursors::Book,
    announced_entities: HashSet<cursors::Entity>,
    pub(super) allow_rewind: bool,
    state: ExternalAgentState,
    wire: usize,
    updates: usize,
    discarded_turns: usize,
    omitted_content: bool,
    clipped_turn: Option<ClippedTurn>,
    pub(super) complete: bool,
    complete_children: HashSet<String>,
    preview: Option<PreviewWindow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Role {
    User,
    Assistant,
    Thought,
}

#[derive(Clone, Copy)]
enum ClippedTurn {
    Input,
    Reply,
}

struct PreviewWindow {
    before_turn: Option<u64>,
    ended: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MessageKey {
    role: Role,
    id: Option<String>,
}

impl MessageKey {
    fn entity(&self) -> Option<cursors::Entity> {
        self.id
            .as_deref()
            .map(|id| cursors::Entity::message(message_kind(&self.role), id))
    }
}

enum Record {
    Blocks { key: MessageKey, blocks: Vec<Value> },
    Tool(ReplayTool),
    Anchor(cursors::Entity),
    Omitted,
    Discarded,
}

struct BufferedRecord {
    value: Record,
    bytes: usize,
    updates: usize,
}

/// A bounded record of discarded native identities. Conservative matches
/// prevent a very late replacement from reopening an evicted turn.
#[derive(Default)]
struct DiscardedIds {
    hash: RandomState,
    bits: Vec<u64>,
}

impl DiscardedIds {
    const WORDS: usize = 32 * 1024;

    fn indexes(&self, key: &impl Hash) -> [usize; 3] {
        let hash = self.hash.hash_one(key);
        let mask = Self::WORDS * 64 - 1;
        [
            hash as usize & mask,
            (hash >> 21) as usize & mask,
            (hash >> 42) as usize & mask,
        ]
    }

    fn mark(&mut self, key: &impl Hash) {
        if self.bits.is_empty() {
            self.bits.resize(Self::WORDS, 0);
        }
        for index in self.indexes(key) {
            self.bits[index / 64] |= 1 << (index % 64);
        }
    }

    fn contains(&self, key: &impl Hash) -> bool {
        !self.bits.is_empty()
            && self
                .indexes(key)
                .into_iter()
                .all(|index| self.bits[index / 64] & (1 << (index % 64)) != 0)
    }
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

fn replay_weight(value: &Value) -> usize {
    let mut budget = ReplayBudget::<MAX_REPLAY_WIRE_BYTES>::default();
    let _ = serde_json::to_writer(&mut budget, value);
    budget.0
}

fn omitted_block(reason: &str) -> Value {
    json!({"type": "text", "text": format!("[{reason}]")})
}

fn reduced_update(update: &Value) -> Value {
    let kind = update["sessionUpdate"].as_str().unwrap_or_default();
    let mut reduced = json!({"sessionUpdate": kind});
    for field in ["messageId", "toolCallId", "title", "status", "kind"] {
        if let Some(value) = bounded_string(&update[field], 4096) {
            reduced[field] = json!(value);
        }
    }
    let block = omitted_block("Large native content omitted from this history view");
    reduced["content"] = match kind {
        "tool_call" | "tool_call_update" => json!([{"type": "content", "content": block}]),
        "tool_call_content_chunk" => json!({"type": "content", "content": block}),
        _ => block,
    };
    reduced
}

fn message_kind(role: &Role) -> &'static str {
    match role {
        Role::User => "user_message",
        Role::Assistant => "agent_message",
        Role::Thought => "agent_thought",
    }
}

impl Replay {
    pub(super) fn preview(before_turn: Option<u64>) -> Self {
        Self {
            preview: Some(PreviewWindow {
                before_turn,
                ended: false,
            }),
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
            if self.preview.is_some() {
                return Ok(());
            }
            let historical = !v2
                || if source == root {
                    !self.complete
                } else {
                    !self.complete_children.contains(source)
                };
            if historical && kind == "subagent_update" && update.get("state").is_some() {
                // The replay establishes the parent/child relationship. Historical
                // subagents must not retain stale running states after import.
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
        if let Some(preview) = &self.preview {
            if message.is_none() && !tool {
                return Ok(());
            }
            // Peers which only replay from the start still send the remainder.
            // Preserve late snapshots of retained IDs within the selected page.
            if preview.ended
                && !message.as_ref().is_some_and(|(role, _)| {
                    bounded_string(&update["messageId"], 4096).is_some_and(|id| {
                        self.messages.contains_key(&MessageKey {
                            role: role.clone(),
                            id: Some(id),
                        })
                    })
                })
                && !(tool
                    && update["toolCallId"]
                        .as_str()
                        .is_some_and(|id| self.tools.contains_key(id)))
            {
                return Ok(());
            }
        }
        if message.is_none() && !tool && !cursor {
            apply_native_update(&mut self.state, root, source, update);
            if !self.complete
                && let Some(entity) = cursors::Entity::from_update(update)
                && self.clipped_turn.is_none()
                && !self.discarded.contains(&entity)
                && self.announced_entities.insert(entity.clone())
            {
                let charge = replay_weight(update).min(MAX_REPLAY_WIRE_BYTES);
                self.push(Record::Anchor(entity), charge);
                self.roll_window();
            }
            return Ok(());
        }
        // Live transcript events can arrive before the response. The marker
        // ends the historical range, even if the peer keeps streaming.
        if self.complete {
            if cursor {
                self.cursors.observe(update, self.allow_rewind, 0);
            }
            return Ok(());
        }
        let charge = replay_weight(update);
        let reduced;
        let (update, charge) = if charge > MAX_REPLAY_WIRE_BYTES {
            self.omitted_content = true;
            reduced = reduced_update(update);
            (&reduced, replay_weight(&reduced))
        } else {
            (update, charge)
        };
        if let Some((role, replace)) = message {
            self.message(role, replace, update, charge);
        } else if tool {
            self.tool(update, charge);
        } else if !cursors::anchored_entity(update)
            .is_some_and(|entity| self.discarded.contains(&entity))
        {
            self.cursors
                .observe(update, self.allow_rewind, MAX_REPLAY_RECORDS);
        }
        self.roll_window();
        Ok(())
    }

    fn record_mut(&mut self, index: usize) -> &mut Record {
        &mut self.entries[index - self.first].value
    }

    fn message(&mut self, role: Role, replace: bool, update: &Value, charge: usize) {
        let key = MessageKey {
            role,
            id: bounded_string(&update["messageId"], 4096),
        };
        if key
            .entity()
            .is_some_and(|entity| self.discarded.contains(&entity))
            && !self.messages.contains_key(&key)
        {
            return;
        }
        if let Some(clipped) = self.clipped_turn {
            if key.role != Role::User {
                self.clipped_turn = Some(ClippedTurn::Reply);
                return;
            }
            if self.messages.contains_key(&key)
                || (key.id.is_none() && matches!(clipped, ClippedTurn::Input))
            {
                return;
            }
            self.clipped_turn = None;
        }
        if replace && update.get("content").is_none() && self.messages.contains_key(&key) {
            return;
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
                for slot in slots.split_off(1) {
                    self.replace_record(slot, Record::Discarded, 0);
                }
                self.replace_record(keep, Record::Blocks { key, blocks }, charge);
                return;
            }
        } else {
            let slot = if key.role == Role::User {
                self.messages
                    .get(&key)
                    .and_then(|slots| slots.first().copied())
            } else {
                None
            };
            let adjacent = self
                .entries
                .len()
                .checked_sub(1)
                .map(|index| self.first + index);
            if let Some(index) = slot.or(adjacent)
                && let Record::Blocks {
                    key: prior,
                    blocks: saved,
                } = self.record_mut(index)
                && *prior == key
            {
                saved.extend(blocks);
                self.charge(index, charge);
                return;
            }
        }
        if key.role == Role::User
            && let Some(preview) = &mut self.preview
            && preview.before_turn.is_some_and(|boundary| {
                (self.discarded_turns + self.user_starts.len()) as u64 + 1 >= boundary
            })
        {
            preview.ended = true;
            return;
        }
        let index = self.push(
            Record::Blocks {
                key: key.clone(),
                blocks,
            },
            charge,
        );
        if key.role == Role::User {
            self.user_starts.push_back(index);
        }
        if key.id.is_some() {
            self.messages.entry(key).or_default().push(index);
        }
    }

    fn tool(&mut self, update: &Value, charge: usize) {
        let Some(id) = bounded_string(&update["toolCallId"], 4092) else {
            return;
        };
        if self
            .discarded
            .contains(&cursors::Entity::message("tool_call", &id))
            && !self.tools.contains_key(&id)
        {
            return;
        }
        if self.clipped_turn.is_some() {
            self.clipped_turn = Some(ClippedTurn::Reply);
            return;
        }
        if let Some(&index) = self.tools.get(&id)
            && let Record::Tool(tool) = self.record_mut(index)
        {
            tool.patch(update);
            self.charge(index, charge);
            return;
        }
        let mut tool = ReplayTool {
            id: id.clone(),
            ..Default::default()
        };
        tool.patch(update);
        let index = self.push(Record::Tool(tool), charge);
        self.tools.insert(id, index);
    }

    fn push(&mut self, value: Record, bytes: usize) -> usize {
        let index = self.first + self.entries.len();
        let updates = usize::from(bytes != 0);
        self.entries.push_back(BufferedRecord {
            value,
            bytes,
            updates,
        });
        self.wire += bytes;
        self.updates += updates;
        index
    }

    fn charge(&mut self, index: usize, bytes: usize) {
        let frame = &mut self.entries[index - self.first];
        frame.bytes += bytes;
        frame.updates += 1;
        self.wire += bytes;
        self.updates += 1;
    }

    fn replace_record(&mut self, index: usize, value: Record, bytes: usize) {
        let frame = &mut self.entries[index - self.first];
        let updates = usize::from(bytes != 0);
        self.wire = self.wire - frame.bytes + bytes;
        self.updates = self.updates - frame.updates + updates;
        *frame = BufferedRecord {
            value,
            bytes,
            updates,
        };
    }

    fn discard_before(&mut self, cut: usize) {
        while self.first < cut {
            let Some(frame) = self.entries.pop_front() else {
                break;
            };
            let index = self.first;
            self.first += 1;
            self.wire -= frame.bytes;
            self.updates -= frame.updates;
            if self.user_starts.front() == Some(&index) {
                self.user_starts.pop_front();
            }
            match frame.value {
                Record::Blocks { key, .. } => {
                    if let Some(slots) = self.messages.get_mut(&key) {
                        slots.retain(|slot| *slot != index);
                        if slots.is_empty() {
                            self.messages.remove(&key);
                            if let Some(entity) = key.entity() {
                                self.cursors.forget(&entity);
                                self.discarded.mark(&entity);
                            }
                        }
                    }
                }
                Record::Tool(tool) => {
                    self.tools.remove(&tool.id);
                    let entity = cursors::Entity::message("tool_call", &tool.id);
                    self.cursors.forget(&entity);
                    self.discarded.mark(&entity);
                }
                Record::Anchor(entity) => {
                    self.announced_entities.remove(&entity);
                    self.cursors.forget(&entity);
                    self.discarded.mark(&entity);
                }
                Record::Omitted | Record::Discarded => {}
            }
        }
    }

    fn roll_window(&mut self) {
        while self.entries.len() > MAX_REPLAY_RECORDS
            || self.updates > MAX_REPLAY_UPDATES
            || self.wire > MAX_REPLAY_WIRE_BYTES
            || (self.preview.is_some() && self.user_starts.len() > MAX_HISTORY_PREVIEW_TURNS)
        {
            if let Some(&next_turn) = self.user_starts.get(1) {
                self.discard_before(next_turn);
                self.discarded_turns += 1;
                continue;
            }
            if let Some(&first_user) = self.user_starts.front()
                && self.first < first_user
            {
                self.discard_before(first_user);
                self.omitted_content = true;
                continue;
            }

            // A single turn can exceed the complete replay budget on its own.
            // Keep its input and suppress its remaining output until the next user.
            let clipped = if self.entries.iter().any(|frame| {
                matches!(
                    &frame.value,
                    Record::Blocks { key, .. } if key.role != Role::User
                ) || matches!(&frame.value, Record::Tool(_))
            }) {
                ClippedTurn::Reply
            } else {
                ClippedTurn::Input
            };
            let saved = if self.user_starts.front() == Some(&self.first) {
                let frame = self.entries.front_mut().expect("replay user boundary");
                let Record::Blocks { key, mut blocks } =
                    std::mem::replace(&mut frame.value, Record::Discarded)
                else {
                    unreachable!("replay boundary must be a user record")
                };
                let mut bytes = frame.bytes;
                if bytes > MAX_USER_VIEW_BYTES {
                    blocks = vec![omitted_block(
                        "Native user message exceeded the view size limit",
                    )];
                    bytes = replay_weight(&Value::Array(blocks.clone()));
                }
                self.messages.remove(&key);
                Some((key, blocks, bytes))
            } else {
                None
            };
            self.discard_before(self.first + self.entries.len());
            if let Some((key, blocks, bytes)) = saved {
                let index = self.push(
                    Record::Blocks {
                        key: key.clone(),
                        blocks,
                    },
                    bytes,
                );
                self.user_starts.push_back(index);
                if key.id.is_some() {
                    self.messages.insert(key, vec![index]);
                }
                self.push(Record::Omitted, 0);
            }
            self.clipped_turn = Some(clipped);
            self.omitted_content = true;
        }
    }

    pub(super) fn finish_preview(self) -> ExternalAgentHistoryPreview {
        let mut turns = Vec::<ExternalAgentHistoryPreviewTurn>::new();
        let mut omitted_content = self.omitted_content;
        for frame in self.entries {
            match frame.value {
                Record::Blocks { key, blocks } if key.role != Role::Thought => {
                    let mut rendered = String::new();
                    let mut block_omitted = false;
                    for block in blocks {
                        let snippet = match block["type"].as_str() {
                            Some("text") => block["text"].as_str().map(str::to_owned),
                            Some("skill") => block["name"]
                                .as_str()
                                .filter(|name| !name.trim().is_empty())
                                .map(|name| format!("/{name}")),
                            Some("resource_link") => block["uri"].as_str().map(str::to_owned),
                            Some("resource") => block["resource"]["text"]
                                .as_str()
                                .map(str::to_owned),
                            _ => None,
                        };
                        match snippet {
                            Some(text) => {
                                if !rendered.is_empty()
                                    && !rendered.ends_with('\n')
                                    && !text.starts_with('\n')
                                    && block["type"] != "text"
                                {
                                    let _ = append_preview(&mut rendered, "\n");
                                }
                                if !append_preview(&mut rendered, &text) {
                                    block_omitted = true;
                                }
                            }
                            None => block_omitted = true,
                        }
                    }
                    if key.role == Role::User {
                        if rendered.is_empty() && !block_omitted {
                            continue;
                        }
                        turns.push(ExternalAgentHistoryPreviewTurn {
                            user: rendered,
                            reply: String::new(),
                        });
                    } else if let Some(turn) = turns.last_mut() {
                        if !turn.reply.is_empty()
                            && !rendered.is_empty()
                            && !turn.reply.ends_with('\n')
                            && !rendered.starts_with('\n')
                        {
                            let _ = append_preview(&mut turn.reply, "\n");
                        }
                        if !append_preview(&mut turn.reply, &rendered) {
                            block_omitted = true;
                        }
                    } else {
                        block_omitted = true;
                    }
                    if block_omitted {
                        omitted_content = true;
                    }
                }
                Record::Blocks { .. } | Record::Tool(_) | Record::Omitted => omitted_content = true,
                Record::Anchor(_) | Record::Discarded => {}
            }
        }
        ExternalAgentHistoryPreview {
            turns,
            next_before_turn: (self.discarded_turns != 0)
                .then_some(self.discarded_turns as u64 + 1),
            omitted_content,
        }
    }

    pub(super) async fn finish(
        mut self,
        agent: ExternalAgent,
        cwd: &Path,
    ) -> Result<(Vec<ExternalAgentHistoryTurn>, ExternalAgentState), AppError> {
        let allow_media = recent_media_budget_flags(&self.entries);
        let mut projection = Projection {
            agent,
            cwd,
            media_bytes: 0,
            omitted_content: self.omitted_content,
            sequence: 0,
        };
        let mut turns = Vec::<ExternalAgentHistoryTurn>::new();
        let mut prelude = Vec::new();
        let mut preceding = None;
        for (index, frame) in self.entries.into_iter().enumerate() {
            let media_allowed = allow_media.get(index).copied().unwrap_or(true);
            match frame.value {
                Record::Blocks { key, blocks } if key.role == Role::User => {
                    let (document, blobs) = projection.user(blocks, media_allowed).await?;
                    if !document.is_empty() {
                        let cursor =
                            self.cursors.for_user(key.id.as_deref(), preceding.as_ref());
                        turns.push(ExternalAgentHistoryTurn {
                            user: document,
                            cursor,
                            blobs,
                            prelude: std::mem::take(&mut prelude),
                            response: Vec::new(),
                        });
                    }
                    preceding = key.entity();
                }
                Record::Blocks { key, blocks } => {
                    preceding = key.entity();
                    let output = turns
                        .last_mut()
                        .map_or(&mut prelude, |turn| &mut turn.response);
                    projection
                        .reply(blocks, key.role == Role::Thought, media_allowed, output)
                        .await?;
                }
                Record::Tool(tool) => {
                    preceding = Some(cursors::Entity::message("tool_call", &tool.id));
                    let part = tool.finish(&mut projection, media_allowed).await?;
                    turns
                        .last_mut()
                        .map_or(&mut prelude, |turn| &mut turn.response)
                        .push(part);
                }
                Record::Anchor(entity) => preceding = Some(entity),
                Record::Omitted => preceding = None,
                Record::Discarded => {}
            }
        }
        if !prelude.is_empty() {
            return Err(AppError::Config(
                "This native history has no recorded user message to associate with its assistant output.".into(),
            ));
        }
        self.state.cursor_tail = preceding.map(cursors::Entity::saved);
        retain_recent_turns(&mut turns, self.discarded_turns, projection.omitted_content)?;
        Ok((turns, self.state.bounded_snapshot(MAX_STATE_SNAPSHOT_BYTES)))
    }
}

fn retain_recent_turns(
    turns: &mut Vec<ExternalAgentHistoryTurn>,
    discarded_turns: usize,
    mut omitted_content: bool,
) -> Result<(), AppError> {
    if turns.is_empty()
        || (discarded_turns == 0 && !omitted_content && projected_size(turns).is_some())
    {
        return Ok(());
    }
    // The retained view must be a contiguous suffix ending at the real
    // conversation head. Reserve space for its array and projection notice.
    let mut remaining = MAX_EXTERNAL_AGENT_HISTORY_METADATA_BYTES - 1024;
    let available_turns = discarded_turns + turns.len();
    let mut retained_turns = 0;
    for turn in turns.iter().rev() {
        let Some(size) = projected_size(turn).filter(|size| size + 1 <= remaining) else {
            break;
        };
        remaining -= size + 1;
        retained_turns += 1;
    }
    if retained_turns == 0 {
        let newest = turns.last_mut().expect("nonempty native history");
        fit_latest_reply(newest, remaining)?;
        retained_turns = 1;
        omitted_content = true;
    }
    turns.drain(..turns.len() - retained_turns);
    let first = &mut turns[0];
    let mut summary = if retained_turns < available_turns {
        // The retained view has no local predecessor before its first user.
        first.cursor = None;
        format!(
            "Agena loaded the latest {retained_turns} of {available_turns} turns. The earlier turns remain in the native conversation."
        )
    } else {
        String::new()
    };
    if omitted_content {
        if !summary.is_empty() {
            summary.push(' ');
        }
        summary.push_str("Some native content exceeded the history view limits. The original content remains in the native conversation.");
    }
    first.prelude.insert(
        0,
        ExternalAgentHistoryPart::Notice {
            text: serde_json::to_string(&agena_runtime_contracts::NoticePart {
                kind: "external_agent_history_window".into(),
                summary,
                detail: None,
                title: Some("Native history".into()),
            })?,
        },
    );
    projected_size(turns).map(|_| ()).ok_or_else(history_budget)
}

fn projected_size(value: &impl serde::Serialize) -> Option<usize> {
    let mut budget = ReplayBudget::<MAX_EXTERNAL_AGENT_HISTORY_METADATA_BYTES>::default();
    serde_json::to_writer(&mut budget, value).ok()?;
    Some(budget.0)
}

fn fit_latest_reply(turn: &mut ExternalAgentHistoryTurn, budget: usize) -> Result<(), AppError> {
    turn.prelude.clear();
    let response = std::mem::take(&mut turn.response);
    let mut remaining = budget
        .checked_sub(projected_size(turn).ok_or_else(history_budget)?)
        .ok_or_else(history_budget)?;
    for mut part in response.into_iter().rev() {
        if let Some(size) = projected_size(&part).filter(|size| size + 1 <= remaining) {
            remaining -= size + 1;
            turn.response.push(part);
            continue;
        }
        if retain_text_suffix(&mut part, remaining) {
            turn.response.push(part);
        }
        break;
    }
    turn.response.reverse();
    Ok(())
}

fn retain_text_suffix(part: &mut ExternalAgentHistoryPart, budget: usize) -> bool {
    let ExternalAgentHistoryPart::Text { text, reasoning } = part else {
        return false;
    };
    let overhead = projected_size(&ExternalAgentHistoryPart::Text {
        text: String::new(),
        reasoning: *reasoning,
    })
    .expect("empty history text size");
    let mut remaining = budget.saturating_sub(overhead + 1);
    let mut start = text.len();
    for (index, character) in text.char_indices().rev() {
        let bytes = match character {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
            '\u{00}'..='\u{1f}' => 6,
            _ => character.len_utf8(),
        };
        let Some(rest) = remaining.checked_sub(bytes) else {
            break;
        };
        remaining = rest;
        start = index;
    }
    if start == text.len() {
        return false;
    }
    text.drain(..start);
    true
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
            if let Some(value) = update.get(key).filter(|value| !value.is_null()) {
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
        media_allowed: bool,
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
        const MAX_TOOL_TEXT_BYTES: usize = 256 * 1024;
        for (chunk, mut item) in self.content {
            match item["type"].as_str() {
                Some("content") => match projection
                    .block(
                        item["content"].take(),
                        media_allowed && blobs.len() < MAX_ENTRY_BLOBS,
                    )
                    .await?
                {
                    Block::Text(value) => {
                        if !value.is_empty() && text.len() < MAX_TOOL_TEXT_BYTES {
                            if !text.is_empty() && !(chunk && adjacent) {
                                text.push('\n');
                            }
                            let room = MAX_TOOL_TEXT_BYTES.saturating_sub(text.len());
                            let end = value.floor_char_boundary(room);
                            text.push_str(&value[..end]);
                            if end < value.len() {
                                projection.omitted_content = true;
                            }
                            adjacent = true;
                            continue;
                        }
                    }
                    Block::Link(link) => items.push(link.tool_item()),
                    Block::Media(attachment, blob) => {
                        blobs.push(blob);
                        raw.attachments.push(attachment);
                    }
                },
                Some("diff") => {
                    if item["changes"].is_array() {
                        items.extend(capture_v2_diff(&item, &mut diff_remaining));
                    } else if let Some(path) = bounded_string(&item["path"], 8192)
                        && diff_remaining > 0
                    {
                        let mut clip_field = |val: &Value, budget: &mut usize| -> Value {
                            match val.as_str() {
                                Some(s) => {
                                    let end = s.floor_char_boundary(*budget);
                                    *budget = budget.saturating_sub(end);
                                    if end < s.len() {
                                        projection.omitted_content = true;
                                    }
                                    json!(&s[..end])
                                }
                                None => val.clone(),
                            }
                        };
                        let old_text = clip_field(&item["oldText"], &mut diff_remaining);
                        let new_text = clip_field(&item["newText"], &mut diff_remaining);
                        items.push(json!({
                            "type":"diff", "path":path,
                            "old_text":old_text, "new_text":new_text
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
            projection.agent.clone(),
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
    omitted_content: bool,
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
        media_allowed: bool,
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
                                self.agent.clone(),
                                name.to_owned(),
                                String::new(),
                            ),
                        ),
                        provenance: Default::default(),
                    }));
                }
                continue;
            }
            let resource = match self
                .block(block, media_allowed && blobs.len() < MAX_ENTRY_BLOBS)
                .await?
            {
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
                    blobs.push(blob);
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
        if serde_json::to_writer(ReplayBudget::<MAX_USER_VIEW_BYTES>::default(), &document).is_err()
        {
            self.omitted_content = true;
            self.media_bytes -= blobs.iter().map(|blob| blob.bytes.len()).sum::<usize>();
            blobs.clear();
            document = ComposerDocument::default();
            push_user_text(
                &mut document,
                "[Native user message exceeded the view size limit]".into(),
            );
        }
        Ok((document, blobs))
    }

    async fn reply(
        &mut self,
        blocks: Vec<Value>,
        reasoning: bool,
        media_allowed: bool,
        output: &mut Vec<ExternalAgentHistoryPart>,
    ) -> Result<(), AppError> {
        let mut text = String::new();
        for value in blocks {
            let (title, raw, blobs) = match self.block(value, media_allowed).await? {
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
                self.agent.clone(),
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

    async fn block(&mut self, value: Value, room_for_media: bool) -> Result<Block, AppError> {
        match value["type"].as_str() {
            Some("text") => Ok(Block::Text(clean_text(
                value["text"].as_str().unwrap_or_default(),
            ))),
            Some("resource_link") => Ok(Link::parse(&value).map_or_else(
                || Block::Text("[Invalid ACP resource reference]".into()),
                Block::Link,
            )),
            Some("resource" | "image" | "audio") => self.media(value, room_for_media).await,
            _ => Ok(Block::Text(String::new())),
        }
    }

    async fn media(&mut self, value: Value, room_for_media: bool) -> Result<Block, AppError> {
        let remaining = MAX_EXTERNAL_AGENT_HISTORY_MEDIA_BYTES - self.media_bytes;
        if !room_for_media || encoded_media_size(&value).is_some_and(|size| size > remaining) {
            self.omitted_content = true;
            return Ok(Block::Text(
                "[Native attachment omitted from this history view due to its attachment limits]"
                    .into(),
            ));
        }
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
        if prepared.bytes.len() > remaining {
            self.omitted_content = true;
            return Ok(Block::Text(
                "[Native attachment omitted from this history view due to its attachment limits]"
                    .into(),
            ));
        }
        self.media_bytes += prepared.bytes.len();
        let id = BlobId::new();
        let mut item = prepared.attachment(None);
        bind_agent_attachment(self.agent.clone(), &mut item, id)?;
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

fn recent_media_budget_flags(entries: &VecDeque<BufferedRecord>) -> Vec<bool> {
    let mut allowed = vec![false; entries.len()];
    let mut remaining = MAX_EXTERNAL_AGENT_HISTORY_MEDIA_BYTES;
    for (index, frame) in entries.iter().enumerate().rev() {
        let mut entry_media = 0usize;
        let mut counted = 0usize;
        let mut add_block = |block: &Value| {
            if counted >= MAX_ENTRY_BLOBS {
                return;
            }
            if matches!(block["type"].as_str(), Some("resource" | "image" | "audio"))
                && let Some(size) = encoded_media_size(block)
                && size <= MAX_EXTERNAL_AGENT_HISTORY_MEDIA_BYTES
            {
                entry_media = entry_media.saturating_add(size);
                counted += 1;
            }
        };
        match &frame.value {
            Record::Blocks { blocks, .. } => {
                for block in blocks {
                    add_block(block);
                }
            }
            Record::Tool(tool) => {
                for (_, item) in &tool.content {
                    if item["type"] == "content" {
                        add_block(&item["content"]);
                    }
                }
            }
            _ => {}
        }
        if entry_media == 0 {
            allowed[index] = true;
        } else if remaining > 0 {
            remaining = remaining.saturating_sub(entry_media);
            allowed[index] = true;
        }
    }
    allowed
}

fn encoded_media_size(value: &Value) -> Option<usize> {
    if value["type"] == "resource" {
        if let Some(text) = value["resource"]["text"].as_str() {
            return Some(text.len());
        }
        return Some(
            value["resource"]["blob"]
                .as_str()?
                .trim_end_matches('=')
                .len()
                * 3
                / 4,
        );
    }
    Some(value["data"].as_str()?.trim_end_matches('=').len() * 3 / 4)
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
    text.chars().filter(|ch| visible_text(*ch)).collect()
}

fn append_preview(output: &mut String, text: &str) -> bool {
    for ch in text.chars().filter(|ch| visible_text(*ch)) {
        if output.len() + ch.len_utf8() > MAX_HISTORY_PREVIEW_TEXT_BYTES {
            return false;
        }
        output.push(ch);
    }
    true
}

fn visible_text(ch: char) -> bool {
    !ch.is_control() || matches!(ch, '\n' | '\r' | '\t')
}

fn history_budget() -> AppError {
    AppError::Config("This native conversation exceeds the history import budget.".into())
}
