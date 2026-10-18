//! The model-input reference contains route and presentation metadata; bytes
//! live only in a published immutable blob authorized by the entry membership.
use agena_domain::{
    AttachmentItem, AttachmentSource, BlobId, ModelRef, ResourceActivity, ResourceReference,
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
pub(crate) fn uri(binding: &MediaBinding) -> Result<String> {
    serde_json::to_vec(binding)
        .map(|bytes| format!("{PREFIX}{}", hex::encode(bytes)))
        .map_err(|e| KernelError::Invalid(e.to_string()))
}
pub(super) async fn hydrate(
    reader: &KernelReader,
    session: SessionId,
    entry: EntryId,
    resource: &ResourceActivity,
    model: &ModelRef,
    budget: &ReadBudget,
) -> Result<(CompletionInputAttachment, ReadLease, usize)> {
    let ResourceReference::Artifact { uri, sha256 } = &resource.reference else {
        return Err(KernelError::Invalid(
            "model input has no prepared immutable binding".into(),
        ));
    };
    hydrate_uri(reader, session, entry, uri, sha256, model, budget).await
}
pub(super) async fn hydrate_attachment(
    reader: &KernelReader,
    session: SessionId,
    entry: EntryId,
    item: &AttachmentItem,
    model: &ModelRef,
    budget: &ReadBudget,
) -> Result<Option<(CompletionInputAttachment, ReadLease, usize)>> {
    match &item.source {
        AttachmentSource::Url { url } if url.starts_with(PREFIX) => {
            let hash = item
                .sha256
                .as_deref()
                .ok_or_else(|| KernelError::Invalid("immutable media has no checksum".into()))?;
            hydrate_uri(reader, session, entry, url, hash, model, budget)
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
) -> Result<(CompletionInputAttachment, ReadLease, usize)> {
    let value = uri.strip_prefix(PREFIX).ok_or_else(|| {
        KernelError::Invalid("model input reference has no immutable media binding".into())
    })?;
    if value.len() > 16 * 1024 {
        return Err(KernelError::Invalid(
            "media binding exceeds its metadata budget".into(),
        ));
    }
    let binding: MediaBinding = serde_json::from_slice(
        &hex::decode(value).map_err(|e| KernelError::Invalid(e.to_string()))?,
    )
    .map_err(|e| KernelError::Invalid(e.to_string()))?;
    if binding.version != 1 || binding.model != *model {
        return Err(KernelError::Invalid("media input belongs to a different provider/model; explicitly attach it again for this route".into()));
    }
    let blobs = reader.entry_blobs(session, entry).await?;
    let descriptor = blobs
        .iter()
        .find(|b| b.id == binding.blob)
        .ok_or(KernelError::Ownership)?;
    if hex::encode(descriptor.sha256) != sha256
        || binding.attachment.sha256.as_deref() != Some(sha256)
    {
        return Err(KernelError::Invalid(
            "media reference checksum differs from its immutable blob".into(),
        ));
    }
    let length =
        usize::try_from(descriptor.byte_len).map_err(|e| KernelError::Invalid(e.to_string()))?;
    if length > agena_runtime_tools::media_input::MAX_MEDIA_BYTES {
        return Err(KernelError::Invalid(
            "media input exceeds its byte limit".into(),
        ));
    }
    let lease = budget.reserve_view(session, length.saturating_mul(4).saturating_add(4096))?;
    let blob = reader.open_blob(session, binding.blob).await?;
    let mut bytes = Vec::with_capacity(length);
    blob.copy_to(&mut bytes).await?;
    let mut attachment = binding.attachment;
    attachment.source = AttachmentSource::ProviderData {
        route: binding.route,
        data: STANDARD.encode(bytes),
    };
    let attachment = completion_attachment(&attachment);
    let modeled = modeled_read_bytes(&attachment)?;
    Ok((attachment, lease, modeled))
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
