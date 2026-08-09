//! Immutable LLM-input projection of session history.
//!
//! `ProviderTranscript` is the **canonical** representation of "what the LLM
//! sees". It is intentionally devoid of any field that mutates over a
//! message's lifetime (status, usage, timestamps, in-memory run IDs, tags,
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

use agena_domain::ToolCallId;

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
/// A fragment of a session transcript.
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
                hash_str(hasher, call_id.as_ref());
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
    pub fn from_blocks(blocks: Vec<TranscriptBlock>) -> Self {
        Self { blocks }
    }

    /// Project a `Message`'s parts into transcript form. Multi-modal fidelity
    /// is preserved: text, reasoning, image and attachment blocks all round-
    /// trip; everything else collapses to its lossy text rendering as a final
    /// fallback so a message never produces an empty body.
    ///
    /// The name retains the `_lossy` suffix because the lossy fallback path
    /// is unavoidable for content kinds (file changes, web searches, …) the
    /// transcript shape simply does not model.
    #[cfg(test)]
    pub fn from_message_lossy(message: &crate::message::Message) -> Self {
        use crate::message::{AttachmentKind, PartContent, RuntimeActivity};
        let mut blocks = Vec::new();
        let mut had_any = false;
        for part in &message.parts {
            match part.content.as_ref() {
                Some(PartContent::Text(text)) => {
                    if !text.text.is_empty() {
                        blocks.push(TranscriptBlock::Text {
                            text: text.text.clone(),
                        });
                        had_any = true;
                    }
                }
                Some(PartContent::Activity(RuntimeActivity::Reasoning(reasoning))) => {
                    let joined = reasoning.preferred_text();
                    if !joined.is_empty() {
                        blocks.push(TranscriptBlock::Reasoning { text: joined });
                        had_any = true;
                    }
                }
                Some(PartContent::Activity(RuntimeActivity::Resource(attachment))) => {
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
                        blocks.push(block);
                        had_any = true;
                    }
                }
                Some(PartContent::Activity(RuntimeActivity::SkillReference(skill_reference)))
                    if !skill_reference.skills.is_empty() =>
                {
                    blocks.push(TranscriptBlock::Text {
                        text: skill_reference.model_context_text(),
                    });
                    had_any = true;
                }
                _ => {}
            }
        }
        if !had_any {
            let fallback = message.as_text_lossy();
            if !fallback.is_empty() {
                blocks.push(TranscriptBlock::Text { text: fallback });
            }
        }
        Self { blocks }
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
/// A block of a session transcript.
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
/// A tool call in a transcript.
pub struct TranscriptToolCall {
    pub call_id: ToolCallId,
    pub name: SmolStr,
    pub arguments: String, // canonical JSON string
}

impl TranscriptToolCall {
    fn hash_into(&self, hasher: &mut blake3::Hasher) {
        hash_str(hasher, self.call_id.as_ref());
        hash_str(hasher, &self.name);
        hash_str(hasher, &self.arguments);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// Output of a transcript tool call.
pub enum TranscriptToolOutput {
    Text { text: String },
    Error { message: String },
}

impl TranscriptToolOutput {
    fn discriminant(&self) -> u8 {
        match self {
            Self::Text { .. } => 0x20,
            Self::Error { .. } => 0x21,
        }
    }

    fn hash_into(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&[self.discriminant()]);
        match self {
            Self::Text { text } | Self::Error { message: text } => hash_str(hasher, text),
        }
    }
}

#[inline]
fn hash_len(hasher: &mut blake3::Hasher, len: u64) {
    hasher.update(&len.to_le_bytes());
}

/// Best-effort short label for an attachment source. The transcript only
/// keeps a content-stable handle (digest / file id), not the raw payload, so
/// any URL is hashed by its byte content via the existing transcript
/// digester; the label here just feeds back into `TranscriptBlock::Image`
/// or `TranscriptBlock::Attachment` so prompt-cache stability stays tied to
/// a value the provider also sees.
#[cfg(test)]
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
#[cfg(test)]
fn short_digest(value: &str) -> String {
    let mut hex = blake3::hash(value.as_bytes()).to_hex().to_string();
    hex.truncate(16);
    hex
}

#[inline]
fn hash_str(hasher: &mut blake3::Hasher, value: &str) {
    let bytes = value.as_bytes();
    hash_len(hasher, bytes.len() as u64);
    hasher.update(bytes);
}

#[inline]
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
