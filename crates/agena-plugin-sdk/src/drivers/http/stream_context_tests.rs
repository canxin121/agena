use std::sync::Mutex;

use axum::http::HeaderMap;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::*;
use crate::host_api::{HostCallbackContext, NoopHostClient};
use crate::plugin::{InitOutcome, ToolStreamSink};
use crate::{PluginManifest, ToolStreamEnd};

#[derive(Default)]
struct StreamingPlugin {
    host: Mutex<Option<Arc<dyn HostClient>>>,
}

#[async_trait::async_trait]
impl Plugin for StreamingPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("test", "http-context", "1.0.0")
    }

    async fn init(&self, _: InitContext, host: Arc<dyn HostClient>) -> crate::Result<InitOutcome> {
        *self.host.lock().unwrap() = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke_stream(
        &self,
        input: ToolInvokeInput,
        sink: ToolStreamSink,
    ) -> crate::Result<ToolStreamEnd> {
        let host = self.host.lock().unwrap().as_ref().unwrap().clone();
        host.read_config(None).await?;
        sink.text("chunk").await;
        match input.input["outcome"].as_str().unwrap() {
            "success" => Ok(ToolStreamEnd::text(sink.stream_id(), "complete")),
            "error" => Err(PluginError::internal("expected plugin failure")),
            "panic" => panic!("expected plugin stream task panic"),
            other => panic!("unexpected test outcome: {other}"),
        }
    }

    async fn tool_invoke(&self, _: ToolInvokeInput) -> crate::Result<ToolInvokeOutput> {
        let host = self.host.lock().unwrap().as_ref().unwrap().clone();
        host.read_config(None).await?;
        Ok(ToolInvokeOutput::text("complete"))
    }
}

#[derive(Debug)]
struct Callback {
    bearer: Option<String>,
    request: Request,
}

