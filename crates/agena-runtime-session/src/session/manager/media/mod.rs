//! Explicit user-authorized media preparation into immutable kernel blobs.
use super::*;
use crate::session::kernel::media::{MediaBinding, uri};
use agena_domain::{
    AccessKind, ActivityPayload, AttachmentSource, PermissionDecision, ResourceDelivery,
    ResourceReference,
};
use agena_runtime_tools::media_input;
pub(super) static MEDIA_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
pub(super) static MEDIA_PREPARATIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
#[cfg(test)]
pub(super) static PREPARATION_BARRIER: StdMutex<
    Option<(RequestId, tokio::sync::oneshot::Sender<()>)>,
> = StdMutex::new(None);
impl SessionManager {
    pub(super) async fn prepare_user_media(
        &self,
        session: SessionId,
        model: &ModelRef,
        document: ComposerDocument,
        request: RequestId,
    ) -> Result<(ComposerDocument, Vec<PreparedBlob>), AppError> {
        self.prepare_user_media_with_state(
            session,
            model,
            document,
            request,
            self.state.load_full(),
        )
        .await
    }

    pub(super) async fn prepare_user_media_with_state(
        &self,
        session: SessionId,
        model: &ModelRef,
        mut document: ComposerDocument,
        request: RequestId,
        state: Arc<ExecutionState>,
    ) -> Result<(ComposerDocument, Vec<PreparedBlob>), AppError> {
        if !document.0.iter().any(|node| {
            matches!(node, ComposerNode::Activity { activity }
            if matches!(&activity.payload, ActivityPayload::Resource(resource)
                if resource.delivery == ResourceDelivery::ModelInput
                    || matches!(resource.reference, ResourceReference::Artifact { .. })))
        }) {
            return Ok((document, Vec::new()));
        }
        #[cfg(test)]
        {
            let mut barrier = PREPARATION_BARRIER.lock().unwrap();
            if barrier
                .as_ref()
                .is_some_and(|(target, _)| *target == request)
                && let Some((_, reached)) = barrier.take()
            {
                let _ = reached.send(());
            }
        }
        let _preparation = MEDIA_PREPARATIONS
            .acquire()
            .await
            .map_err(|_| AppError::Cancelled)?;
        let context = self.execution_context_with_state(session, &state).await?;
        let executor = state.executor.for_session_context_async(&context).await;
        let root = executor
            .workspace_root()
            .canonicalize()
            .map_err(|e| AppError::Config(e.to_string()))?;
        let route = state.providers.media_route_binding(model)?;
        let mut blobs = Vec::new();
        let mut bytes = 0usize;
        for (index, node) in document.0.iter_mut().enumerate() {
            let ComposerNode::Activity { activity } = node else {
                continue;
            };
            let ActivityPayload::Resource(resource) = &mut activity.payload else {
                continue;
            };
            if resource.delivery != ResourceDelivery::ModelInput
                && !matches!(resource.reference, ResourceReference::Artifact { .. })
            {
                continue;
            }
            if blobs.len() >= media_input::MAX_MEDIA_INPUTS {
                return Err(AppError::Config(
                    "at most eight media resources may be sent in one message".into(),
                ));
            }
            let (prepared, existing_blob) = match &resource.reference {
                ResourceReference::WorkspacePath { path } => {
                    let target = root.join(path).canonicalize().map_err(|error| {
                        AppError::Config(format!("selected attachment is unavailable: {error}"))
                    })?;
                    if !target.starts_with(&root)
                        && agena_runtime_tools::managed_attachment_root(&root, &target).is_none()
                    {
                        return Err(AppError::Config(
                            "selected attachment escapes the active workspace and its managed attachments".into(),
                        ));
                    }
                    if !matches!(
                        executor.principal().authorize_path_access(AccessKind::Read, &root, &target),
                        PermissionDecision::Allow
                    ) {
                        return Err(AppError::Config(
                            "attachment read requires permission; no content was sent".into(),
                        ));
                    }
                    let root = root.clone();
                    let path = path.clone();
                    let expected = activity.provenance.content_hash.clone();
                    let prepared = MEDIA_WORKERS
                        .run(move || media_input::read_local(&root, &path, expected.as_deref()))
                        .await
                        .map_err(|error| AppError::Internal(error.to_string()))?
                        .map_err(AppError::Config)?;
                    (prepared, None)
                }
                ResourceReference::Artifact { uri, sha256 } => {
                    // Editing queued input preserves its frozen bytes, even
                    // after the original file moved. Session membership gates
                    // access; a URI alone cannot grant another session a blob.
                    let binding = crate::session::kernel::media::decode_binding(uri)
                        .map_err(AppError::Config)?;
                    if binding.version != 1 || binding.attachment.sha256.as_ref() != Some(sha256) {
                        return Err(AppError::Config("invalid stored attachment binding".into()));
                    }
                    let reader = self.kernel().reader.open_blob(session, binding.blob)
                        .await.map_err(kernel_error)?;
                    if reader.descriptor().byte_len > media_input::MAX_MEDIA_BYTES as u64
                        || hex::encode(reader.descriptor().sha256) != *sha256
                    {
                        return Err(AppError::Config(
                            "stored attachment size or checksum does not match".into(),
                        ));
                    }
                    if resource.delivery == ResourceDelivery::Reference {
                        blobs.push(reader.into_prepared());
                        continue;
                    }
                    let mut stored = Vec::with_capacity(reader.descriptor().byte_len as usize);
                    reader.copy_to(&mut stored).await.map_err(kernel_error)?;
                    let name = resource.name.clone();
                    let prepared = MEDIA_WORKERS
                        .run(move || media_input::from_bytes(&name, stored, None))
                        .await
                        .map_err(|error| AppError::Internal(error.to_string()))?
                        .map_err(AppError::Config)?;
                    (prepared, Some(reader.into_prepared()))
                }
                _ => return Err(AppError::Config(
                    "model input must be a selected workspace file or a stored attachment in this session".into(),
                )),
            };
            bytes = bytes.saturating_add(prepared.bytes.len());
            if bytes > media_input::MAX_MEDIA_BATCH_BYTES {
                return Err(AppError::Config(
                    "message media exceeds the 40 MiB input limit".into(),
                ));
            }
            let bound = prepared.attachment(Some(&route));
            state
                .providers
                .validate_media_inputs(model, std::slice::from_ref(&bound))?;
            let blob_request =
                request_key(&format!("media:{session:?}:{request}"), &index.to_string());
            let blob = match existing_blob {
                Some(blob) => blob,
                None => {
                    self.prepare_immutable_blob(blob_request, session, &prepared.bytes)
                        .await?
                }
            };
            let mut attachment = bound;
            attachment.source = AttachmentSource::Base64 {
                data: String::new(),
            };
            let binding = MediaBinding {
                version: 1,
                model: model.clone(),
                route: route.clone(),
                blob: blob.descriptor().id,
                attachment,
            };
            resource.reference = ResourceReference::Artifact {
                sha256: prepared.sha256,
                uri: uri(&binding).map_err(kernel_error)?,
            };
            resource.media_type = Some(prepared.mime);
            resource.size_bytes = Some(prepared.size_bytes);
            resource.width = prepared.width;
            resource.height = prepared.height;
            blobs.push(blob);
        }
        Ok((document, blobs))
    }
}
