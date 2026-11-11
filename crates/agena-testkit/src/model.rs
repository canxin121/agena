//! A deterministic loopback multi-protocol model provider for ordinary Rust tests.
//!
//! Expectations match requests, not a global request queue. Register expectations
//! before the action that can send them. Predicates and checks are synchronous:
//! they must be quick and must not block on another request. The server never
//! contacts a real model or any other network service.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    convert::Infallible,
    future::Future,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, Response, StatusCode, header},
};
use bytes::Bytes;
use futures_core::Stream;
use futures_util::FutureExt;
use hyper::{body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc, oneshot, watch},
    task::{AbortHandle, JoinHandle},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

mod matcher;
mod protocol;
mod reply;
mod request;
mod wire;
pub use matcher::RequestMatch;
pub use protocol::{ModelEvent, ModelProtocol};
use reply::PreparedReply;
pub use reply::{FunctionCall, Reply, Usage};

/// The model driver retains ModelHandle as a source-compatible name.
pub type ModelDriver = ModelHandle;

/// Additional native protocol endpoints served by the strict scripted model.
/// A route permits transport only: each request still requires an expectation.
#[derive(Clone, Debug)]
pub struct ModelRoute {
    pub method: String,
    pub path: String,
    pub prefix: bool,
    /// Protocol metadata for custom routes, not inferred from their URL spelling.
    pub protocol: Option<ModelProtocol>,
}

impl ModelRoute {
    pub fn post(path: impl Into<String>) -> Self {
        Self {
            method: "POST".into(),
            path: path.into(),
            prefix: false,
            protocol: None,
        }
    }
    pub fn get(path: impl Into<String>) -> Self {
        Self {
            method: "GET".into(),
            path: path.into(),
            prefix: false,
            protocol: None,
        }
    }
    pub fn post_prefix(path: impl Into<String>) -> Self {
        Self {
            method: "POST".into(),
            path: path.into(),
            prefix: true,
            protocol: None,
        }
    }

    /// Interpret matching requests as this protocol, even behind a custom gateway prefix.
    pub fn for_protocol(mut self, protocol: ModelProtocol) -> Self {
        self.protocol = Some(protocol);
        self
    }

    fn matches(&self, method: &Method, path: &str) -> bool {
        method.as_str() == self.method
            && if self.prefix {
                path.starts_with(&self.path)
            } else {
                path == self.path
            }
    }
}

/// Resource limits and deadlines for one isolated scripted provider.
#[derive(Clone, Debug)]
pub struct ModelOptions {
    /// Default wire format for custom routes. Standard endpoints identify themselves.
    pub protocol: ModelProtocol,
    pub models: Vec<String>,
    pub routes: Vec<ModelRoute>,
    pub auto_openai_models: bool,
    pub default_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub max_request_bytes: usize,
    /// Maximum retained JSON bytes per request. Checks still see the full body.
    pub max_record_bytes: usize,
    pub max_recorded_requests: usize,
    pub max_failures: usize,
    pub max_steps: usize,
    pub max_connections: usize,
    pub max_response_bytes: usize,
    pub max_response_chunks: usize,
    pub stream_buffer: usize,
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self {
            protocol: ModelProtocol::OpenAiResponses,
            models: vec!["fake-model".into()],
            routes: Vec::new(),
            auto_openai_models: true,
            default_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(5),
            max_request_bytes: 8 * 1024 * 1024,
            max_record_bytes: 64 * 1024,
            max_recorded_requests: 128,
            max_failures: 16,
            max_steps: 256,
            max_connections: 64,
            max_response_bytes: 8 * 1024 * 1024,
            max_response_chunks: 16_384,
            stream_buffer: 8,
        }
    }
}

impl ModelOptions {
    pub fn for_protocol(protocol: ModelProtocol) -> Self {
        Self {
            protocol,
            models: vec![protocol.model_id().to_owned()],
            routes: protocol.routes(),
            auto_openai_models: matches!(
                protocol,
                ModelProtocol::OpenAiResponses | ModelProtocol::OpenAiChat
            ),
            ..Self::default()
        }
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            !self.models.is_empty() && self.models.iter().all(|model| !model.trim().is_empty()),
            "scripted model needs at least one non-empty model ID"
        );
        ensure!(
            !self.default_timeout.is_zero() && !self.shutdown_timeout.is_zero(),
            "scripted model deadlines must be positive"
        );
        for route in &self.routes {
            ensure!(
                matches!(route.method.as_str(), "POST" | "GET")
                    && route.path.starts_with('/')
                    && !route.path.contains(".."),
                "invalid native model HTTP route: {route:?}"
            );
        }
        for (name, limit) in [
            ("max_request_bytes", self.max_request_bytes),
            ("max_record_bytes", self.max_record_bytes),
            ("max_recorded_requests", self.max_recorded_requests),
            ("max_failures", self.max_failures),
            ("max_steps", self.max_steps),
            ("max_connections", self.max_connections),
            ("max_response_bytes", self.max_response_bytes),
            ("max_response_chunks", self.max_response_chunks),
            ("stream_buffer", self.stream_buffer),
        ] {
            ensure!(limit > 0, "{name} must be positive");
        }
        Ok(())
    }
}

/// A request seen by the fixture.
///
/// Matchers and checks always see the complete body, limited by max_request_bytes.
/// Retained diagnostics and StepHandle::observed may have body_truncated set: in
/// that case body is null and body_preview contains a bounded UTF-8 preview.
#[derive(Clone)]
pub struct ModelRequest {
    pub sequence: u64,
    pub method: String,
    pub path: String,
    /// Original path and query, not automatically printed in fixture diagnostics.
    raw_uri: String,
    /// Parsed, ordered query pairs; duplicates are preserved for exact matching.
    query_pairs: Vec<(String, String)>,
    route_protocol: Option<ModelProtocol>,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
    pub body_bytes: usize,
    pub body_truncated: bool,
    pub body_preview: String,
    pub step_id: Option<u64>,
}

