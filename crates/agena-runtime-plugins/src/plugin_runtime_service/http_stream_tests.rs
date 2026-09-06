use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use agena_plugin_host::config::{ConfiguredPlugin, HttpAuth, PluginPackage, PluginsConfig};
use agena_plugin_host::host::{PluginHost, PluginHostBuildConfig, ToolInvokeStream};
use agena_plugin_sdk::host_api::{HostCallbackContext, HostClient, NoopHostClient};
use agena_plugin_sdk::rpc::{
    ErrorObject, JsonRpcVersion, Request, RequestId, Response, ResponsePayload, codes, method,
};
use agena_plugin_sdk::{
    InitContext, InitOutcome, Plugin, PluginError, PluginManifest, ToolInvokeInput, ToolStreamEnd,
    ToolStreamSink,
};
use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

const KEY: &str = "test.http-ingress";
const WAIT: Duration = Duration::from_secs(5);

struct ConfigHost;

#[async_trait::async_trait]
impl HostClient for ConfigHost {
    async fn log(&self, _: agena_plugin_sdk::host_api::LogLevel, _: String, _: Value) {}
    async fn publish_event(
        &self,
        event: agena_plugin_sdk::EventEnvelope,
    ) -> agena_plugin_sdk::Result<()> {
        NoopHostClient.publish_event(event).await
    }
    async fn subscribe_events(
        &self,
        filter: agena_plugin_sdk::EventFilter,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::host_api::EventSubscription> {
        NoopHostClient.subscribe_events(filter).await
    }
    async fn read_config(&self, _: Option<String>) -> agena_plugin_sdk::Result<Value> {
        Ok(json!({"fixture": true}))
    }
    async fn invoke_tool(
        &self,
        tool: String,
        input: Value,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::ToolInvokeOutput> {
        NoopHostClient.invoke_tool(tool, input).await
    }
}

enum Action {
    Chunks(usize),
    Finish,
    Fail,
    Panic,
}

struct Started {
    context: HostCallbackContext,
    stream_id: String,
    actions: mpsc::UnboundedSender<Action>,
}

struct ControlledPlugin {
    host: Mutex<Option<Arc<dyn HostClient>>>,
    started: mpsc::UnboundedSender<Started>,
}

#[async_trait::async_trait]
impl Plugin for ControlledPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("test", "http-ingress", "1.0.0");
        manifest.tools.push(
            serde_json::from_value(json!({
                "name": "stream", "docs": {"summary": "controlled stream"},
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
        *self.host.lock().unwrap() = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke_stream(
        &self,
        _: ToolInvokeInput,
        sink: ToolStreamSink,
    ) -> agena_plugin_sdk::Result<ToolStreamEnd> {
        let host = self.host.lock().unwrap().as_ref().unwrap().clone();
        host.read_config(None).await?;
        let (actions, mut commands) = mpsc::unbounded_channel();
        self.started
            .send(Started {
                context: agena_plugin_sdk::host_api::current_host_callback_context().unwrap(),
                stream_id: sink.stream_id().to_string(),
                actions,
            })
            .unwrap();
        while let Some(action) = commands.recv().await {
            match action {
                Action::Chunks(count) => {
                    for index in 0..count {
                        sink.text(format!("chunk-{index}")).await;
                    }
                }
                Action::Finish => return Ok(ToolStreamEnd::text(sink.stream_id(), "complete")),
                Action::Fail => return Err(PluginError::internal("controlled plugin failure")),
                Action::Panic => panic!("controlled plugin stream task panic"),
            }
        }
        Err(PluginError::internal("test controller dropped"))
    }
}

struct CallbackRecord {
    method: String,
    succeeded: bool,
}

struct BridgeState {
    host: OnceLock<Weak<PluginHost>>,
    records: mpsc::UnboundedSender<CallbackRecord>,
}

async fn callback(
    State(state): State<Arc<BridgeState>>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Json<Response> {
    let id = request.id.clone();
    let method = request.method.clone();
    let token = headers
        .get("authorization")
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))
        .map(str::to_owned);
    let host = state.host.get().unwrap().upgrade().unwrap();
    let response = super::dispatch_plugin_rpc(host, KEY, token, request).await;
    // This fixture exposes the real bridge over HTTP. Presentation-level
    // ServerError envelopes are outside this test; preserve bridge failures
    // as a failed JSON-RPC response for the SDK's callback client.
    let response = response.unwrap_or_else(|error| Response {
        jsonrpc: JsonRpcVersion,
        id,
        payload: ResponsePayload::Err {
            error: ErrorObject {
                code: codes::PLUGIN_GENERIC,
                message: error.to_string(),
                data: None,
            },
        },
    });
    let _ = state.records.send(CallbackRecord {
        method,
        succeeded: matches!(response.payload, ResponsePayload::Ok { .. }),
    });
    Json(response)
}

async fn delay_stream_response(
    State(gate): State<Arc<Notify>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
    let envelope: Request = serde_json::from_slice(&bytes).unwrap();
    let response = next
        .run(axum::extract::Request::from_parts(parts, bytes.into()))
        .await;
    if envelope.method == method::HOOK_TOOL_INVOKE_STREAM {
        gate.notified().await;
    }
    response
}

struct Fixture {
    host: Arc<PluginHost>,
    callback_url: String,
    token: String,
    client: reqwest::Client,
    started: mpsc::UnboundedReceiver<Started>,
    records: mpsc::UnboundedReceiver<CallbackRecord>,
    response_gate: Arc<Notify>,
    servers: Vec<JoinHandle<()>>,
    _root: tempfile::TempDir,
}

impl Fixture {
    async fn new(delay_response: bool) -> Self {
        let (records_tx, records) = mpsc::unbounded_channel();
        let bridge = Arc::new(BridgeState {
            host: OnceLock::new(),
            records: records_tx,
        });
        let (base, callback_server) = serve(
            Router::new()
                .route("/plugin-rpc/{plugin_id}", post(callback))
                .with_state(bridge.clone()),
        )
        .await;
        let (started_tx, started) = mpsc::unbounded_channel();
        let mut router = agena_plugin_sdk::drivers::http::router(
            move || ControlledPlugin {
                host: Mutex::new(None),
                started: started_tx.clone(),
            },
            Arc::new(NoopHostClient),
        );
        let response_gate = Arc::new(Notify::new());
        if delay_response {
            router = router.layer(axum::middleware::from_fn_with_state(
                response_gate.clone(),
                delay_stream_response,
            ));
        }
        let (plugin_url, plugin_server) = serve(router).await;
        let mut config = PluginsConfig::default();
        config.list.insert(
            KEY.into(),
            ConfiguredPlugin {
                package: PluginPackage::Http {
                    url: format!("{plugin_url}/rpc").parse().unwrap(),
                    auth: HttpAuth::None,
                },
                ..Default::default()
            },
        );
        let root = tempfile::tempdir().unwrap();
        let host = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![],
            config,
            workspace_root: root.path().into(),
            agena_version: "test".into(),
            callback_base_url: Some(base.clone()),
            host_client: Some(Arc::new(ConfigHost)),
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .unwrap();
        assert_eq!(host.registered_tools().len(), 1);
        bridge.host.set(Arc::downgrade(&host)).unwrap();
        let token = host.host_handle().callback_token(KEY).await.unwrap();
        Self {
            host,
            callback_url: format!("{base}/plugin-rpc/{KEY}"),
            token,
            client: reqwest::Client::builder().timeout(WAIT).build().unwrap(),
            started,
            records,
            response_gate,
            servers: vec![callback_server, plugin_server],
            _root: root,
        }
    }

    fn invoke(&self, call_id: i64) -> JoinHandle<ToolInvokeStream> {
        let host = self.host.clone();
        let tool = host.registered_tools()[0].clone();
        tokio::spawn(async move {
            host.invoke_tool_stream(
                &tool,
                ToolInvokeInput {
                    tool_name: "stream".into(),
                    session_id: call_id + 10,
                    call_id,
                    workspace_root: format!("/test/workspace-{call_id}"),
                    input: json!({}),
                },
            )
            .await
            .unwrap()
        })
    }

    async fn start(&mut self, call_id: i64) -> (ToolInvokeStream, Started) {
        let invocation = self.invoke(call_id);
        let stream = tokio::time::timeout(WAIT, invocation)
            .await
            .unwrap()
            .unwrap();
        let started = self.next_start().await;
        assert_eq!(stream.stream_id, started.stream_id);
        assert_eq!(started.context.call_id, Some(call_id));
        (stream, started)
    }

    async fn next_start(&mut self) -> Started {
        tokio::time::timeout(WAIT, self.started.recv())
            .await
            .unwrap()
            .unwrap()
    }

    async fn send(&self, method: &str, params: Value) -> Response {
        self.client
            .post(&self.callback_url)
            .bearer_auth(&self.token)
            .json(&Request {
                jsonrpc: JsonRpcVersion,
                id: RequestId::Num(100),
                method: method.into(),
                params: Some(params),
                context: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn terminal_callback(&mut self) -> bool {
        self.callback_until(&[method::TOOL_STREAM_END, method::TOOL_STREAM_ERROR])
            .await
    }

    async fn callback_until(&mut self, methods: &[&str]) -> bool {
        tokio::time::timeout(WAIT, async {
            let mut succeeded = true;
            loop {
                let record = self.records.recv().await.unwrap();
                succeeded &= record.succeeded;
                if methods.contains(&record.method.as_str()) {
                    return succeeded;
                }
            }
        })
        .await
        .unwrap()
    }

    async fn finish(&self) {
        tokio::time::timeout(WAIT, self.host.shutdown())
            .await
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.response_gate.notify_waiters();
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

fn event(method: &str, stream_id: &str, context: Value) -> Value {
    let mut params = match method {
        method::TOOL_STREAM_CHUNK => json!({"stream_id": stream_id, "text_delta": "injected"}),
        method::TOOL_STREAM_END => {
            serde_json::to_value(ToolStreamEnd::text(stream_id, "injected")).unwrap()
        }
        method::TOOL_STREAM_ERROR => {
            json!({"stream_id": stream_id, "error": PluginError::internal("injected")})
        }
        _ => unreachable!(),
    };
    params["context"] = context;
    params
}

async fn collect(
    mut stream: ToolInvokeStream,
) -> (Vec<String>, Result<ToolStreamEnd, PluginError>) {
    tokio::time::timeout(WAIT, async {
        let mut chunks = Vec::new();
        while let Some(chunk) = stream.chunks.recv().await {
            chunks.push(chunk.text_delta.unwrap());
        }
        (chunks, stream.end.await.unwrap())
    })
    .await
    .unwrap()
}

fn denied(response: &Response) -> bool {
    matches!(response.payload, ResponsePayload::Err { .. })
}

#[tokio::test]
async fn stream_ingress_rejects_missing_malformed_and_forged_contexts() {
    let mut fixture = Fixture::new(false).await;
    let (stream, started) = fixture.start(1).await;
    let mut forged = serde_json::to_value(&started.context).unwrap();
    forged["authority_token"] = json!("ctx-not-issued");
    let mut altered = serde_json::to_value(&started.context).unwrap();
    altered["session_id"] = json!(999);
    let contexts = [
        json!({}),
        json!([]),
        Value::Null,
        json!("bad"),
        json!({"call_id": "bad"}),
        json!({"session_id": 11}),
        forged,
        altered,
    ];
    let mut responses = Vec::new();
    for method in [
        method::TOOL_STREAM_CHUNK,
        method::TOOL_STREAM_END,
        method::TOOL_STREAM_ERROR,
    ] {
        for context in &contexts {
            responses.push(
                fixture
                    .send(method, event(method, &stream.stream_id, context.clone()))
                    .await,
            );
        }
        let mut missing = event(method, &stream.stream_id, json!({}));
        missing.as_object_mut().unwrap().remove("context");
        responses.push(fixture.send(method, missing).await);
    }
    started.actions.send(Action::Chunks(1)).unwrap();
    started.actions.send(Action::Finish).unwrap();
    let (chunks, end) = collect(stream).await;
    fixture.finish().await;
    assert!(
        responses.iter().all(denied),
        "untrusted stream callbacks were accepted: {responses:?}"
    );
    assert_eq!(chunks, ["chunk-0"]);
    assert_eq!(end.unwrap().output_text, "complete");
}

#[tokio::test]
async fn live_authority_cannot_write_or_finish_another_stream() {
    let mut fixture = Fixture::new(false).await;
    let (first, first_call) = fixture.start(1).await;
    let (second, second_call) = fixture.start(2).await;
    let mut responses = Vec::new();
    for method in [
        method::TOOL_STREAM_CHUNK,
        method::TOOL_STREAM_END,
        method::TOOL_STREAM_ERROR,
    ] {
        responses.push(
            fixture
                .send(
                    method,
                    event(
                        method,
                        &second.stream_id,
                        serde_json::to_value(&first_call.context).unwrap(),
                    ),
                )
                .await,
        );
    }
    for call in [&first_call, &second_call] {
        call.actions.send(Action::Chunks(1)).unwrap();
        call.actions.send(Action::Finish).unwrap();
    }
    let (first_result, second_result) = tokio::join!(collect(first), collect(second));
    fixture.finish().await;
    assert!(
        responses.iter().all(denied),
        "another live call's authority was accepted: {responses:?}"
    );
    for (chunks, end) in [first_result, second_result] {
        assert_eq!(chunks, ["chunk-0"]);
        assert_eq!(end.unwrap().output_text, "complete");
    }
}

#[tokio::test]
async fn completed_call_authority_cannot_be_replayed() {
    let mut fixture = Fixture::new(false).await;
    let (stream, started) = fixture.start(1).await;
    started.actions.send(Action::Finish).unwrap();
    assert!(collect(stream).await.1.is_ok());
    let mut responses = Vec::new();
    for method in [
        method::TOOL_STREAM_CHUNK,
        method::TOOL_STREAM_END,
        method::TOOL_STREAM_ERROR,
    ] {
        responses.push(
            fixture
                .send(
                    method,
                    event(
                        method,
                        &started.stream_id,
                        serde_json::to_value(&started.context).unwrap(),
                    ),
                )
                .await,
        );
    }
    fixture.finish().await;
    assert!(
        responses.iter().all(denied),
        "expired stream authority was accepted: {responses:?}"
    );
}

async fn early_stream(count: usize) -> (Vec<String>, Result<ToolStreamEnd, PluginError>) {
    let mut fixture = Fixture::new(true).await;
    let invocation = fixture.invoke(1);
    let started = fixture.next_start().await;
    started.actions.send(Action::Chunks(count)).unwrap();
    started.actions.send(Action::Finish).unwrap();
    let callbacks_succeeded = fixture.terminal_callback().await;
    assert!(
        !invocation.is_finished(),
        "initial stream response is still held by HTTP middleware"
    );
    fixture.response_gate.notify_one();
    let stream = tokio::time::timeout(WAIT, invocation)
        .await
        .unwrap()
        .unwrap();
    let result = collect(stream).await;
    fixture.finish().await;
    assert!(
        callbacks_succeeded,
        "SDK callbacks must be accepted before the initial response"
    );
    result
}

#[tokio::test]
async fn sixty_four_early_chunks_survive_the_terminal_event() {
    let (chunks, end) = early_stream(64).await;
    assert_eq!(
        chunks,
        (0..64)
            .map(|index| format!("chunk-{index}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(end.unwrap().output_text, "complete");
}

#[tokio::test]
async fn early_overflow_cannot_be_replaced_by_success() {
    let (_, end) = early_stream(128).await;
    let error = end.expect_err("early buffer overflow must remain an error");
    assert!(error.diagnostic_message().contains("64-chunk"));
}

#[tokio::test]
async fn cancelled_http_invocations_release_pending_stream_capacity() {
    let mut fixture = Fixture::new(true).await;
    for call_id in 0..128 {
        let invocation = fixture.invoke(call_id);
        let started = fixture.next_start().await;
        started.actions.send(Action::Chunks(1)).unwrap();
        assert!(fixture.callback_until(&[method::TOOL_STREAM_CHUNK]).await);
        invocation.abort();
        match invocation.await {
            Err(error) => assert!(error.is_cancelled()),
            Ok(_) => panic!("the HTTP invocation must remain pending until cancelled"),
        }
        started.actions.send(Action::Finish).unwrap();
        assert!(
            !fixture.terminal_callback().await,
            "cancelled authority must be revoked"
        );
        fixture.response_gate.notify_waiters();
    }
    let invocation = fixture.invoke(128);
    let started = fixture.next_start().await;
    started.actions.send(Action::Chunks(1)).unwrap();
    started.actions.send(Action::Finish).unwrap();
    assert!(fixture.terminal_callback().await);
    fixture.response_gate.notify_one();
    let stream = tokio::time::timeout(WAIT, invocation)
        .await
        .unwrap()
        .unwrap();
    let (chunks, end) = collect(stream).await;
    fixture.finish().await;
    assert_eq!(chunks, ["chunk-0"]);
    assert_eq!(end.unwrap().output_text, "complete");
}

async fn terminal_failure(action: Action) -> PluginError {
    let mut fixture = Fixture::new(false).await;
    let (stream, started) = fixture.start(1).await;
    started.actions.send(Action::Chunks(1)).unwrap();
    started.actions.send(action).unwrap();
    let (chunks, end) = collect(stream).await;
    fixture.finish().await;
    assert_eq!(chunks, ["chunk-0"]);
    end.expect_err("the plugin must report failure")
}

#[tokio::test]
async fn authorized_plugin_error_reaches_the_stream_consumer() {
    let error = terminal_failure(Action::Fail).await;
    assert!(
        error
            .diagnostic_message()
            .contains("controlled plugin failure")
    );
}

#[tokio::test]
async fn authorized_plugin_task_panic_reaches_the_stream_consumer() {
    let error = terminal_failure(Action::Panic).await;
    assert!(
        error
            .diagnostic_message()
            .contains("before sending its final frame")
    );
}
