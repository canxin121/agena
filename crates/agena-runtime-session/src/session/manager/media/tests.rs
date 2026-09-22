use super::*;
use crate::{
    ContextGovernor, RuntimeSessionManagerConfig,
    authorization::ExecutionPrincipal,
    permission::{PermissionPolicy, ToolPermissionPolicy},
    provider::{ModelRuntime, ProviderError, ProviderRegistry},
    session::SessionProcessor,
    tool::ToolExecutor,
};
use agena_domain::{
    ActivityId, ActivityPayload, CapabilitySupport, ComposerActivity, ComposerDocument,
    ComposerNode, ModelCapabilities, ModelId, ResourceActivity, ResourceDelivery, ResourceKind,
    ResourceReference,
};
use agena_plugin_host::{PluginHost, PluginHostBuildConfig};
use agena_provider::{CompletionRequest, CompletionResponse, PromptCacheShape};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct MediaProvider {
    model: ModelId,
    shape: AtomicUsize,
    image: CapabilitySupport,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl ModelRuntime for MediaProvider {
    fn id(&self) -> &str {
        "fixture"
    }
    fn default_model(&self) -> &ModelId {
        &self.model
    }
    fn model_capabilities(&self, _: &ModelId) -> ModelCapabilities {
        ModelCapabilities {
            image_input: self.image,
            document_input: CapabilitySupport::Supported,
            ..Default::default()
        }
    }
    fn prompt_cache_shape(&self, _: &ModelId) -> Option<PromptCacheShape> {
        Some(PromptCacheShape::from_fields(
            "fixture",
            [(
                "endpoint_and_account",
                self.shape.load(Ordering::SeqCst).to_string(),
            )],
        ))
    }
    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            serde_json::to_string(&request).unwrap().contains(PNG),
            "provider must receive actual media, not a local path or metadata-only placeholder"
        );
        Err(ProviderError::Config(
            "synthetic request must not reach a network".into(),
        ))
    }
}
async fn fixture(
    image: CapabilitySupport,
) -> (
    tempfile::TempDir,
    SessionManager,
    Session,
    Arc<MediaProvider>,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let plugins = PluginHost::new(PluginHostBuildConfig {
        workspace_root: root.clone(),
        agena_version: "test".into(),
        config: Default::default(),
        static_plugins: Vec::new(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        root,
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        plugins.clone(),
        None,
        None,
        None,
    );
    let provider = Arc::new(MediaProvider {
        model: ModelId::new("vision"),
        shape: AtomicUsize::new(1),
        image,
        calls: AtomicUsize::new(0),
    });
    let mut registry = ProviderRegistry::new();
    registry.register_arc(provider.clone());
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    agena_storage_sqlite::initialize_schema(&db).await.unwrap();
    let manager = SessionManager::new(
        db,
        Arc::new(registry),
        ContextGovernor::new(Default::default()),
        SessionProcessor::new(plugins),
        executor,
        RuntimeSessionManagerConfig::default(),
    );
    let session = manager
        .create_session(crate::SessionCreateRequest {
            title: "media-test".into(),
            parent_session_id: None,
        })
        .await
        .unwrap();
    (dir, manager, session, provider)
}
fn parts(path: &str, delivery: ResourceDelivery, sha: Option<String>) -> Vec<TypedContent> {
    let mut activity = ComposerActivity {
        id: ActivityId::new(),
        payload: ActivityPayload::Resource(ResourceActivity {
            delivery,
            kind: ResourceKind::Image,
            reference: ResourceReference::WorkspacePath { path: path.into() },
            name: "input.png".into(),
            media_type: Some("image/png".into()),
            size_bytes: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        }),
        provenance: Default::default(),
    };
    activity.provenance.content_hash = sha;
    super::super::part_contents_from_composer_document(ComposerDocument(vec![
        ComposerNode::activity(activity),
    ]))
    .unwrap()
}
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQVDL+DwACFAFmBODefwAAAABJRU5ErkJggg==";
#[tokio::test]
async fn explicit_send_snapshots_actual_bytes_and_binds_the_provider_connection() {
    let (dir, manager, session, provider) = fixture(CapabilitySupport::Supported).await;
    let bytes = STANDARD.decode(PNG).unwrap();
    std::fs::write(dir.path().join("input.png"), &bytes).unwrap();
    let route = ModelRef::new("fixture", "vision");
    let output = manager
        .materialize_user_media(
            &session,
            &route,
            parts("input.png", ResourceDelivery::ModelInput, None),
        )
        .await
        .unwrap();
    let TypedContent::FileRef(reference) = &output[0] else {
        panic!("file reference expected")
    };
    let attachment = attachment_from_file_ref(reference).attachments.remove(0);
    let AttachmentSource::ProviderData {
        route: binding,
        data,
    } = &attachment.source
    else {
        panic!("explicit input was not materialized")
    };
    assert_eq!(STANDARD.decode(data).unwrap(), bytes);
    assert_eq!(
        *binding,
        manager
            .execution_state()
            .provider_registry
            .media_route_binding(&route)
            .unwrap()
    );
    assert_ne!(*binding, route.to_string());
    assert_eq!(reference.extra["input_transport"], "inline");
    assert!(
        !reference.extra.contains_key("source"),
        "avoid duplicating raw base64 snapshots"
    );
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        0,
        "preparation is not the network send"
    );
    std::fs::write(dir.path().join("input.png"), "changed later").unwrap();
    assert_eq!(
        STANDARD.decode(data).unwrap(),
        bytes,
        "persisted input is an immutable snapshot"
    );
    let input_part =
        agena_runtime_provider::provider::wire_message::completion_input_part_from_wire(
            agena_runtime_provider::provider::wire_message::WirePart::Attachment {
                item: attachment.clone(),
            },
        );
    let request: CompletionRequest = serde_json::from_value(
        serde_json::json!({"model":"vision","messages":[{"role":"user","parts":[input_part]}]}),
    )
    .unwrap();
    let error = manager
        .execution_state()
        .provider_registry
        .complete(&route, request.clone())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("synthetic request"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    provider.shape.store(2, Ordering::SeqCst);
    let error = manager
        .execution_state()
        .provider_registry
        .complete(&route, request)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("different provider/model/endpoint/account")
    );
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "changed destination must be rejected before provider invocation"
    );

    assert_ne!(
        *binding,
        manager
            .execution_state()
            .provider_registry
            .media_route_binding(&route)
            .unwrap(),
        "same provider alias with different endpoint/account must have a new route"
    );
}
#[tokio::test]
async fn reference_only_does_not_read_missing_file_or_resolve_model_route() {
    let (_dir, manager, session, provider) = fixture(CapabilitySupport::Unsupported).await;
    let output = manager
        .materialize_user_media(
            &session,
            &ModelRef::new("not-configured", "none"),
            parts("missing.png", ResourceDelivery::Reference, None),
        )
        .await
        .unwrap();
    let TypedContent::FileRef(reference) = &output[0] else {
        panic!("file reference expected")
    };
    assert!(matches!(
        attachment_from_file_ref(reference).attachments[0].source,
        AttachmentSource::LocalPath { .. }
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn malformed_stale_and_unsupported_inputs_fail_without_provider_invocation() {
    let (dir, manager, session, provider) = fixture(CapabilitySupport::Unsupported).await;
    std::fs::write(dir.path().join("input.png"), STANDARD.decode(PNG).unwrap()).unwrap();
    let model = ModelRef::new("fixture", "vision");
    assert!(
        manager
            .materialize_user_media(
                &session,
                &model,
                parts("input.png", ResourceDelivery::ModelInput, None)
            )
            .await
            .is_err()
    );
    assert!(
        manager
            .materialize_user_media(
                &session,
                &model,
                parts(
                    "input.png",
                    ResourceDelivery::ModelInput,
                    Some("stale".into())
                )
            )
            .await
            .is_err()
    );
    std::fs::write(dir.path().join("invalid.png"), "not an image").unwrap();
    assert!(
        manager
            .materialize_user_media(
                &session,
                &model,
                parts("invalid.png", ResourceDelivery::ModelInput, None)
            )
            .await
            .is_err()
    );
    let external = tempfile::tempdir().unwrap();
    std::fs::write(
        external.path().join("outside.png"),
        STANDARD.decode(PNG).unwrap(),
    )
    .unwrap();
    assert!(
        manager
            .materialize_user_media(
                &session,
                &model,
                parts(
                    external.path().join("outside.png").to_str().unwrap(),
                    ResourceDelivery::ModelInput,
                    None
                )
            )
            .await
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unknown_media_capabilities_fail_closed_before_sending() {
    let (dir, manager, session, provider) = fixture(CapabilitySupport::Unknown).await;
    std::fs::write(dir.path().join("input.png"), STANDARD.decode(PNG).unwrap()).unwrap();
    assert!(
        manager
            .materialize_user_media(
                &session,
                &ModelRef::new("fixture", "vision"),
                parts("input.png", ResourceDelivery::ModelInput, None)
            )
            .await
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}
