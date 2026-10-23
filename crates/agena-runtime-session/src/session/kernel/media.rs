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