impl std::fmt::Debug for ModelRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Query strings often carry API keys. Never include their values in
        // retained model.txt diagnostics or mismatch previews.
        f.debug_struct("ModelRequest")
            .field("sequence", &self.sequence)
            .field("method", &self.method)
            .field("path", &self.path)
            .field(
                "query_names",
                &self
                    .query_pairs
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
            )
            .field("route_protocol", &self.route_protocol)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("body", &self.body)
            .field("body_bytes", &self.body_bytes)
            .field("body_truncated", &self.body_truncated)
            .field("body_preview", &self.body_preview)
            .field("step_id", &self.step_id)
            .finish()
    }
}

impl ModelRequest {
    fn routing_summary(&self) -> String {
        let input = self.body.get("input");
        let recent_user = input.and_then(|input| {
            if input.is_string() {
                Some(input)
            } else {
                input.as_array()?.iter().rev().find_map(|item| {
                    (item.get("role").and_then(Value::as_str) == Some("user"))
                        .then(|| item.get("content"))
                        .flatten()
                })
            }
        });
        let mut user_text = Vec::new();
        if let Some(content) = recent_user {
            collect_text(content, &mut user_text);
        }
        let items = input.and_then(Value::as_array);
        let output_count = items
            .into_iter()
            .flatten()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
            .count();
        let call_ids = items
            .into_iter()
            .flatten()
            .rev()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
            .take(6)
            .map(|item| {
                bounded_text(
                    item.get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("<missing>"),
                    80,
                )
            })
            .collect::<Vec<_>>();
        format!(
            "{} {}; model {:?}, stream {}, body {} bytes; latest user text {:?}; tool outputs {output_count}, recent call_ids {call_ids:?}",
            self.method,
            self.path,
            self.model().map(|model| bounded_text(model, 160)),
            self.streaming(),
            self.body_bytes,
            bounded_text(&user_text.join("\n"), 512)
        )
    }

    fn retained(&self, raw: &[u8], limit: usize) -> Self {
        let truncated = raw.len() > limit;
        Self {
            sequence: self.sequence,
            method: self.method.clone(),
            path: self.path.clone(),
            raw_uri: self.raw_uri.clone(),
            query_pairs: self.query_pairs.clone(),
            route_protocol: self.route_protocol,
            headers: self
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), bounded_text(value, 4096)))
                .collect(),
            body: if truncated {
                Value::Null
            } else {
                self.body.clone()
            },
            body_bytes: raw.len(),
            body_truncated: truncated,
            body_preview: bounded_bytes(raw, limit),
            step_id: self.step_id,
        }
    }
}

fn collect_text(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::String(text) => output.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_text(item, output);
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                output.push(text.to_owned());
            }
        }
        _ => {}
    }
}

type RequestCheck = Arc<dyn Fn(&ModelRequest) -> Result<()> + Send + Sync>;

#[derive(Clone, Default)]
struct StepProgress {
    observed: Option<Arc<ModelRequest>>,
    finished: bool,
    disconnected: bool,
    chunks_sent: usize,
}

#[derive(Default)]
struct ResponseConnection {
    cancel: Option<CancellationToken>,
    forced: bool,
}

struct StepSignals {
    progress: watch::Sender<StepProgress>,
    released: watch::Sender<bool>,
    connection: Mutex<ResponseConnection>,
    expected_disconnection: AtomicBool,
}

impl StepSignals {
    fn new() -> Self {
        let (progress, _) = watch::channel(StepProgress::default());
        let (released, _) = watch::channel(false);
        Self {
            progress,
            released,
            connection: Mutex::new(ResponseConnection::default()),
            expected_disconnection: AtomicBool::new(false),
        }
    }

    fn body_ended(&self, normally: bool) {
        // Detach before publishing a terminal state: a stale StepHandle must
        // never close the pooled connection of a later request.
        let mut connection = lock(&self.connection);
        connection.cancel.take();
        self.progress.send_modify(|progress| {
            if !progress.finished && !progress.disconnected {
                if normally && !connection.forced {
                    progress.finished = true;
                } else {
                    progress.disconnected = true;
                }
            }
        });
    }

    fn terminal(&self) -> bool {
        let progress = self.progress.borrow();
        progress.finished || progress.disconnected
    }
}

struct Dependency {
    id: u64,
    signals: Arc<StepSignals>,
    observed_only: bool,
}

impl Dependency {
    fn ready(&self) -> bool {
        if self.observed_only {
            self.signals.progress.borrow().observed.is_some()
        } else {
            self.signals.terminal()
        }
    }
}

struct Step {
    id: u64,
    name: Arc<str>,
    matcher: RequestMatch,
    checks: Vec<RequestCheck>,
    dependencies: Vec<Dependency>,
    reply: Reply,
    claimed: AtomicBool,
    signals: Arc<StepSignals>,
}

/// A synchronous expectation builder. Calling reply registers exactly one step.
pub struct ExpectRequest {
    state: Arc<ModelState>,
    matcher: RequestMatch,
    checks: Vec<RequestCheck>,
    dependencies: Vec<Dependency>,
    name: Option<String>,
    invalid: Option<String>,
}

impl ExpectRequest {
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Require the previous response to have ended or been disconnected.
    pub fn after(self, previous: &StepHandle) -> Self {
        self.dependency(previous, false)
    }

    /// Allow a request once the previous request was observed, even if held open.
    pub fn after_observed(self, previous: &StepHandle) -> Self {
        self.dependency(previous, true)
    }

    fn dependency(mut self, previous: &StepHandle, observed_only: bool) -> Self {
        if !Arc::ptr_eq(&self.state, &previous.state) {
            self.invalid = Some("expectation dependency belongs to another scripted model".into());
        }
        self.dependencies.push(Dependency {
            id: previous.id,
            signals: previous.signals.clone(),
            observed_only,
        });
        self
    }

