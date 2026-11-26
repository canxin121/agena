use agena_domain::{BlobId, ContentDocument, MAX_DOCUMENT_BLOCKS};
use agena_storage::kernel::{AssistantDocumentUpdate, EventCursor};

use super::*;
use crate::session::manager::results::prepare_immutable_blob;

const TRUNCATED: &str = "truncated";
const MANAGED_ASSET_LIMIT: usize = 16;

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
}

impl NativeMessages {
    pub(in crate::session::manager::external_agent) async fn apply(
        &mut self,
        target: MessageTarget<'_>,
        update: MessageUpdate<'_>,
        allow_new: bool,
    ) -> Result<bool, AppError> {
        let (kernel, session, agent, native_id, incarnation) = match &target {
            MessageTarget::Round(turn, link) => (
                turn.manager.kernel(),
                turn.control.session,
                turn.agent,
                link.native_id.as_str(),
                link.incarnation,
            ),
            MessageTarget::Parked(kernel, resume, agent) => (
                *kernel,
                resume.session,
                *agent,
                resume.native_id.as_str(),
                resume.incarnation,
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
            let document = NativeMessage::load(&kernel.reader, session, &key).await?;
            self.current = Some((key, document));
        }
        let (key, message) = self.current.as_mut().expect("selected native message");
        if !allow_new && message.cursor.is_none() {
            return Ok(false);
        }
        let blocks = if update.snapshot {
            update.content.as_array().map_or(&[][..], Vec::as_slice)
        } else {
            std::slice::from_ref(update.content)
        };
        let mut document = if update.snapshot {
            ContentDocument::default()
        } else {
            message.document.clone()
        };
        let mut blobs = Vec::new();
        let mut assets = message.assets.clone();
        let mut patch = None;
        for block in blocks {
            if document.blocks.last().and_then(ViewBlock::block_id) == Some(TRUNCATED) {
                break;
            }
            let block_id = format!("content-{}", document.blocks.len());
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
                            resource["text"].as_str().unwrap_or_default()
                        ),
                    })
                }
                Some("image" | "audio" | "resource") => {
                    let prepared = match NativeMedia::from_content(block) {
                        Some(media) => media.prepare().await?,
                        None => None,
                    };
                    let artifact = if let Some(prepared) = prepared {
                        let blob_id = if let Some(id) = assets.get(&prepared.sha256) {
                            Some(*id)
                        } else if assets.len() < MANAGED_ASSET_LIMIT {
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
                            .map(|id| agent_attachment(agent, &prepared, id))
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
            patch = append_block(&mut document, projected);
            if let Some((digest, blob)) = new_asset
                && patch.is_some()
            {
                assets.insert(digest, blob.descriptor().id);
                blobs.push(blob);
            }
        }
        if message.cursor.is_some() && document == message.document {
            return Ok(true);
        }
        let request = RequestId::new();
        let write = AssistantDocumentUpdate {
            previous: message.cursor,
            key: key.clone(),
            document: Box::new(document),
            patch: if update.snapshot {
                None
            } else {
                patch.map(Box::new)
            },
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
        message.document = *write.document;
        message.assets = assets;
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
            return Ok(Self::default());
        };
        let document = snapshot.document.as_deref().cloned().ok_or_else(|| {
            AppError::Internal("native assistant document has no structured projection".into())
        })?;
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
        })
    }
}

fn append_block(document: &mut ContentDocument, mut block: ViewBlock) -> Option<DocumentMutation> {
    // Leave room for the final marker even when an update exceeds the
    // semantic-document budget. Keep text on a valid UTF-8 boundary.
    let available = MAX_DOCUMENT_BYTES.saturating_sub(document.byte_len() + 256);
    let markdown = matches!(&block, ViewBlock::Markdown { .. });
    let merges = markdown && matches!(document.blocks.last(), Some(ViewBlock::Markdown { .. }));
    if document.blocks.len() == MAX_DOCUMENT_BLOCKS - 1 && !merges {
        mark_truncated(document);
        return None;
    }
    let fits =
        !markdown && serde_json::to_vec(&block).is_ok_and(|bytes| bytes.len() + 1 <= available);
    let (mutation, truncated) = match &mut block {
        ViewBlock::Markdown { text, .. } => {
            if text.is_empty() {
                return None;
            }
            let end = text_prefix(text, available.saturating_sub(64));
            let truncated = end < text.len();
            text.truncate(end);
            if let Some(ViewBlock::Markdown { id: Some(id), .. }) = document.blocks.last() {
                (
                    Some(DocumentMutation::AppendText {
                        block_id: id.clone(),
                        text: text.clone(),
                    }),
                    truncated,
                )
            } else {
                (insert_after(document, block), truncated)
            }
        }
        _ if fits => (insert_after(document, block), false),
        _ => (None, true),
    };
    let next = mutation.as_ref().map(|event| document.updated(event));
    if let Some(Ok(next)) = next {
        *document = next;
    } else if mutation.is_some() || truncated {
        mark_truncated(document);
        return None;
    }
    if truncated {
        mark_truncated(document);
        return None;
    }
    mutation
}

fn insert_after(document: &ContentDocument, block: ViewBlock) -> Option<DocumentMutation> {
    Some(DocumentMutation::Insert {
        after: document
            .blocks
            .last()
            .and_then(ViewBlock::block_id)
            .map(str::to_owned),
        block,
    })
}

fn mark_truncated(document: &mut ContentDocument) {
    document.blocks.push(ViewBlock::Text {
        id: Some(TRUNCATED.into()),
        text: "\n[ACP message truncated.]".into(),
    });
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
