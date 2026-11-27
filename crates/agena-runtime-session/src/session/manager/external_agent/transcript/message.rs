use std::time::Duration;

use agena_domain::{BlobId, ContentDocument, MAX_DOCUMENT_BLOCKS};
use agena_storage::kernel::{AssistantDocumentUpdate, EventCursor, PreparedBlob};
use tokio::time::Instant;

use super::*;
use crate::session::manager::results::prepare_immutable_blob;

const TRUNCATED: &str = "truncated";
const MANAGED_ASSET_LIMIT: usize = 16;
const CHUNK_FLUSH_INTERVAL: Duration = Duration::from_millis(60);
const CHUNK_FLUSH_BYTES: usize = 4 * 1024;

pub(in crate::session::manager::external_agent) struct MessageUpdate<'a> {
    pub(in crate::session::manager::external_agent) id: &'a str,
    pub(in crate::session::manager::external_agent) reasoning: bool,
    pub(in crate::session::manager::external_agent) snapshot: bool,
    content: &'a Value,
}

impl<'a> MessageUpdate<'a> {
    pub(in crate::session::manager::external_agent) fn parse(update: &'a Value) -> Option<Self> {
        let (reasoning, snapshot) = match update["sessionUpdate"].as_str()? {
            "agent_message" => (false, true),
            "agent_thought" => (true, true),
            "agent_message_chunk" => (false, false),
            "agent_thought_chunk" => (true, false),
            _ => return None,
        };
        let id = content_id(update, "messageId")?;
        let content = update.get("content")?;
        if snapshot && !(content.is_array() || content.is_null()) {
            return None;
        }
        Some(Self {
            id,
            reasoning,
            snapshot,
            content,
        })
    }
}

pub(in crate::session::manager::external_agent) enum MessageTarget<'a> {
    Round(&'a ExternalTurn, &'a ExternalAgentLink),
    Parked(&'a KernelPorts, &'a ExternalAgentResume, ExternalAgent),
}

#[derive(Default)]
pub(in crate::session::manager::external_agent) struct NativeMessages {
    // The durable index restores the document whenever its ID changes. Keep
    // only one message in memory even if the peer emits many different IDs.
    current: Option<(AssistantDocumentKey, NativeMessage)>,
}

#[derive(Default)]
struct NativeMessage {
    cursor: Option<EventCursor>,
    document: ContentDocument,
    assets: BTreeMap<String, BlobId>,
    approx_bytes: usize,
    dirty: bool,
    pending_patch: Option<DocumentMutation>,
    pending_is_snapshot: bool,
    pending_blobs: Vec<PreparedBlob>,
    pending_bytes: usize,
    last_flushed: Option<Instant>,
}

impl NativeMessages {
    pub(in crate::session::manager::external_agent) async fn flush(
        &mut self,
        target: MessageTarget<'_>,
    ) -> Result<(), AppError> {
        let Some((key, message)) = self.current.as_mut() else {
            return Ok(());
        };
        if !message.dirty {
            return Ok(());
        }
        let patch = if message.pending_is_snapshot {
            None
        } else {
            message.pending_patch.take().map(Box::new)
        };
        let blobs = std::mem::take(&mut message.pending_blobs);
        let request = RequestId::new();
        let write = AssistantDocumentUpdate {
            previous: message.cursor,
            key: key.clone(),
            document: Box::new(message.document.clone()),
            patch,
        };
        let receipt = persist(|| async {
            match &target {
                MessageTarget::Round(turn, _) => {
                    turn.round
                        .write_assistant_document(request, write.clone(), &blobs)
                        .await
                }
                MessageTarget::Parked(kernel, resume, _) => {
                    kernel
                        .executor
                        .write_parked_assistant_document(request, resume, write.clone(), &blobs)
                        .await
                }
            }
        })
        .await
        .map_err(kernel_error)?;
        message.cursor = Some(receipt.value.event_cursor().map_err(kernel_error)?);
        message.dirty = false;
        message.pending_is_snapshot = false;
        message.pending_bytes = 0;
        message.last_flushed = Some(Instant::now());
        Ok(())
    }