    pub fn check(
        mut self,
        check: impl Fn(&ModelRequest) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.checks.push(Arc::new(check));
        self
    }

    pub fn check_tool_output_contains(
        self,
        call_id: impl Into<String>,
        expected: impl Into<String>,
    ) -> Self {
        let (call_id, expected) = (call_id.into(), expected.into());
        self.check(move |request| request.assert_tool_output_contains(&call_id, &expected))
    }

    pub fn reply(self, reply: Reply) -> StepHandle {
        let id = self.state.next_step.fetch_add(1, Ordering::Relaxed);
        let name: Arc<str> = self
            .name
            .unwrap_or_else(|| self.matcher.label.clone())
            .into();
        let signals = Arc::new(StepSignals::new());
        let handle = StepHandle {
            id,
            name: name.clone(),
            signals: signals.clone(),
            state: self.state.clone(),
            timeout: self.state.options.default_timeout,
        };
        if let Some(error) = self.invalid {
            self.state.fail(error);
            return handle;
        }
        if self.state.shutdown.is_cancelled() {
            self.state
                .fail("registered an expectation after model shutdown");
            return handle;
        }
        let mut registry = lock(&self.state.registry);
        if registry.steps.len() >= self.state.options.max_steps {
            drop(registry);
            self.state.fail("scripted model expectation limit exceeded");
            return handle;
        }
        registry.steps.push(Arc::new(Step {
            id,
            name,
            matcher: self.matcher,
            checks: self.checks,
            dependencies: self.dependencies,
            reply,
            claimed: AtomicBool::new(false),
            signals,
        }));
        handle
    }
}

/// Per-step causal signals. Cloning a handle does not consume its observations.
#[derive(Clone)]
pub struct StepHandle {
    id: u64,
    name: Arc<str>,
    signals: Arc<StepSignals>,
    state: Arc<ModelState>,
    timeout: Duration,
}

impl std::fmt::Debug for StepHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let progress = self.signals.progress.borrow();
        formatter
            .debug_struct("StepHandle")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("observed", &progress.observed.is_some())
            .field("finished", &progress.finished)
            .field("disconnected", &progress.disconnected)
            .field("chunks_sent", &progress.chunks_sent)
            .finish()
    }
}

impl StepHandle {
    pub(crate) fn belongs_to(&self, model: &ModelHandle) -> bool {
        Arc::ptr_eq(&self.state, &model.state)
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn release(&self) {
        self.signals.released.send_replace(true);
    }

    pub fn chunks_sent(&self) -> usize {
        self.signals.progress.borrow().chunks_sent
    }

    /// Abruptly close only this request's actual HTTP connection and await the
    /// body-drop observation. Call on a held response for a deterministic cut.
    /// A completed step is rejected so an old handle cannot kill a later
    /// keep-alive request. This never shuts down the model listener.
    pub async fn disconnect(&self) -> Result<()> {
        self.observed().await?;
        {
            let mut connection = lock(&self.signals.connection);
            if let Some(cancel) = connection.cancel.take() {
                connection.forced = true;
                cancel.cancel();
            } else {
                ensure!(
                    connection.forced || self.signals.progress.borrow().disconnected,
                    "step {} ({:?}) response body already finished normally; no active connection to disconnect",
                    self.id,
                    self.name
                );
            }
        }
        self.disconnected().await
    }

    pub async fn observed(&self) -> Result<Arc<ModelRequest>> {
        self.wait_for("request to be observed", |progress| {
            progress.observed.clone().map(Ok)
        })
        .await
    }

    /// The HTTP server consumed the complete planned body. This is not a claim
    /// that Agena's session or a UI has finished processing it.
    pub async fn finished(&self) -> Result<()> {
        self.wait_for("response body to finish", |progress| {
            if progress.finished {
                Some(Ok(()))
            } else if progress.disconnected {
                Some(Err(anyhow!(
                    "response disconnected before its body finished"
                )))
            } else {
                None
            }
        })
        .await
    }

    /// Declare that case shutdown must disconnect this response. Unlike
    /// `disconnected().await`, this is called *before* the case begins cleanup;
    /// assert_finished checks that the promised disconnect really occurred.
    /// It is used for lifecycle tests that intentionally leave a run in flight
    /// so the product's shutdown path can cancel it.
    pub fn expect_disconnection(&self) -> Result<()> {
        ensure!(
            !self.signals.progress.borrow().finished,
            "cannot expect a disconnect after the response already finished"
        );
        self.signals
            .expected_disconnection
            .store(true, Ordering::Release);
        Ok(())
    }

    /// A response body was dropped before its planned end, as on cancellation.
    /// Awaiting and verifying this marks the disconnect as intentionally tested;
    /// otherwise case cleanup treats a disconnected scripted reply as a failure.
    pub async fn disconnected(&self) -> Result<()> {
        self.wait_for("response body to disconnect", |progress| {
            if progress.disconnected {
                Some(Ok(()))
            } else if progress.finished {
                Some(Err(anyhow!("response body already finished normally")))
            } else {
                None
            }
        })
        .await?;
        self.signals
            .expected_disconnection
            .store(true, Ordering::Release);
        Ok(())
    }

    async fn wait_for<T>(
        &self,
        description: &str,
        ready: impl Fn(&StepProgress) -> Option<Result<T>>,
    ) -> Result<T> {
        let mut changes = self.signals.progress.subscribe();
        let wait = async {
            loop {
                if self.state.fatal.is_cancelled() {
                    return Err(self.state.failure_error());
                }
                let progress = changes.borrow().clone();
                if let Some(result) = ready(&progress) {
                    return result;
                }
                tokio::select! {
                    _ = self.state.fatal.cancelled() => return Err(self.state.failure_error()),
                    _ = self.state.shutdown.cancelled() => bail!("scripted model shut down while waiting"),
                    changed = changes.changed() => {
                        changed.context("scripted model step observation closed")?;
                    }
                }
            }
        };
        tokio::time::timeout(self.timeout, wait)
            .await
            .with_context(|| {
                format!(
                    "timed out after {:?} waiting for step {} ({:?}) {}",
                    self.timeout, self.id, self.name, description
                )
            })?
            .with_context(|| format!("step {} ({:?}): {description}", self.id, self.name))
    }
}

/// Cloneable control and observations for one model server.
#[derive(Clone)]
pub struct ModelHandle {
    state: Arc<ModelState>,
}

impl std::fmt::Debug for ModelHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModelHandle")
            .field("base_url", &self.base_url())
            .field("failures", &self.failures())
            .field("requests", &self.requests())
            .finish()
    }
}

