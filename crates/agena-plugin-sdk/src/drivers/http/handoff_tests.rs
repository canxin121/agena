use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

use axum::http::HeaderMap;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::*;
use crate::host_api::NoopHostClient;
use crate::{InitOutcome, PluginManifest};

const WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct RetainedPlugin {
    host: Arc<Mutex<Option<Arc<dyn HostClient>>>>,
    inits: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Plugin for RetainedPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("test", "http-rebind", "1.0.0")
    }

    async fn init(&self, _: InitContext, host: Arc<dyn HostClient>) -> crate::Result<InitOutcome> {
        *self.host.lock().unwrap() = Some(host);
        self.inits.fetch_add(1, Ordering::SeqCst);
        Ok(InitOutcome::ack(self.manifest()))
    }
}

#[derive(Default)]
struct FallbackHost(AtomicUsize);

#[async_trait::async_trait]
impl HostClient for FallbackHost {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> crate::Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, filter: EventFilter) -> crate::Result<EventSubscription> {
        NoopHostClient.subscribe_events(filter).await
    }
    async fn invoke_tool(&self, tool: String, input: Value) -> crate::Result<ToolInvokeOutput> {
        NoopHostClient.invoke_tool(tool, input).await
    }
    async fn read_config(&self, _: Option<String>) -> crate::Result<Value> {
        Ok(json!({"host": "fallback"}))
    }
    async fn unsubscribe_events(&self, id: String) -> crate::Result<()> {
        assert_eq!(id, "retained-subscription");
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct Fixture {
    state: Arc<HttpDriverState<RetainedPlugin>>,
    retained: Arc<Mutex<Option<Arc<dyn HostClient>>>>,
    inits: Arc<AtomicUsize>,
    fallback: Arc<FallbackHost>,
    servers: Vec<JoinHandle<()>>,
}

impl Fixture {
    async fn new(binding: HttpCallbackBinding) -> Self {
        let plugin = RetainedPlugin::default();
        let retained = plugin.host.clone();
        let inits = plugin.inits.clone();
        let fallback = Arc::new(FallbackHost::default());
        let factory_host = retained.clone();
        let factory_inits = inits.clone();
        let fixture = Self {
            state: Arc::new(HttpDriverState::new(
                move || RetainedPlugin {
                    host: factory_host.clone(),
                    inits: factory_inits.clone(),
                },
                fallback.clone(),
            )),
            retained,
            inits,
            fallback,
            servers: vec![],
        };
        let init = InitContext {
            agena_version: "test".into(),
            workspace_root: "/test".into(),
            plugin_id: "test.http-rebind".parse().unwrap(),
            host_callback_url: binding.url,
            host_callback_token: binding.token,
            settings: json!({}),
            protocol_version: crate::rpc::PROTOCOL_VERSION,
        };
        success(
            fixture
                .send(method::META_INIT, serde_json::to_value(init).unwrap())
                .await,
        );
        fixture
    }

    fn host(&self) -> Arc<dyn HostClient> {
        self.retained.lock().unwrap().as_ref().unwrap().clone()
    }

    async fn listener(&mut self, marker: &'static str) -> HttpCallbackBinding {
        let router = Router::new()
            .route("/callback", post(callback))
            .with_state(marker);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/callback", listener.local_addr().unwrap());
        self.servers.push(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }));
        HttpCallbackBinding {
            url: Some(url),
            token: Some(marker.into()),
        }
    }

    async fn send(&self, method_name: &str, params: Value) -> Response {
        tokio::time::timeout(
            WAIT,
            handle_rpc(
                State(self.state.clone()),
                HeaderMap::new(),
                Json(Request {
                    jsonrpc: JsonRpcVersion,
                    id: RequestId::Num(1),
                    method: method_name.into(),
                    params: Some(params),
                    context: None,
                }),
            ),
        )
        .await
        .unwrap()
        .0
    }

    async fn rebind(&self, expected: HttpCallbackBinding, next: HttpCallbackBinding) -> Response {
        self.send(
            method::META_HOST_REBIND,
            serde_json::to_value(HttpCallbackRebind { expected, next }).unwrap(),
        )
        .await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
    }
}

async fn callback(
    State(marker): State<&'static str>,
    headers: HeaderMap,
    Json(req): Json<Request>,
) -> Json<Response> {
    let expected = format!("Bearer {marker}");
    let payload =
        if headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(expected.as_str()) {
            ResponsePayload::Ok {
                result: json!({"host": marker}),
            }
        } else {
            ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::PLUGIN_GENERIC,
                    message: "callback URL and bearer were mixed across generations".into(),
                    data: None,
                },
            }
        };
    Json(Response {
        jsonrpc: JsonRpcVersion,
        id: if marker == "mismatched-id" {
            RequestId::Str("unrelated-request".into())
        } else {
            req.id
        },
        payload,
    })
}

fn disabled() -> HttpCallbackBinding {
    HttpCallbackBinding {
        url: None,
        token: None,
    }
}

fn success(response: Response) {
    if let ResponsePayload::Err { error } = response.payload {
        panic!("rebind failed: {error:?}");
    }
}

#[tokio::test]
async fn mismatched_callback_response_id_cannot_satisfy_a_retained_host_call() {
    let mut fixture = Fixture::new(disabled()).await;
    let binding = fixture.listener("mismatched-id").await;
    success(fixture.rebind(disabled(), binding).await);
    let error = fixture.host().read_config(None).await.unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::Disconnected);
    assert!(error.diagnostic_message().contains("response ID"));
}

