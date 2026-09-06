use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use agena_plugin_host::config::{ConfiguredPlugin, HttpAuth, PluginPackage, PluginsConfig};
use agena_plugin_host::host::{PluginHost, PluginHostBuildConfig};
use agena_plugin_sdk::drivers::http::HttpCallbackRebind;
use agena_plugin_sdk::host_api::{EventSubscription, HostClient, LogLevel, NoopHostClient};
use agena_plugin_sdk::rpc::{
    ErrorObject, JsonRpcVersion, Request, RequestId, Response, ResponsePayload, codes, method,
};
use agena_plugin_sdk::{
    EventEnvelope, EventFilter, InitContext, InitOutcome, Plugin, PluginManifest, ToolInvokeInput,
    ToolInvokeOutput, ToolStreamEnd, ToolStreamSink,
};
use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

const KEY: &str = "test.http-handoff";
const WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
    dropped: AtomicBool,
}

struct Dropped<'a>(&'a AtomicBool);
impl Drop for Dropped<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct MarkerHost {
    marker: &'static str,
    gate: Arc<Gate>,
}

#[async_trait::async_trait]
impl HostClient for MarkerHost {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, event: EventEnvelope) -> agena_plugin_sdk::Result<()> {
        NoopHostClient.publish_event(event).await
    }
    async fn subscribe_events(
        &self,
        filter: EventFilter,
    ) -> agena_plugin_sdk::Result<EventSubscription> {
        NoopHostClient.subscribe_events(filter).await
    }
    async fn read_config(&self, path: Option<String>) -> agena_plugin_sdk::Result<Value> {
        if path.as_deref() == Some("blocked") {
            let _dropped = Dropped(&self.gate.dropped);
            self.gate.entered.notify_one();
            self.gate.release.notified().await;
        }
        Ok(json!({"host": self.marker}))
    }
    async fn invoke_tool(
        &self,
        tool: String,
        input: Value,
    ) -> agena_plugin_sdk::Result<ToolInvokeOutput> {
        NoopHostClient.invoke_tool(tool, input).await
    }
}

#[derive(Default)]
struct PluginState {
    host: Mutex<Option<Arc<dyn HostClient>>>,
    init_count: AtomicUsize,
    init_config: Mutex<Option<Value>>,
    remote: Gate,
    remote_context: Mutex<Option<agena_plugin_sdk::host_api::HostCallbackContext>>,
    stream_release: Notify,
    active: AtomicBool,
    shutdowns: AtomicUsize,
}

struct CallbackPlugin {
    state: Arc<PluginState>,
    callback_during_init: bool,
}

