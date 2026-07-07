//! Stdio transport — spawns a child process and frames JSON-RPC over its
//! stdin/stdout (LSP-style Content-Length). Supervised: when the child
//! exits, the transport reconnects according to the configured
//! [`crate::config::RestartPolicy`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::config::{RestartMode, RestartPolicy};
use crate::error::TransportError;
use crate::logs::PluginLogStore;
use crate::sdk::PluginError;
use crate::sdk::rpc::{
    ErrorObject, Frame, JsonRpcVersion, Notification, Request, RequestId, Response,
    ResponsePayload, method,
};
use crate::sdk::{
    PluginKey, ToolInvokeInput, ToolInvokeStreamHandle, ToolStreamChunk, ToolStreamEnd,
    ToolStreamError,
};
use crate::status::StatusRegistry;
use crate::transport::PluginTransport;

pub type HostHandler = Arc<
    dyn Fn(
            String,
            serde_json::Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, PluginError>> + Send>,
        > + Send
        + Sync,
>;

#[derive(Clone)]
struct SpawnSpec {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: Option<PathBuf>,
}

pub struct StdioTransport {
    inner: Arc<Inner>,
}

struct Inner {
    spawn_spec: SpawnSpec,
    restart_policy: RestartPolicy,
    /// Active child / stdin pair. `None` while a respawn is in flight.
    handles: Mutex<Option<ChildHandles>>,
    next_id: Mutex<i64>,
    pending: DashMap<RequestId, oneshot::Sender<Response>>,
    active_streams: Mutex<HashMap<String, ActiveStreamState>>,
    buffered_streams: Mutex<HashMap<String, Vec<BufferedStreamEvent>>>,
    host_handler: Mutex<Option<HostHandler>>,
    closed: std::sync::atomic::AtomicBool,
    restart_attempts: AtomicU32,
    plugin_id: Option<PluginKey>,
    status_sink: Option<Arc<StatusRegistry>>,
    log_sink: Option<Arc<PluginLogStore>>,
}

struct ChildHandles {
    child: Child,
    stdin: tokio::process::ChildStdin,
}

struct ActiveStreamState {
    chunks: mpsc::Sender<ToolStreamChunk>,
    end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
}

#[derive(Debug)]
enum BufferedStreamEvent {
    Chunk(ToolStreamChunk),
    End(ToolStreamEnd),
    Error(ToolStreamError),
}

impl StdioTransport {
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&PathBuf>,
        host_handler: Option<HostHandler>,
    ) -> Result<Self, TransportError> {
        Self::spawn_with_policy_and_status(
            command,
            args,
            env,
            cwd,
            host_handler,
            RestartPolicy::default(),
            None,
            None,
            None,
        )
        .await
    }

    pub async fn spawn_with_policy(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&PathBuf>,
        host_handler: Option<HostHandler>,
        restart_policy: RestartPolicy,
    ) -> Result<Self, TransportError> {
        Self::spawn_with_policy_and_status(
            command,
            args,
            env,
            cwd,
            host_handler,
            restart_policy,
            None,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_with_policy_and_status(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&PathBuf>,
        host_handler: Option<HostHandler>,
        restart_policy: RestartPolicy,
        plugin_id: Option<PluginKey>,
        status_sink: Option<Arc<StatusRegistry>>,
        log_sink: Option<Arc<PluginLogStore>>,
    ) -> Result<Self, TransportError> {
        let spawn_spec = SpawnSpec {
            command: command.to_string(),
            args: args.to_vec(),
            env: env.clone(),
            cwd: cwd.cloned(),
        };

        let inner = Arc::new(Inner {
            spawn_spec: spawn_spec.clone(),
            restart_policy,
            handles: Mutex::new(None),
            next_id: Mutex::new(1),
            pending: DashMap::new(),
            active_streams: Mutex::new(HashMap::new()),
            buffered_streams: Mutex::new(HashMap::new()),
            host_handler: Mutex::new(host_handler),
            closed: std::sync::atomic::AtomicBool::new(false),
            restart_attempts: AtomicU32::new(0),
            plugin_id,
            status_sink,
            log_sink,
        });

        Inner::spawn_child(&inner, false).await?;
        Ok(Self { inner })
    }

    pub async fn set_host_handler(&self, handler: HostHandler) {
        *self.inner.host_handler.lock().await = Some(handler);
    }
}

impl Inner {
    /// Spawn (or respawn) the child process. `is_restart` toggles backoff.
    fn spawn_child(
        self: &Arc<Self>,
        is_restart: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), TransportError>> + Send + '_>>
    {
        let this = Arc::clone(self);
        Box::pin(async move { this.spawn_child_inner(is_restart).await })
    }

    async fn spawn_child_inner(self: Arc<Self>, is_restart: bool) -> Result<(), TransportError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(TransportError::Disconnected);
        }
        if is_restart {
            let attempt = self.restart_attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt > self.restart_policy.max_retries {
                tracing::warn!(
                    target: "agena_plugin_host::stdio",
                    attempts = attempt,
                    max = self.restart_policy.max_retries,
                    "stdio plugin exhausted restart budget"
                );
                self.record_status(|sink, plugin_id| {
                    sink.record_spawn_failure(plugin_id, "restart budget exhausted");
                });
                self.record_log(
                    "warn",
                    "host",
                    "restart budget exhausted",
                    serde_json::Value::Null,
                );
                return Err(TransportError::Disconnected);
            }
            let min = self.restart_policy.min_backoff.0;
            let max = self.restart_policy.max_backoff.0;
            let backoff = exp_backoff(min, max, attempt);
            tracing::info!(
                target: "agena_plugin_host::stdio",
                attempt,
                backoff_ms = backoff.as_millis() as u64,
                "respawning stdio plugin after exit"
            );
            tokio::time::sleep(backoff).await;
        }

        let mut cmd = Command::new(&self.spawn_spec.command);
        cmd.args(&self.spawn_spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &self.spawn_spec.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = &self.spawn_spec.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                let message = err.to_string();
                self.record_status(|sink, plugin_id| {
                    sink.record_spawn_failure(plugin_id, message.clone());
                });
                self.record_log(
                    "error",
                    "host",
                    format!("spawn failed: {message}"),
                    serde_json::Value::Null,
                );
                return Err(err.into());
            }
        };
        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| TransportError::Io("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::Io("no stdout".into()))?;
        let stderr = child.stderr.take();

        // stdout reader
        {
            let this = Arc::clone(&self);
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_frame(&mut reader).await {
                        Ok(Some(frame)) => {
                            this.handle_inbound(frame).await;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            tracing::warn!(
                                target: "agena_plugin_host::stdio",
                                "stdio read error: {e}"
                            );
                            break;
                        }
                    }
                }
                // Reader exited -> child likely gone. Tear down and respawn.
                this.handle_child_exit().await;
            });
        }

        // stderr drain → tracing
        if let Some(stderr) = stderr {
            let plugin_id = self.plugin_id.clone();
            let log_sink = self.log_sink.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::info!(target: "agena_plugin_host::stdio_err", "{line}");
                    if let (Some(sink), Some(plugin_id)) = (log_sink.as_ref(), plugin_id.as_ref()) {
                        sink.append(
                            plugin_id,
                            "info",
                            "stderr",
                            line.clone(),
                            serde_json::Value::Null,
                        );
                    }
                }
            });
        }

        *self.handles.lock().await = Some(ChildHandles { child, stdin });
        self.record_status(|sink, plugin_id| {
            sink.record_started(plugin_id, pid, is_restart);
        });
        self.record_log(
            "info",
            "host",
            if is_restart {
                format!(
                    "plugin started after restart (pid={})",
                    pid.unwrap_or_default()
                )
            } else {
                format!("plugin started (pid={})", pid.unwrap_or_default())
            },
            serde_json::Value::Null,
        );
        Ok(())
    }

    async fn handle_child_exit(self: Arc<Self>) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        // Drop current handles + fail every in-flight request.
        let exit_code = {
            let mut handles_lock = self.handles.lock().await;
            let exit_code = if let Some(handles) = handles_lock.as_mut() {
                handles
                    .child
                    .try_wait()
                    .ok()
                    .flatten()
                    .and_then(|status| status.code())
            } else {
                None
            };
            *handles_lock = None;
            exit_code
        };
        let pending: Vec<_> = self.pending.iter().map(|e| e.key().clone()).collect();
        for id in pending {
            if let Some((_, slot)) = self.pending.remove(&id) {
                let _ = slot.send(Response {
                    jsonrpc: JsonRpcVersion,
                    id: id.clone(),
                    payload: ResponsePayload::Err {
                        error: ErrorObject {
                            code: crate::sdk::rpc::codes::PLUGIN_DISCONNECTED,
                            message: "plugin disconnected".into(),
                            data: None,
                        },
                    },
                });
            }
        }
        self.fail_active_streams(PluginError::new("plugin disconnected"))
            .await;

        let will_restart = matches!(
            self.restart_policy.policy,
            RestartMode::OnFailure | RestartMode::Always
        );
        self.record_status(|sink, plugin_id| {
            sink.record_exit(plugin_id, will_restart, exit_code, None);
        });
        self.record_log(
            if will_restart { "warn" } else { "error" },
            "host",
            if will_restart {
                format!(
                    "plugin exited with code {}; scheduling restart",
                    exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "<unknown>".into())
                )
            } else {
                format!(
                    "plugin exited with code {}",
                    exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "<unknown>".into())
                )
            },
            serde_json::Value::Null,
        );

        match self.restart_policy.policy {
            RestartMode::Never => {}
            RestartMode::OnFailure | RestartMode::Always => {
                let inner = Arc::clone(&self);
                tokio::spawn(async move {
                    if let Err(err) = Inner::spawn_child(&inner, true).await {
                        tracing::warn!(
                            target: "agena_plugin_host::stdio",
                            "respawn failed: {err}"
                        );
                    }
                });
            }
        }
    }

    async fn handle_inbound(self: &Arc<Self>, frame: Frame) {
        match frame {
            Frame::Response(resp) => {
                if let Some((_, tx)) = self.pending.remove(&resp.id) {
                    let _ = tx.send(resp);
                }
            }
            Frame::Request(req) => {
                let inner = Arc::clone(self);
                tokio::spawn(async move {
                    let id = req.id.clone();
                    let result = if let Some(handler) = inner.host_handler.lock().await.clone() {
                        handler(req.method, req.params.unwrap_or(serde_json::Value::Null)).await
                    } else {
                        Err(PluginError {
                            code: crate::sdk::PluginErrorCode::HostUnavailable,
                            message: "no host handler installed".into(),
                            hook: None,
                            plugin: None,
                            data: None,
                        })
                    };
                    let resp = match result {
                        Ok(v) => Response {
                            jsonrpc: JsonRpcVersion,
                            id,
                            payload: ResponsePayload::Ok { result: v },
                        },
                        Err(e) => Response {
                            jsonrpc: JsonRpcVersion,
                            id,
                            payload: ResponsePayload::Err {
                                error: ErrorObject {
                                    code: crate::sdk::rpc::codes::PLUGIN_GENERIC,
                                    message: e.message.clone(),
                                    data: serde_json::to_value(&e).ok(),
                                },
                            },
                        },
                    };
                    let body = serde_json::to_vec(&resp).expect("response serialize");
                    let _ = inner.write_frame(&body).await;
                });
            }
            Frame::Notification(notif) => {
                if matches!(
                    notif.method.as_str(),
                    method::TOOL_STREAM_CHUNK | method::TOOL_STREAM_END | method::TOOL_STREAM_ERROR
                ) {
                    self.clone().handle_notification(notif).await;
                } else {
                    let inner = Arc::clone(self);
                    tokio::spawn(async move {
                        inner.handle_notification(notif).await;
                    });
                }
            }
        }
    }

    async fn write_frame(&self, body: &[u8]) -> Result<(), TransportError> {
        let mut handles = self.handles.lock().await;
        let h = handles.as_mut().ok_or(TransportError::Disconnected)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        h.stdin.write_all(header.as_bytes()).await?;
        h.stdin.write_all(body).await?;
        h.stdin.flush().await?;
        Ok(())
    }

    async fn next_id(&self) -> i64 {
        let mut id = self.next_id.lock().await;
        let v = *id;
        *id += 1;
        v
    }

    fn record_status<F>(&self, mutate: F)
    where
        F: FnOnce(&StatusRegistry, &PluginKey),
    {
        if let (Some(sink), Some(plugin_id)) = (self.status_sink.as_ref(), self.plugin_id.as_ref())
        {
            mutate(sink.as_ref(), plugin_id);
        }
    }

    fn record_log(
        &self,
        level: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
        fields: serde_json::Value,
    ) {
        if let (Some(sink), Some(plugin_id)) = (self.log_sink.as_ref(), self.plugin_id.as_ref()) {
            sink.append(
                plugin_id,
                level.into(),
                source.into(),
                message.into(),
                fields,
            );
        }
    }

    async fn handle_notification(self: Arc<Self>, notif: Notification) {
        match notif.method.as_str() {
            method::TOOL_STREAM_CHUNK => {
                if let Some(chunk) = parse_notification::<ToolStreamChunk>(&notif) {
                    self.deliver_stream_chunk(chunk).await;
                }
            }
            method::TOOL_STREAM_END => {
                if let Some(end) = parse_notification::<ToolStreamEnd>(&notif) {
                    self.finish_stream(end.stream_id.clone(), Ok(end)).await;
                }
            }
            method::TOOL_STREAM_ERROR => {
                if let Some(err) = parse_notification::<ToolStreamError>(&notif) {
                    self.finish_stream(err.stream_id.clone(), Err(err.error))
                        .await;
                }
            }
            _ => {
                if let Some(handler) = self.host_handler.lock().await.clone() {
                    let method = notif.method;
                    let params = notif.params.unwrap_or(serde_json::Value::Null);
                    let _ = handler(method, params).await;
                }
            }
        }
    }

    async fn deliver_stream_chunk(&self, chunk: ToolStreamChunk) {
        let stream_id = chunk.stream_id.clone();
        let sender = {
            let active = self.active_streams.lock().await;
            active.get(&stream_id).map(|state| state.chunks.clone())
        };
        if let Some(sender) = sender {
            let _ = sender.send(chunk).await;
            return;
        }
        let mut buffered = self.buffered_streams.lock().await;
        buffered
            .entry(stream_id)
            .or_default()
            .push(BufferedStreamEvent::Chunk(chunk));
    }

    async fn finish_stream(&self, stream_id: String, result: Result<ToolStreamEnd, PluginError>) {
        let state = {
            let mut active = self.active_streams.lock().await;
            active.remove(&stream_id)
        };
        if let Some(state) = state {
            let _ = state.end.send(result);
            return;
        }
        let event = match result {
            Ok(end) => BufferedStreamEvent::End(end),
            Err(error) => BufferedStreamEvent::Error(ToolStreamError {
                stream_id: stream_id.clone(),
                error,
            }),
        };
        let mut buffered = self.buffered_streams.lock().await;
        buffered.entry(stream_id).or_default().push(event);
    }

    async fn register_stream(
        &self,
        stream_id: String,
        chunks: mpsc::Sender<ToolStreamChunk>,
        end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
    ) {
        {
            let mut active = self.active_streams.lock().await;
            active.insert(stream_id.clone(), ActiveStreamState { chunks, end });
        }
        self.drain_buffered_stream_events(stream_id).await;
    }

    async fn fail_active_streams(&self, error: PluginError) {
        let active = {
            let mut active = self.active_streams.lock().await;
            active.drain().map(|(_, state)| state).collect::<Vec<_>>()
        };
        {
            let mut buffered = self.buffered_streams.lock().await;
            buffered.clear();
        }
        for state in active {
            let _ = state.end.send(Err(error.clone()));
        }
    }

    async fn drain_buffered_stream_events(&self, stream_id: String) {
        let events = {
            let mut buffered = self.buffered_streams.lock().await;
            buffered.remove(&stream_id).unwrap_or_default()
        };
        for event in events {
            match event {
                BufferedStreamEvent::Chunk(chunk) => self.deliver_stream_chunk(chunk).await,
                BufferedStreamEvent::End(end) => {
                    self.finish_stream(stream_id.clone(), Ok(end)).await;
                    break;
                }
                BufferedStreamEvent::Error(err) => {
                    self.finish_stream(stream_id.clone(), Err(err.error)).await;
                    break;
                }
            }
        }
    }
}

