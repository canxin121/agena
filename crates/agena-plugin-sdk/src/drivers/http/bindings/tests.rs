//! Wire-level lifecycle contracts use real loopback HTTP in both directions.

use super::super::{EXPECTED_REVISION_HEADER, INSTANCE_HEADER, router as serve_http};
use super::*;
use crate::host_api::{HostCallbackContext, HostClient};
use crate::rpc::{JsonRpcVersion, Request, RequestId, Response, ResponsePayload, method};
use crate::{
    InitContext, InitOutcome, Plugin, ToolInvokeInput, ToolInvokeOutput, ToolStreamEnd,
    ToolStreamSink,
};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc};

mod lifecycle;
mod streaming;

const WAIT: Duration = Duration::from_secs(5);

struct Server {
    url: String,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Server {
    async fn start(router: Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = CancellationToken::new();
        let shutdown = stop.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
                .unwrap();
        });
        Self {
            url,
            stop,
            task: Some(task),
        }
    }

    async fn finish(&mut self) {
        self.stop.cancel();
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(WAIT, &mut task).await {
                Ok(result) => result.unwrap(),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    panic!("HTTP protocol fixture did not drain");
                }
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

struct Peer {
    server: Server,
    binding: HttpCallbackBinding,
    requests: Arc<Mutex<Vec<Request>>>,
    changed: Arc<tokio::sync::Notify>,
}

impl Peer {
    async fn start(marker: &'static str) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let changed = Arc::new(tokio::sync::Notify::new());
        let records = requests.clone();
        let events = changed.clone();
        let token = format!("{marker}-callback-token");
        let expected_token = token.clone();
        let server = Server::start(Router::new().route(
            "/callback",
            post(move |headers: HeaderMap, Json(request): Json<Request>| {
                let records = records.clone();
                let events = events.clone();
                let expected_token = expected_token.clone();
                async move {
                    assert_eq!(headers["authorization"], format!("Bearer {expected_token}"));
                    let response = Response {
                        jsonrpc: JsonRpcVersion,
                        id: request.id.clone(),
                        payload: ResponsePayload::Ok {
                            result: if request.method == method::HOST_CONFIG_READ {
                                json!({"marker":marker})
                            } else {
                                Value::Null
                            },
                        },
                    };
                    records.lock().unwrap().push(request);
                    events.notify_waiters();
                    Json(response)
                }
            }),
        ))
        .await;
        let binding = HttpCallbackBinding {
            url: Some(format!("{}/callback", server.url)),
            token: Some(token),
        };
        Self {
            server,
            binding,
            requests,
            changed,
        }
    }

    async fn wait_method(&self, method_name: &str) {
        tokio::time::timeout(WAIT, async {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|request| request.method == method_name)
                {
                    return;
                }
                changed.await;
            }
        })
        .await
        .expect("callback did not arrive");
    }
}

struct Control {
    started: mpsc::UnboundedSender<i64>,
    release: Semaphore,
    initialized: AtomicUsize,
    shutdowns: AtomicUsize,
    fail_shutdown: AtomicBool,
    shutdown_markers: Mutex<Vec<String>>,
    saved_hosts: Mutex<Vec<Arc<dyn HostClient>>>,
}

struct Probe {
    control: Arc<Control>,
    host: OnceLock<Arc<dyn HostClient>>,
}

impl Probe {
    async fn read(&self, input: ToolInvokeInput) -> crate::Result<String> {
        if input.input["hold"] == true {
            self.control
                .started
                .send(input.call_id)
                .map_err(|_| PluginError::internal("protocol test observer stopped"))?;
            self.control
                .release
                .acquire()
                .await
                .map_err(|_| PluginError::internal("protocol test release stopped"))?
                .forget();
        }
        let host = self.host.get().unwrap().clone();
        let value = if input.input["process"] == true {
            // A process worker intentionally has no inherited request context.
            tokio::spawn(async move { host.read_config(None).await })
                .await
                .map_err(|error| PluginError::internal_error(&error))??
        } else {
            host.read_config(None).await?
        };
        Ok(value["marker"].as_str().unwrap().to_owned())
    }
}

