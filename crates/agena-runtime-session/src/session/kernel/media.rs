//! The model-input reference contains route and presentation metadata; bytes
//! live only in a published immutable blob authorized by the entry membership.
//!
//! Hydration is deliberately non-fatal. History outlives the route it was
//! captured on: a provider switch, a model switch, a re-selected endpoint, or
//! a blob that is no longer a member of this conversation must degrade the one
//! attachment to an explicit [`AttachmentSource::Unavailable`] notice instead
//! of failing the whole request. A conversation that cannot re-deliver its
//! bytes is still a readable conversation; a conversation that errors on load
//! is a lost one.
use agena_domain::{
    AttachmentItem, AttachmentKind, AttachmentSource, BlobId, ModelRef, ResourceActivity,
    ResourceKind, ResourceReference,
};
use agena_provider::{
    CompletionInputAttachment, CompletionInputAttachmentKind, CompletionInputAttachmentSource,
};
use agena_storage::kernel::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

const PREFIX: &str = "agena:media:v1:";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MediaBinding {
    pub version: u8,
    pub model: ModelRef,
    pub route: String,
    pub blob: BlobId,
    pub attachment: AttachmentItem,
}
/// One hydrated model input. `lease` is absent exactly when the attachment
/// degraded to an unavailable notice and no bytes were admitted.
pub(super) struct HydratedMedia {
    pub attachment: CompletionInputAttachment,
    pub lease: Option<ReadLease>,
    pub modeled: usize,
}
pub(crate) fn uri(binding: &MediaBinding) -> Result<String> {
    serde_json::to_vec(binding)
        .map(|bytes| format!("{PREFIX}{}", hex::encode(bytes)))
        .map_err(|e| KernelError::Invalid(e.to_string()))
}
/// The placeholder a model receives when its bytes can no longer be delivered.
/// The attachment keeps its kind, MIME, and display metadata so the
/// conversation still reads correctly to a human.
fn degraded(item: AttachmentItem, reason: impl Into<String>) -> HydratedMedia {
    let mut item = item;
    item.source = AttachmentSource::Unavailable {
        reason: reason.into(),
    };
    let attachment = completion_attachment(&item);
    let modeled = modeled_read_bytes(&attachment).unwrap_or(0);
    HydratedMedia {
        attachment,
        lease: None,
        modeled,
    }
}
/// A model input whose binding never existed (a legacy entry, or a resource
/// that was marked as model input without prepared bytes) still carries enough
/// presentation metadata to describe itself.
fn resource_attachment(resource: &ResourceActivity) -> AttachmentItem {
    let kind = match resource.kind {
        ResourceKind::Image => AttachmentKind::Image,
        ResourceKind::Audio => AttachmentKind::Audio,
        ResourceKind::Video => AttachmentKind::Video,
        ResourceKind::Pdf => AttachmentKind::Pdf,
        _ => AttachmentKind::File,
    };
    AttachmentItem {
        kind,
        mime: resource.media_type.clone().unwrap_or_default(),
        source: AttachmentSource::Unavailable {
            reason: String::new(),
        },
        filename: Some(resource.name.clone()),
        title: None,
        size_bytes: resource.size_bytes,
        sha256: None,
        width: resource.width,
        height: resource.height,
        duration_ms: resource.duration_ms,
        page_count: resource.page_count,
    }
}
pub(super) async fn hydrate(
    reader: &KernelReader,
    session: SessionId,
    entry: EntryId,
    resource: &ResourceActivity,
    model: &ModelRef,
    budget: &ReadBudget,
) -> Result<HydratedMedia> {
    let fallback = resource_attachment(resource);
    let ResourceReference::Artifact { uri, sha256 } = &resource.reference else {
        return Ok(degraded(
            fallback,
            "it was recorded without a prepared immutable binding",
        ));
    };
    hydrate_uri(reader, session, entry, uri, sha256, model, budget, fallback).await
}
pub(super) async fn hydrate_attachment(
    reader: &KernelReader,
    session: SessionId,
    entry: EntryId,
    item: &AttachmentItem,
    model: &ModelRef,
    budget: &ReadBudget,
) -> Result<Option<HydratedMedia>> {
    match &item.source {
        AttachmentSource::Url { url } if url.starts_with(PREFIX) => {
            let Some(hash) = item.sha256.as_deref() else {
                return Ok(Some(degraded(
                    item.clone(),
                    "its recorded content has no checksum",
                )));
            };
            hydrate_uri(
                reader,
                session,
                entry,
                url,
                hash,
                model,
                budget,
                item.clone(),
            )
            .await
            .map(Some)
        }
        _ => Ok(None),
    }
}
async fn hydrate_uri(
    reader: &KernelReader,
    session: SessionId,
    entry: EntryId,
    uri: &str,
    sha256: &str,
    model: &ModelRef,
    budget: &ReadBudget,
    fallback: AttachmentItem,
) -> Result<HydratedMedia> {
    let binding = match decode_binding(uri) {
        Ok(binding) => binding,
        Err(reason) => return Ok(degraded(fallback, reason)),
    };
    if binding.version != 1 {
        return Ok(degraded(
            binding.attachment,
            "it was recorded by an unsupported version of this application",
        ));
    }
    if binding.model != *model {
        return Ok(degraded(
            binding.attachment,
            format!("it was attached for model `{}`", binding.model),
        ));
    }
    let mut item = binding.attachment;
    let blobs = reader.entry_blobs(session, entry).await?;
    let Some(descriptor) = blobs.iter().find(|b| b.id == binding.blob) else {
        return Ok(degraded(
            item,
            "its stored content is no longer part of this conversation",
        ));
    };
    if hex::encode(descriptor.sha256) != sha256 || item.sha256.as_deref() != Some(sha256) {
        return Ok(degraded(
            item,
            "its stored content no longer matches the recorded attachment",
        ));
    }
    let Ok(length) = usize::try_from(descriptor.byte_len) else {
        return Ok(degraded(
            item,
            "its stored content exceeds the attachment size limit",
        ));
    };
    if length > agena_runtime_tools::media_input::MAX_MEDIA_BYTES {
        return Ok(degraded(
            item,
            "its stored content exceeds the attachment size limit",
        ));
    }
    let Ok(lease) = budget.reserve_view(session, length.saturating_mul(4).saturating_add(4096))
    else {
        return Ok(degraded(
            item,
            "it could not be loaded within the current read budget",
        ));
    };
    let mut bytes = Vec::with_capacity(length);
    let read = async {
        let blob = reader.open_blob(session, binding.blob).await?;
        blob.copy_to(&mut bytes).await
    }
    .await;
    if read.is_err() {
        return Ok(degraded(
            item,
            "its stored content could not be read from this conversation",
        ));
    }
    item.source = AttachmentSource::ProviderData {
        route: binding.route,
        data: STANDARD.encode(bytes),
    };
    let attachment = completion_attachment(&item);
    let modeled = modeled_read_bytes(&attachment)?;
    Ok(HydratedMedia {
        attachment,
        lease: Some(lease),
        modeled,
    })
}
/// Read a published attachment for presentation or download. Viewing bytes
/// does not depend on the selected model. The URI is only a descriptor:
/// open_blob checks session history membership, including forked history.
pub async fn read_managed_attachment(
    reader: &KernelReader,
    session: SessionId,
    uri: &str,
) -> Result<(AttachmentItem, Vec<u8>)> {
    let binding = decode_binding(uri).map_err(KernelError::Invalid)?;
    if binding.version != 1 {
        return Err(KernelError::Invalid(
            "unsupported managed media version".into(),
        ));
    }
    let blob = reader.open_blob(session, binding.blob).await?;
    let descriptor = blob.descriptor();
    if descriptor.byte_len > agena_runtime_tools::media_input::MAX_MEDIA_BYTES as u64 {
        return Err(KernelError::Invalid(
            "managed attachment exceeds the 20 MiB download limit".into(),
        ));
    }
    let hash = hex::encode(descriptor.sha256);
    if binding.attachment.sha256.as_deref() != Some(hash.as_str())
        || binding.attachment.size_bytes != Some(descriptor.byte_len)
    {
        return Err(KernelError::Invalid(
            "managed attachment size or checksum does not match".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(descriptor.byte_len as usize);
    blob.copy_to(&mut bytes).await?;
    Ok((binding.attachment, bytes))
}

/// Resolve a local attachment through a visible tool result. A fork inherits
/// that history entry, not blanket access to the original session directory.
pub async fn authorized_tool_attachment(
    reader: &KernelReader,
    session: SessionId,
    tool: ToolId,
    path: &str,
) -> Result<AttachmentItem> {
    let tool = reader.tool(session, tool).await?;
    let raw = tool
        .result
        .as_ref()
        .and_then(|value| value.get("raw_output"))
        .ok_or_else(|| KernelError::NotFound("tool attachment".into()))?;
    let raw: agena_domain::RawOutput = serde_json::from_value(raw.clone())
        .map_err(|_| KernelError::Invalid("invalid stored tool attachment".into()))?;
    raw.attachments.into_iter().find(|item| {
        matches!(&item.source, AttachmentSource::LocalPath { path: recorded } if recorded == path)
            && item.sha256.is_some() && item.size_bytes.is_some()
    }).ok_or_else(|| KernelError::NotFound("tool attachment".into()))
}

/// Decode the immutable binding embedded in a media reference. Every failure is
/// a per-attachment degradation reason, never a request-level error.
pub(crate) fn decode_binding(uri: &str) -> std::result::Result<MediaBinding, String> {
    let value = uri
        .strip_prefix(PREFIX)
        .ok_or("its recorded reference is not an immutable media binding")?;
    if value.len() > 16 * 1024 {
        return Err("its recorded media binding exceeds the metadata budget".into());
    }
    let bytes = hex::decode(value).map_err(|_| "its recorded media binding is malformed")?;
    serde_json::from_slice(&bytes).map_err(|_| "its recorded media binding is malformed".to_owned())
}
pub(crate) fn completion_attachment(item: &AttachmentItem) -> CompletionInputAttachment {
    CompletionInputAttachment {
        kind: match item.kind {
            agena_domain::AttachmentKind::Image => CompletionInputAttachmentKind::Image,
            agena_domain::AttachmentKind::Audio => CompletionInputAttachmentKind::Audio,
            agena_domain::AttachmentKind::Video => CompletionInputAttachmentKind::Video,
            agena_domain::AttachmentKind::Pdf => CompletionInputAttachmentKind::Pdf,
            agena_domain::AttachmentKind::File => CompletionInputAttachmentKind::File,
        },
        mime: item.mime.clone(),
        source: match &item.source {
            AttachmentSource::Url { url } => {
                CompletionInputAttachmentSource::Url { url: url.clone() }
            }
            AttachmentSource::DataUrl { url } => {
                CompletionInputAttachmentSource::DataUrl { url: url.clone() }
            }
            AttachmentSource::Base64 { data } => {
                CompletionInputAttachmentSource::Base64 { data: data.clone() }
            }
            AttachmentSource::ProviderData { route, data } => {
                CompletionInputAttachmentSource::ProviderData {
                    route: route.clone(),
                    data: data.clone(),
                }
            }
            AttachmentSource::FileId { file_id } => CompletionInputAttachmentSource::FileId {
                id: file_id.clone(),
            },
            AttachmentSource::LocalPath { path } => {
                CompletionInputAttachmentSource::LocalPath { path: path.clone() }
            }
            AttachmentSource::Unavailable { reason } => {
                CompletionInputAttachmentSource::Unavailable {
                    reason: reason.clone(),
                }
            }
        },
        filename: item.filename.clone(),
        title: item.title.clone(),
        size_bytes: item.size_bytes,
        sha256: item.sha256.clone(),
        width: item.width,
        height: item.height,
        duration_ms: item.duration_ms,
        page_count: item.page_count,
    }
}

#[cfg(test)]
mod attachment_download_tests {
    use super::*;
    use agena_domain::{ComposerDocument, ComposerNode, ModelId, ProviderId};
    use std::io::Cursor;

    async fn fixture() -> (
        agena_storage_sqlite::SqliteKernel,
        SessionId,
        SessionId,
        PreparedBlob,
        MediaBinding,
        Vec<u8>,
    ) {
        let kernel = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = kernel
            .workspaces
            .ensure_id("/attachment-download-test")
            .await
            .unwrap();
        let session = kernel
            .ports
            .sessions
            .create(RequestId::new(), workspace, "owner".into())
            .await
            .unwrap()
            .value
            .id;
        let foreign = kernel
            .ports
            .sessions
            .create(RequestId::new(), workspace, "foreign".into())
            .await
            .unwrap()
            .value
            .id;
        let bytes = b"immutable original attachment".to_vec();
        let blob = kernel
            .ports
            .sessions
            .prepare_blob(
                RequestId::new(),
                session,
                bytes.len() as u64,
                &mut Cursor::new(&bytes),
            )
            .await
            .unwrap();
        let binding = MediaBinding {
            version: 1,
            model: ModelRef {
                provider_id: ProviderId::new("test"),
                adapter_id: None,
                model_id: ModelId::new("original-model"),
            },
            route: "original-route".into(),
            blob: blob.descriptor().id,
            attachment: AttachmentItem {
                kind: AttachmentKind::File,
                mime: "text/plain".into(),
                source: AttachmentSource::Base64 {
                    data: String::new(),
                },
                filename: Some("原始附件.txt".into()),
                title: None,
                size_bytes: Some(bytes.len() as u64),
                sha256: Some(hex::encode(blob.descriptor().sha256)),
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            },
        };
        (kernel, session, foreign, blob, binding, bytes)
    }
    fn document() -> ComposerDocument {
        ComposerDocument(vec![ComposerNode::Text {
            text: "attached".into(),
        }])
    }

    #[tokio::test]
    async fn only_published_session_members_can_download_and_forks_keep_access() {
        let (kernel, session, foreign, blob, binding, bytes) = fixture().await;
        let reference = uri(&binding).unwrap();
        assert!(
            read_managed_attachment(&kernel.ports.reader, session, &reference)
                .await
                .is_err()
        );
        let execution = kernel
            .ports
            .sessions
            .send_with_blobs(RequestId::new(), session, document(), &[blob])
            .await
            .unwrap()
            .value;
        let (metadata, downloaded) =
            read_managed_attachment(&kernel.ports.reader, session, &reference)
                .await
                .unwrap();
        assert_eq!(downloaded, bytes);
        assert_eq!(metadata.filename.as_deref(), Some("原始附件.txt"));
        assert!(matches!(
            read_managed_attachment(&kernel.ports.reader, foreign, &reference).await,
            Err(KernelError::Ownership)
        ));
        let attempt = kernel
            .ports
            .executor
            .claim_next(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value;
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        let child = kernel
            .ports
            .sessions
            .fork(RequestId::new(), session, execution.message_id)
            .await
            .unwrap()
            .value
            .id;
        assert_eq!(
            read_managed_attachment(&kernel.ports.reader, child, &reference)
                .await
                .unwrap()
                .1,
            bytes
        );
    }

    #[tokio::test]
    async fn inherited_generated_files_require_the_exact_visible_tool_attachment() {
        let (kernel, session, foreign, blob, binding, _) = fixture().await;
        let execution = kernel
            .ports
            .sessions
            .send_with_blobs(RequestId::new(), session, document(), &[blob])
            .await
            .unwrap()
            .value;
        let attempt = kernel
            .ports
            .executor
            .claim_next(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_provider_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "ig_inherited".into(),
                    name: "image_generation".into(),
                    input: serde_json::json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        let path = format!(
            "/managed/generated_images/{}/provider-hosted-image-{}.png",
            session.get(),
            tool.id().get()
        );
        let mut attachment = binding.attachment;
        attachment.source = AttachmentSource::LocalPath { path: path.clone() };
        let raw = agena_domain::RawOutput {
            attachments: vec![attachment.clone()],
            ..Default::default()
        };
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"raw_output":raw}),
            },
        )
        .await
        .unwrap();
        let entry = kernel
            .ports
            .reader
            .tool(session, tool.id())
            .await
            .unwrap()
            .entry_id;
        assert_eq!(
            authorized_tool_attachment(&kernel.ports.reader, session, tool.id(), &path)
                .await
                .unwrap(),
            attachment
        );
        assert!(
            authorized_tool_attachment(&kernel.ports.reader, foreign, tool.id(), &path)
                .await
                .is_err()
        );
        assert!(
            authorized_tool_attachment(
                &kernel.ports.reader,
                session,
                tool.id(),
                "/private/unrelated.png"
            )
            .await
            .is_err()
        );
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        let child = kernel
            .ports
            .sessions
            .fork(RequestId::new(), session, entry)
            .await
            .unwrap()
            .value
            .id;
        assert_eq!(
            authorized_tool_attachment(&kernel.ports.reader, child, tool.id(), &path)
                .await
                .unwrap(),
            attachment
        );
    }

    #[tokio::test]
    async fn forged_hash_size_version_and_malformed_bindings_are_rejected() {
        let (kernel, session, _, blob, mut binding, _) = fixture().await;
        kernel
            .ports
            .sessions
            .send_with_blobs(RequestId::new(), session, document(), &[blob])
            .await
            .unwrap();
        let original_hash = binding.attachment.sha256.clone();
        binding.attachment.sha256 = Some("0".repeat(64));
        assert!(
            read_managed_attachment(&kernel.ports.reader, session, &uri(&binding).unwrap())
                .await
                .is_err()
        );
        binding.attachment.sha256 = original_hash;
        let original_size = binding.attachment.size_bytes;
        binding.attachment.size_bytes = Some(999);
        assert!(
            read_managed_attachment(&kernel.ports.reader, session, &uri(&binding).unwrap())
                .await
                .is_err()
        );
        binding.attachment.size_bytes = original_size;
        binding.version = 2;
        assert!(
            read_managed_attachment(&kernel.ports.reader, session, &uri(&binding).unwrap())
                .await
                .is_err()
        );
        for reference in [
            "agena:media:v1:zz".into(),
            "https://example.org/image".into(),
            format!("{PREFIX}{}", "a".repeat(16 * 1024 + 1)),
        ] {
            assert!(
                read_managed_attachment(&kernel.ports.reader, session, &reference)
                    .await
                    .is_err()
            );
        }
    }
}