    pub(in crate::session::manager::external_agent) async fn apply(
        &mut self,
        target: MessageTarget<'_>,
        update: MessageUpdate<'_>,
        allow_new: bool,
    ) -> Result<bool, AppError> {
        let (kernel, session, agent, native_id, incarnation, is_parked) = match &target {
            MessageTarget::Round(turn, link) => (
                turn.manager.kernel(),
                turn.control.session,
                turn.agent.clone(),
                link.native_id.as_str(),
                link.incarnation,
                false,
            ),
            MessageTarget::Parked(kernel, resume, agent) => (
                *kernel,
                resume.session,
                agent.clone(),
                resume.native_id.as_str(),
                resume.incarnation,
                true,
            ),
        };
        let Some(incarnation) = incarnation else {
            return Ok(false);
        };
        let key = AssistantDocumentKey {
            native_id: native_id.to_owned(),
            incarnation,
            message_id: update.id.to_owned(),
            reasoning: update.reasoning,
        };
        if self
            .current
            .as_ref()
            .is_none_or(|(current, _)| current != &key)
        {
            self.flush(match &target {
                MessageTarget::Round(turn, link) => MessageTarget::Round(turn, link),
                MessageTarget::Parked(kernel, resume, agent) => {
                    MessageTarget::Parked(kernel, resume, agent.clone())
                }
            })
            .await?;
            let document = NativeMessage::load(&kernel.reader, session, &key).await?;
            self.current = Some((key, document));
        }
        let (_, message) = self.current.as_mut().expect("selected native message");
        if !allow_new && message.cursor.is_none() {
            return Ok(false);
        }
        let blocks = if update.snapshot {
            update.content.as_array().map_or(&[][..], Vec::as_slice)
        } else if let Some(array) = update.content.as_array() {
            array.as_slice()
        } else {
            std::slice::from_ref(update.content)
        };
        if update.snapshot {
            message.document = ContentDocument::default();
            message.approx_bytes = 16;
            message.pending_patch = None;
            message.pending_is_snapshot = true;
        }
        let mut new_blobs = Vec::new();
        let mut changed = update.snapshot;
        let mut added_bytes = 0usize;
        for block in blocks {
            if message
                .document
                .blocks
                .last()
                .and_then(ViewBlock::block_id)
                == Some(TRUNCATED)
            {
                break;
            }
            let block_id = format!("content-{}", message.document.blocks.len());
            let mut new_asset = None;
            let projected = match block["type"].as_str() {
                Some("text") => block["text"].as_str().map(|text| ViewBlock::Markdown {
                    id: Some(block_id),
                    text: text.to_owned(),
                }),
                Some("resource_link") => Some(ViewBlock::Text {
                    id: Some(block_id),
                    text: format!("\n{}\n", resource_label(block)),
                }),
                Some("resource") if block["resource"]["text"].is_string() => {
                    let resource = &block["resource"];
                    Some(ViewBlock::Text {
                        id: Some(block_id),
                        text: format!(
                            "\n{}\n{}\n",
                            resource_label(resource),
                            bounded(
                                resource["text"].as_str().unwrap_or_default(),
                                TOOL_OUTPUT_BYTES
                            )
                        ),
                    })
                }
                Some("image" | "audio" | "resource") => {
                    let prepared = match NativeMedia::from_content(block) {
                        Some(media) => media.prepare().await?,
                        None => None,
                    };
                    let artifact = if let Some(prepared) = prepared {
                        let blob_id = if let Some(id) = message.assets.get(&prepared.sha256) {
                            Some(*id)
                        } else if message.assets.len() < MANAGED_ASSET_LIMIT {
                            let blob = prepare_immutable_blob(
                                &kernel.sessions,
                                RequestId::new(),
                                session,
                                &prepared.bytes,
                            )
                            .await?;
                            let id = blob.descriptor().id;
                            new_asset = Some((prepared.sha256.clone(), blob));
                            Some(id)
                        } else {
                            None
                        };
                        blob_id
                            .map(|id| agent_attachment(agent.clone(), &prepared, id))
                            .transpose()?
                            .as_ref()
                            .map(BuiltinHumanRenderer::attachment_artifact)
                    } else {
                        None
                    };
                    Some(match artifact {
                        Some(artifact) => ViewBlock::Media {
                            id: Some(block_id),
                            artifact,
                        },
                        None => ViewBlock::Text {
                            id: Some(block_id),
                            text: "[ACP attachment unavailable]".into(),
                        },
                    })
                }
                _ => None,
            };
            let Some(projected) = projected else { continue };
            let (mutation, delta) =
                append_block(&mut message.document, &mut message.approx_bytes, projected);
            if let Some(mutation) = mutation {
                changed = true;
                added_bytes += delta;
                if let Some((digest, blob)) = new_asset {
                    message.assets.insert(digest, blob.descriptor().id);
                    new_blobs.push(blob);
                }
                if !message.pending_is_snapshot {
                    match (&mut message.pending_patch, mutation) {
                        (
                            Some(DocumentMutation::AppendText {
                                block_id: existing,
                                text: target,
                            }),
                            DocumentMutation::AppendText { block_id, text },
                        ) if *existing == block_id => {
                            target.push_str(&text);
                        }
                        (None, mutation) if !message.dirty => {
                            message.pending_patch = Some(mutation);
                        }
                        _ => {
                            // Multiple heterogeneous mutations in one batch fall
                            // back to a clean checkpoint snapshot on flush.
                            message.pending_patch = None;
                            message.pending_is_snapshot = true;
                        }
                    }
                }
            } else if delta > 0 {
                // Truncation marker was appended.
                changed = true;
                message.pending_patch = None;
                message.pending_is_snapshot = true;
            }
        }
        if message.cursor.is_none() && message.document.blocks.is_empty() {
            return Ok(true);
        }
        if !changed {
            return Ok(true);
        }
        message.dirty = true;
        message.pending_bytes += added_bytes;
        let has_new_blobs = !new_blobs.is_empty();
        message.pending_blobs.extend(new_blobs);
        let should_flush = is_parked
            || update.snapshot
            || has_new_blobs
            || message.cursor.is_none()
            || message.pending_bytes >= CHUNK_FLUSH_BYTES
            || message
                .last_flushed
                .is_none_or(|ts| ts.elapsed() >= CHUNK_FLUSH_INTERVAL);
        if should_flush {
            self.flush(target).await?;
        }
        Ok(true)
    }
}