#[async_trait::async_trait]
impl Plugin for Probe {
    fn manifest(&self) -> crate::PluginManifest {
        crate::PluginManifest::new("test", "callback-generations", "1.0.0")
    }

    async fn init(&self, _: InitContext, host: Arc<dyn HostClient>) -> crate::Result<InitOutcome> {
        self.control.initialized.fetch_add(1, Ordering::SeqCst);
        self.control.saved_hosts.lock().unwrap().push(host.clone());
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("duplicate init"))?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> crate::Result<ToolInvokeOutput> {
        self.read(input).await.map(ToolInvokeOutput::text)
    }

    async fn tool_invoke_stream(
        &self,
        input: ToolInvokeInput,
        sink: ToolStreamSink,
    ) -> crate::Result<ToolStreamEnd> {
        let text = self.read(input).await?;
        sink.text(text.clone()).await;
        Ok(ToolStreamEnd::text(sink.stream_id(), text))
    }

    async fn shutdown(&self) -> crate::Result<()> {
        self.control.shutdowns.fetch_add(1, Ordering::SeqCst);
        if self.control.fail_shutdown.load(Ordering::SeqCst) {
            return Err(PluginError::internal(
                "deliberate generation cleanup failure",
            ));
        }
        let value = self
            .host
            .get()
            .unwrap()
            .read_config(Some("shutdown".into()))
            .await?;
        self.control
            .shutdown_markers
            .lock()
            .unwrap()
            .push(value["marker"].as_str().unwrap().to_owned());
        Ok(())
    }
}

#[derive(Clone)]
struct Wire {
    client: reqwest::Client,
    url: String,
    ids: Arc<AtomicI64>,
}

impl Wire {
    fn headers(owner: &str, binding: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(INSTANCE_HEADER, owner.parse().unwrap());
        if let Some(binding) = binding {
            headers.insert(BINDING_HEADER, binding.parse().unwrap());
        }
        headers
    }

    async fn send(
        &self,
        headers: HeaderMap,
        method_name: &str,
        params: Value,
        context: Option<HostCallbackContext>,
    ) -> ResponsePayload {
        let id = self.ids.fetch_add(1, Ordering::SeqCst);
        let response = self
            .client
            .post(&self.url)
            .headers(headers)
            .json(&Request {
                jsonrpc: JsonRpcVersion,
                id: RequestId::Num(id),
                method: method_name.into(),
                params: Some(params),
                context,
            })
            .send()
            .await
            .unwrap_or_else(|error| {
                panic!("HTTP fixture request {method_name} (id {id}) failed: {error}")
            });
        let status = response.status();
        let body = response.bytes().await.unwrap_or_else(|error| {
            panic!("HTTP fixture response {method_name} (id {id}, status {status}) failed: {error}")
        });
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "HTTP fixture response {method_name} (id {id}): {}",
            String::from_utf8_lossy(&body)
        );
        let response: Response = serde_json::from_slice(&body).unwrap_or_else(|error| {
            panic!(
                "HTTP fixture response {method_name} (id {id}, status {status}) was not JSON-RPC: {error}; body: {}",
                String::from_utf8_lossy(&body)
            )
        });
        assert_eq!(response.id, RequestId::Num(id));
        response.payload
    }

    async fn call(&self, binding: &str, method_name: &str, params: Value) -> ResponsePayload {
        self.send(
            Self::headers("owner-a", Some(binding)),
            method_name,
            params,
            Some(HostCallbackContext {
                authority_token: Some(format!("authority-{binding}")),
                ..Default::default()
            }),
        )
        .await
    }

    async fn invoke(&self, binding: &str, call_id: i64, input: Value) -> ResponsePayload {
        self.call(
            binding,
            method::HOOK_TOOL_INVOKE,
            invocation(call_id, input),
        )
        .await
    }

    async fn retire(&self, binding: &str) -> HttpCallbackRetired {
        serde_json::from_value(ok(self
            .call(
                binding,
                method::META_HOST_RETIRE,
                json!({"binding_id":binding}),
            )
            .await))
        .unwrap()
    }
}

