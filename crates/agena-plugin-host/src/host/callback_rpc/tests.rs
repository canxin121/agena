use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::poll;
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::sdk::host_api::NoopHostClient;
use crate::sdk::rpc::{RequestId, method};

const KEY: &str = "test.callback-rpc";
const WAIT: Duration = Duration::from_secs(5);

fn key() -> PluginKey {
    KEY.parse().unwrap()
}

#[derive(Default)]
struct InnerHost {
    calls: AtomicUsize,
    blocked: bool,
    release: Notify,
    dropped: CancellationToken,
    replace_and_register: Mutex<Option<Weak<HostHandle>>>,
}

#[async_trait::async_trait]
impl HostClient for InnerHost {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> crate::sdk::Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, filter: EventFilter) -> crate::sdk::Result<EventSubscription> {
        NoopHostClient.subscribe_events(filter).await
    }
    async fn invoke_tool(
        &self,
        tool: String,
        input: Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        NoopHostClient.invoke_tool(tool, input).await
    }
    async fn read_config(&self, _: Option<String>) -> crate::sdk::Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let replace = self.replace_and_register.lock().unwrap().take();
        if let Some(weak) = replace {
            let host = weak.upgrade().unwrap();
            host.begin_plugin_instance(key());
            host.theme_register(
                KEY,
                serde_json::from_value(json!({
                    "id": "admitted", "display_name": "Original callback owner", "colors": {}
                }))
                .unwrap(),
            )?;
        }
        let _dropped = self.dropped.clone().drop_guard();
        if self.blocked {
            self.release.notified().await;
        }
        Ok(json!({"host": "admitted"}))
    }
}

fn new_host(inner: Arc<dyn HostClient>, lineage: Option<&HostHandle>) -> Arc<HostHandle> {
    let mut host = HostHandle::new(inner);
    host.callback_base_url = Some("http://127.0.0.1:1".into());
    if let Some(lineage) = lineage {
        host.callback_routes = lineage.callback_routes.clone();
    }
    Arc::new(host)
}

async fn setup(inner: Arc<InnerHost>) -> (Arc<HostHandle>, Arc<PluginEffectScope>, String) {
    let host = new_host(inner, None);
    let scope = host.begin_plugin_instance(key());
    let token = host.callback_token(KEY).await.unwrap();
    (host, scope, token)
}

fn request(context: Value) -> Request {
    Request {
        jsonrpc: JsonRpcVersion,
        id: RequestId::Num(9),
        method: method::HOST_CONFIG_READ.into(),
        params: Some(json!({"context": context})),
        context: None,
    }
}

fn success(response: Response) -> Value {
    match response.payload {
        ResponsePayload::Ok { result } => result,
        ResponsePayload::Err { error } => panic!("callback failed: {error:?}"),
    }
}

fn stopped(response: Response) {
    let ResponsePayload::Err { error } = response.payload else {
        panic!("stopped callback succeeded")
    };
    let error = crate::transport::plugin_error_from_rpc(error, "callback fixture");
    assert_eq!(error.kind, PluginErrorKind::HostUnavailable);
}

#[tokio::test]
async fn credential_issuance_requires_a_configured_endpoint_and_an_accepting_scope() {
    let unconfigured = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
    unconfigured.begin_plugin_instance(key());
    assert!(unconfigured.callback_token(KEY).await.is_none());
    let host = new_host(Arc::new(NoopHostClient), None);
    assert!(host.callback_token(KEY).await.is_none());
    assert!(host.callback_token("invalid").await.is_none());
    let scope = host.begin_plugin_instance(key());
    let token = host.callback_token(KEY).await.unwrap();
    assert_eq!(
        host.callback_token(KEY).await.as_deref(),
        Some(token.as_str())
    );
    assert!(host.validate_callback_token(KEY, Some(&token)).await);
    scope.quiesce().await;
    assert!(host.callback_token(KEY).await.is_none());
    assert!(!host.validate_callback_token(KEY, Some(&token)).await);
    assert_eq!(
        host.dispatch_callback_rpc(KEY, Some(&token), request(json!({})))
            .await
            .unwrap_err(),
        PluginCallbackRpcError::InvalidCallbackToken
    );
    scope.dispose().await;
    assert!(host.callback_token(KEY).await.is_none());
    assert!(!host.validate_callback_token(KEY, Some(&token)).await);
}

#[tokio::test]
async fn replacing_a_scope_rotates_its_credential_and_old_cleanup_preserves_the_new_one() {
    let (host, old, token) = setup(Arc::new(InnerHost::default())).await;
    let successor = host.begin_plugin_instance(key());
    assert!(!host.validate_callback_token(KEY, Some(&token)).await);
    let next = host.callback_token(KEY).await.unwrap();
    assert_ne!(token, next);
    assert_eq!(
        host.dispatch_callback_rpc(KEY, Some(&token), request(json!({})))
            .await
            .unwrap_err(),
        PluginCallbackRpcError::InvalidCallbackToken
    );
    host.dispose_plugin_resources_for_scope(&key(), &old).await;
    assert!(host.validate_callback_token(KEY, Some(&next)).await);
    assert_eq!(host.callback_routes.lock().len(), 1);
    host.dispose_plugin_resources_for_scope(&key(), &successor)
        .await;
    assert!(host.callback_routes.lock().is_empty());
}