impl ModelHandle {
    pub fn base_url(&self) -> &str {
        &self.state.base_url
    }

    pub fn model_id(&self) -> &str {
        &self.state.options.models[0]
    }

    pub fn expect(&self, matcher: RequestMatch) -> ExpectRequest {
        ExpectRequest {
            state: self.state.clone(),
            matcher,
            checks: Vec::new(),
            dependencies: Vec::new(),
            name: None,
            invalid: None,
        }
    }

    pub fn requests(&self) -> Vec<ModelRequest> {
        lock(&self.state.registry)
            .requests
            .iter()
            .map(|request| (**request).clone())
            .collect()
    }

    pub fn dropped_request_records(&self) -> u64 {
        lock(&self.state.registry).dropped_request_records
    }

    pub fn failures(&self) -> Vec<String> {
        lock(&self.state.registry)
            .failures
            .iter()
            .cloned()
            .collect()
    }

    pub fn failure_token(&self) -> CancellationToken {
        self.state.fatal.clone()
    }

    /// A sticky failure signal, safe to race against the user's test future.
    pub async fn failed(&self) -> anyhow::Error {
        self.state.fatal.cancelled().await;
        self.state.failure_error()
    }

    pub async fn wait_failure(&self) -> anyhow::Error {
        self.failed().await
    }

    pub async fn failure_notified(&self) -> anyhow::Error {
        self.failed().await
    }

    /// Every step must be consumed exactly once and reach an HTTP body terminal
    /// state. Explicit cancellation may end a step through disconnection.
    pub fn assert_finished(&self) -> Result<()> {
        let (steps, failures) = {
            let registry = lock(&self.state.registry);
            (
                registry.steps.clone(),
                registry.failures.iter().cloned().collect::<Vec<_>>(),
            )
        };
        ensure!(
            failures.is_empty(),
            "scripted model failures:\n{}",
            failures.join("\n")
        );
        let pending: Vec<_> = steps
            .iter()
            .filter(|step| !step.claimed.load(Ordering::Acquire))
            .map(|step| format!("{} ({:?})", step.id, step.name))
            .collect();
        ensure!(
            pending.is_empty(),
            "unconsumed model expectations: {}",
            pending.join(", ")
        );
        let active: Vec<_> = steps
            .iter()
            .filter(|step| !step.signals.terminal())
            .map(|step| format!("{} ({:?})", step.id, step.name))
            .collect();
        ensure!(
            active.is_empty(),
            "model responses still active: {}",
            active.join(", ")
        );
        let unexplained_disconnections = steps
            .iter()
            .filter(|step| {
                step.signals.progress.borrow().disconnected
                    && !step.signals.expected_disconnection.load(Ordering::Acquire)
            })
            .map(|step| format!("{} ({:?})", step.id, step.name))
            .collect::<Vec<_>>();
        ensure!(
            unexplained_disconnections.is_empty(),
            "model replies disconnected without an explicit step.disconnected() assertion: {}",
            unexplained_disconnections.join(", ")
        );
        let unrealized_disconnections = steps
            .iter()
            .filter(|step| {
                step.signals.expected_disconnection.load(Ordering::Acquire)
                    && !step.signals.progress.borrow().disconnected
            })
            .map(|step| format!("{} ({:?})", step.id, step.name))
            .collect::<Vec<_>>();
        ensure!(
            unrealized_disconnections.is_empty(),
            "model replies expected to disconnect but did not: {}",
            unrealized_disconnections.join(", ")
        );
        Ok(())
    }
}

/// Owns the loopback listener and every connection/response task.
///
/// Call shutdown to await cleanup. Drop is only a cancellation/abort fallback.
pub struct ScriptedModel {
    state: Arc<ModelState>,
    accept_task: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for ScriptedModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScriptedModel")
            .field("base_url", &self.base_url())
            .finish()
    }
}

impl ScriptedModel {
    pub async fn start() -> Result<Self> {
        Self::start_with_options(ModelOptions::default()).await
    }

