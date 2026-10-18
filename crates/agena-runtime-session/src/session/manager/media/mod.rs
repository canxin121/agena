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
        mut document: ComposerDocument,
        request: RequestId,
    ) -> Result<(ComposerDocument, Vec<PreparedBlob>), AppError> {
        if !document.0.iter().any(|node|matches!(node,ComposerNode::Activity{activity} if matches!(&activity.payload,ActivityPayload::Resource(resource) if resource.delivery==ResourceDelivery::ModelInput))){return Ok((document,Vec::new()));}
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
        let state = self.state.load_full();
        let context = self.execution_context_value(session).await?;
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
            if resource.delivery != ResourceDelivery::ModelInput {
                continue;
            }
            if blobs.len() >= media_input::MAX_MEDIA_INPUTS {
                return Err(AppError::Config(
                    "at most eight media resources may be sent in one message".into(),
                ));
            }
            let ResourceReference::WorkspacePath { path } = &resource.reference else {
                return Err(AppError::Config(
                    "model input must be an explicitly selected workspace file".into(),
                ));
            };
            let target = root.join(path).canonicalize().map_err(|e| {
                AppError::Config(format!("selected attachment is unavailable: {e}"))
            })?;
            if !target.starts_with(&root) {
                return Err(AppError::Config(
                    "selected attachment escapes the active workspace".into(),
                ));
            }
            if !matches!(
                executor
                    .principal()
                    .authorize_path_access(AccessKind::Read, &root, &target),
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
                .map_err(|e| AppError::Internal(e.to_string()))?
                .map_err(AppError::Config)?;
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
            let blob = self
                .prepare_immutable_blob(blob_request, session, &prepared.bytes)
                .await?;
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