#[tokio::test]
async fn authentication_waiting_on_tokens_rechecks_the_exact_scope() {
    let inner = Arc::new(InnerHost::default());
    let (host, old, token) = setup(inner.clone()).await;
    let tokens = host.tokens.lock().await;
    let mut callback = Box::pin(host.dispatch_callback_rpc(KEY, Some(&token), request(json!({}))));
    assert!(poll!(&mut callback).is_pending());
    let next = host.begin_plugin_instance(key());
    drop(tokens);
    assert_eq!(
        callback.await.unwrap_err(),
        PluginCallbackRpcError::InvalidCallbackToken
    );
    assert_eq!(old.active_leases(), 0);
    assert_eq!(next.active_leases(), 0);
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn credential_issuance_waiting_on_tokens_rechecks_scope_cancellation() {
    let (host, scope, token) = setup(Arc::new(InnerHost::default())).await;
    let tokens = host.tokens.lock().await;
    let mut issue = Box::pin(host.callback_token(KEY));
    let mut validate = Box::pin(host.validate_callback_token(KEY, Some(&token)));
    assert!(poll!(&mut issue).is_pending());
    assert!(poll!(&mut validate).is_pending());
    scope.quiesce().await;
    drop(tokens);
    assert!(issue.await.is_none());
    assert!(!validate.await);
}

#[tokio::test]
async fn callback_lease_covers_the_inner_host_lock_wait_and_is_cancelled() {
    let inner = Arc::new(InnerHost::default());
    let (host, scope, token) = setup(inner.clone()).await;
    let lock = host.inner.write().await;
    let mut callback = Box::pin(host.dispatch_callback_rpc(KEY, Some(&token), request(json!({}))));
    assert!(poll!(&mut callback).is_pending());
    assert_eq!(scope.active_leases(), 1);
    scope.stop_admission();
    stopped(tokio::time::timeout(WAIT, callback).await.unwrap().unwrap());
    assert_eq!(scope.active_leases(), 0);
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
    drop(lock);
    assert!(scope.dispose().await.errors.is_empty());
}

#[tokio::test]
async fn disposal_drops_an_active_callback_before_running_owned_cleanup() {
    let inner = Arc::new(InnerHost {
        blocked: true,
        ..Default::default()
    });
    let (host, scope, token) = setup(inner.clone()).await;
    let dropped = inner.dropped.clone();
    scope
        .own_sync("fixture", "callback-order", move || {
            assert!(dropped.is_cancelled());
            Ok(())
        })
        .unwrap();
    let mut callback = Box::pin(host.dispatch_callback_rpc(KEY, Some(&token), request(json!({}))));
    assert!(poll!(&mut callback).is_pending());
    assert_eq!(scope.active_leases(), 1);
    let (response, report) =
        tokio::time::timeout(WAIT, async { tokio::join!(callback, scope.dispose()) })
            .await
            .unwrap();
    stopped(response.unwrap());
    assert!(report.errors.is_empty());
    assert!(inner.dropped.is_cancelled());
    assert_eq!(scope.active_leases(), 0);
}

#[tokio::test]
async fn completed_callback_releases_its_lease() {
    let inner = Arc::new(InnerHost {
        blocked: true,
        ..Default::default()
    });
    let (host, scope, token) = setup(inner.clone()).await;
    let mut callback = Box::pin(host.dispatch_callback_rpc(KEY, Some(&token), request(json!({}))));
    assert!(poll!(&mut callback).is_pending());
    assert_eq!(scope.active_leases(), 1);
    inner.release.notify_one();
    assert_eq!(
        success(callback.await.unwrap()),
        json!({"host": "admitted"})
    );
    assert_eq!(scope.active_leases(), 0);
}

#[tokio::test]
async fn dynamic_registration_retains_the_admitted_owner_across_scope_replacement() {
    let inner = Arc::new(InnerHost::default());
    let (host, admitted, token) = setup(inner.clone()).await;
    *inner.replace_and_register.lock().unwrap() = Some(Arc::downgrade(&host));
    success(
        host.dispatch_callback_rpc(KEY, Some(&token), request(json!({})))
            .await
            .unwrap(),
    );
    let next = host.effect_scope(&key()).unwrap();
    assert!(!Arc::ptr_eq(&admitted, &next));
    assert_eq!(host.theme_list_response().themes.len(), 1);
    next.dispose().await;
    assert_eq!(host.theme_list_response().themes.len(), 1);
    admitted.dispose().await;
    assert!(host.theme_list_response().themes.is_empty());
}

#[tokio::test]
async fn callback_routes_are_weak_and_confined_to_one_runtime_lineage() {
    let inner = Arc::new(InnerHost::default());
    let (first, first_scope, first_token) = setup(inner.clone()).await;
    let next_inner = Arc::new(InnerHost::default());
    let next = new_host(next_inner.clone(), Some(&first));
    let next_scope = next.begin_plugin_instance(key());
    let token = next.callback_token(KEY).await.unwrap();
    assert!(!first.validate_callback_token(KEY, Some(&token)).await);
    success(
        first
            .dispatch_callback_rpc(KEY, Some(&token), request(json!({})))
            .await
            .unwrap(),
    );
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
    assert_eq!(next_inner.calls.load(Ordering::SeqCst), 1);
    let unrelated = new_host(Arc::new(NoopHostClient), None);
    unrelated.begin_plugin_instance(key());
    assert_eq!(
        unrelated
            .dispatch_callback_rpc(KEY, Some(&token), request(json!({})))
            .await
            .unwrap_err(),
        PluginCallbackRpcError::InvalidCallbackToken
    );
    assert_eq!(
        first
            .dispatch_callback_rpc("test.other", Some(&token), request(json!({})))
            .await
            .unwrap_err(),
        PluginCallbackRpcError::InvalidCallbackToken
    );
    let weak_host = Arc::downgrade(&next);
    let weak_scope = Arc::downgrade(&next_scope);
    drop(next_scope);
    drop(next);
    assert!(weak_host.upgrade().is_none());
    assert!(weak_scope.upgrade().is_none());
    assert_eq!(first.callback_routes.lock().len(), 1);
    assert_eq!(
        first
            .dispatch_callback_rpc(KEY, Some(&token), request(json!({})))
            .await
            .unwrap_err(),
        PluginCallbackRpcError::InvalidCallbackToken
    );
    assert!(first.validate_callback_token(KEY, Some(&first_token)).await);
    first
        .dispose_plugin_resources_for_scope(&key(), &first_scope)
        .await;
    assert!(first.callback_routes.lock().is_empty());
}

#[tokio::test]
async fn invalid_auth_or_context_never_reaches_the_host_operation() {
    let inner = Arc::new(InnerHost::default());
    let (host, scope, token) = setup(inner.clone()).await;
    for credential in [None, Some("unknown-token")] {
        assert_eq!(
            host.dispatch_callback_rpc(KEY, credential, request(json!({})))
                .await
                .unwrap_err(),
            PluginCallbackRpcError::InvalidCallbackToken
        );
    }
    for context in [
        Value::Null,
        json!([]),
        json!("invalid"),
        json!({"call_id": "invalid"}),
    ] {
        assert_eq!(
            host.dispatch_callback_rpc(KEY, Some(&token), request(context))
                .await
                .unwrap_err(),
            PluginCallbackRpcError::MissingCallbackContext
        );
        assert_eq!(scope.active_leases(), 0);
    }
    let response = host
        .dispatch_callback_rpc(KEY, Some(&token), request(json!({"call_id": 8})))
        .await
        .unwrap();
    assert!(matches!(response.payload, ResponsePayload::Err { .. }));
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn preconstructed_dispatcher_serves_the_first_host_and_preserves_reload_lineage() {
    let build = || PluginHostBuildConfig {
        static_plugins: vec![],
        config: crate::PluginsConfig::default(),
        workspace_root: std::env::temp_dir(),
        agena_version: "test".into(),
        callback_base_url: Some("http://127.0.0.1:1".into()),
        host_client: Some(Arc::new(InnerHost::default())),
        previous: None,
        previous_plugins: HashMap::new(),
    };
    let dispatcher = PluginCallbackDispatcher::default();
    let host = PluginHost::new_with_callback_dispatcher(build(), dispatcher.clone())
        .await
        .unwrap();
    let handle = host.host_handle();
    let _scope = handle.begin_plugin_instance(key());
    let token = handle.callback_token(KEY).await.unwrap();
    let response = dispatcher
        .dispatch(KEY, Some(&token), request(json!({})))
        .await
        .unwrap();
    assert!(matches!(response.payload, ResponsePayload::Ok { .. }));
    let mut next = build();
    next.previous = Some(host.clone());
    assert!(
        matches!(PluginHost::new_with_callback_dispatcher(next, PluginCallbackDispatcher::default()).await,
        Err(HostError::Config(message)) if message.contains("previous host lineage"))
    );
    drop(host);
    drop(handle);
    assert_eq!(
        dispatcher
            .dispatch(KEY, Some(&token), request(json!({})))
            .await
            .unwrap_err(),
        PluginCallbackRpcError::InvalidCallbackToken
    );
}