    pub async fn start_with_options(options: ModelOptions) -> Result<Self> {
        options.validate()?;
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("bind scripted model loopback listener")?;
        let address = listener
            .local_addr()
            .context("read scripted model address")?;
        let state = Arc::new(ModelState {
            base_url: format!("http://{address}/v1"),
            connections: Arc::new(Semaphore::new(options.max_connections)),
            options,
            registry: Mutex::new(Registry::default()),
            next_step: AtomicU64::new(1),
            next_request: AtomicU64::new(1),
            next_task: AtomicU64::new(1),
            aborts: Mutex::new(HashMap::new()),
            tasks: TaskTracker::new(),
            shutdown: CancellationToken::new(),
            fatal: CancellationToken::new(),
        });
        let accept_state = state.clone();
        let accept_task = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    biased;
                    _ = accept_state.shutdown.cancelled() => break,
                    accepted = listener.accept() => accepted,
                };
                let (socket, _) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        accept_state.fail(format!("scripted model accept failed: {error}"));
                        break;
                    }
                };
                let permit = match accept_state.connections.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        accept_state.fail("scripted model concurrent connection limit exceeded");
                        drop(socket);
                        continue;
                    }
                };
                let _ = socket.set_nodelay(true);
                let connection_state = accept_state.clone();
                accept_state.spawn_tracked(async move {
                    let _permit = permit;
                    let service_state = connection_state.clone();
                    let disconnect = CancellationToken::new();
                    let request_disconnect = disconnect.clone();
                    let service = service_fn(move |request: Request<Incoming>| {
                        handle_request(request, service_state.clone(), request_disconnect.clone())
                    });
                    let mut builder = http1::Builder::new();
                    builder.keep_alive(true).max_buf_size(64 * 1024);
                    let connection = builder.serve_connection(TokioIo::new(socket), service);
                    tokio::select! {
                        biased;
                        _ = connection_state.shutdown.cancelled() => {}
                        _ = disconnect.cancelled() => {}
                        // A cancelled provider request is an expected connection
                        // close. Step body drop records its precise outcome.
                        _ = connection => {}
                    }
                });
            }
        });
        Ok(Self {
            state,
            accept_task: Some(accept_task),
        })
    }

    pub fn handle(&self) -> ModelHandle {
        ModelHandle {
            state: self.state.clone(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.state.base_url
    }

    pub fn model_id(&self) -> &str {
        &self.state.options.models[0]
    }

    /// Stops accepting connections, closes active sockets and joins all owned
    /// tasks. Expectations are checked separately with assert_finished.
    pub async fn shutdown(mut self) -> Result<()> {
        self.state.shutdown.cancel();
        let timeout = self.state.options.shutdown_timeout;
        let mut errors = Vec::new();
        if let Some(mut accept_task) = self.accept_task.take() {
            match tokio::time::timeout(timeout, &mut accept_task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    errors.push(format!("scripted model listener task failed: {error}"))
                }
                Err(_) => {
                    accept_task.abort();
                    if tokio::time::timeout(timeout, &mut accept_task)
                        .await
                        .is_err()
                    {
                        errors.push("scripted model listener did not stop after abort".into());
                    }
                    errors.push("scripted model listener exceeded its shutdown deadline".into());
                }
            }
        }
        self.state.tasks.close();
        if tokio::time::timeout(timeout, self.state.tasks.wait())
            .await
            .is_err()
        {
            self.state.abort_tasks();
            if tokio::time::timeout(timeout, self.state.tasks.wait())
                .await
                .is_err()
            {
                errors.push("scripted model tasks did not stop after abort".into());
            } else {
                errors.push("scripted model tasks required forced abort during shutdown".into());
            }
        }
        ensure!(errors.is_empty(), "{}", errors.join("\n"));
        Ok(())
    }
}

impl Drop for ScriptedModel {
    fn drop(&mut self) {
        self.state.shutdown.cancel();
        if let Some(task) = self.accept_task.take() {
            task.abort();
        }
        self.state.abort_tasks();
    }
}

#[derive(Default)]
struct Registry {
    steps: Vec<Arc<Step>>,
    requests: VecDeque<Arc<ModelRequest>>,
    failures: VecDeque<String>,
    dropped_request_records: u64,
    /// A `tools_call` reply held back until the runtime has the tool's help.
    /// The next request that carries the synthetic `tools_help` output
    /// receives exactly this prepared reply.
    help_answer: Option<PendingHelpAnswer>,
}

/// A scripted `tools_call` reply waiting for its contract round trip.
struct PendingHelpAnswer {
    /// Tools whose contract the synthetic `tools_help` round taught.
    taught_tools: Vec<String>,
    step_id: u64,
    planned: PreparedReply,
}

impl PendingHelpAnswer {
    /// A request carries the taught contracts when it shows their rendered
    /// first line. Call IDs cannot identify this: a protocol such as Ollama
    /// numbers calls by position, so the synthetic id never reaches the wire.
    fn matches(&self, observed: &ModelRequest) -> bool {
        let body = observed.body.to_string();
        if self
            .taught_tools
            .iter()
            .any(|tool| body.contains(&format!("Tool: {tool}")))
        {
            return true;
        }
        // A tool outside the session's catalog has no contract to teach; the
        // help round returns `tool.help_required` instead. Release the held
        // call only for the round that actually carries that help answer, so
        // an unrelated session's request cannot consume it.
        let help_call = format!("call_help_gate_{}_", self.step_id);
        body.contains("tool.help_required") && body.contains(&help_call)
    }
}

struct ModelState {
    base_url: String,
    options: ModelOptions,
    registry: Mutex<Registry>,
    next_step: AtomicU64,
    next_request: AtomicU64,
    next_task: AtomicU64,
    tasks: TaskTracker,
    aborts: Mutex<HashMap<u64, AbortHandle>>,
    connections: Arc<Semaphore>,
    shutdown: CancellationToken,
    fatal: CancellationToken,
}

impl ModelState {
    fn hold_help_answer(&self, answer: PendingHelpAnswer) {
        lock(&self.registry).help_answer = Some(answer);
    }

    /// Whether this request is the one that shows the held help output.
    fn help_answer_matches(&self, observed: &ModelRequest) -> bool {
        let registry = lock(&self.registry);
        registry
            .help_answer
            .as_ref()
            .is_some_and(|answer| answer.matches(observed))
    }

    fn held_step_id(&self) -> Option<u64> {
        lock(&self.registry)
            .help_answer
            .as_ref()
            .map(|answer| answer.step_id)
    }

    /// Take the held reply when this request is the one that shows the help
    /// output. A different request leaves it waiting.
    fn take_help_answer(&self, observed: &ModelRequest) -> Option<PendingHelpAnswer> {
        let mut registry = lock(&self.registry);
        if !registry
            .help_answer
            .as_ref()
            .is_some_and(|answer| answer.matches(observed))
        {
            return None;
        }
        registry.help_answer.take()
    }
}