impl NativeMessage {
    async fn load(
        reader: &KernelReader,
        session: SessionId,
        key: &AssistantDocumentKey,
    ) -> Result<Self, AppError> {
        let content = key.content_id(session);
        let snapshot = read_with_busy_retry(session, "external_agent_message", || async {
            match reader.event_snapshot(session, content).await {
                Ok(snapshot) => Ok(Some(snapshot)),
                Err(KernelError::Ownership) => Ok(None),
                Err(error) => Err(error),
            }
        })
        .await?;
        let Some(snapshot) = snapshot else {
            return Ok(Self {
                approx_bytes: 16,
                ..Self::default()
            });
        };
        let document = snapshot.document.as_deref().cloned().ok_or_else(|| {
            AppError::Internal("native assistant document has no structured projection".into())
        })?;
        let approx_bytes = document.byte_len();
        let entry = snapshot.stream.entry;
        let cursor = Some(snapshot.through);
        drop(snapshot);
        let blobs = read_with_busy_retry(session, "external_agent_message_assets", || {
            reader.entry_blobs(session, entry)
        })
        .await?;
        let assets = blobs
            .iter()
            .map(|blob| (hex::encode(blob.sha256), blob.id))
            .collect();
        Ok(Self {
            cursor,
            document,
            assets,
            approx_bytes,
            ..Self::default()
        })
    }
}