#[async_trait::async_trait]
impl Plugin for CallbackPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("test", "http-handoff", "1.0.0");
        manifest.settings = Some(agena_plugin_sdk::SettingsContract::bounded_json(
            "Settings",
            "Fixture settings",
            1024,
            8,
        ));
        manifest.tools.push(
            serde_json::from_value(json!({
                "name": "config", "docs": {"summary": "read owning host configuration"},
                "contract": {"input_schema": {"type": "object"}}
            }))
            .unwrap(),
        );
        manifest
    }

    async fn init(
        &self,
        _: InitContext,
        host: Arc<dyn HostClient>,
    ) -> agena_plugin_sdk::Result<InitOutcome> {
        self.state.init_count.fetch_add(1, Ordering::SeqCst);
        if self.callback_during_init {
            let config = host.read_config(None).await?;
            *self.state.init_config.lock().unwrap() = Some(config);
        }
        *self.state.host.lock().unwrap() = Some(host);
        self.state.active.store(true, Ordering::SeqCst);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn shutdown(&self) -> agena_plugin_sdk::Result<()> {
        self.state.active.store(false, Ordering::SeqCst);
        self.state.shutdowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn tool_invoke(
        &self,
        input: ToolInvokeInput,
    ) -> agena_plugin_sdk::Result<ToolInvokeOutput> {
        if !self.state.active.load(Ordering::SeqCst) {
            return Err(agena_plugin_sdk::PluginError::internal(
                "HTTP plugin instance has shut down",
            ));
        }
        if input.input["block_remote"] == true {
            *self.state.remote_context.lock().unwrap() =
                agena_plugin_sdk::host_api::current_host_callback_context();
            self.state.remote.entered.notify_one();
            self.state.remote.release.notified().await;
        }
        let host = self.state.host.lock().unwrap().as_ref().unwrap().clone();
        let path = (input.input["block_callback"] == true).then(|| "blocked".into());
        Ok(ToolInvokeOutput::text(
            host.read_config(path).await?.to_string(),
        ))
    }

    async fn tool_invoke_stream(
        &self,
        _: ToolInvokeInput,
        sink: ToolStreamSink,
    ) -> agena_plugin_sdk::Result<ToolStreamEnd> {
        for index in 0..48 {
            sink.text(format!("chunk-{index}")).await;
        }
        self.state.stream_release.notified().await;
        Ok(ToolStreamEnd::text(sink.stream_id(), "complete"))
    }
}

type CurrentHost = Arc<RwLock<Weak<PluginHost>>>;

struct Bridge {
    current: CurrentHost,
    chunks: AtomicUsize,
}

async fn callback(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Json<Response> {
    let id = request.id.clone();
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned);
    let stream_chunk = request.method == method::TOOL_STREAM_CHUNK;
    let host = bridge.current.read().unwrap().upgrade().unwrap();
    let result = super::dispatch_plugin_rpc(host, KEY, token, request).await;
    if stream_chunk
        && result
            .as_ref()
            .is_ok_and(|response| matches!(response.payload, ResponsePayload::Ok { .. }))
    {
        bridge.chunks.fetch_add(1, Ordering::SeqCst);
    }
    Json(result.unwrap_or_else(|error| Response {
        jsonrpc: JsonRpcVersion,
        id,
        payload: ResponsePayload::Err {
            error: ErrorObject {
                code: codes::PLUGIN_GENERIC,
                message: error.to_string(),
                data: None,
            },
        },
    }))
}

#[derive(Default)]
struct HandoffControl {
    mode: AtomicUsize,
    gate: Gate,
    next: Mutex<Option<HttpCallbackRebind>>,
}

async fn control_handoff(
    State(control): State<Arc<HandoffControl>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
    let envelope: Request = serde_json::from_slice(&bytes).unwrap();
    let mut actual_handoff = false;
    let mode = control.mode.load(Ordering::SeqCst);
    if envelope.method == method::META_SHUTDOWN && mode == 5 {
        control.gate.entered.notify_one();
        control.gate.release.notified().await;
    }
    if envelope.method == method::META_HTTP_STATE && matches!(mode, 6 | 7) {
        return Json(Response {
            jsonrpc: JsonRpcVersion,
            id: envelope.id,
            payload: if mode == 6 {
                ResponsePayload::Ok {
                    result: json!({"version": 999, "revision": null}),
                }
            } else {
                ResponsePayload::Err {
                    error: ErrorObject {
                        code: codes::METHOD_NOT_FOUND,
                        message: "HTTP instance control unsupported".into(),
                        data: None,
                    },
                }
            },
        })
        .into_response();
    }
    if envelope.method == method::META_HOST_REBIND {
        let binding: HttpCallbackRebind =
            serde_json::from_value(envelope.params.clone().unwrap()).unwrap();
        actual_handoff = binding.expected != binding.next;
        if actual_handoff {
            *control.next.lock().unwrap() = Some(binding);
        }
        if mode == 1 || (mode == 4 && actual_handoff) {
            return Json(Response {
                jsonrpc: JsonRpcVersion,
                id: envelope.id,
                payload: ResponsePayload::Err {
                    error: ErrorObject {
                        code: codes::METHOD_NOT_FOUND,
                        message: "handoff unavailable in fixture".into(),
                        data: None,
                    },
                },
            })
            .into_response();
        }
        if mode == 2 && actual_handoff {
            control.gate.entered.notify_one();
            control.gate.release.notified().await;
        }
    }
    let response = next
        .run(axum::extract::Request::from_parts(parts, bytes.into()))
        .await;
    if mode == 8 && envelope.method == method::META_HTTP_STATE {
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let mut response: Response = serde_json::from_slice(&body).unwrap();
        response.id = agena_plugin_host::sdk::rpc::RequestId::Str("unrelated-request".into());
        return Json(response).into_response();
    }
    if mode == 3 && actual_handoff {
        control.gate.entered.notify_one();
        control.gate.release.notified().await;
    }
    response
}

struct Fixture {
    current: CurrentHost,
    base: String,
    config: PluginsConfig,
    servers: Vec<JoinHandle<()>>,
    root: tempfile::TempDir,
    callbacks_enabled: bool,
    host_gate: Arc<Gate>,
    bridge: Arc<Bridge>,
    control: Arc<HandoffControl>,
}

impl Fixture {
    async fn new() -> (Self, Arc<PluginState>) {
        let current = Arc::new(RwLock::new(Weak::new()));
        let bridge = Arc::new(Bridge {
            current: current.clone(),
            chunks: AtomicUsize::new(0),
        });
        let (base, server) = serve(
            Router::new()
                .route("/plugin-rpc/{plugin_id}", post(callback))
                .with_state(bridge.clone()),
        )
        .await;
        let mut fixture = Self {
            current,
            base,
            config: PluginsConfig::default(),
            servers: vec![server],
            root: tempfile::tempdir().unwrap(),
            callbacks_enabled: true,
            host_gate: Arc::new(Gate::default()),
            bridge,
            control: Arc::new(HandoffControl::default()),
        };
        let plugin = fixture.plugin(false).await;
        (fixture, plugin)
    }

    async fn plugin(&mut self, callback_during_init: bool) -> Arc<PluginState> {
        let state = Arc::new(PluginState::default());
        let factory_state = state.clone();
        let (url, server) = serve(
            agena_plugin_sdk::drivers::http::router(
                move || CallbackPlugin {
                    state: factory_state.clone(),
                    callback_during_init,
                },
                Arc::new(MarkerHost {
                    marker: "fallback",
                    gate: self.host_gate.clone(),
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                self.control.clone(),
                control_handoff,
            )),
        )
        .await;
        self.servers.push(server);
        self.config.list.insert(
            KEY.into(),
            ConfiguredPlugin {
                package: PluginPackage::Http {
                    url: format!("{url}/rpc").parse().unwrap(),
                    auth: HttpAuth::None,
                },
                ..Default::default()
            },
        );
        state
    }

    async fn build(
        &self,
        marker: &'static str,
        previous: Option<Arc<PluginHost>>,
        previous_config: Option<&PluginsConfig>,
    ) -> Arc<PluginHost> {
        self.build_result(marker, previous, previous_config)
            .await
            .unwrap()
    }

    async fn build_result(
        &self,
        marker: &'static str,
        previous: Option<Arc<PluginHost>>,
        previous_config: Option<&PluginsConfig>,
    ) -> Result<Arc<PluginHost>, agena_plugin_host::HostError> {
        tokio::time::timeout(
            WAIT,
            PluginHost::new(PluginHostBuildConfig {
                static_plugins: vec![],
                config: self.config.clone(),
                workspace_root: self.root.path().into(),
                agena_version: "test".into(),
                callback_base_url: self.callbacks_enabled.then(|| self.base.clone()),
                host_client: Some(Arc::new(MarkerHost {
                    marker,
                    gate: self.host_gate.clone(),
                })),
                previous,
                previous_plugins: previous_config
                    .map(PluginHostBuildConfig::previous_plugins)
                    .unwrap_or_default(),
            }),
        )
        .await
        .unwrap()
    }

    fn publish(&self, host: &Arc<PluginHost>) {
        *self.current.write().unwrap() = Arc::downgrade(host);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
    }
}

async fn serve(router: Router) -> (String, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, server)
}

async fn invoke(host: &PluginHost, call: i64) -> agena_plugin_sdk::Result<Value> {
    invoke_input(host, call, json!({})).await
}

async fn invoke_input(
    host: &PluginHost,
    call: i64,
    input: Value,
) -> agena_plugin_sdk::Result<Value> {
    // Keep the invocation descriptor available after catalog cleanup so the
    // stopped transport itself must reject a caller that retained the tool.
    let plugin = &host.plugins()[0];
    let tool = agena_plugin_host::registry::RegisteredTool::new(
        plugin.key(),
        plugin.manifest.tools[0].clone(),
    )
    .unwrap();
    let output = tokio::time::timeout(
        WAIT,
        host.invoke_tool(
            &tool,
            ToolInvokeInput {
                tool_name: "config".into(),
                session_id: 11,
                call_id: call,
                workspace_root: "/test/workspace".into(),
                input,
            },
            None,
        ),
    )
    .await
    .unwrap()?;
    Ok(serde_json::from_str(&output.output_text).unwrap())
}

#[tokio::test]
async fn reused_http_plugin_callbacks_follow_the_successor_without_reinitialization() {
    let (fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    assert_eq!(invoke(&first, 1).await.unwrap(), json!({"host": "first"}));
    let second = fixture
        .build("second", Some(first.clone()), Some(&fixture.config))
        .await;
    fixture.publish(&second);
    let result = invoke(&second, 2).await;
    first.shutdown().await;
    let after_old_shutdown = invoke(&second, 3).await;
    second.shutdown().await;
    assert_eq!(
        plugin.init_count.load(Ordering::SeqCst),
        1,
        "a retained plugin must not repeat init"
    );
    assert_eq!(result.unwrap(), json!({"host": "second"}));
    assert_eq!(after_old_shutdown.unwrap(), json!({"host": "second"}));
}

#[tokio::test]
async fn candidate_http_initialization_callbacks_work_before_snapshot_publication() {
    let (mut fixture, _) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let previous_config = fixture.config.clone();
    let candidate = fixture.plugin(true).await;
    let second = fixture
        .build("second", Some(first.clone()), Some(&previous_config))
        .await;
    let tools = second.registered_tools();
    let init_config = candidate.init_config.lock().unwrap().clone();
    fixture.publish(&second);
    first.shutdown().await;
    second.shutdown().await;
    assert_eq!(
        tools.len(),
        1,
        "candidate init must succeed before publication"
    );
    assert_eq!(init_config, Some(json!({"host": "second"})));
}

#[tokio::test]
async fn reused_http_plugin_can_enable_disable_and_reenable_callbacks() {
    let (mut fixture, plugin) = Fixture::new().await;
    fixture.callbacks_enabled = false;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    assert_eq!(
        invoke(&first, 1).await.unwrap(),
        json!({"host": "fallback"})
    );
    fixture.callbacks_enabled = true;
    let second = fixture
        .build("second", Some(first.clone()), Some(&fixture.config))
        .await;
    fixture.publish(&second);
    assert_eq!(invoke(&second, 2).await.unwrap(), json!({"host": "second"}));
    fixture.callbacks_enabled = false;
    let third = fixture
        .build("third", Some(second.clone()), Some(&fixture.config))
        .await;
    fixture.publish(&third);
    assert_eq!(
        invoke(&third, 3).await.unwrap(),
        json!({"host": "fallback"})
    );
    fixture.callbacks_enabled = true;
    let fourth = fixture
        .build("fourth", Some(third.clone()), Some(&fixture.config))
        .await;
    fixture.publish(&fourth);
    first.shutdown().await;
    second.shutdown().await;
    third.shutdown().await;
    assert_eq!(invoke(&fourth, 4).await.unwrap(), json!({"host": "fourth"}));
    assert_eq!(plugin.init_count.load(Ordering::SeqCst), 1);
    fourth.shutdown().await;
}

#[tokio::test]
async fn handoff_reaches_a_changed_callback_listener_with_a_rotated_credential() {
    let (mut fixture, _) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let old_token = first.host_handle().callback_token(KEY).await.unwrap();
    let (base, server) = serve(
        Router::new()
            .route("/plugin-rpc/{plugin_id}", post(callback))
            .with_state(fixture.bridge.clone()),
    )
    .await;
    fixture.base = base;
    fixture.servers.push(server);
    let second = fixture
        .build("second", Some(first.clone()), Some(&fixture.config))
        .await;
    fixture.publish(&second);
    fixture.servers[0].abort();
    let new_token = second.host_handle().callback_token(KEY).await.unwrap();
    assert_ne!(old_token, new_token);
    assert!(
        !second
            .host_handle()
            .validate_callback_token(KEY, Some(&old_token))
            .await
    );
    assert_eq!(invoke(&second, 2).await.unwrap(), json!({"host": "second"}));
    first.shutdown().await;
    second.shutdown().await;
}

#[tokio::test]
async fn unsupported_handoff_fails_before_quiescing_and_preserves_the_predecessor() {
    let (fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    fixture.control.mode.store(1, Ordering::SeqCst);
    let result = fixture
        .build_result("second", Some(first.clone()), Some(&fixture.config))
        .await;
    assert!(
        result.is_err(),
        "unsupported handoff must not repeat init on the live server"
    );
    assert!(first.plugins()[0].effect_scope.is_accepting());
    assert_eq!(plugin.init_count.load(Ordering::SeqCst), 1);
    assert_eq!(invoke(&first, 1).await.unwrap(), json!({"host": "first"}));
    first.shutdown().await;
}

#[tokio::test]
async fn old_outbound_request_is_cancelled_during_handoff() {
    let (fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let mut blocked = Box::pin(invoke_input(&first, 1, json!({"block_remote": true})));
    tokio::select! {
        result = &mut blocked => panic!("remote call should block: {result:?}"),
        _ = plugin.remote.entered.notified() => {}
    }
    let build = fixture.build("second", Some(first.clone()), Some(&fixture.config));
    let (second, result) = tokio::time::timeout(WAIT, async { tokio::join!(build, blocked) })
        .await
        .unwrap();
    assert!(result.is_err());
    fixture.publish(&second);
    plugin.remote.release.notify_one();
    assert_eq!(invoke(&second, 2).await.unwrap(), json!({"host": "second"}));
    first.shutdown().await;
    second.shutdown().await;
}

#[tokio::test]
async fn old_host_callback_future_is_dropped_during_handoff() {
    let (fixture, _) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let mut blocked = Box::pin(invoke_input(&first, 1, json!({"block_callback": true})));
    tokio::select! {
        result = &mut blocked => panic!("host callback should block: {result:?}"),
        _ = fixture.host_gate.entered.notified() => {}
    }
    let build = fixture.build("second", Some(first.clone()), Some(&fixture.config));
    let (second, result) = tokio::time::timeout(WAIT, async { tokio::join!(build, blocked) })
        .await
        .unwrap();
    assert!(result.is_err());
    assert!(fixture.host_gate.dropped.load(Ordering::SeqCst));
    fixture.publish(&second);
    assert_eq!(invoke(&second, 2).await.unwrap(), json!({"host": "second"}));
    first.shutdown().await;
    second.shutdown().await;
}

fn callback_request() -> Request {
    Request {
        jsonrpc: JsonRpcVersion,
        id: RequestId::Num(1),
        method: method::HOST_CONFIG_READ.into(),
        params: Some(json!({"context": {}})),
        context: None,
    }
}

async fn cancelled_handoff(mode: usize, close: bool) {
    let (fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let owner = first.plugins()[0].effect_scope.clone();
    let transport = first.plugins()[0].transport.clone();
    fixture.control.mode.store(mode, Ordering::SeqCst);
    let mut build =
        Box::pin(fixture.build_result("second", Some(first.clone()), Some(&fixture.config)));
    tokio::select! {
        result = &mut build => panic!("handoff must reach its controlled wait: {}", result.is_ok()),
        _ = fixture.control.gate.entered.notified() => {}
    }
    if close {
        let (closed, result) =
            tokio::time::timeout(WAIT, async { tokio::join!(transport.close(), &mut build) })
                .await
                .unwrap();
        closed.unwrap();
        assert!(result.is_err());
    }
    drop(build);
    assert!(!owner.is_accepting());
    assert!(!transport.can_reuse(&owner).await);
    let next = fixture.control.next.lock().unwrap().clone().unwrap();
    assert_eq!(
        first
            .host_handle()
            .dispatch_callback_rpc(KEY, next.next.token.as_deref(), callback_request())
            .await
            .unwrap_err(),
        super::PluginRuntimeRpcError::InvalidCallbackToken
    );
    assert!(invoke(&first, 1).await.is_err());
    assert_eq!(plugin.init_count.load(Ordering::SeqCst), 1);
    fixture.control.gate.release.notify_one();
    first.shutdown().await;
}

#[tokio::test]
async fn cancelling_handoff_before_remote_update_retires_partial_bindings() {
    cancelled_handoff(2, false).await;
}

#[tokio::test]
async fn cancelling_handoff_after_remote_update_retires_partial_bindings() {
    cancelled_handoff(3, false).await;
}

#[tokio::test]
async fn closing_during_handoff_prevents_successful_publication() {
    cancelled_handoff(3, true).await;
}

#[tokio::test]
async fn remote_handoff_failure_closes_local_admission_and_candidate_credentials() {
    let (fixture, _) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    fixture.control.mode.store(4, Ordering::SeqCst);
    assert!(
        fixture
            .build_result("second", Some(first.clone()), Some(&fixture.config))
            .await
            .is_err()
    );
    let next = fixture.control.next.lock().unwrap().clone().unwrap();
    assert!(!first.plugins()[0].effect_scope.is_accepting());
    assert_eq!(
        first
            .host_handle()
            .dispatch_callback_rpc(KEY, next.next.token.as_deref(), callback_request())
            .await
            .unwrap_err(),
        super::PluginRuntimeRpcError::InvalidCallbackToken
    );
    assert!(invoke(&first, 1).await.is_err());
    first.shutdown().await;
}

#[tokio::test]
async fn stale_owner_cannot_reclaim_a_transport_transferred_to_its_successor() {
    use agena_plugin_host::transport::initialization::PluginInitialization;
    let (fixture, _) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let second = fixture
        .build("second", Some(first.clone()), Some(&fixture.config))
        .await;
    fixture.publish(&second);
    let stale = first.plugins()[0].clone();
    let handle = first.host_handle();
    let candidate = handle.begin_plugin_instance(KEY.parse().unwrap());
    let result = stale
        .transport
        .try_rebind_host(
            handle.clone(),
            candidate.clone(),
            stale.effect_scope.clone(),
            PluginInitialization {
                host: Arc::downgrade(&handle),
                plugin_id: KEY.parse().unwrap(),
                manifest: stale.manifest.clone(),
                agena_version: "test".into(),
                workspace_root: fixture.root.path().into(),
                settings: json!({}),
            },
        )
        .await;
    assert!(result.is_err());
    assert_eq!(invoke(&second, 2).await.unwrap(), json!({"host": "second"}));
    candidate.dispose().await;
    first.shutdown().await;
    second.shutdown().await;
}

#[tokio::test]
async fn handoff_interrupts_an_unread_stream_without_stranding_its_terminal_receiver() {
    let (fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let stream = first
        .invoke_tool_stream(
            &first.registered_tools()[0],
            ToolInvokeInput {
                tool_name: "config".into(),
                session_id: 11,
                call_id: 1,
                workspace_root: "/test/workspace".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(WAIT, async {
        while fixture.bridge.chunks.load(Ordering::SeqCst) < 48 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let second = fixture
        .build("second", Some(first.clone()), Some(&fixture.config))
        .await;
    fixture.publish(&second);
    let terminal = tokio::time::timeout(WAIT, stream.end).await;
    drop(stream.chunks);
    plugin.stream_release.notify_one();
    first.shutdown().await;
    second.shutdown().await;
    assert!(
        terminal
            .expect("handoff error must bypass unread chunks")
            .unwrap()
            .is_err()
    );
}

async fn raw_callback(
    fixture: &Fixture,
    host: &PluginHost,
    method_name: &str,
    mut params: Value,
) -> Value {
    params
        .as_object_mut()
        .unwrap()
        .entry("context")
        .or_insert(json!({}));
    let token = host.host_handle().callback_token(KEY).await.unwrap();
    let response: Response = reqwest::Client::new()
        .post(format!("{}/plugin-rpc/{KEY}", fixture.base))
        .timeout(WAIT)
        .bearer_auth(token)
        .json(&Request {
            jsonrpc: JsonRpcVersion,
            id: RequestId::Num(1),
            method: method_name.into(),
            params: Some(params),
            context: None,
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    match response.payload {
        ResponsePayload::Ok { result } => result,
        ResponsePayload::Err { error } => panic!("resource callback failed: {error:?}"),
    }
}

#[tokio::test]
async fn dynamic_http_contributions_transfer_and_old_cleanup_preserves_them() {
    let (fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    raw_callback(
        &fixture,
        &first,
        method::HOST_TOOL_REGISTRY_REGISTER,
        json!({"request": {"tool": {
            "name": "dynamic", "docs": {"summary": "retained dynamic tool"},
            "contract": {"input_schema": {"type": "object"}}
        }}}),
    )
    .await;
    raw_callback(
        &fixture,
        &first,
        "host/ui.display.contribute",
        json!({"request": {"contribution": {
            "id": "retained", "kind": "status_line_text", "priority": 0,
            "content": {"kind": "text", "text": "retained display"}
        }}}),
    )
    .await;
    raw_callback(
        &fixture,
        &first,
        "host/ui.theme.register",
        json!({"request": {
            "id": "retained", "display_name": "retained theme", "colors": {}
        }}),
    )
    .await;
    let before_display = first.host_handle().display_list_response();
    let before_themes = first.host_handle().theme_list_response().themes;
    let second = fixture
        .build("second", Some(first.clone()), Some(&fixture.config))
        .await;
    fixture.publish(&second);
    first.shutdown().await;
    assert_eq!(second.registered_tools().len(), 2);
    assert!(
        second
            .registered_tools()
            .iter()
            .any(|tool| tool.tool_name() == "dynamic")
    );
    assert_eq!(
        serde_json::to_value(second.host_handle().display_list_response()).unwrap(),
        serde_json::to_value(before_display).unwrap()
    );
    assert_eq!(
        serde_json::to_value(second.host_handle().theme_list_response().themes).unwrap(),
        serde_json::to_value(before_themes).unwrap()
    );
    assert_eq!(plugin.init_count.load(Ordering::SeqCst), 1);
    second.shutdown().await;
    assert!(second.registered_tools().is_empty());
    assert!(second.host_handle().display_list_response().is_empty());
    assert!(second.host_handle().theme_list_response().themes.is_empty());
}

#[tokio::test]
async fn session_scoped_http_tool_overrides_survive_host_handoff() {
    use agena_plugin_host::PluginScopeKey;
    let (fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let mut invoking = Box::pin(invoke_input(&first, 1, json!({"block_remote": true})));
    tokio::select! {
        result = &mut invoking => panic!("invocation must keep its authority alive: {result:?}"),
        _ = plugin.remote.entered.notified() => {}
    }
    let context = plugin.remote_context.lock().unwrap().clone().unwrap();
    raw_callback(
        &fixture,
        &first,
        method::HOST_TOOL_REGISTRY_REGISTER,
        json!({
            "context": context, "request": {"tool": {
                "name": "config", "docs": {"summary": "session-11 override"},
                "contract": {"input_schema": {"type": "object"}}
            }}
        }),
    )
    .await;
    plugin.remote.release.notify_one();
    invoking.await.unwrap();
    let second = fixture
        .build("second", Some(first.clone()), Some(&fixture.config))
        .await;
    fixture.publish(&second);
    first.shutdown().await;
    let summaries = [11, 22].map(|session| {
        second.registered_tools_for_scope(Some(&PluginScopeKey::session(session)))[0]
            .definition
            .docs
            .summary
            .clone()
    });
    assert_eq!(
        summaries,
        [
            Some("session-11 override".into()),
            Some("read owning host configuration".into())
        ]
    );
    assert_eq!(plugin.init_count.load(Ordering::SeqCst), 1);
    second.shutdown().await;
    assert!(
        second
            .registered_tools_for_scope(Some(&PluginScopeKey::session(11)))
            .is_empty()
    );
}

#[tokio::test]
async fn predecessor_cleanup_failure_is_reported_and_prevents_handoff() {
    let (fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    first.plugins()[0]
        .effect_scope
        .own_sync("fixture", "failing-cleanup", || {
            Err("injected predecessor cleanup failure".into())
        })
        .unwrap();
    let result = fixture
        .build_result("second", Some(first.clone()), Some(&fixture.config))
        .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("failed cleanup must not commit handoff"),
    };
    assert!(
        error
            .to_string()
            .contains("injected predecessor cleanup failure")
    );
    assert!(
        fixture.control.next.lock().unwrap().is_none(),
        "failed cleanup must not update the remote destination"
    );
    assert!(!first.plugins()[0].effect_scope.is_accepting());
    assert!(invoke(&first, 1).await.is_err());
    assert_eq!(plugin.init_count.load(Ordering::SeqCst), 1);
    first.shutdown().await;
}

#[tokio::test]
async fn changed_http_settings_keep_the_successor_alive_after_old_host_shutdown() {
    let (mut fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let previous_config = fixture.config.clone();
    fixture.config.list.get_mut(KEY).unwrap().settings = json!({"revision": 2});
    let second = fixture
        .build("second", Some(first.clone()), Some(&previous_config))
        .await;
    fixture.publish(&second);
    assert_eq!(
        plugin.init_count.load(Ordering::SeqCst),
        2,
        "changed settings require initialization"
    );
    assert_eq!(invoke(&second, 2).await.unwrap(), json!({"host": "second"}));
    first.shutdown().await;
    assert_eq!(plugin.shutdowns.load(Ordering::SeqCst), 1);
    assert!(invoke(&first, 9).await.is_err());
    let after = invoke(&second, 3).await;
    second.shutdown().await;
    assert_eq!(plugin.shutdowns.load(Ordering::SeqCst), 2);
    assert_eq!(after.unwrap(), json!({"host": "second"}));
}

#[tokio::test]
async fn delayed_old_http_shutdown_cannot_stop_a_reinitialized_instance() {
    let (mut fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let previous_config = fixture.config.clone();
    fixture.config.list.get_mut(KEY).unwrap().settings = json!({"revision": 2});
    fixture.control.mode.store(5, Ordering::SeqCst);
    let mut shutdown = Box::pin(first.shutdown());
    tokio::select! {
        () = &mut shutdown => panic!("old shutdown must pause before reaching the SDK"),
        _ = fixture.control.gate.entered.notified() => {}
    }
    let second = fixture
        .build("second", Some(first.clone()), Some(&previous_config))
        .await;
    fixture.publish(&second);
    fixture.control.mode.store(0, Ordering::SeqCst);
    fixture.control.gate.release.notify_one();
    tokio::time::timeout(WAIT, shutdown).await.unwrap();
    assert_eq!(plugin.init_count.load(Ordering::SeqCst), 2);
    assert_eq!(plugin.shutdowns.load(Ordering::SeqCst), 1);
    assert_eq!(invoke(&second, 2).await.unwrap(), json!({"host": "second"}));
    second.shutdown().await;
}

async fn rejects_unsupported_instance_control(mode: usize) {
    let (mut fixture, plugin) = Fixture::new().await;
    let first = fixture.build("first", None, None).await;
    fixture.publish(&first);
    let previous_config = fixture.config.clone();
    fixture.config.list.get_mut(KEY).unwrap().settings = json!({"revision": 2});
    fixture.control.mode.store(mode, Ordering::SeqCst);
    let rejected = fixture
        .build("second", Some(first.clone()), Some(&previous_config))
        .await;
    assert!(rejected.plugins().is_empty());
    assert_eq!(plugin.init_count.load(Ordering::SeqCst), 1);
    assert_eq!(plugin.shutdowns.load(Ordering::SeqCst), 0);
    assert_eq!(invoke(&first, 1).await.unwrap(), json!({"host": "first"}));
    rejected.shutdown().await;
    first.shutdown().await;
}

#[tokio::test]
async fn incompatible_http_instance_protocol_does_not_touch_the_live_plugin() {
    rejects_unsupported_instance_control(6).await;
}

#[tokio::test]
async fn missing_http_instance_protocol_does_not_touch_the_live_plugin() {
    rejects_unsupported_instance_control(7).await;
}

#[tokio::test]
async fn mismatched_http_state_response_id_does_not_touch_the_live_plugin() {
    rejects_unsupported_instance_control(8).await;
}