impl ModelState {
    fn fail(&self, error: impl Into<String>) {
        let error = bounded_text(&error.into(), 8192);
        {
            let mut registry = lock(&self.registry);
            if registry.failures.len() == self.options.max_failures {
                registry.failures.pop_front();
            }
            registry.failures.push_back(error);
        }
        self.fatal.cancel();
    }

    fn failure_error(&self) -> anyhow::Error {
        let failures = lock(&self.registry)
            .failures
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        anyhow!("scripted model failure:\n{}", failures.join("\n"))
    }

    fn record(&self, request: Arc<ModelRequest>) {
        let mut registry = lock(&self.registry);
        if registry.requests.len() == self.options.max_recorded_requests {
            registry.requests.pop_front();
            registry.dropped_request_records = registry.dropped_request_records.saturating_add(1);
        }
        registry.requests.push_back(request);
    }

    fn claim(&self, request: &ModelRequest) -> Result<Arc<Step>> {
        loop {
            // User predicates run without any registry lock held.
            let steps = lock(&self.registry).steps.clone();
            let mut eligible = Vec::new();
            let mut blocked = Vec::new();
            let mut mismatches = Vec::new();
            let mut consumed = Vec::new();
            for step in steps {
                if step.claimed.load(Ordering::Acquire) {
                    consumed.push(format!("{} ({:?})", step.id, bounded_text(&step.name, 128)));
                    continue;
                }
                if let Some(mismatch) = step.matcher.first_mismatch(request)? {
                    mismatches.push((
                        mismatch.matched_conditions,
                        step.id,
                        format!(
                            "{} ({:?}): {}",
                            step.id,
                            bounded_text(&step.name, 128),
                            mismatch.detail
                        ),
                    ));
                    continue;
                }
                let missing: Vec<_> = step
                    .dependencies
                    .iter()
                    .filter(|dependency| !dependency.ready())
                    .map(|dependency| dependency.id)
                    .collect();
                if missing.is_empty() {
                    eligible.push(step);
                } else {
                    blocked.push(format!(
                        "{} ({:?}) waits for {missing:?}",
                        step.id,
                        bounded_text(&step.name, 128)
                    ));
                }
            }
            if eligible.len() > 1 {
                bail!(
                    "ambiguous model request {} matched multiple expectations: {}; {}",
                    request.sequence,
                    diagnostic_list(eligible.iter().map(|step| format!(
                        "{} ({:?})",
                        step.id,
                        bounded_text(&step.name, 128)
                    ))),
                    request.routing_summary()
                );
            }
            let Some(step) = eligible.pop() else {
                // Prefer candidates whose earlier conditions matched. Merely
                // registering many unrelated steps must not hide a routing typo.
                mismatches
                    .sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
                bail!(
                    "unexpected model request {}; {}; candidate mismatches: {}; blocked: {}; already consumed: {}",
                    request.sequence,
                    request.routing_summary(),
                    diagnostic_list(mismatches.into_iter().map(|(_, _, detail)| detail)),
                    diagnostic_list(blocked),
                    diagnostic_list(consumed)
                );
            };
            if step
                .claimed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(step);
            }
            // A concurrent request won this step. Re-evaluate, never consume it twice.
        }
    }

    fn spawn_tracked(self: &Arc<Self>, future: impl Future<Output = ()> + Send + 'static) {
        let id = self.next_task.fetch_add(1, Ordering::Relaxed);
        let state = self.clone();
        let guard = TaskGuard {
            id,
            state: Arc::downgrade(self),
        };
        let (armed, ready) = oneshot::channel();
        let task = self.tasks.spawn(async move {
            let _guard = guard;
            let _ = ready.await;
            if let Err(payload) = AssertUnwindSafe(future).catch_unwind().await {
                state.fail(format!(
                    "scripted model task panicked: {}",
                    panic_message(payload)
                ));
            }
        });
        lock(&self.aborts).insert(id, task.abort_handle());
        let _ = armed.send(());
    }

    fn abort_tasks(&self) {
        for task in lock(&self.aborts).values() {
            task.abort();
        }
    }
}

struct TaskGuard {
    id: u64,
    state: Weak<ModelState>,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            lock(&state.aborts).remove(&self.id);
        }
    }
}

