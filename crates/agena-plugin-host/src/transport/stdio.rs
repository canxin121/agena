//! Stdio transport — spawns a child process and frames JSON-RPC over its
//! stdin/stdout (LSP-style Content-Length). Supervised: when the child
//! exits, the transport reconnects according to the configured
//! [`crate::config::RestartPolicy`].

use portable_atomic::{AtomicI64, AtomicU64};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use agena_process::ManagedChild;
use agena_stdio_codec::ContentLengthCodec;
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
use tokio_util::codec::{FramedRead, FramedWrite};
use tokio_util::sync::CancellationToken;

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

const HOST_CALLBACK_CONCURRENCY: usize = 64;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_BUFFERED_STREAMS: usize = 128;
const MAX_BUFFERED_STREAM_EVENTS: usize = 64;

#[derive(Clone)]
struct SpawnSpec {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: Option<PathBuf>,
}

/// Plugin transport over a stdio subprocess.
pub struct StdioTransport {
    inner: Arc<Inner>,
}

struct Inner {
    spawn_spec: SpawnSpec,
    restart_policy: RestartPolicy,
    /// Active child and its writer mailbox. `None` while a respawn is in
    /// flight. The writer task owns stdin so lifecycle code never waits for
    /// pipe I/O while holding this mutex.
    handles: Mutex<Option<ChildHandles>>,
    /// Serialize spawn/backoff/install against close. Reader tasks carry a
    /// generation and may only tear down the child they were created for.
    spawn_lock: Mutex<()>,
    child_generation: AtomicU64,
    next_id: AtomicI64,
    pending: DashMap<RequestId, oneshot::Sender<Response>>,
    active_streams: Mutex<HashMap<String, ActiveStreamState>>,
    buffered_streams: Mutex<HashMap<String, Vec<BufferedStreamEvent>>>,
    host_handler: Mutex<Option<HostHandler>>,
    host_call_slots: Arc<Semaphore>,
    shutdown: CancellationToken,
    closed: std::sync::atomic::AtomicBool,
    restart_attempts: AtomicU32,
    plugin_id: Option<PluginKey>,
    status_sink: Option<Arc<StatusRegistry>>,
    log_sink: Option<Arc<PluginLogStore>>,
}

struct ChildHandles {
    generation: u64,
    child: ManagedChild,
    writer: mpsc::Sender<WriteRequest>,
}

struct WriteRequest {
    body: Bytes,
    completion: oneshot::Sender<Result<(), String>>,
}

struct ActiveStreamState {
    chunks: mpsc::Sender<ToolStreamChunk>,
    end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
    monitor_stop: CancellationToken,
}

/// Remove an in-flight request slot when its dispatch future is dropped by a
/// timeout or execution cancellation. The plugin may still send a late JSON-
/// RPC response, but it can no longer leak an orphaned sender in the host.
struct PendingRequestGuard {
    inner: Arc<Inner>,
    request_id: RequestId,
}