fn invocation(call_id: i64, input: Value) -> Value {
    serde_json::to_value(ToolInvokeInput {
        tool_name: "probe".into(),
        session_id: 1,
        call_id,
        workspace_root: "/tmp".into(),
        input,
    })
    .unwrap()
}

fn ok(response: ResponsePayload) -> Value {
    match response {
        ResponsePayload::Ok { result } => result,
        ResponsePayload::Err { error } => panic!("unexpected protocol failure: {error:?}"),
    }
}

fn error(response: ResponsePayload) -> crate::rpc::ErrorObject {
    match response {
        ResponsePayload::Err { error } => error,
        ResponsePayload::Ok { result } => panic!("expected protocol failure, received {result}"),
    }
}

struct Fixture {
    old: Peer,
    new: Peer,
    server: Server,
    wire: Wire,
    control: Arc<Control>,
    started: mpsc::UnboundedReceiver<i64>,
}

impl Fixture {
    async fn start() -> Self {
        let old = Peer::start("old").await;
        let new = Peer::start("new").await;
        let (started, signals) = mpsc::unbounded_channel();
        let control = Arc::new(Control {
            started,
            release: Semaphore::new(0),
            initialized: AtomicUsize::new(0),
            shutdowns: AtomicUsize::new(0),
            fail_shutdown: AtomicBool::new(false),
            shutdown_markers: Mutex::new(Vec::new()),
            saved_hosts: Mutex::new(Vec::new()),
        });
        let shared = control.clone();
        let server = Server::start(serve_http(
            move || Probe {
                control: shared.clone(),
                host: OnceLock::new(),
            },
            Arc::new(crate::NoopHostClient),
        ))
        .await;
        let wire = Wire {
            client: reqwest::Client::builder().timeout(WAIT).build().unwrap(),
            url: format!("{}/rpc", server.url),
            ids: Arc::new(AtomicI64::new(1)),
        };
        let fixture = Self {
            old,
            new,
            server,
            wire,
            control,
            started: signals,
        };
        ok(fixture
            .initialize("owner-a", "old", &fixture.old.binding)
            .await);
        fixture
    }

    async fn initialize(
        &self,
        owner: &str,
        binding: &str,
        callback: &HttpCallbackBinding,
    ) -> ResponsePayload {
        let state = ok(self
            .wire
            .send(
                Wire::headers(owner, Some(binding)),
                method::META_HTTP_STATE,
                json!({}),
                None,
            )
            .await);
        let mut headers = Wire::headers(owner, Some(binding));
        headers.insert(
            EXPECTED_REVISION_HEADER,
            state["revision"].as_str().unwrap_or("-").parse().unwrap(),
        );
        self.wire
            .send(
                headers,
                method::META_INIT,
                serde_json::to_value(InitContext {
                    agena_version: "protocol-test".into(),
                    workspace_root: "/tmp".into(),
                    plugin_id: "test.callback-generations".parse().unwrap(),
                    host_callback_url: callback.url.clone(),
                    host_callback_token: callback.token.clone(),
                    settings: json!({}),
                    protocol_version: crate::rpc::PROTOCOL_VERSION,
                })
                .unwrap(),
                None,
            )
            .await
    }

    async fn fork(&self) {
        ok(self
            .wire
            .call(
                "old",
                method::META_HOST_FORK,
                serde_json::to_value(HttpCallbackFork {
                    expected_id: "old".into(),
                    expected: self.old.binding.clone(),
                    next_id: "new".into(),
                    next: self.new.binding.clone(),
                })
                .unwrap(),
            )
            .await);
    }

    async fn started(&mut self, expected: i64) {
        assert_eq!(
            tokio::time::timeout(WAIT, self.started.recv())
                .await
                .unwrap(),
            Some(expected)
        );
    }

    async fn finish(mut self) {
        self.server.finish().await;
        self.old.server.finish().await;
        self.new.server.finish().await;
    }
}
