use super::*;
use agena_domain::{ActivityPayload, ResourceDelivery, ResourceKind, ResourceReference};
use agena_storage::kernel::PreparedBlob;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use tokio::sync::OnceCell;

// Align non-final slices to base64 groups and leave room for other replies in
// the function hook's JSON frame.
const MEDIA_SLICE_BYTES: u64 = 384 * 1024;

pub(super) type PreparedPromptMedia = (ComposerDocument, Vec<PreparedBlob>);

struct Attachment {
    index: usize,
    metadata: Value,
}

pub(super) struct PromptMedia<'a> {
    manager: &'a SessionManager,
    session: SessionId,
    state: Arc<ExecutionState>,
    context: SessionExecutionContext,
    model: agena_domain::ModelRef,
    request: RequestId,
    document: ComposerDocument,
    snapshots: UserMediaSnapshots,
    attachments: Vec<Attachment>,
    prepared: OnceCell<PreparedPromptMedia>,
}

impl<'a> PromptMedia<'a> {
    pub(super) fn for_submission(
        manager: &'a SessionManager,
        submission: &PendingSubmission,
    ) -> Option<Self> {
        let document = &submission.message.document;
        let attachments: Vec<_> = document
            .0
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let ComposerNode::Activity { activity } = node else {
                    return None;
                };
                let ActivityPayload::Resource(resource) = &activity.payload else {
                    return None;
                };
                if resource.delivery != ResourceDelivery::ModelInput {
                    return None;
                }
                let kind = match resource.kind {
                    ResourceKind::Image => "image",
                    ResourceKind::Audio => "audio",
                    _ => "document",
                };
                let mut metadata = json!({"type":kind, "id":uuid::Uuid::new_v4()});
                if let Some(mime) = &resource.media_type {
                    metadata["mediaType"] = json!(mime);
                }
                if !resource.name.is_empty() {
                    metadata["filename"] = json!(resource.name);
                }
                Some(Attachment { index, metadata })
            })
            .collect();
        if attachments.is_empty() {
            return None;
        }
        Some(Self {
            manager,
            session: submission.session,
            state: submission.state.clone(),
            context: submission.context.clone(),
            model: submission.options.model.clone(),
            request: submission.send_id,
            document: document.clone(),
            snapshots: submission.snapshots.clone(),
            attachments,
            prepared: OnceCell::new(),
        })
    }

    pub(super) fn metadata(&self) -> Vec<&Value> {
        self.attachments
            .iter()
            .map(|attachment| &attachment.metadata)
            .collect()
    }

    pub(super) fn prepared(&self) -> Option<&PreparedPromptMedia> {
        self.prepared.get()
    }

    pub(super) async fn read(&self, id: String, offset: u64) -> Value {
        let Some(attachment) = self
            .attachments
            .iter()
            .find(|attachment| attachment.metadata["id"].as_str() == Some(id.as_str()))
        else {
            return json!({"deny":"No attachment with that handle belongs to this submission"});
        };
        let prepared = self
            .prepared
            .get_or_try_init(|| async {
                self.manager
                    .prepare_user_media_with_state(
                        self.session,
                        &self.model,
                        self.document.clone(),
                        self.request,
                        self.state.clone(),
                        Some(self.context.clone()),
                        Some(&self.snapshots),
                    )
                    .await
            })
            .await;
        let (document, blobs) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return json!({"unreadable":error.failure().user.fallback}),
        };
        let unreadable = || json!({"unreadable":"The prepared attachment could not be read"});
        let Some(ComposerNode::Activity { activity }) = document.0.get(attachment.index) else {
            return unreadable();
        };
        let ActivityPayload::Resource(resource) = &activity.payload else {
            return unreadable();
        };
        let kind = match resource.kind {
            ResourceKind::Image => "image",
            ResourceKind::File | ResourceKind::Pdf => "document",
            _ => return json!({"deny":"This attachment has no inline image or document bytes"}),
        };
        let ResourceReference::Artifact { uri, .. } = &resource.reference else {
            return unreadable();
        };
        let Ok(binding) = crate::session::kernel::media::decode_binding(uri) else {
            return unreadable();
        };
        let Some(blob) = blobs
            .iter()
            .find(|blob| blob.descriptor().id == binding.blob)
        else {
            return unreadable();
        };
        let size = blob.descriptor().byte_len;
        if offset > size {
            return json!({"deny":"The byte offset exceeds the attachment length"});
        }
        let end = size.min(offset + MEDIA_SLICE_BYTES);
        let mut bytes = Vec::with_capacity((end - offset) as usize);
        let mut position = offset;
        let reader = blob.reader();
        while position < end {
            let Ok(page) = reader.range(position, (end - position) as usize).await else {
                return unreadable();
            };
            if page.next <= position || page.next > end {
                return unreadable();
            }
            bytes.extend_from_slice(&page.bytes);
            position = page.next;
        }
        let mut result = json!({
            "type":kind,
            "mediaType":resource.media_type.as_deref().unwrap_or("application/octet-stream"),
            "base64":STANDARD.encode(bytes),
            "size":size,
        });
        if end < size {
            result["next"] = json!(end);
        }
        result
    }
}