impl PendingRequestGuard {
    fn new(inner: Arc<Inner>, request_id: RequestId) -> Self {
        Self { inner, request_id }
    }
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        self.inner.pending.remove(&self.request_id);
    }
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
            spawn_lock: Mutex::new(()),
            child_generation: AtomicU64::new(0),
            next_id: AtomicI64::new(1),
            pending: DashMap::new(),
            active_streams: Mutex::new(HashMap::new()),
            buffered_streams: Mutex::new(HashMap::new()),
            host_handler: Mutex::new(host_handler),
            host_call_slots: Arc::new(Semaphore::new(HOST_CALLBACK_CONCURRENCY)),
            shutdown: CancellationToken::new(),
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
        let _spawn_guard = self.spawn_lock.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err(TransportError::disconnected(
                "stdio plugin transport is already closed",
            ));
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
                return Err(TransportError::disconnected(format!(
                    "stdio plugin exhausted its restart budget after {attempt} attempts (maximum {})",
                    self.restart_policy.max_retries
                )));
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
            tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => return Err(TransportError::disconnected(
                    "stdio plugin transport was shut down during restart backoff",
                )),
                _ = tokio::time::sleep(backoff) => {}
            }
            if self.closed.load(Ordering::SeqCst) {
                return Err(TransportError::disconnected(
                    "stdio plugin transport closed before restart could spawn the child",
                ));
            }
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
        let mut child = match agena_process::spawn(cmd) {
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
            .stdin()
            .take()
            .ok_or_else(|| TransportError::Io("no stdin".into()))?;
        let stdout = child
            .stdout()
            .take()
            .ok_or_else(|| TransportError::Io("no stdout".into()))?;
        let stderr = child.stderr().take();
        let generation = self.child_generation.fetch_add(1, Ordering::SeqCst) + 1;

        // One task owns stdin for the lifetime of this child. A bounded
        // mailbox preserves frame ordering and applies backpressure without
        // coupling a blocked pipe write to the child/restart state mutex.
        let (writer, mut write_requests) = mpsc::channel::<WriteRequest>(64);
        let writer_shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            let mut stdin = FramedWrite::new(stdin, ContentLengthCodec::new(MAX_FRAME_BYTES));
            loop {
                let request = tokio::select! {
                    biased;
                    _ = writer_shutdown.cancelled() => return,
                    request = write_requests.recv() => request,
                };
                let Some(request) = request else { return };
                let write = async {
                    tokio::select! {
                        biased;
                        _ = writer_shutdown.cancelled() => {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::Interrupted,
                                "plugin transport closed",
                            ))
                        }
                        result = stdin.send(request.body) => {
                            result.map_err(std::io::Error::other)
                        },
                    }
                };
                let result = tokio::time::timeout(WRITE_TIMEOUT, write)
                    .await
                    .map_err(|error| {
                        agena_failure::diagnostic::format_error_chain_with_context(
                            format!(
                                "plugin stdin write timed out after {}ms",
                                WRITE_TIMEOUT.as_millis()
                            ),
                            &error,
                        )
                    })
                    .and_then(|result| {
                        result.map_err(|error| {
                            agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to write a frame to plugin stdin",
                                &error,
                            )
                        })
                    });
                let failed = result.is_err();
                if request.completion.send(result).is_err() {
                    tracing::debug!(
                        target: "agena_plugin_host::stdio",
                        "plugin stdin write-result receiver was dropped"
                    );
                }
                if failed {
                    break;
                }
            }
        });

        // Publish lifecycle state before a short-lived child can make its
        // stdout reader observe EOF. Otherwise the exit task can run while
        // handles is still None, then startup publishes an already-dead child.
        *self.handles.lock().await = Some(ChildHandles {
            generation,
            child,
            writer,
        });

        // stdout reader
        {
            let this = Arc::clone(&self);
            tokio::spawn(async move {
                let mut reader = FramedRead::new(stdout, ContentLengthCodec::new(MAX_FRAME_BYTES));
                loop {
                    let frame = tokio::select! {
                        biased;
                        _ = this.shutdown.cancelled() => break,
                        frame = reader.next() => frame,
                    };
                    match frame {
                        Some(Ok(body)) => match serde_json::from_slice::<Frame>(body.as_ref()) {
                            Ok(frame) => this.handle_inbound(frame).await,
                            Err(error) => {
                                tracing::warn!(
                                    target: "agena_plugin_host::stdio",
                                    "stdio JSON-RPC decode error: {error}"
                                );
                                break;
                            }
                        },
                        None => break,
                        Some(Err(error)) => {
                            tracing::warn!(
                                target: "agena_plugin_host::stdio",
                                "stdio framing error: {error}"
                            );
                            break;
                        }
                    }
                }
                // Reader exited -> child likely gone. Tear down and respawn.
                this.handle_child_exit(generation).await;
            });
        }

        // stderr drain → tracing
        if let Some(stderr) = stderr {
            let plugin_id = self.plugin_id.clone();
            let log_sink = self.log_sink.clone();
            let shutdown = self.shutdown.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                loop {
                    let line = tokio::select! {
                        biased;
                        _ = shutdown.cancelled() => return,
                        line = reader.next_line() => line,
                    };
                    let Ok(Some(line)) = line else { return };
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

    async fn handle_child_exit(self: Arc<Self>, generation: u64) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        // Drop current handles + fail every in-flight request.
        let Some(exit_code) = ({
            let mut handles_lock = self.handles.lock().await;
            match handles_lock.as_mut() {
                Some(handles) if handles.generation == generation => {
                    let exit_code = match handles.child.try_wait() {
                        Ok(status) => status.and_then(|status| status.code()),
                        Err(error) => {
                            self.record_log(
                                "error",
                                "host",
                                agena_failure::diagnostic::format_error_chain_with_context(
                                    "inspect the exited stdio plugin process",
                                    &error,
                                ),
                                serde_json::Value::Null,
                            );
                            None
                        }
                    };
                    *handles_lock = None;
                    Some(exit_code)
                }
                // A stale reader must never tear down a newer generation.
                _ => None,
            }
        }) else {
            return;
        };
        self.fail_pending("plugin disconnected");
        self.fail_active_streams(PluginError::internal("plugin disconnected"))
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
                if let Err(err) = Inner::spawn_child(&self, true).await {
                    tracing::warn!(
                        target: "agena_plugin_host::stdio",
                        "respawn failed: {err}"
                    );
                }
            }
        }
    }

    async fn handle_inbound(self: &Arc<Self>, frame: Frame) {
        match frame {
            Frame::Response(resp) => {
                if let Some((_, tx)) = self.pending.remove(&resp.id) {
                    if tx.send(resp).is_err() {
                        tracing::debug!(
                            target: "agena_plugin_host::stdio",
                            "plugin response receiver was dropped before delivery"
                        );
                    }
                }
            }
            Frame::Request(req) => {
                let callback_slot = Arc::clone(&self.host_call_slots).try_acquire_owned();
                let Ok(callback_slot) = callback_slot else {
                    let response = Response {
                        jsonrpc: JsonRpcVersion,
                        id: req.id,
                        payload: ResponsePayload::Err {
                            error: ErrorObject {
                                code: crate::sdk::rpc::codes::PLUGIN_GENERIC,
                                message: "host callback capacity exhausted".to_string(),
                                data: None,
                            },
                        },
                    };
                    match serde_json::to_vec(&response) {
                        Ok(body) => {
                            if let Err(error) = self.write_frame(&body).await {
                                tracing::warn!(
                                    target: "agena_plugin_host::stdio",
                                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                        "write the plugin host-capacity error response",
                                        &error,
                                    ),
                                    "plugin host-capacity error response could not be delivered"
                                );
                            }
                        }
                        Err(error) => {
                            tracing::error!(
                                target: "agena_plugin_host::stdio",
                                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                    "serialize the plugin host-capacity error response",
                                    &error,
                                ),
                                "plugin host-capacity error response could not be encoded"
                            );
                        }
                    }
                    return;
                };
                let inner = Arc::clone(self);
                tokio::spawn(async move {
                    let _callback_slot = callback_slot;
                    let id = req.id.clone();
                    let callback = async {
                        let handler = { inner.host_handler.lock().await.clone() };
                        if let Some(handler) = handler {
                            handler(req.method, req.params.unwrap_or(serde_json::Value::Null)).await
                        } else {
                            Err(PluginError::from_kind(
                                crate::sdk::PluginErrorKind::HostUnavailable,
                                "no host handler installed",
                            ))
                        }
                    };
                    let result = tokio::select! {
                        biased;
                        _ = inner.shutdown.cancelled() => return,
                        result = callback => result,
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
                                    message: e.to_string(),
                                    data: e.rpc_error_data(),
                                },
                            },
                        },
                    };
                    let body = match serde_json::to_vec(&resp) {
                        Ok(body) => body,
                        Err(error) => {
                            tracing::error!(
                                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                    "serialize host callback response for stdio plugin",
                                    &error,
                                ),
                                "stdio plugin host callback response was not sent"
                            );
                            return;
                        }
                    };
                    if let Err(error) = inner.write_frame(&body).await {
                        tracing::error!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "write host callback response to stdio plugin",
                                &error,
                            ),
                            "stdio plugin host callback response write failed"
                        );
                    }
                });
            }
            Frame::Notification(notif) => {
                if matches!(
                    notif.method.as_str(),
                    method::TOOL_STREAM_CHUNK | method::TOOL_STREAM_END | method::TOOL_STREAM_ERROR
                ) {
                    self.clone().handle_notification(notif).await;
                } else {
                    let callback_slot = Arc::clone(&self.host_call_slots).try_acquire_owned();
                    let Ok(callback_slot) = callback_slot else {
                        return;
                    };
                    let inner = Arc::clone(self);
                    tokio::spawn(async move {
                        let _callback_slot = callback_slot;
                        inner.handle_notification(notif).await;
                    });
                }
            }
        }
    }

    async fn write_frame(&self, body: &[u8]) -> Result<(), TransportError> {
        let writer = {
            let handles = self.handles.lock().await;
            handles
                .as_ref()
                .map(|handles| handles.writer.clone())
                .ok_or_else(|| {
                    TransportError::disconnected("stdio plugin child has no active stdin writer")
                })?
        };
        let (completion, result) = oneshot::channel();
        let write = async {
            writer
                .send(WriteRequest {
                    body: Bytes::copy_from_slice(body),
                    completion,
                })
                .await
                .map_err(|error| {
                    TransportError::disconnected_error(
                        "stdio plugin stdin writer queue closed before accepting a frame",
                        &error,
                    )
                })?;
            result
                .await
                .map_err(|error| {
                    TransportError::disconnected_error(
                        "stdio plugin stdin writer exited before confirming a frame",
                        &error,
                    )
                })?
                .map_err(TransportError::Io)
        };
        tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => Err(TransportError::disconnected(
                "stdio plugin transport was shut down while writing a frame",
            )),
            result = tokio::time::timeout(WRITE_TIMEOUT, write) => {
                result.map_err(|error| {
                    TransportError::Io(
                        agena_failure::diagnostic::format_error_chain_with_context(
                            format!(
                                "stdio plugin frame write timed out after {}ms",
                                WRITE_TIMEOUT.as_millis()
                            ),
                            &error,
                        ),
                    )
                })?
            }
        }
    }

    fn next_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn fail_pending(&self, message: &str) {
        let pending = self
            .pending
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for id in pending {
            if let Some((_, slot)) = self.pending.remove(&id) {
                if slot
                    .send(Response {
                        jsonrpc: JsonRpcVersion,
                        id,
                        payload: ResponsePayload::Err {
                            error: ErrorObject {
                                code: crate::sdk::rpc::codes::PLUGIN_DISCONNECTED,
                                message: message.to_string(),
                                data: None,
                            },
                        },
                    })
                    .is_err()
                {
                    tracing::debug!(
                        target: "agena_plugin_host::stdio",
                        "disconnected plugin request receiver was already dropped"
                    );
                }
            }
        }
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
                let handler = { self.host_handler.lock().await.clone() };
                if let Some(handler) = handler {
                    let method = notif.method;
                    let params = notif.params.unwrap_or(serde_json::Value::Null);
                    tokio::select! {
                        biased;
                        _ = self.shutdown.cancelled() => {}
                        _ = handler(method, params) => {}
                    }
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
            match sender.try_send(chunk) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    self.remove_abandoned_stream(stream_id.as_str()).await;
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    let state = {
                        let mut active = self.active_streams.lock().await;
                        active.remove(stream_id.as_str())
                    };
                    if let Some(state) = state {
                        state.monitor_stop.cancel();
                        if state
                            .end
                            .send(Err(PluginError::internal(
                                "plugin stream consumer exceeded the 64-chunk buffer",
                            )))
                            .is_err()
                        {
                            tracing::debug!(
                                target: "agena_plugin_host::stdio",
                                stream_id,
                                "overflowed stdio plugin stream terminal receiver was already dropped"
                            );
                        }
                    }
                }
            }
            return;
        }
        let mut buffered = self.buffered_streams.lock().await;
        if !buffered.contains_key(stream_id.as_str()) && buffered.len() >= MAX_BUFFERED_STREAMS {
            tracing::warn!(
                target: "agena_plugin_host::stdio",
                stream_id,
                "dropping an event for an unknown plugin stream because the pre-registration buffer is full"
            );
            return;
        }
        let events = buffered.entry(stream_id.clone()).or_default();
        if events.len() >= MAX_BUFFERED_STREAM_EVENTS {
            events.clear();
            events.push(BufferedStreamEvent::Error(ToolStreamError {
                stream_id,
                error: PluginError::internal(
                    "plugin stream exceeded the 64-event pre-registration buffer",
                ),
            }));
        } else {
            events.push(BufferedStreamEvent::Chunk(chunk));
        }
    }

    async fn finish_stream(&self, stream_id: String, result: Result<ToolStreamEnd, PluginError>) {
        let state = {
            let mut active = self.active_streams.lock().await;
            active.remove(&stream_id)
        };
        if let Some(state) = state {
            state.monitor_stop.cancel();
            if state.end.send(result).is_err() {
                tracing::debug!(
                    target: "agena_plugin_host::stdio",
                    stream_id,
                    "stdio plugin stream terminal-result receiver was dropped"
                );
            }
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
        if !buffered.contains_key(stream_id.as_str()) && buffered.len() >= MAX_BUFFERED_STREAMS {
            tracing::warn!(
                target: "agena_plugin_host::stdio",
                stream_id,
                "dropping a terminal event for an unknown plugin stream because the pre-registration buffer is full"
            );
            return;
        }
        let events = buffered.entry(stream_id).or_default();
        if events.len() >= MAX_BUFFERED_STREAM_EVENTS {
            events.clear();
        }
        events.push(event);
    }

    async fn register_stream(
        self: &Arc<Self>,
        stream_id: String,
        chunks: mpsc::Sender<ToolStreamChunk>,
        end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
    ) {
        let monitor_stop = CancellationToken::new();
        {
            let mut active = self.active_streams.lock().await;
            active.insert(
                stream_id.clone(),
                ActiveStreamState {
                    chunks: chunks.clone(),
                    end,
                    monitor_stop: monitor_stop.clone(),
                },
            );
        }
        let weak = Arc::downgrade(self);
        let shutdown = self.shutdown.clone();
        let abandoned_stream_id = stream_id.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = monitor_stop.cancelled() => {}
                _ = shutdown.cancelled() => {}
                _ = chunks.closed() => {
                    if let Some(inner) = weak.upgrade() {
                        inner.remove_abandoned_stream(abandoned_stream_id.as_str()).await;
                    }
                }
            }
        });
        self.drain_buffered_stream_events(stream_id).await;
    }

    async fn remove_abandoned_stream(&self, stream_id: &str) {
        if let Some(state) = self.active_streams.lock().await.remove(stream_id) {
            state.monitor_stop.cancel();
        }
        self.buffered_streams.lock().await.remove(stream_id);
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
            state.monitor_stop.cancel();
            if state.end.send(Err(error.clone())).is_err() {
                tracing::debug!(
                    target: "agena_plugin_host::stdio",
                    "failed stdio plugin stream receiver was already dropped during transport shutdown"
                );
            }
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
    match serde_json::from_value(notif.params.clone().unwrap_or(serde_json::Value::Null)) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(
                method = %notif.method,
                notification_type = %std::any::type_name::<T>(),
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode a stdio plugin notification",
                    &error,
                ),
                "malformed stdio plugin notification was rejected"
            );
            None
        }
    }
}

