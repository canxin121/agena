//! Immutable LLM-input projection of session history.
//!
//! `ProviderTranscript` is the **canonical** representation of "what the LLM
//! sees". It is intentionally devoid of any field that mutates over a
//! message's lifetime (status, usage, timestamps, in-memory turn IDs, tags,
//! …). Anything that *does* end up in this structure is stable once the
//! producing event has been appended to the history log.
//!
//! Provider prompt-cache stability is derived directly from this invariant:
//! `digest()` hashes the transcript with a canonical encoding and the result
//! is the *only* signal `prompt_window` uses to decide whether an upstream
//! cache prefix is still valid.

use blake3::Hash;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::ToolCallId;

/// Ordered, append-only list of fragments delivered to the provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderTranscript {
    pub fragments: Vec<TranscriptFragment>,
}

impl ProviderTranscript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, fragment: TranscriptFragment) {
        self.fragments.push(fragment);
    }

    pub fn extend<I: IntoIterator<Item = TranscriptFragment>>(&mut self, iter: I) {
        self.fragments.extend(iter);
    }

    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    pub fn len(&self) -> usize {
        self.fragments.len()
    }

    /// Canonical, cache-stable digest of the transcript.
    ///
    /// Encoding rules — these are *the* contract with prompt-cache stability,
    /// touch with care:
    ///
    /// * Each fragment is encoded into a small fixed framing (kind tag byte +
    ///   length-prefixed components). No JSON / serde representation: the
    ///   serde format may evolve (new optional fields, renames) but the
    ///   on-the-wire bytes here MUST NOT.
    /// * Only fields that are observable to the provider participate.
    pub fn digest(&self) -> Hash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"agena-transcript-v1\0");
        for fragment in &self.fragments {
            fragment.hash_into(&mut hasher);
        }
        hasher.finalize()
    }

    /// Hex-encoded digest, convenient for logging and DB columns.
    pub fn digest_hex(&self) -> String {
        self.digest().to_hex().to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[display("{}", self.kind_tag())]
pub enum TranscriptFragment {
    System {
        text: String,
    },
    User {
        content: TranscriptContent,
    },
    Assistant {
        content: TranscriptContent,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<TranscriptToolCall>,
    },
    ToolResult {
        call_id: ToolCallId,
        output: TranscriptToolOutput,
    },
}

impl TranscriptFragment {
    fn kind_tag(&self) -> &'static str {
        match self {
            Self::System { .. } => "system",
            Self::User { .. } => "user",
            Self::Assistant { .. } => "assistant",
            Self::ToolResult { .. } => "tool_result",
        }
    }

    fn discriminant(&self) -> u8 {
        match self {
            Self::System { .. } => 0x01,
            Self::User { .. } => 0x02,
            Self::Assistant { .. } => 0x03,
            Self::ToolResult { .. } => 0x04,
        }
    }

    fn hash_into(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&[self.discriminant()]);
        match self {
            Self::System { text } => hash_str(hasher, text),
            Self::User { content } => content.hash_into(hasher),
            Self::Assistant {
                content,
                tool_calls,
            } => {
                content.hash_into(hasher);
                hash_len(hasher, tool_calls.len() as u64);
                for call in tool_calls {
                    call.hash_into(hasher);
                }
            }
            Self::ToolResult { call_id, output } => {
                hash_str(hasher, call_id.as_str());
                output.hash_into(hasher);
            }
        }
    }
}

/// Multi-modal content blocks delivered as a single message body.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptContent {
    pub blocks: Vec<TranscriptBlock>,
}

