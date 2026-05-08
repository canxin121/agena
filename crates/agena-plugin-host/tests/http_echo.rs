//! HTTP transport end-to-end. Spins up the SDK's http driver on
//! `127.0.0.1:0`, then drives it via the host's `HttpTransport`.

use std::collections::BTreeMap;
use std::sync::Arc;

use agena_plugin_host::transport::{PluginTransport, http::HttpTransport};
use agena_plugin_host::{PluginEntry, PluginHost, PluginHostBuilder, PluginsConfig};
use agena_plugin_sdk::host_api::NoopHostClient;
use agena_plugin_sdk::prelude::*;
use agena_plugin_sdk::rpc::{
    ErrorObject, JsonRpcVersion, Request, Response, ResponsePayload, codes, method,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, header::AUTHORIZATION},
    routing::post,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock};

#[derive(Default)]
struct EchoHttpPlugin;

#[derive(Default)]
struct CallbackServerState {
    host: RwLock<Option<Arc<PluginHost>>>,
    expected_token: RwLock<Option<String>>,
    observations: Mutex<CallbackObservations>,
}

#[derive(Clone, Debug, Default)]
struct CallbackObservations {
    methods: Vec<String>,
    saw_context: bool,
    saw_expected_auth: bool,
    saw_expected_entry_name: bool,
}

#[async_trait]
impl Plugin for EchoHttpPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("echo-http", "0.0.1")
            .hooks(
                HookSubscription::TOOL_INVOKE
                    | HookSubscription::TOOL_INVOKE_STREAM
                    | HookSubscription::SHELL_ENV,
            )
            .entry(
                PluginEntryDecl::new(
                    "echo",
                    json!({"type":"object","properties":{"text":{"type":"string"}}}),
                )
                .description("Echo via HTTP.")
                .streaming(EntryStreamingMode::Streaming),
            )
            .build()
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        let text = input
            .input
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(ToolInvokeOutput::text(format!("http-echo: {text}")))
    }

    async fn tool_invoke_stream(
        &self,
        input: ToolInvokeInput,
        sink: ToolStreamSink,
    ) -> Result<ToolStreamEnd> {
        let text = input
            .input
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        sink.text("http-").await;
        sink.text(format!("echo: {text}")).await;
        Ok(ToolStreamEnd {
            stream_id: sink.stream_id().to_string(),
            title: String::new(),
            output_text: format!("http-echo: {text}"),
            payload: None,
            metadata: Default::default(),
            attachments: Vec::new(),
        })
    }

    async fn shell_env(&self, _: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        Ok(Some(ShellEnvPatch::set("AGENA_HTTP_PLUGIN", "1")))
    }
}

async fn spawn_test_server() -> String {
    let host: Arc<dyn HostClient> = Arc::new(NoopHostClient);
    let app = agena_plugin_sdk::drivers::http::router(EchoHttpPlugin, host);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    format!("http://{local}/rpc")
}

async fn spawn_callback_server(state: Arc<CallbackServerState>) -> String {
    let app = Router::new()
        .route("/plugin-rpc/{plugin_id}", post(handle_callback))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    format!("http://{local}")
}

async fn handle_callback(
    State(state): State<Arc<CallbackServerState>>,
    Path(plugin_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<Request>,
) -> Json<Response> {
    let id = req.id.clone();
    let params = req.params.unwrap_or(Value::Null);
    let expected_token = state.expected_token.read().await.clone();
    let got_token = bearer_token(&headers);
    let context = params.get("context");
    let entry_name = context
        .and_then(|value| value.get("entry_name"))
        .and_then(Value::as_str);

    {
        let mut observations = state.observations.lock().await;
        observations.methods.push(req.method.clone());
        observations.saw_context |= context.is_some();
        observations.saw_expected_auth |= expected_token.as_deref() == got_token.as_deref();
        observations.saw_expected_entry_name |= entry_name == Some("echo");
    }

    if expected_token.as_deref() != got_token.as_deref() {
        return Json(error_response(id, "invalid callback bearer token"));
    }
    if context.is_none() {
        return Json(error_response(id, "missing callback context"));
    }

    let host = state
        .host
        .read()
        .await
        .clone()
        .expect("host should be installed");
    match host
        .host_handle()
        .ingest_stream_event_for_plugin(&plugin_id, &req.method, params)
        .await
    {
        Ok(true) => Json(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok {
                result: Value::Object(Default::default()),
            },
        }),
        Ok(false) => Json(error_response(id, "unexpected callback method")),
        Err(err) => Json(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::PLUGIN_GENERIC,
                    message: err.message.clone(),
                    data: serde_json::to_value(&err).ok(),
                },
            },
        }),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let mut parts = raw.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") || parts.next().is_some() {
        return None;
    }
    Some(token.to_string())
}