async fn handle_request(
    request: Request<Incoming>,
    state: Arc<ModelState>,
    disconnect: CancellationToken,
) -> std::result::Result<Response<Body>, Infallible> {
    let path = request.uri().path().trim_end_matches('/').to_owned();
    if state.options.auto_openai_models && request.method() == Method::GET && path == "/v1/models" {
        let models: Vec<_> = state.options.models.iter().map(|id| {
            json!({"id": id, "object": "model", "created": 0, "owned_by": "agena-testkit"})
        }).collect();
        return Ok(json_response(
            StatusCode::OK,
            json!({"object": "list", "data": models}),
        ));
    }
    let matched_route = state
        .options
        .routes
        .iter()
        .find(|route| route.matches(request.method(), &path));
    let default_responses_route = state.options.protocol == ModelProtocol::OpenAiResponses
        && request.method() == Method::POST
        && path == "/v1/responses";
    if matched_route.is_none() && !default_responses_route {
        let error = format!(
            "unexpected model HTTP request {} {} (query values omitted)",
            request.method(),
            path
        );
        state.fail(error.clone());
        return Ok(error_response(StatusCode::NOT_FOUND, &error));
    }
    if state.fatal.is_cancelled() {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "scripted model already failed",
        ));
    }
    let method = request.method().to_string();
    let raw_uri = request.uri().to_string();
    let query_pairs = request
        .uri()
        .query()
        .map(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .map(|(name, value)| (name.into_owned(), value.into_owned()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let route_protocol = matched_route
        .and_then(|route| route.protocol)
        .or_else(|| ModelProtocol::from_path(&path))
        .or(Some(state.options.protocol));
    let headers = request
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = Body::new(request.into_body());
    let bytes = tokio::select! {
        _ = state.shutdown.cancelled() => {
            return Ok(error_response(StatusCode::SERVICE_UNAVAILABLE, "scripted model is shutting down"));
        }
        body = tokio::time::timeout(
            state.options.default_timeout,
            to_bytes(body, state.options.max_request_bytes),
        ) => match body {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => {
                let error = format!("read scripted model request body: {error}");
                state.fail(error.clone());
                return Ok(error_response(StatusCode::PAYLOAD_TOO_LARGE, &error));
            }
            Err(_) => {
                state.fail("timed out reading scripted model request body");
                return Ok(error_response(StatusCode::REQUEST_TIMEOUT, "model request body timed out"));
            }
        }
    };
    let sequence = state.next_request.fetch_add(1, Ordering::Relaxed);
    let body = match if bytes.is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_slice::<Value>(&bytes)
    } {
        Ok(body) => body,
        Err(error) => {
            state.record(Arc::new(ModelRequest {
                sequence,
                method,
                path,
                raw_uri,
                query_pairs,
                route_protocol,
                headers,
                body: Value::Null,
                body_bytes: bytes.len(),
                body_truncated: true,
                body_preview: bounded_bytes(&bytes, state.options.max_record_bytes),
                step_id: None,
            }));
            let error = format!("invalid model request JSON: {error}");
            state.fail(error.clone());
            return Ok(error_response(StatusCode::BAD_REQUEST, &error));
        }
    };
    let mut observed = ModelRequest {
        sequence,
        method,
        path,
        raw_uri,
        query_pairs,
        route_protocol,
        headers,
        body,
        body_bytes: bytes.len(),
        body_truncated: false,
        body_preview: String::new(),
        step_id: None,
    };
    // The request that carries a taught help output receives the reply the
    // scripted step was holding; it must be answered before script matching,
    // because the scenario only registered the eventual tool output.
    if state.help_answer_matches(&observed) {
        observed.step_id = state.held_step_id();
        state.record(Arc::new(
            observed.retained(&bytes, state.options.max_record_bytes),
        ));
        let answer = state
            .take_help_answer(&observed)
            .expect("held help answer must survive its match");
        return Ok(prepared_response(answer.planned, &state));
    }
    let step = match state.claim(&observed) {
        Ok(step) => step,
        Err(error) => {
            state.record(Arc::new(
                observed.retained(&bytes, state.options.max_record_bytes),
            ));
            let error = error.to_string();
            state.fail(error.clone());
            return Ok(error_response(StatusCode::BAD_REQUEST, &error));
        }
    };
    lock(&step.signals.connection).cancel = Some(disconnect);
    // If the connection is cut after observation but before the response body
    // exists, dropping the in-flight service future still reports disconnection.
    let mut claim_guard = ResponseClaimGuard {
        signals: step.signals.clone(),
        handed_off: false,
    };
    observed.step_id = Some(step.id);
    let retained = Arc::new(observed.retained(&bytes, state.options.max_record_bytes));
    state.record(retained.clone());
    step.signals
        .progress
        .send_modify(|progress| progress.observed = Some(retained));
    // Checks run for every request that claims the step, including the proxied
    // help round: a step's assertions are about this request, not its reply.
    for check in &step.checks {
        let checked = std::panic::catch_unwind(AssertUnwindSafe(|| check(&observed)));
        let error = match checked {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => format!("step {} ({:?}) check failed: {error:#}", step.id, step.name),
            Err(payload) => format!(
                "step {} ({:?}) check panicked: {}",
                step.id,
                step.name,
                panic_message(payload)
            ),
        };
        state.fail(error.clone());
        return Ok(error_response(StatusCode::BAD_REQUEST, &error));
    }
    // A scripted tools_call must read the target's help first, because the
    // runtime refuses to execute a tool whose contract is not in context.
    // Hold this reply, answer the help round trip, then release it.
    if step.reply.reads_required_help() {
        let prepared = match step.reply.prepare(&observed, step.id, &state.options) {
            Ok(prepared) => prepared,
            Err(error) => {
                let error = format!(
                    "step {} ({:?}) reply is invalid: {error:#}",
                    step.id, step.name
                );
                state.fail(error.clone());
                return Ok(error_response(StatusCode::INTERNAL_SERVER_ERROR, &error));
            }
        };
        // Only tools whose contract is not already in this request need the
        // synthetic round: the runtime applies the same rule.
        let body = observed.body.to_string();
        let tools = step
            .reply
            .called_tools()
            .into_iter()
            .filter(|tool| !body.contains(&format!("Tool: {tool}")))
            .collect::<Vec<_>>();
        let help_calls = tools
            .iter()
            .enumerate()
            .map(|(index, tool)| {
                FunctionCall::new(
                    format!("call_help_gate_{}_{index}", step.id),
                    "tools_help",
                    json!({"tool": tool}),
                )
            })
            .collect::<Vec<_>>();
        if !help_calls.is_empty() {
            let help = Reply::function_calls(help_calls);
            match help.prepare(&observed, step.id, &state.options) {
                Ok(help) => {
                    // The step's own signals must end with this proxied round,
                    // or every `.after(&step)` dependency stays blocked
                    // forever. The real reply is a synthetic response that no
                    // step handle tracks.
                    claim_guard.handed_off = true;
                    step.signals.body_ended(true);
                    state.hold_help_answer(PendingHelpAnswer {
                        taught_tools: tools,
                        step_id: step.id,
                        planned: prepared,
                    });
                    return Ok(prepared_response(help, &state));
                }
                Err(error) => {
                    let error = format!(
                        "step {} ({:?}) help reply is invalid: {error:#}",
                        step.id, step.name
                    );
                    state.fail(error.clone());
                    return Ok(error_response(StatusCode::INTERNAL_SERVER_ERROR, &error));
                }
            }
        }
    }
    let prepared = match step.reply.prepare(&observed, step.id, &state.options) {
        Ok(prepared) => prepared,
        Err(error) => {
            let error = format!(
                "step {} ({:?}) reply is invalid: {error:#}",
                step.id, step.name
            );
            state.fail(error.clone());
            return Ok(error_response(StatusCode::INTERNAL_SERVER_ERROR, &error));
        }
    };
    // These status codes have no HTTP body, so a body stream cannot carry a gate.
    if prepared.status == StatusCode::NO_CONTENT || prepared.status == StatusCode::NOT_MODIFIED {
        step.signals.body_ended(true);
        let mut response = Response::new(Body::empty());
        *response.status_mut() = prepared.status;
        *response.headers_mut() = prepared.headers;
        return Ok(response);
    }
    let (tx, rx) = mpsc::channel(state.options.stream_buffer);
    let signals = step.signals.clone();
    let producer_state = state.clone();
    state.spawn_tracked(async move {
        let mut released = signals.released.subscribe();
        let count = prepared.chunks.len();
        for (index, chunk) in prepared.chunks.into_iter().enumerate() {
            if prepared.hold_after == Some(index)
                && !wait_release(&mut released, &tx, &producer_state).await
            {
                return;
            }
            let sent = tokio::select! {
                biased;
                _ = producer_state.shutdown.cancelled() => false,
                _ = producer_state.fatal.cancelled() => false,
                sent = tx.send(chunk) => sent.is_ok(),
            };
            if !sent {
                return;
            }
            signals
                .progress
                .send_modify(|progress| progress.chunks_sent += 1);
        }
        if prepared.hold_after == Some(count) {
            let _ = wait_release(&mut released, &tx, &producer_state).await;
        }
        // Dropping tx lets the HTTP body observe its planned EOF.
    });
    claim_guard.handed_off = true;
    let mut response = Response::new(Body::from_stream(ObservedBody {
        rx,
        signals: step.signals.clone(),
        ended: false,
    }));
    *response.status_mut() = prepared.status;
    *response.headers_mut() = prepared.headers;
    Ok(response)
}

/// Stream one prepared reply without a scenario step. Used for the synthetic
/// `tools_help` round and for re-issuing the held `tools_call` reply.
fn prepared_response(prepared: PreparedReply, state: &Arc<ModelState>) -> Response<Body> {
    let (tx, rx) = mpsc::channel(state.options.stream_buffer);
    let producer_state = state.clone();
    let signals = Arc::new(StepSignals::new());
    let producer_signals = signals.clone();
    state.spawn_tracked(async move {
        for chunk in prepared.chunks {
            let sent = tokio::select! {
                biased;
                _ = producer_state.shutdown.cancelled() => false,
                _ = producer_state.fatal.cancelled() => false,
                sent = tx.send(chunk) => sent.is_ok(),
            };
            if !sent {
                return;
            }
            producer_signals
                .progress
                .send_modify(|progress| progress.chunks_sent += 1);
        }
    });
    let mut response = Response::new(Body::from_stream(ObservedBody {
        rx,
        signals,
        ended: false,
    }));
    *response.status_mut() = prepared.status;
    *response.headers_mut() = prepared.headers;
    response
}

async fn wait_release(
    release: &mut watch::Receiver<bool>,
    tx: &mpsc::Sender<Bytes>,
    state: &ModelState,
) -> bool {
    loop {
        if *release.borrow() {
            return true;
        }
        tokio::select! {
            _ = state.shutdown.cancelled() => return false,
            _ = state.fatal.cancelled() => return false,
            _ = tx.closed() => return false,
            changed = release.changed() => {
                if changed.is_err() {
                    return false;
                }
            }
        }
    }
}

struct ResponseClaimGuard {
    signals: Arc<StepSignals>,
    handed_off: bool,
}

impl Drop for ResponseClaimGuard {
    fn drop(&mut self) {
        if !self.handed_off {
            self.signals.body_ended(false);
        }
    }
}

struct ObservedBody {
    rx: mpsc::Receiver<Bytes>,
    signals: Arc<StepSignals>,
    ended: bool,
}

impl Stream for ObservedBody {
    type Item = std::result::Result<Bytes, Infallible>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.rx.poll_recv(context) {
            Poll::Ready(None) => {
                self.ended = true;
                self.signals.body_ended(true);
                Poll::Ready(None)
            }
            Poll::Ready(Some(bytes)) => Poll::Ready(Some(Ok(bytes))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for ObservedBody {
    fn drop(&mut self) {
        if !self.ended {
            self.signals.body_ended(false);
        }
    }
}

fn json_response(status: StatusCode, value: Value) -> Response<Body> {
    let mut response = Response::new(Body::from(value.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response
}

fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    json_response(
        status,
        json!({"error": {
            "message": bounded_text(message, 8192),
            "type": "testkit_expectation_failed",
            "code": "testkit_expectation_failed"
        }}),
    )
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn diagnostic_list(items: impl IntoIterator<Item = String>) -> String {
    let mut shown = Vec::new();
    let mut omitted = 0;
    for item in items {
        if shown.len() < 4 {
            shown.push(bounded_text(&item, 512));
        } else {
            omitted += 1;
        }
    }
    if shown.is_empty() {
        return "none".into();
    }
    if omitted > 0 {
        shown.push(format!("{omitted} more omitted"));
    }
    shown.join("; ")
}

fn bounded_bytes(bytes: &[u8], limit: usize) -> String {
    bounded_text(
        &String::from_utf8_lossy(&bytes[..bytes.len().min(limit)]),
        limit,
    )
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".into()
    }
}

#[cfg(test)]
mod tests;