#[tokio::test]
async fn retained_host_enables_disables_and_restores_the_full_fallback_api() {
    let mut fixture = Fixture::new(disabled()).await;
    let host = fixture.host();
    assert_eq!(
        host.read_config(None).await.unwrap(),
        json!({"host": "fallback"})
    );
    host.unsubscribe_events("retained-subscription".into())
        .await
        .unwrap();
    let first = fixture.listener("first").await;
    success(fixture.rebind(disabled(), first.clone()).await);
    assert_eq!(
        host.read_config(None).await.unwrap(),
        json!({"host": "first"})
    );
    let old_remote = fixture
        .state
        .instances
        .current()
        .unwrap()
        .callback_client
        .read()
        .await
        .clone()
        .unwrap();
    success(fixture.rebind(first, disabled()).await);
    assert_eq!(
        old_remote.read_config(None).await.unwrap_err().kind,
        PluginErrorKind::HostUnavailable
    );
    assert_eq!(
        host.read_config(None).await.unwrap(),
        json!({"host": "fallback"})
    );
    host.unsubscribe_events("retained-subscription".into())
        .await
        .unwrap();
    let second = fixture.listener("second").await;
    success(fixture.rebind(disabled(), second).await);
    assert_eq!(
        host.read_config(None).await.unwrap(),
        json!({"host": "second"})
    );
    assert!(Arc::ptr_eq(&host, &fixture.host()));
    assert_eq!(fixture.inits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.fallback.0.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn changed_url_and_bearer_reach_the_new_listener_without_init() {
    let mut fixture = Fixture::new(disabled()).await;
    let first = fixture.listener("first").await;
    let second = fixture.listener("second").await;
    success(fixture.rebind(disabled(), first.clone()).await);
    let host = fixture.host();
    assert_eq!(
        host.read_config(None).await.unwrap(),
        json!({"host": "first"})
    );
    success(fixture.rebind(first, second).await);
    assert_eq!(
        host.read_config(None).await.unwrap(),
        json!({"host": "second"})
    );
    assert_eq!(fixture.inits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stale_compare_and_replace_leaves_the_live_binding_unchanged() {
    let mut fixture = Fixture::new(disabled()).await;
    let first = fixture.listener("first").await;
    let second = fixture.listener("second").await;
    success(fixture.rebind(disabled(), first.clone()).await);
    success(fixture.rebind(first.clone(), second).await);
    let stale = fixture.rebind(first, disabled()).await;
    assert!(matches!(stale.payload, ResponsePayload::Err { .. }));
    assert_eq!(
        fixture.host().read_config(None).await.unwrap(),
        json!({"host": "second"})
    );
}

#[tokio::test]
async fn malformed_binding_objects_are_rejected_without_mutating_callbacks() {
    let fixture = Fixture::new(disabled()).await;
    for params in [
        json!([]),
        json!({}),
        json!({"expected": [], "next": {}}),
        json!({"expected": {}, "next": []}),
        json!({"expected": {}, "next": {"token": "orphan"}}),
        json!({"expected": {}, "next": {"url": "file:///tmp/invalid"}}),
        json!({"expected": {}, "next": {"url": "invalid"}}),
        json!({"expected": {}, "next": {}, "legacy": true}),
        json!({"expected": {"legacy": true}, "next": {}}),
        json!({"expected": {}, "next": {"legacy": true}}),
    ] {
        assert!(
            matches!(
                fixture
                    .send(method::META_HOST_REBIND, params.clone())
                    .await
                    .payload,
                ResponsePayload::Err { .. }
            ),
            "accepted malformed binding: {params}"
        );
    }
    assert_eq!(
        fixture.host().read_config(None).await.unwrap(),
        json!({"host": "fallback"})
    );
}

#[tokio::test]
async fn full_dispatch_capacity_cannot_starve_callback_handoff() {
    let mut fixture = Fixture::new(disabled()).await;
    let binding = fixture.listener("successor").await;
    let _slots = fixture.state.dispatch_slots.acquire_many(64).await.unwrap();
    success(fixture.rebind(disabled(), binding).await);
    assert_eq!(
        fixture.host().read_config(None).await.unwrap(),
        json!({"host": "successor"})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_callbacks_snapshot_a_complete_url_and_bearer_pair() {
    let mut fixture = Fixture::new(disabled()).await;
    let first = fixture.listener("first").await;
    let second = fixture.listener("second").await;
    success(fixture.rebind(disabled(), first.clone()).await);
    let callbacks = async {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let host = fixture.host();
            tasks.spawn(async move {
                for _ in 0..16 {
                    let result = host.read_config(None).await.unwrap();
                    assert!(
                        result == json!({"host": "first"}) || result == json!({"host": "second"})
                    );
                }
            });
        }
        while let Some(task) = tasks.join_next().await {
            task.unwrap();
        }
    };
    let handoffs = async {
        for _ in 0..128 {
            success(fixture.rebind(first.clone(), second.clone()).await);
            tokio::task::yield_now().await;
            success(fixture.rebind(second.clone(), first.clone()).await);
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(WAIT, async {
        tokio::join!(callbacks, handoffs);
    })
    .await
    .unwrap();
}