async fn record_callback(
    State(sender): State<mpsc::UnboundedSender<Callback>>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Json<Response> {
    let id = request.id.clone();
    sender
        .send(Callback {
            bearer: headers
                .get("authorization")
                .map(|value| value.to_str().unwrap().to_owned()),
            request,
        })
        .unwrap();
    Json(Response {
        jsonrpc: JsonRpcVersion,
        id,
        payload: ResponsePayload::Ok {
            result: serde_json::json!({}),
        },
    })
}

struct Fixture {
    url: String,
    client: reqwest::Client,
    callbacks: mpsc::UnboundedReceiver<Callback>,
    servers: Vec<JoinHandle<()>>,
}

impl Fixture {
    async fn start() -> Self {
        Self::start_with_ambient(None).await
    }

    async fn start_with_ambient(ambient: Option<HostCallbackContext>) -> Self {
        let (sender, callbacks) = mpsc::unbounded_channel();
        let callback_router = Router::new()
            .route("/callback", post(record_callback))
            .with_state(sender);
        let (callback_url, callback_server) = serve(callback_router).await;
        let mut plugin_router = router(StreamingPlugin::default, Arc::new(NoopHostClient));
        if let Some(ambient) = ambient {
            plugin_router = plugin_router.layer(axum::middleware::from_fn_with_state(
                ambient,
                enclosing_context,
            ));
        }
        let (plugin_url, plugin_server) = serve(plugin_router).await;
        let fixture = Self {
            url: format!("{plugin_url}/rpc"),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
            callbacks,
            servers: vec![callback_server, plugin_server],
        };
        let init = InitContext {
            agena_version: "test".into(),
            workspace_root: "/test/workspace".into(),
            plugin_id: "test.http-context".parse().unwrap(),
            host_callback_url: Some(format!("{callback_url}/callback")),
            host_callback_token: Some("test-bearer".into()),
            settings: serde_json::json!({}),
            protocol_version: crate::rpc::PROTOCOL_VERSION,
        };
        fixture
            .send(method::META_INIT, serde_json::to_value(init).unwrap(), None)
            .await;
        fixture
    }

    async fn send(
        &self,
        method: &str,
        params: serde_json::Value,
        context: Option<HostCallbackContext>,
    ) -> serde_json::Value {
        let response: Response = self
            .client
            .post(&self.url)
            .json(&Request {
                jsonrpc: JsonRpcVersion,
                id: RequestId::Num(1),
                method: method.into(),
                params: Some(params),
                context,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        match response.payload {
            ResponsePayload::Ok { result } => result,
            ResponsePayload::Err { error } => panic!("SDK request failed: {error:?}"),
        }
    }

    async fn invoke(&self, context: HostCallbackContext, outcome: &str) -> String {
        let input = ToolInvokeInput {
            tool_name: context.tool_name.clone().unwrap(),
            session_id: context.session_id.unwrap(),
            call_id: context.call_id.unwrap(),
            workspace_root: context.workspace_root.clone().unwrap(),
            input: serde_json::json!({"outcome": outcome}),
        };
        let result = self
            .send(
                method::HOOK_TOOL_INVOKE_STREAM,
                serde_json::to_value(input).unwrap(),
                Some(context),
            )
            .await;
        serde_json::from_value::<ToolInvokeStreamHandle>(result)
            .unwrap()
            .stream_id
    }

    async fn receive(&mut self) -> Callback {
        tokio::time::timeout(Duration::from_secs(3), self.callbacks.recv())
            .await
            .expect("SDK must send a callback promptly")
            .expect("callback server remains open")
    }
}

async fn enclosing_context(
    State(context): State<HostCallbackContext>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    crate::host_api::run_in_host_callback_context(context, next.run(request)).await
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
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, task)
}

fn context(call_id: i64) -> HostCallbackContext {
    HostCallbackContext {
        plugin_id: Some("test.http-context".into()),
        session_id: Some(call_id + 10),
        call_id: Some(call_id),
        workspace_root: Some(format!("/test/workspace-{call_id}")),
        tool_name: Some("stream".into()),
        authority_token: Some(format!("test-authority-{call_id}")),
    }
}

fn assert_context(callback: &Callback, expected: &HostCallbackContext) {
    assert_eq!(callback.bearer.as_deref(), Some("Bearer test-bearer"));
    let actual: HostCallbackContext =
        serde_json::from_value(callback.request.params.as_ref().unwrap()["context"].clone())
            .unwrap();
    assert_eq!(
        &actual, expected,
        "{} must echo the full request authority",
        callback.request.method
    );
}

async fn stream_echoes_authority(outcome: &str, terminal: &str) {
    let mut fixture = Fixture::start().await;
    let expected = context(1);
    let stream_id = fixture.invoke(expected.clone(), outcome).await;
    let mut callbacks = Vec::new();
    for _ in 0..3 {
        callbacks.push(fixture.receive().await);
    }
    for (callback, method) in callbacks.iter().zip([
        method::HOST_CONFIG_READ,
        method::TOOL_STREAM_CHUNK,
        terminal,
    ]) {
        assert_eq!(callback.request.method, method);
        assert_context(callback, &expected);
        if method != method::HOST_CONFIG_READ {
            assert_eq!(
                callback.request.params.as_ref().unwrap()["stream_id"],
                stream_id
            );
        }
    }
}

#[tokio::test]
async fn successful_stream_callbacks_echo_request_authority() {
    stream_echoes_authority("success", method::TOOL_STREAM_END).await;
}

#[tokio::test]
async fn failed_stream_callbacks_echo_request_authority() {
    stream_echoes_authority("error", method::TOOL_STREAM_ERROR).await;
}

#[tokio::test]
async fn panicked_stream_callbacks_echo_request_authority() {
    stream_echoes_authority("panic", method::TOOL_STREAM_ERROR).await;
}

#[tokio::test]
async fn concurrent_streams_keep_distinct_request_authorities() {
    let mut fixture = Fixture::start().await;
    let first = context(1);
    let second = context(2);
    let (first_id, second_id) = tokio::join!(
        fixture.invoke(first.clone(), "success"),
        fixture.invoke(second.clone(), "error")
    );
    assert_ne!(first_id, second_id);
    let mut first_methods = Vec::new();
    let mut second_methods = Vec::new();
    let mut callbacks = Vec::new();
    for _ in 0..6 {
        callbacks.push(fixture.receive().await);
    }
    for callback in callbacks {
        let params = callback.request.params.as_ref().unwrap();
        if params["context"]["call_id"] == 1 {
            assert_context(&callback, &first);
            if callback.request.method != method::HOST_CONFIG_READ {
                assert_eq!(params["stream_id"], first_id);
            }
            first_methods.push(callback.request.method);
        } else {
            assert_context(&callback, &second);
            if callback.request.method != method::HOST_CONFIG_READ {
                assert_eq!(params["stream_id"], second_id);
            }
            second_methods.push(callback.request.method);
        }
    }
    assert_eq!(
        first_methods,
        [
            method::HOST_CONFIG_READ,
            method::TOOL_STREAM_CHUNK,
            method::TOOL_STREAM_END
        ]
    );
    assert_eq!(
        second_methods,
        [
            method::HOST_CONFIG_READ,
            method::TOOL_STREAM_CHUNK,
            method::TOOL_STREAM_ERROR
        ]
    );
}

async fn missing_request_context_is_isolated(streaming: bool) {
    let mut fixture = Fixture::start_with_ambient(Some(context(999))).await;
    let expected = HostCallbackContext {
        plugin_id: None,
        authority_token: None,
        ..context(1)
    };
    let input = ToolInvokeInput {
        tool_name: expected.tool_name.clone().unwrap(),
        session_id: expected.session_id.unwrap(),
        call_id: expected.call_id.unwrap(),
        workspace_root: expected.workspace_root.clone().unwrap(),
        input: serde_json::json!({"outcome": "success"}),
    };
    let method = if streaming {
        method::HOOK_TOOL_INVOKE_STREAM
    } else {
        method::HOOK_TOOL_INVOKE
    };
    fixture
        .send(method, serde_json::to_value(input).unwrap(), None)
        .await;
    let count = if streaming { 3 } else { 1 };
    let mut callbacks = Vec::new();
    for _ in 0..count {
        callbacks.push(fixture.receive().await);
    }
    for callback in callbacks {
        assert_context(&callback, &expected);
    }
}

#[tokio::test]
async fn stream_without_context_does_not_borrow_middleware_authority() {
    missing_request_context_is_isolated(true).await;
}

#[tokio::test]
async fn request_without_context_does_not_borrow_middleware_authority() {
    missing_request_context_is_isolated(false).await;
}