fn error_response(id: agena_plugin_sdk::rpc::RequestId, message: &str) -> Response {
    Response {
        jsonrpc: JsonRpcVersion,
        id,
        payload: ResponsePayload::Err {
            error: ErrorObject {
                code: codes::PLUGIN_GENERIC,
                message: message.to_string(),
                data: None,
            },
        },
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn http_transport_round_trip_via_low_level_dispatch() {
    let url = spawn_test_server().await;
    let transport = HttpTransport::new(
        url.parse().unwrap(),
        agena_plugin_host::HttpAuth::None,
        &|_| None,
        false,
    );

    // meta/init
    let init = transport
        .dispatch(
            agena_plugin_sdk::rpc::method::META_INIT,
            json!({
                "agena_version": "test",
                "workspace_root": ".",
                "plugin_id": "echo-http",
                "options": {},
                "protocol_version": 1
            }),
        )
        .await
        .expect("init ok");
    assert!(init.get("manifest").is_some());

    // tool.invoke
    let out = transport
        .dispatch(
            agena_plugin_sdk::rpc::method::HOOK_TOOL_INVOKE,
            json!({
                "tool_name": "echo",
                "session_id": 1,
                "call_id": 1,
                "workspace_root": ".",
                "input": { "text": "hi" }
            }),
        )
        .await
        .expect("invoke ok");
    assert_eq!(
        out.get("output_text").and_then(|v| v.as_str()),
        Some("http-echo: hi")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn http_transport_round_trip_via_plugin_host() {
    let url = spawn_test_server().await;

    let mut list = BTreeMap::new();
    list.insert(
        "echo-http".to_string(),
        PluginEntry::Http {
            url: url.parse().unwrap(),
            auth: agena_plugin_host::HttpAuth::None,
            options: serde_json::Value::Null,
            timeouts: Default::default(),
        },
    );
    let host = PluginHostBuilder::new(std::env::current_dir().unwrap(), "test")
        .with_config(PluginsConfig {
            enabled: true,
            timeouts: Default::default(),
            list,
            trusted_keys: Default::default(),
            default_quota: Default::default(),
            quotas: Default::default(),
        })
        .build()
        .await
        .expect("plugin host should build");

    assert_eq!(host.plugins().len(), 1);
    let resolved = host.lookup_entry("echo").expect("tool exposed");
    let out = host
        .invoke_tool(
            &resolved.handle,
            ToolInvokeInput {
                tool_name: "echo".into(),
                session_id: 7,
                call_id: 11,
                workspace_root: ".".into(),
                input: json!({ "text": "world" }),
            },
        )
        .expect("invoke");
    assert_eq!(out.output_text, "http-echo: world");
}

#[tokio::test(flavor = "multi_thread")]
async fn http_transport_streams_via_callbacks() {
    let url = spawn_test_server().await;
    let callback_state = Arc::new(CallbackServerState::default());
    let callback_base_url = spawn_callback_server(callback_state.clone()).await;

    let mut list = BTreeMap::new();
    list.insert(
        "echo-http".to_string(),
        PluginEntry::Http {
            url: url.parse().unwrap(),
            auth: agena_plugin_host::HttpAuth::None,
            options: serde_json::Value::Null,
            timeouts: Default::default(),
        },
    );
    let host = PluginHostBuilder::new(std::env::current_dir().unwrap(), "test")
        .with_callback_base_url(callback_base_url)
        .with_config(PluginsConfig {
            enabled: true,
            timeouts: Default::default(),
            list,
            trusted_keys: Default::default(),
            default_quota: Default::default(),
            quotas: Default::default(),
        })
        .build()
        .await
        .expect("plugin host should build");

    let token = host
        .host_handle()
        .callback_token("echo-http")
        .await
        .expect("callback token should exist");
    *callback_state.host.write().await = Some(host.clone());
    *callback_state.expected_token.write().await = Some(token);

    let resolved = host.lookup_entry("echo").expect("tool exposed");
    let mut stream = host
        .invoke_tool_stream(
            &resolved.handle,
            ToolInvokeInput {
                tool_name: "echo".into(),
                session_id: 9,
                call_id: 13,
                workspace_root: ".".into(),
                input: json!({ "text": "stream" }),
            },
        )
        .await
        .expect("stream invoke");

    let stream_id = stream.stream_id.clone();
    assert!(
        !stream_id.starts_with("emu-"),
        "expected native HTTP streaming, got emulated stream id {stream_id}"
    );

    let mut deltas = Vec::new();
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), stream.chunks.recv()).await {
            Ok(Some(chunk)) => {
                if let Some(delta) = chunk.text_delta {
                    deltas.push(delta);
                }
            }
            Ok(None) => break,
            Err(_) => {
                let observations = callback_state.observations.lock().await.clone();
                panic!("timed out waiting for stream chunks to close: {observations:?}");
            }
        }
    }
    assert_eq!(
        deltas,
        vec!["http-".to_string(), "echo: stream".to_string()]
    );

    let end = tokio::time::timeout(std::time::Duration::from_secs(5), stream.end)
        .await
        .expect("timed out waiting for stream end")
        .expect("stream end channel")
        .expect("stream result");
    assert_eq!(end.stream_id, stream_id);
    assert_eq!(end.output_text, "http-echo: stream");

    let observations = callback_state.observations.lock().await.clone();
    assert!(
        observations.saw_context,
        "expected callback context in stream events"
    );
    assert!(
        observations.saw_expected_auth,
        "expected callback bearer token on stream events"
    );
    assert!(
        observations.saw_expected_entry_name,
        "expected callback context to preserve entry name"
    );
    assert!(
        observations
            .methods
            .iter()
            .any(|name| name == method::TOOL_STREAM_CHUNK),
        "expected at least one stream chunk callback"
    );
    assert!(
        observations
            .methods
            .iter()
            .any(|name| name == method::TOOL_STREAM_END),
        "expected a terminal stream end callback"
    );
}