impl TranscriptContent {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            blocks: vec![TranscriptBlock::Text { text: text.into() }],
        }
    }

    pub fn push_text(&mut self, text: impl Into<String>) {
        self.blocks
            .push(TranscriptBlock::Text { text: text.into() });
    }

    /// Project a `Message`'s parts into transcript form. Multi-modal fidelity
    /// is preserved: text, reasoning, image and attachment blocks all round-
    /// trip; everything else collapses to its lossy text rendering as a final
    /// fallback so a message never produces an empty body.
    ///
    /// The name retains the `_lossy` suffix because the lossy fallback path
    /// is unavoidable for content kinds (file changes, web searches, …) the
    /// transcript shape simply does not model.
    pub fn from_message_lossy(message: &crate::message::Message) -> Self {
        use crate::message::{AttachmentKind, PartContent};
        let mut content = Self::default();
        let mut had_any = false;
        for part in &message.parts {
            match part.content.as_ref() {
                Some(PartContent::Text(text)) => {
                    if !text.text.is_empty() {
                        content.blocks.push(TranscriptBlock::Text {
                            text: text.text.clone(),
                        });
                        had_any = true;
                    }
                }
                Some(PartContent::Reasoning(reasoning)) => {
                    let joined = reasoning.summary.join("\n");
                    if !joined.is_empty() {
                        content
                            .blocks
                            .push(TranscriptBlock::Reasoning { text: joined });
                        had_any = true;
                    }
                }
                Some(PartContent::Attachment(attachment)) => {
                    for item in &attachment.attachments {
                        let media_type: SmolStr = item.mime.as_str().into();
                        let block = match item.kind {
                            AttachmentKind::Image => TranscriptBlock::Image {
                                media_type,
                                digest: attachment_digest_label(&item.source),
                            },
                            _ => TranscriptBlock::Attachment {
                                file_id: attachment_digest_label(&item.source).into(),
                                media_type: Some(media_type),
                            },
                        };
                        content.blocks.push(block);
                        had_any = true;
                    }
                }
                _ => {}
            }
        }
        if !had_any {
            let fallback = message.as_text_lossy();
            if !fallback.is_empty() {
                content
                    .blocks
                    .push(TranscriptBlock::Text { text: fallback });
            }
        }
        content
    }

    fn hash_into(&self, hasher: &mut blake3::Hasher) {
        hash_len(hasher, self.blocks.len() as u64);
        for block in &self.blocks {
            block.hash_into(hasher);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranscriptBlock {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Image {
        media_type: SmolStr,
        digest: String,
    },
    Attachment {
        file_id: SmolStr,
        media_type: Option<SmolStr>,
    },
}

impl TranscriptBlock {
    fn discriminant(&self) -> u8 {
        match self {
            Self::Text { .. } => 0x10,
            Self::Reasoning { .. } => 0x11,
            Self::Image { .. } => 0x12,
            Self::Attachment { .. } => 0x13,
        }
    }

    fn hash_into(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&[self.discriminant()]);
        match self {
            Self::Text { text } | Self::Reasoning { text } => hash_str(hasher, text),
            Self::Image { media_type, digest } => {
                hash_str(hasher, media_type);
                hash_str(hasher, digest);
            }
            Self::Attachment {
                file_id,
                media_type,
            } => {
                hash_str(hasher, file_id);
                hash_opt_str(hasher, media_type.as_deref());
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptToolCall {
    pub call_id: ToolCallId,
    pub name: SmolStr,
    pub arguments: String, // canonical JSON string — see ProviderTranscriptBuilder
}

impl TranscriptToolCall {
    fn hash_into(&self, hasher: &mut blake3::Hasher) {
        hash_str(hasher, self.call_id.as_str());
        hash_str(hasher, &self.name);
        hash_str(hasher, &self.arguments);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptToolOutput {
    Text { text: String },
    Pruned { replacement: String },
    Error { message: String },
}

impl TranscriptToolOutput {
    fn discriminant(&self) -> u8 {
        match self {
            Self::Text { .. } => 0x20,
            Self::Pruned { .. } => 0x21,
            Self::Error { .. } => 0x22,
        }
    }

    fn hash_into(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&[self.discriminant()]);
        match self {
            Self::Text { text }
            | Self::Pruned { replacement: text }
            | Self::Error { message: text } => hash_str(hasher, text),
        }
    }
}

#[inline]
#[allow(dead_code)]
fn hash_len(hasher: &mut blake3::Hasher, len: u64) {
    hasher.update(&len.to_le_bytes());
}

/// Best-effort short label for an attachment source. The transcript only
/// keeps a content-stable handle (digest / file id), not the raw payload, so
/// any URL is hashed by its byte content via the existing transcript
/// digester; the label here just feeds back into `TranscriptBlock::Image`
/// or `TranscriptBlock::Attachment` so prompt-cache stability stays tied to
/// a value the provider also sees.
fn attachment_digest_label(source: &crate::message::AttachmentSource) -> String {
    use crate::message::AttachmentSource;
    match source {
        AttachmentSource::Url { url } | AttachmentSource::DataUrl { url } => url.clone(),
        AttachmentSource::Base64 { data } => format!("base64:{}", short_digest(data)),
        AttachmentSource::FileId { file_id } => file_id.clone(),
        AttachmentSource::LocalPath { path } => path.clone(),
    }
}

/// 16-char prefix of a blake3 digest of `value`. Used to keep an
/// attachment's transcript label content-stable without storing the full
/// payload in the event log.
fn short_digest(value: &str) -> String {
    let mut hex = blake3::hash(value.as_bytes()).to_hex().to_string();
    hex.truncate(16);
    hex
}

#[inline]
#[allow(dead_code)]
fn hash_str(hasher: &mut blake3::Hasher, value: &str) {
    let bytes = value.as_bytes();
    hash_len(hasher, bytes.len() as u64);
    hasher.update(bytes);
}

#[inline]
#[allow(dead_code)]
fn hash_opt_str(hasher: &mut blake3::Hasher, value: Option<&str>) {
    match value {
        Some(v) => {
            hasher.update(&[1]);
            hash_str(hasher, v);
        }
        None => {
            hasher.update(&[0]);
            hash_len(hasher, 0);
        }
    }
}

// ─── HistoryFold-driven builder ────────────────────────────────────────────

use std::collections::{HashMap, HashSet};

use super::{
    event::{
        AssistantMessageCompleted, MessageRevised, RevisionKind, SystemNoticeAppended,
        ToolCallCompleted, ToolCallIssued, TurnAborted, TurnCompleted, TurnStarted,
        UserMessageAppended,
    },
    projection::HistoryFold,
};
use crate::event::{DomainEvent, EventKind};
use crate::session::ids::{MessageId, TurnId};

/// Errors raised while folding history events into a [`ProviderTranscript`].
#[derive(Debug, thiserror::Error)]
pub enum ProviderTranscriptError {
    #[error("tool_call_completed for unknown call_id={0}")]
    UnknownToolCall(ToolCallId),
}

/// HistoryFold implementation projecting the append-only event log into a
/// stable [`ProviderTranscript`].
///
/// Folding rules:
/// * Events from a turn that ended with `TurnAborted` are dropped wholesale.
/// * Events from a turn that has not yet seen a `TurnCompleted` / `TurnAborted`
///   marker are also dropped (the turn is in-flight).
/// * `MessageRevised { Compacted }` — the target message is dropped from the
///   transcript at finalize. The compaction summary itself arrives as a
///   `SystemNoticeAppended { CompactionSummary }`.
/// * `MessageRevised { ToolResultPruned }` rewrites the matching `ToolResult`
///   fragment's output to the supplied replacement text.
/// * `MessageRevised { AttachmentStripped }` is recorded but currently
///   informational — the attachment-bearing block has already been excluded
///   from the transcript content by upstream serialization.
#[derive(Debug, Default)]
pub struct ProviderTranscriptBuilder {
    /// Pending fragments emitted by each turn, keyed by turn id, in insertion
    /// order. Drained into `finalized_fragments` only when the turn closes
    /// successfully.
    pending_turns: HashMap<TurnId, Vec<PendingFragment>>,
    turn_order: Vec<TurnId>,
    aborted_turns: HashSet<TurnId>,
    /// Closed turns, in insertion order, ready to be flattened on finish.
    closed_turn_order: Vec<TurnId>,
    closed_turns: HashMap<TurnId, Vec<PendingFragment>>,
    /// Map from message_id → (turn_id, fragment index) so post-event mutators
    /// (compaction, pruning) can locate fragments without rescanning.
    message_index: HashMap<MessageId, MessageLocation>,
    /// Messages dropped by a `MessageRevised { Compacted }` revision.
    compacted_messages: HashSet<i64>,
    /// Tool-output rewrites applied at finalize.
    tool_pruned: HashMap<ToolCallId, String>,
}

#[derive(Debug, Clone)]
enum PendingFragment {
    Materialized(TranscriptFragment),
    AssistantWithCalls {
        message_id: MessageId,
        content: TranscriptContent,
        tool_calls: Vec<TranscriptToolCall>,
    },
    UserMessage {
        message_id: MessageId,
        content: TranscriptContent,
    },
    SystemNotice {
        message_id: MessageId,
        text: String,
    },
}

#[derive(Debug, Clone, Copy)]
struct MessageLocation {
    turn_id: TurnId,
    fragment_index: usize,
}

impl ProviderTranscriptBuilder {
    fn record_turn(&mut self, turn_id: TurnId) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.pending_turns.entry(turn_id) {
            e.insert(Vec::new());
            self.turn_order.push(turn_id);
        }
    }

    fn push_pending(&mut self, turn_id: TurnId, fragment: PendingFragment) -> usize {
        self.record_turn(turn_id);
        let bucket = self
            .pending_turns
            .get_mut(&turn_id)
            .expect("bucket inserted above");
        let index = bucket.len();
        bucket.push(fragment);
        index
    }

    fn record_message_location(&mut self, message_id: MessageId, turn_id: TurnId, index: usize) {
        self.message_index.insert(
            message_id,
            MessageLocation {
                turn_id,
                fragment_index: index,
            },
        );
    }

    fn close_turn(&mut self, turn_id: TurnId) {
        if let Some(fragments) = self.pending_turns.remove(&turn_id) {
            self.closed_turn_order.push(turn_id);
            self.closed_turns.insert(turn_id, fragments);
        }
    }

    fn abort_turn(&mut self, turn_id: TurnId) {
        self.pending_turns.remove(&turn_id);
        self.aborted_turns.insert(turn_id);
        // Remove any messages that referenced this turn from the index.
        self.message_index.retain(|_, loc| loc.turn_id != turn_id);
    }
}

impl HistoryFold for ProviderTranscriptBuilder {
    type Output = Result<ProviderTranscript, ProviderTranscriptError>;
    type Error = ProviderTranscriptError;

    fn fold(&mut self, event: &DomainEvent) -> Result<(), Self::Error> {
        match &event.kind {
            EventKind::TurnStarted(TurnStarted { turn_id, .. }) => {
                self.record_turn(*turn_id);
            }
            EventKind::TurnCompleted(TurnCompleted { turn_id, .. }) => {
                self.close_turn(*turn_id);
            }
            EventKind::TurnAborted(TurnAborted { turn_id, .. }) => {
                self.abort_turn(*turn_id);
            }
            EventKind::UserMessageAppended(UserMessageAppended {
                message_id,
                turn_id,
                content,
                ..
            }) => {
                let idx = self.push_pending(
                    *turn_id,
                    PendingFragment::UserMessage {
                        message_id: *message_id,
                        content: content.clone(),
                    },
                );
                self.record_message_location(*message_id, *turn_id, idx);
            }
            EventKind::AssistantMessageCompleted(AssistantMessageCompleted {
                message_id,
                turn_id,
                content,
                ..
            }) => {
                let idx = self.push_pending(
                    *turn_id,
                    PendingFragment::AssistantWithCalls {
                        message_id: *message_id,
                        content: content.clone(),
                        tool_calls: Vec::new(),
                    },
                );
                self.record_message_location(*message_id, *turn_id, idx);
            }
            EventKind::ToolCallIssued(ToolCallIssued {
                message_id,
                turn_id,
                call_id,
                name,
                arguments,
                ..
            }) => {
                let bucket = self.pending_turns.entry(*turn_id).or_default();
                if let Some(loc) = self.message_index.get(message_id).copied()
                    && loc.turn_id == *turn_id
                    && let Some(PendingFragment::AssistantWithCalls { tool_calls, .. }) =
                        bucket.get_mut(loc.fragment_index)
                {
                    tool_calls.push(TranscriptToolCall {
                        call_id: call_id.clone(),
                        name: name.clone(),
                        arguments: canonical_json_string(arguments),
                    });
                }
            }
            EventKind::ToolCallCompleted(ToolCallCompleted {
                call_id,
                turn_id,
                output,
                ..
            }) => {
                self.push_pending(
                    *turn_id,
                    PendingFragment::Materialized(TranscriptFragment::ToolResult {
                        call_id: call_id.clone(),
                        output: output.clone(),
                    }),
                );
            }
            EventKind::SystemNoticeAppended(SystemNoticeAppended {
                message_id, text, ..
            }) => {
                // System notices are not part of any turn; allocate a fresh
                // synthetic turn id per notice so they never collide with each
                // other or with a real turn.
                let synthetic = TurnId::new();
                let idx = self.push_pending(
                    synthetic,
                    PendingFragment::SystemNotice {
                        message_id: *message_id,
                        text: text.clone(),
                    },
                );
                self.record_message_location(*message_id, synthetic, idx);
                self.close_turn(synthetic);
            }
            EventKind::MessageRevised(MessageRevised {
                target_message_id,
                kind,
            }) => match kind {
                RevisionKind::Compacted => {
                    self.compacted_messages.insert(*target_message_id);
                }
                RevisionKind::Uncompacted => {
                    self.compacted_messages.remove(target_message_id);
                }
                RevisionKind::ToolResultPruned {
                    call_id,
                    replacement,
                } => {
                    self.tool_pruned
                        .insert(call_id.clone(), replacement.clone());
                }
                RevisionKind::AttachmentStripped { .. } => {
                    // Stripping is recorded for audit but doesn't directly
                    // edit the transcript: the transcript is built from the
                    // current message blocks, which the upstream stripper has
                    // already replaced with placeholders.
                }
            },
            // Runtime / UI projection events do not feed the provider
            // transcript.
            EventKind::RunStarted(_)
            | EventKind::RunFailed(_)
            | EventKind::StreamError(_)
            | EventKind::MessagePartUpdated(_)
            | EventKind::MessagePartDelta(_)
            | EventKind::CommandBegin(_)
            | EventKind::CommandOutputDelta(_)
            | EventKind::CommandEnd(_)
            | EventKind::PermissionRequested(_)
            | EventKind::PermissionReplied(_)
            | EventKind::PermissionRuleCreated(_)
            | EventKind::PermissionRuleUpdated(_)
            | EventKind::PermissionRuleRevoked(_)
            | EventKind::PluginEvent(_) => {}
        }
        Ok(())
    }

    fn finish(self) -> Self::Output {
        let ProviderTranscriptBuilder {
            mut closed_turn_order,
            mut closed_turns,
            compacted_messages,
            tool_pruned,
            ..
        } = self;

        // Drop turns whose every message has been compacted away.
        closed_turn_order.retain(|turn_id| {
            let Some(fragments) = closed_turns.get(turn_id) else {
                return false;
            };
            fragments.iter().any(|f| match f {
                PendingFragment::UserMessage { message_id, .. }
                | PendingFragment::SystemNotice { message_id, .. }
                | PendingFragment::AssistantWithCalls { message_id, .. } => {
                    !compacted_messages.contains(&message_id.raw())
                }
                PendingFragment::Materialized(_) => true,
            })
        });

        let mut transcript = ProviderTranscript::new();
        for turn_id in closed_turn_order {
            let Some(fragments) = closed_turns.remove(&turn_id) else {
                continue;
            };
            for fragment in fragments {
                match fragment {
                    PendingFragment::Materialized(frag) => match frag {
                        TranscriptFragment::ToolResult { call_id, output } => {
                            let output = match tool_pruned.get(&call_id) {
                                Some(replacement) => TranscriptToolOutput::Pruned {
                                    replacement: replacement.clone(),
                                },
                                None => output,
                            };
                            transcript.push(TranscriptFragment::ToolResult { call_id, output });
                        }
                        other => transcript.push(other),
                    },
                    PendingFragment::UserMessage {
                        message_id,
                        content,
                    } => {
                        if compacted_messages.contains(&message_id.raw()) {
                            continue;
                        }
                        transcript.push(TranscriptFragment::User { content });
                    }
                    PendingFragment::AssistantWithCalls {
                        message_id,
                        content,
                        tool_calls,
                        ..
                    } => {
                        if compacted_messages.contains(&message_id.raw()) {
                            continue;
                        }
                        transcript.push(TranscriptFragment::Assistant {
                            content,
                            tool_calls,
                        });
                    }
                    PendingFragment::SystemNotice {
                        message_id, text, ..
                    } => {
                        if compacted_messages.contains(&message_id.raw()) {
                            continue;
                        }
                        transcript.push(TranscriptFragment::System { text });
                    }
                }
            }
        }

        Ok(transcript)
    }
}

/// Render a `serde_json::Value` to a deterministic, key-sorted string so the
/// transcript digest does not drift on serialization order changes.
fn canonical_json_string(value: &serde_json::Value) -> String {
    fn write(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::Null => out.push_str("null"),
            serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            serde_json::Value::Number(n) => out.push_str(&n.to_string()),
            serde_json::Value::String(s) => {
                out.push_str(&serde_json::to_string(s).unwrap_or_default())
            }
            serde_json::Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (i, key) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(key).unwrap_or_default());
                    out.push(':');
                    if let Some(child) = map.get(key) {
                        write(child, out);
                    }
                }
                out.push('}');
            }
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_transcript() -> ProviderTranscript {
        let mut t = ProviderTranscript::new();
        t.push(TranscriptFragment::System {
            text: "you are a helpful agent".into(),
        });
        t.push(TranscriptFragment::User {
            content: TranscriptContent::from_text("hi"),
        });
        t.push(TranscriptFragment::Assistant {
            content: TranscriptContent::from_text("hello!"),
            tool_calls: vec![TranscriptToolCall {
                call_id: ToolCallId::new("call_1"),
                name: SmolStr::new("read_file"),
                arguments: "{\"path\":\"a.rs\"}".into(),
            }],
        });
        t.push(TranscriptFragment::ToolResult {
            call_id: ToolCallId::new("call_1"),
            output: TranscriptToolOutput::Text {
                text: "fn main(){}".into(),
            },
        });
        t
    }

    #[test]
    fn digest_is_deterministic() {
        let a = sample_transcript().digest();
        let b = sample_transcript().digest();
        assert_eq!(a, b);
    }

    #[test]
    fn digest_changes_on_text_edit() {
        let mut t = sample_transcript();
        if let Some(TranscriptFragment::User { content }) = t.fragments.get_mut(1) {
            content.push_text(" there");
        }
        assert_ne!(sample_transcript().digest(), t.digest());
    }

    #[test]
    fn digest_independent_of_serde_ordering() {
        // Round-tripping through JSON shouldn't move the digest, because the
        // digest doesn't go through serde at all.
        let original = sample_transcript();
        let json = serde_json::to_string(&original).unwrap();
        let back: ProviderTranscript = serde_json::from_str(&json).unwrap();
        assert_eq!(original.digest(), back.digest());
    }

    #[test]
    fn fragment_kind_changes_digest() {
        let mut a = ProviderTranscript::new();
        a.push(TranscriptFragment::User {
            content: TranscriptContent::from_text("hi"),
        });
        let mut b = ProviderTranscript::new();
        b.push(TranscriptFragment::Assistant {
            content: TranscriptContent::from_text("hi"),
            tool_calls: vec![],
        });
        assert_ne!(a.digest(), b.digest());
    }

    // ── Builder tests ──────────────────────────────────────────────────────

    use crate::event::{DomainEvent, EventKind};
    use crate::event::{EventMeta, envelope::ENVELOPE_SCHEMA_VERSION};
    use crate::message::MessageMetadata;
    use crate::session::history::{
        AssistantMessageCompleted, FinishReason, ToolCallCompleted, ToolCallIssued,
        TurnAbortReason, TurnAborted, TurnCompleted, TurnStarted, UserMessageAppended,
        fold_history,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn record(kind: EventKind) -> DomainEvent {
        DomainEvent {
            meta: EventMeta {
                id: Uuid::new_v4(),
                seq_global: 0,
                seq_session: Some(0),
                session_id: Some(1),
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: ENVELOPE_SCHEMA_VERSION,
            },
            kind,
        }
    }

    fn turn(turn_id: TurnId) -> Vec<DomainEvent> {
        vec![
            record(EventKind::TurnStarted(TurnStarted {
                turn_id,
                model_id: "m".into(),
                provider_id: "p".into(),
                request_digest: None,
            })),
            record(EventKind::UserMessageAppended(UserMessageAppended {
                message_id: MessageId(1),
                turn_id,
                created_at: Utc::now(),
                content: TranscriptContent::from_text("hi"),
                parts: Vec::new(),

                metadata: MessageMetadata::default(),
            })),
            record(EventKind::AssistantMessageCompleted(
                AssistantMessageCompleted {
                    message_id: MessageId(2),
                    turn_id,
                    created_at: Utc::now(),
                    content: TranscriptContent::from_text("hello!"),
                    usage: None,
                    finish_reason: FinishReason::Stop,
                    parts: Vec::new(),

                    metadata: MessageMetadata::default(),
                },
            )),
            record(EventKind::TurnCompleted(TurnCompleted {
                turn_id,
                finish_reason: FinishReason::Stop,
            })),
        ]
    }

    #[test]
    fn builder_projects_complete_turn() {
        let records = turn(TurnId::new());
        let transcript: ProviderTranscript = fold_history::<ProviderTranscriptBuilder>(&records)
            .unwrap()
            .unwrap();
        assert_eq!(transcript.fragments.len(), 2);
        assert!(matches!(
            transcript.fragments[0],
            TranscriptFragment::User { .. }
        ));
        assert!(matches!(
            transcript.fragments[1],
            TranscriptFragment::Assistant { .. }
        ));
    }

    #[test]
    fn builder_drops_in_flight_turn() {
        let mut records = turn(TurnId::new());
        records.pop(); // remove TurnCompleted — turn is now in flight
        let transcript: ProviderTranscript = fold_history::<ProviderTranscriptBuilder>(&records)
            .unwrap()
            .unwrap();
        assert!(
            transcript.fragments.is_empty(),
            "in-flight turn must be skipped"
        );
    }

    #[test]
    fn builder_drops_aborted_turn() {
        let turn_id = TurnId::new();
        let mut records = turn(turn_id);
        records.pop();
        records.push(record(EventKind::TurnAborted(TurnAborted {
            turn_id,
            reason: TurnAbortReason::ProcessRestart,
            message: None,
        })));
        let transcript: ProviderTranscript = fold_history::<ProviderTranscriptBuilder>(&records)
            .unwrap()
            .unwrap();
        assert!(transcript.fragments.is_empty());
    }

    #[test]
    fn digest_is_stable_across_runtime_metadata_changes() {
        // Two separate event sequences that vary only in turn_id, message_id
        // and timestamps — the resulting transcripts (and thus digests) must
        // be identical because none of those runtime fields enter the digest.
        let a = fold_history::<ProviderTranscriptBuilder>(&turn(TurnId::new()))
            .unwrap()
            .unwrap();
        let b = fold_history::<ProviderTranscriptBuilder>(&turn(TurnId::new()))
            .unwrap()
            .unwrap();
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn builder_attaches_tool_calls_and_results() {
        let turn_id = TurnId::new();
        let call: ToolCallId = "call_x".into();
        let mut records = vec![
            record(EventKind::TurnStarted(TurnStarted {
                turn_id,
                model_id: "m".into(),
                provider_id: "p".into(),
                request_digest: None,
            })),
            record(EventKind::AssistantMessageCompleted(
                AssistantMessageCompleted {
                    message_id: MessageId(10),
                    turn_id,
                    created_at: Utc::now(),
                    content: TranscriptContent::from_text("calling"),
                    usage: None,
                    finish_reason: FinishReason::ToolCalls,
                    parts: Vec::new(),

                    metadata: MessageMetadata::default(),
                },
            )),
            record(EventKind::ToolCallIssued(ToolCallIssued {
                message_id: MessageId(10),
                turn_id,
                call_id: call.clone(),
                name: SmolStr::new("read"),
                arguments: serde_json::json!({"path": "x"}),
                created_at: Utc::now(),
            })),
            record(EventKind::ToolCallCompleted(ToolCallCompleted {
                message_id: MessageId(11),
                call_id: call.clone(),
                turn_id,
                tool_name: SmolStr::new("read"),
                output: TranscriptToolOutput::Text { text: "ok".into() },
                completed_at: Utc::now(),
            })),
            record(EventKind::TurnCompleted(TurnCompleted {
                turn_id,
                finish_reason: FinishReason::ToolCalls,
            })),
        ];
        records
            .iter_mut()
            .enumerate()
            .for_each(|(i, r)| r.meta.seq_global = i as i64);
        let transcript = fold_history::<ProviderTranscriptBuilder>(&records)
            .unwrap()
            .unwrap();
        assert_eq!(transcript.fragments.len(), 2);
        match &transcript.fragments[0] {
            TranscriptFragment::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].call_id, call);
                assert_eq!(tool_calls[0].name.as_str(), "read");
                assert_eq!(tool_calls[0].arguments, "{\"path\":\"x\"}");
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
        match &transcript.fragments[1] {
            TranscriptFragment::ToolResult { call_id, output } => {
                assert_eq!(*call_id, call);
                assert!(matches!(output, TranscriptToolOutput::Text { .. }));
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn canonical_json_sorts_object_keys() {
        let v = serde_json::json!({"b": 2, "a": 1, "c": {"y": 2, "x": 1}});
        assert_eq!(
            canonical_json_string(&v),
            "{\"a\":1,\"b\":2,\"c\":{\"x\":1,\"y\":2}}"
        );
    }
}
