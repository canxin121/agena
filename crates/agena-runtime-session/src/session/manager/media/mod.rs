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

/// The run owns the media destination. Tool arguments can choose local input
/// bytes, but cannot choose a provider, adapter, or authorization scope.
pub(super) fn tool_media_input_binder(
    model: ModelRef,
    providers: Arc<ProviderRegistry>,
) -> Arc<crate::tool::MediaInputBinder> {
    Arc::new(move |mut attachment| {
        let AttachmentSource::Base64 { data } = attachment.source else {
            return Err(ToolError::invalid_input(
                "media input must contain an immutable local byte snapshot",
            ));
        };
        let route = providers
            .media_route_binding(&model)
            .map_err(|error| ToolError::invalid_input_error(&error))?;
        attachment.source = AttachmentSource::ProviderData { route, data };
        providers
            .validate_media_inputs(&model, std::slice::from_ref(&attachment))
            .map_err(|error| ToolError::invalid_input_error(&error))?;
        Ok(attachment)
    })
}

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
            // A staged upload's disk basename contains a UUID; preserve the
            // user's display name in its immutable attachment metadata.
            if !resource.name.trim().is_empty() {
                attachment.filename = Some(resource.name.clone());
            }
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
            // UI MIME/extension hints cannot override the validated bytes.
            resource.kind = match prepared.kind {
                agena_domain::AttachmentKind::Image => agena_domain::ResourceKind::Image,
                agena_domain::AttachmentKind::Audio => agena_domain::ResourceKind::Audio,
                agena_domain::AttachmentKind::Video => agena_domain::ResourceKind::Video,
                agena_domain::AttachmentKind::Pdf => agena_domain::ResourceKind::Pdf,
                agena_domain::AttachmentKind::File => agena_domain::ResourceKind::File,
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

#[cfg(test)]
mod tool_media_tests {
    use super::*;
    use agena_domain::{
        AdapterId, AttachmentItem, CapabilitySupport, Model, ModelCapabilities, ModelId, ProviderId,
    };
    use agena_provider::{CompletionRequest, PromptCacheShape};
    use agena_runtime_provider::{
        ProviderError,
        provider::{CompletionResponse, ModelRuntime},
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    type Observed = Arc<StdMutex<Vec<(Option<AdapterId>, ModelId, AttachmentItem)>>>;

    struct MediaProvider {
        default: ModelId,
        image_support: CapabilitySupport,
        route_revision: Option<&'static str>,
        reject_transport: bool,
        observed: Observed,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for MediaProvider {
        fn id(&self) -> &str {
            "media-test"
        }
        fn default_model(&self) -> &ModelId {
            &self.default
        }
        fn model_capabilities(&self, _: &ModelId) -> ModelCapabilities {
            ModelCapabilities {
                image_input: self.image_support,
                ..Default::default()
            }
        }
        fn prompt_cache_shape(&self, _: &ModelId) -> Option<PromptCacheShape> {
            self.route_revision.map(|revision| {
                PromptCacheShape::from_fields(self.id(), [("destination_revision", revision)])
            })
        }
        fn validate_media_inputs_for_adapter(
            &self,
            adapter: Option<&AdapterId>,
            model: &ModelId,
            items: &[AttachmentItem],
        ) -> Result<(), ProviderError> {
            for item in items {
                assert!(matches!(
                    &item.source,
                    AttachmentSource::ProviderData { .. }
                ));
                self.observed
                    .lock()
                    .unwrap()
                    .push((adapter.cloned(), model.clone(), item.clone()));
            }
            if self.reject_transport {
                return Err(ProviderError::Config(
                    "the selected adapter rejects image input".into(),
                ));
            }
            Ok(())
        }
        async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
            Ok(Vec::new())
        }
        async fn complete(
            &self,
            _: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            panic!("binding local media must never invoke a model")
        }
    }

    fn registry(
        support: CapabilitySupport,
        revision: Option<&'static str>,
        reject_transport: bool,
    ) -> (Arc<ProviderRegistry>, Observed) {
        let observed = Observed::default();
        let mut providers = ProviderRegistry::new();
        providers.register(MediaProvider {
            default: ModelId::new("server-default"),
            image_support: support,
            route_revision: revision,
            reject_transport,
            observed: observed.clone(),
        });
        (Arc::new(providers), observed)
    }

    fn selected() -> ModelRef {
        ModelRef {
            provider_id: ProviderId::new("media-test"),
            model_id: ModelId::new("session-vision"),
            adapter_id: Some(AdapterId::new("session-adapter")),
        }
    }

    fn snapshot() -> media_input::PreparedMedia {
        let bytes = STANDARD.decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNgYPgPAAEDAQAIicLsAAAAAElFTkSuQmCC",
        ).unwrap();
        media_input::from_bytes("image.png", bytes, None).unwrap()
    }

    #[test]
    fn tool_media_uses_the_owning_model_adapter_and_immutable_bytes() {
        let (providers, observed) = registry(CapabilitySupport::Supported, Some("first"), false);
        let model = selected();
        let prepared = snapshot();
        let bound =
            tool_media_input_binder(model.clone(), providers.clone())(prepared.attachment(None))
                .unwrap();
        let route = providers.media_route_binding(&model).unwrap();
        assert_eq!(bound, prepared.attachment(Some(&route)));
        let recorded = observed.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, model.adapter_id);
        assert_eq!(recorded[0].1, model.model_id);
        assert_eq!(recorded[0].2, bound);
        drop(recorded);

        // A different session's model/adapter and a changed destination each
        // require a different authorization scope for the same local bytes.
        let other = ModelRef {
            model_id: ModelId::new("other-model"),
            adapter_id: None,
            ..model.clone()
        };
        let other_bound =
            tool_media_input_binder(other, providers)(prepared.attachment(None)).unwrap();
        assert_ne!(bound.source, other_bound.source);
        let (changed, _) = registry(CapabilitySupport::Supported, Some("changed"), false);
        let changed_bound =
            tool_media_input_binder(model, changed)(prepared.attachment(None)).unwrap();
        assert_ne!(bound.source, changed_bound.source);
        assert_eq!(bound.sha256, changed_bound.sha256);
        assert_eq!(bound.size_bytes, changed_bound.size_bytes);
    }

    #[test]
    fn tool_media_requires_supported_transport_and_a_trusted_route() {
        let prepared = snapshot();
        for (support, revision, reject_transport, expected) in [
            (
                CapabilitySupport::Unsupported,
                Some("first"),
                false,
                "no confirmed support",
            ),
            (
                CapabilitySupport::Unknown,
                Some("first"),
                false,
                "no confirmed support",
            ),
            (
                CapabilitySupport::Supported,
                Some("first"),
                true,
                "selected adapter rejects",
            ),
            (
                CapabilitySupport::Supported,
                None,
                false,
                "stable media destination",
            ),
        ] {
            let (providers, _) = registry(support, revision, reject_transport);
            let error = tool_media_input_binder(selected(), providers)(prepared.attachment(None))
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
        let missing = tool_media_input_binder(selected(), Arc::new(ProviderRegistry::new()));
        assert!(missing(prepared.attachment(None)).is_err());

        let (providers, observed) = registry(CapabilitySupport::Supported, Some("first"), false);
        let binder = tool_media_input_binder(selected(), providers);
        for source in [
            AttachmentSource::ProviderData {
                route: "foreign-route".into(),
                data: STANDARD.encode(&prepared.bytes),
            },
            AttachmentSource::Url {
                url: "https://example.invalid/image.png".into(),
            },
            AttachmentSource::LocalPath {
                path: "/unread/image.png".into(),
            },
        ] {
            let mut item = prepared.attachment(None);
            item.source = source;
            assert!(binder(item).is_err());
        }
        assert!(observed.lock().unwrap().is_empty());
    }
}