fn append_block(
    document: &mut ContentDocument,
    approx_bytes: &mut usize,
    mut block: ViewBlock,
) -> (Option<DocumentMutation>, usize) {
    // Leave room for the final marker even when an update exceeds the
    // semantic-document budget. Keep text on a valid UTF-8 boundary.
    let available = MAX_DOCUMENT_BYTES.saturating_sub(*approx_bytes + 256);
    let markdown = matches!(&block, ViewBlock::Markdown { .. });
    let merges = markdown && matches!(document.blocks.last(), Some(ViewBlock::Markdown { .. }));
    if document.blocks.len() == MAX_DOCUMENT_BLOCKS - 1 && !merges {
        mark_truncated(document, approx_bytes);
        return (None, 1);
    }
    if let ViewBlock::Markdown { text, .. } = &mut block {
        if text.is_empty() {
            return (None, 0);
        }
        let overhead = if merges { 0 } else { 64 };
        let budget = available.saturating_sub(overhead);
        let end = text_prefix(text, budget);
        let truncated = end < text.len();
        text.truncate(end);
        let added = encoded_text_len(text);
        if text.is_empty() && truncated {
            mark_truncated(document, approx_bytes);
            return (None, 1);
        }
        let mutation = if let Some(ViewBlock::Markdown {
            id: Some(id),
            text: existing,
        }) = document.blocks.last_mut()
        {
            existing.push_str(text);
            *approx_bytes += added;
            Some(DocumentMutation::AppendText {
                block_id: id.clone(),
                text: std::mem::take(text),
            })
        } else {
            let block_bytes = serde_json::to_vec(&block).map_or(added + 64, |b| b.len() + 1);
            if *approx_bytes + block_bytes + 256 > MAX_DOCUMENT_BYTES {
                mark_truncated(document, approx_bytes);
                return (None, 1);
            }
            let after = document
                .blocks
                .last()
                .and_then(ViewBlock::block_id)
                .map(str::to_owned);
            document.blocks.push(block.clone());
            *approx_bytes += block_bytes;
            Some(DocumentMutation::Insert { after, block })
        };
        if truncated {
            mark_truncated(document, approx_bytes);
            return (None, added.max(1));
        }
        return (mutation, added);
    }
    let block_bytes = serde_json::to_vec(&block).map_or(usize::MAX, |bytes| bytes.len() + 1);
    if block_bytes > available {
        mark_truncated(document, approx_bytes);
        return (None, 1);
    }
    let after = document
        .blocks
        .last()
        .and_then(ViewBlock::block_id)
        .map(str::to_owned);
    document.blocks.push(block.clone());
    *approx_bytes += block_bytes;
    (Some(DocumentMutation::Insert { after, block }), block_bytes)
}

fn mark_truncated(document: &mut ContentDocument, approx_bytes: &mut usize) {
    let block = ViewBlock::Text {
        id: Some(TRUNCATED.into()),
        text: "\n[ACP message truncated.]".into(),
    };
    *approx_bytes += serde_json::to_vec(&block).map_or(64, |b| b.len() + 1);
    document.blocks.push(block);
}

fn encoded_text_len(text: &str) -> usize {
    text.chars()
        .map(|ch| match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\x08' | '\x0c' => 2,
            '\0'..='\x1f' => 6,
            _ => ch.len_utf8(),
        })
        .sum()
}

fn text_prefix(text: &str, bytes: usize) -> usize {
    let mut remaining = bytes;
    for (offset, ch) in text.char_indices() {
        let escaped = match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\x08' | '\x0c' => 2,
            '\0'..='\x1f' => 6,
            _ => ch.len_utf8(),
        };
        let Some(next) = remaining.checked_sub(escaped) else {
            return offset;
        };
        remaining = next;
    }
    text.len()
}