fn parse_notification<T: serde::de::DeserializeOwned>(notif: &Notification) -> Option<T> {
    serde_json::from_value(notif.params.clone().unwrap_or(serde_json::Value::Null)).ok()
}

fn exp_backoff(min: Duration, max: Duration, attempt: u32) -> Duration {
    let factor = 1u64
        .checked_shl(attempt.saturating_sub(1).min(10))
        .unwrap_or(1);
    let scaled = min.checked_mul(factor as u32).unwrap_or(max);
    scaled.min(max)
}

async fn read_frame<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> Result<Option<Frame>, TransportError> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                rest.trim()
                    .parse()
                    .map_err(|_| TransportError::Rpc("bad Content-Length".into()))?,
            );
        }
    }
    let len = content_length.ok_or_else(|| TransportError::Rpc("missing Content-Length".into()))?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    let frame: Frame = serde_json::from_slice(&buf)?;
    Ok(Some(frame))
}

#[async_trait]
impl PluginTransport for StdioTransport {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let id = self.inner.next_id().await;
        let req_id = RequestId::Num(id);
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: req_id.clone(),
            method: method.to_string(),
            params: Some(params),
        };
        let body = serde_json::to_vec(&req)?;
        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(req_id.clone(), tx);
        if let Err(e) = self.inner.write_frame(&body).await {
            self.inner.pending.remove(&req_id);
            return Err(e);
        }
        let resp = rx.await.map_err(|_| TransportError::Disconnected)?;
        match resp.payload {
            ResponsePayload::Ok { result } => Ok(result),
            ResponsePayload::Err { error } => {
                let pe: Option<PluginError> = error
                    .data
                    .as_ref()
                    .and_then(|d| serde_json::from_value(d.clone()).ok());
                let pe = pe.unwrap_or(PluginError {
                    code: crate::sdk::PluginErrorCode::Generic,
                    message: error.message,
                    hook: None,
                    plugin: None,
                    data: error.data,
                });
                Err(TransportError::Plugin(pe))
            }
        }
    }

    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), TransportError> {
        let n = Notification {
            jsonrpc: JsonRpcVersion,
            method: method.to_string(),
            params: Some(params),
        };
        let body = serde_json::to_vec(&n)?;
        self.inner.write_frame(&body).await
    }

    async fn invoke_stream(
        &self,
        input: ToolInvokeInput,
    ) -> Result<Option<crate::transport::ToolStreamHandle>, TransportError> {
        let id = self.inner.next_id().await;
        let req_id = RequestId::Num(id);
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: req_id.clone(),
            method: method::HOOK_TOOL_INVOKE_STREAM.to_string(),
            params: Some(serde_json::to_value(&input)?),
        };
        let body = serde_json::to_vec(&req)?;
        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(req_id.clone(), tx);
        if let Err(e) = self.inner.write_frame(&body).await {
            self.inner.pending.remove(&req_id);
            return Err(e);
        }
        let resp = rx.await.map_err(|_| TransportError::Disconnected)?;
        let handle: ToolInvokeStreamHandle = match resp.payload {
            ResponsePayload::Ok { result } => {
                serde_json::from_value(result).map_err(|e| TransportError::Rpc(e.to_string()))?
            }
            ResponsePayload::Err { error } => {
                let pe: Option<PluginError> = error
                    .data
                    .as_ref()
                    .and_then(|d| serde_json::from_value(d.clone()).ok());
                let pe = pe.unwrap_or(PluginError {
                    code: crate::sdk::PluginErrorCode::Generic,
                    message: error.message,
                    hook: None,
                    plugin: None,
                    data: error.data,
                });
                return Err(TransportError::Plugin(pe));
            }
        };

        let (chunk_tx, chunk_rx) = mpsc::channel::<ToolStreamChunk>(64);
        let (end_tx, end_rx) = oneshot::channel::<Result<ToolStreamEnd, PluginError>>();
        self.inner
            .register_stream(handle.stream_id.clone(), chunk_tx, end_tx)
            .await;
        Ok(Some(crate::transport::ToolStreamHandle {
            stream_id: handle.stream_id,
            chunks: chunk_rx,
            end: end_rx,
        }))
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.inner
            .fail_active_streams(PluginError::new("plugin transport closed"))
            .await;
        if let Some(mut h) = self.inner.handles.lock().await.take() {
            let _ = h.child.start_kill();
        }
        self.inner.record_status(|sink, plugin_id| {
            sink.record_stopped(plugin_id);
        });
        self.inner.record_log(
            "info",
            "host",
            "plugin transport closed",
            serde_json::Value::Null,
        );
        Ok(())
    }
}