fn exp_backoff(min: Duration, max: Duration, attempt: u32) -> Duration {
    let factor = 1u64
        .checked_shl(attempt.saturating_sub(1).min(10))
        .unwrap_or(1);
    let scaled = min.checked_mul(factor as u32).unwrap_or(max);
    scaled.min(max)
}

#[async_trait]
impl PluginTransport for StdioTransport {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let id = self.inner.next_id();
        let req_id = RequestId::Num(id);
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: req_id.clone(),
            method: method.to_string(),
            params: Some(params),
            context: crate::sdk::host_api::current_host_callback_context(),
        };
        let body = serde_json::to_vec(&req)?;
        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(req_id.clone(), tx);
        let _pending_guard = PendingRequestGuard::new(Arc::clone(&self.inner), req_id.clone());
        self.inner.write_frame(&body).await?;
        let resp = rx.await.map_err(|error| {
            TransportError::disconnected_error(
                format!(
                    "stdio plugin response channel closed before method `{method}` request {id} completed"
                ),
                &error,
            )
        })?;
        match resp.payload {
            ResponsePayload::Ok { result } => Ok(result),
            ResponsePayload::Err { error } => {
                let pe = super::plugin_error_from_rpc(error, "decode stdio plugin dispatch error");
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
        let id = self.inner.next_id();
        let req_id = RequestId::Num(id);
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: req_id.clone(),
            method: method::HOOK_TOOL_INVOKE_STREAM.to_string(),
            params: Some(serde_json::to_value(&input)?),
            context: crate::sdk::host_api::current_host_callback_context(),
        };
        let body = serde_json::to_vec(&req)?;
        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(req_id.clone(), tx);
        let _pending_guard = PendingRequestGuard::new(Arc::clone(&self.inner), req_id.clone());
        self.inner.write_frame(&body).await?;
        let resp = rx.await.map_err(|error| {
            TransportError::disconnected_error(
                format!(
                    "stdio plugin response channel closed before streaming request {id} completed"
                ),
                &error,
            )
        })?;
        let handle: ToolInvokeStreamHandle = match resp.payload {
            ResponsePayload::Ok { result } => {
                serde_json::from_value(result).map_err(TransportError::from)?
            }
            ResponsePayload::Err { error } => {
                let pe = super::plugin_error_from_rpc(
                    error,
                    "decode stdio plugin streaming-dispatch error",
                );
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
        self.inner.shutdown.cancel();
        self.inner.fail_pending("plugin transport closed");
        // A spawn already past its closed check may still be installing a
        // child. Wait for that transaction, then take exactly the published
        // generation. A spawn sleeping in restart backoff observes `closed`
        // after waking and exits without resurrecting the transport.
        self.inner
            .fail_active_streams(PluginError::internal("plugin transport closed"))
            .await;
        let _spawn_guard = self.inner.spawn_lock.lock().await;
        let handles = { self.inner.handles.lock().await.take() };
        if let Some(mut h) = handles {
            h.child.start_kill().map_err(TransportError::from)?;
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

impl Drop for StdioTransport {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.inner.shutdown.cancel();
        self.inner.fail_pending("plugin transport dropped");
        if let Ok(mut handles) = self.inner.handles.try_lock()
            && let Some(handles) = handles.as_mut()
        {
            if let Err(error) = handles.child.start_kill() {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to kill the stdio plugin process tree while dropping its transport",
                        &error,
                    ),
                    "stdio plugin transport cleanup failed"
                );
            }
        }
    }
}
