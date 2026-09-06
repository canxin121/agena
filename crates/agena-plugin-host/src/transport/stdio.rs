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

use agena_stdio_codec::ContentLengthCodec;
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use futures_util::{FutureExt as _, SinkExt as _, StreamExt as _};
use tokio::process::Command;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
use tokio_util::codec::{FramedRead, FramedWrite};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

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
const MAX_BUFFERED_STREAM_CHUNKS: usize = 64;

mod hosted;
mod lifecycle;
mod stderr;
mod streams;
#[cfg(feature = "signing")]
mod verification;

struct SpawnSpec {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: Option<PathBuf>,
    #[cfg(feature = "signing")]
    sha256: Option<String>,
    #[cfg(feature = "signing")]
    resolved_command: std::sync::OnceLock<Result<verification::ResolvedCommand, String>>,
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
    streams: Mutex<streams::StreamRegistry>,
    host_handler: Mutex<Option<HostHandler>>,
    host_call_slots: Arc<Semaphore>,
    shutdown: CancellationToken,
    closed: std::sync::atomic::AtomicBool,
    close_outcome: Mutex<Option<Result<(), String>>>,
    hosted: Mutex<Option<hosted::HostedState>>,
    restart_attempts: AtomicU32,
    plugin_id: Option<PluginKey>,
    status_sink: std::sync::RwLock<Option<Arc<StatusRegistry>>>,
    log_sink: Option<Arc<PluginLogStore>>,
}

struct ChildHandles {
    generation: u64,
    writer: mpsc::Sender<WriteRequest>,
    stop: CancellationToken,
    ready: bool,
    finished:
        futures_util::future::Shared<futures_util::future::BoxFuture<'static, Result<(), String>>>,
}

struct WriteRequest {
    body: Bytes,
    completion: oneshot::Sender<Result<(), String>>,
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
        sha256: Option<&str>,
    ) -> Result<Self, TransportError> {
        #[cfg(not(feature = "signing"))]
        if sha256.is_some() {
            return Err(TransportError::Io(
                "stdio.sha256 set but the `signing` feature is disabled".into(),
            ));
        }
        let spawn_spec = SpawnSpec {
            command: command.to_string(),
            args: args.to_vec(),
            env: env.clone(),
            cwd: cwd.cloned(),
            #[cfg(feature = "signing")]
            sha256: sha256.map(str::to_owned),
            #[cfg(feature = "signing")]
            resolved_command: Default::default(),
        };

        let inner = Arc::new(Inner {
            spawn_spec,
            restart_policy,
            handles: Mutex::new(None),
            spawn_lock: Mutex::new(()),
            child_generation: AtomicU64::new(0),
            next_id: AtomicI64::new(1),
            pending: DashMap::new(),
            streams: Mutex::new(streams::StreamRegistry::default()),
            host_handler: Mutex::new(host_handler),
            host_call_slots: Arc::new(Semaphore::new(HOST_CALLBACK_CONCURRENCY)),
            shutdown: CancellationToken::new(),
            closed: std::sync::atomic::AtomicBool::new(false),
            close_outcome: Mutex::new(None),
            hosted: Mutex::new(None),
            restart_attempts: AtomicU32::new(0),
            plugin_id,
            status_sink: std::sync::RwLock::new(status_sink),
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

        let (command, cwd) = (
            PathBuf::from(&self.spawn_spec.command),
            self.spawn_spec.cwd.clone(),
        );
        #[cfg(feature = "signing")]
        let (command, cwd) = if let Some(expected) = self.spawn_spec.sha256.clone() {
            let this = Arc::clone(&self);
            let verification = tokio::task::spawn_blocking(move || {
                let resolved = this
                    .spawn_spec
                    .resolved_command
                    .get_or_init(|| {
                        verification::resolve(
                            &this.spawn_spec.command,
                            &this.spawn_spec.env,
                            this.spawn_spec.cwd.as_deref(),
                        )
                    })
                    .clone()?;
                crate::loader::verify_sha256(&resolved.executable, &expected)?;
                Ok::<_, String>(resolved)
            })
            .await
            .map_err(|error| format!("stdio executable verification worker failed: {error}"))
            .and_then(|result| result);
            let resolved = match verification {
                Ok(resolved) => resolved,
                Err(error) => {
                    self.record_spawn_failure(&error);
                    return Err(TransportError::Io(error));
                }
            };
            // A close may arrive while the executable is being hashed. Its
            // cancellation must prevent this generation from starting.
            if self.closed.load(Ordering::SeqCst) {
                return Err(TransportError::disconnected(
                    "stdio plugin closed during executable verification",
                ));
            }
            (resolved.executable, Some(resolved.cwd))
        } else {
            (command, cwd)
        };
        let restart_initialization = if is_restart {
            self.hosted
                .lock()
                .await
                .as_ref()
                .and_then(|state| state.initialization.clone())
        } else {
            None
        };
        let mut cmd = Command::new(command);
        cmd.args(&self.spawn_spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &self.spawn_spec.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = &cwd {
            cmd.current_dir(cwd);
        }
        let mut child = match agena_process::spawn(cmd) {
            Ok(child) => child,
            Err(err) => {
                let message = err.to_string();
                self.record_spawn_failure(&message);
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
        let tasks = TaskTracker::new();
        let generation_stop = self.shutdown.child_token();
        let writer_shutdown = generation_stop.clone();
        let (writer_failed_tx, writer_failed) = oneshot::channel();
        tasks.spawn(async move {
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
                let failure = result.as_ref().err().cloned();
                if request.completion.send(result).is_err() {
                    tracing::debug!(
                        target: "agena_plugin_host::stdio",
                        "plugin stdin write-result receiver was dropped"
                    );
                }
                if let Some(error) = failure {
                    let _ = writer_failed_tx.send(error);
                    break;
                }
            }
        });

        // Publish handles and status before a short-lived child can exit.
        let (finished_tx, finished) = oneshot::channel();
        let finished = async move {
            finished.await.map_err(|error| {
                format!("stdio plugin supervisor stopped before cleanup: {error}")
            })?
        }
        .boxed()
        .shared();
        *self.handles.lock().await = Some(ChildHandles {
            generation,
            writer,
            stop: generation_stop.clone(),
            ready: restart_initialization.is_none(),
            finished,
        });
        if restart_initialization.is_none() {
            self.record_status(|sink, plugin_id| sink.record_started(plugin_id, pid, is_restart));
        }
        self.record_log(
            "info",
            "host",
            format!(
                "plugin {} (pid={})",
                if is_restart { "restarted" } else { "started" },
                pid.unwrap_or_default()
            ),
            serde_json::Value::Null,
        );

        let (reader_finished_tx, reader_finished) = oneshot::channel();
        {
            let this = Arc::clone(&self);
            let stop = generation_stop.clone();
            let callbacks = tasks.clone();
            tasks.spawn(async move {
                let mut reader = FramedRead::new(stdout, ContentLengthCodec::new(MAX_FRAME_BYTES));
                let outcome = loop {
                    let frame = tokio::select! {
                        biased;
                        _ = stop.cancelled() => return,
                        frame = reader.next() => frame,
                    };
                    match frame {
                        Some(Ok(body)) => match serde_json::from_slice::<Frame>(body.as_ref()) {
                            Ok(frame) => {
                                tokio::select! {
                                    biased;
                                    _ = stop.cancelled() => return,
                                    _ = this.handle_inbound(frame, generation, &stop, &callbacks) => {}
                                }
                            }
                            Err(error) => {
                                break lifecycle::ReaderEnd::Failure(format!(
                                    "stdio JSON-RPC decode error: {error}"
                                ));
                            }
                        },
                        None => break lifecycle::ReaderEnd::Eof,
                        Some(Err(error)) => {
                            break lifecycle::ReaderEnd::Failure(format!(
                                "stdio framing error: {error}"
                            ));
                        }
                    }
                };
                let _ = reader_finished_tx.send(outcome);
            });
        }
        if let Some(stderr) = stderr {
            tasks.spawn(Arc::clone(&self).drain_stderr(stderr, generation_stop.clone()));
        }
        tasks.close();
        tokio::spawn(Arc::clone(&self).supervise_child(
            generation,
            child,
            lifecycle::ChildTasks {
                reader: reader_finished,
                writer: writer_failed,
                stop: generation_stop.clone(),
                tracker: tasks,
            },
            finished_tx,
        ));

        if let Some(initialization) = restart_initialization {
            let initialized = async {
                self.prepare_hosted_generation(generation).await?;
                initialization
                    .reinitialize(&hosted::GenerationTransport {
                        inner: Arc::clone(&self),
                        generation,
                    })
                    .await?;
                Ok::<_, TransportError>(())
            };
            let result = tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => Err(TransportError::disconnected("plugin closed during restart initialization")),
                result = initialized => result,
            };
            if let Err(error) = result {
                if !self.closed.load(Ordering::SeqCst) {
                    self.record_spawn_failure(&error.to_string());
                }
                generation_stop.cancel();
                return Err(error);
            }
            self.mark_generation_ready(generation).await?;
            self.record_status(|sink, plugin_id| sink.record_started(plugin_id, pid, is_restart));
        }

        Ok(())
    }

    async fn handle_inbound(
        self: &Arc<Self>,
        frame: Frame,
        generation: u64,
        stop: &CancellationToken,
        tasks: &TaskTracker,
    ) {
        match frame {
            Frame::Response(resp) => {
                if let Some((_, tx)) = self.pending.remove(&resp.id)
                    && tx.send(resp).is_err()
                {
                    tracing::debug!(
                        target: "agena_plugin_host::stdio",
                        "plugin response receiver was dropped before delivery"
                    );
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
                            if let Err(error) = self
                                .write_frame_for_generation(&body, Some(generation))
                                .await
                            {
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
                let stop = stop.clone();
                tasks.spawn(async move {
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
                        _ = stop.cancelled() => return,
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
                    if let Err(error) = inner
                        .write_frame_for_generation(&body, Some(generation))
                        .await
                    {
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
                    let stop = stop.clone();
                    tasks.spawn(async move {
                        let _callback_slot = callback_slot;
                        tokio::select! {
                            biased;
                            _ = stop.cancelled() => {}
                            _ = inner.handle_notification(notif) => {}
                        }
                    });
                }
            }
        }
    }

    async fn write_frame(&self, body: &[u8]) -> Result<u64, TransportError> {
        self.write_frame_for_generation(body, None).await
    }

    async fn write_frame_for_generation(
        &self,
        body: &[u8],
        expected: Option<u64>,
    ) -> Result<u64, TransportError> {
        let (writer, generation, stop) = {
            let handles = self.handles.lock().await;
            let handles = handles
                .as_ref()
                .filter(|handles| {
                    !self.closed.load(Ordering::SeqCst)
                        && !handles.stop.is_cancelled()
                        && expected
                            .map_or(handles.ready, |generation| generation == handles.generation)
                })
                .ok_or_else(|| {
                    TransportError::disconnected(
                        "stdio plugin child has no matching active stdin writer",
                    )
                })?;
            (
                handles.writer.clone(),
                handles.generation,
                handles.stop.clone(),
            )
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
            },
            _ = stop.cancelled() => Err(TransportError::disconnected(
                "stdio plugin transport was shut down while writing a frame",
            )),
        }
        .map(|()| generation)
    }

    fn next_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn begin_request(
        &self,
        request_id: RequestId,
        response: oneshot::Sender<Response>,
        expected_generation: Option<u64>,
    ) -> Result<(u64, CancellationToken), TransportError> {
        // Publish the response slot against one specific child while holding
        // the same lock used to remove that child on exit. An exit must not
        // fail this request and then let its frame reach a replacement child.
        let handles = self.handles.lock().await;
        let handles = handles
            .as_ref()
            .filter(|handles| {
                !self.closed.load(Ordering::SeqCst)
                    && !handles.stop.is_cancelled()
                    && expected_generation
                        .map_or(handles.ready, |generation| generation == handles.generation)
            })
            .ok_or_else(|| {
                TransportError::disconnected("stdio plugin has no active child for this request")
            })?;
        self.pending.insert(request_id, response);
        Ok((handles.generation, handles.stop.clone()))
    }

    pub(super) async fn dispatch_to_generation(
        self: &Arc<Self>,
        expected_generation: Option<u64>,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let id = self.next_id();
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
        let (generation, stop) = self
            .begin_request(req_id.clone(), tx, expected_generation)
            .await?;
        let _pending_guard = PendingRequestGuard::new(Arc::clone(self), req_id.clone());
        self.write_frame_for_generation(&body, Some(generation))
            .await?;
        let response = tokio::select! {
            biased;
            response = rx => response,
            _ = stop.cancelled() => return Err(TransportError::disconnected("stdio plugin exited while awaiting a response")),
        };
        let resp = response.map_err(|error| {
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

    fn fail_pending(&self, message: &str) {
        let pending = self
            .pending
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for id in pending {
            if let Some((_, slot)) = self.pending.remove(&id)
                && slot
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

    fn record_status<F>(&self, mutate: F)
    where
        F: FnOnce(&StatusRegistry, &PluginKey),
    {
        let binding = self
            .status_sink
            .read()
            .unwrap_or_else(|error| error.into_inner());
        if let (Some(sink), Some(plugin_id)) = (binding.as_ref(), self.plugin_id.as_ref()) {
            mutate(sink.as_ref(), plugin_id);
        }
    }

    fn record_spawn_failure(&self, message: &str) {
        self.record_status(|sink, plugin_id| {
            sink.record_spawn_failure(plugin_id, message.to_owned());
        });
        self.record_log(
            "error",
            "host",
            format!("spawn failed: {message}"),
            serde_json::Value::Null,
        );
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
    async fn bind_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
    ) -> Result<(), TransportError> {
        self.inner
            .bind_host_inner(host, scope, None, None)
            .await
            .map(|_| ())
    }

    async fn initialize(
        &self,
        initialization: super::initialization::PluginInitialization,
    ) -> Result<crate::sdk::InitOutcome, TransportError> {
        self.inner.initialize_hosted(initialization).await
    }

    async fn can_reuse(&self, owner: &Arc<crate::effect_scope::PluginEffectScope>) -> bool {
        let hosted = self.inner.hosted.lock().await;
        let handles = self.inner.handles.lock().await;
        self.inner
            .can_reuse_hosted(hosted.as_ref(), handles.as_ref(), owner)
    }

    async fn try_rebind_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
        previous_scope: Arc<crate::effect_scope::PluginEffectScope>,
        initialization: super::initialization::PluginInitialization,
    ) -> Result<bool, TransportError> {
        self.inner
            .bind_host_inner(host, scope, Some(previous_scope), Some(initialization))
            .await
    }

    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        self.inner
            .dispatch_to_generation(None, method, params)
            .await
    }

    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), TransportError> {
        let n = Notification {
            jsonrpc: JsonRpcVersion,
            method: method.to_string(),
            params: Some(params),
        };
        let body = serde_json::to_vec(&n)?;
        self.inner.write_frame(&body).await.map(|_| ())
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
        let (generation, _stop) = self.inner.begin_request(req_id.clone(), tx, None).await?;
        let _pending_guard = PendingRequestGuard::new(Arc::clone(&self.inner), req_id.clone());
        self.inner
            .write_frame_for_generation(&body, Some(generation))
            .await?;
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
            .register_stream(handle.stream_id.clone(), chunk_tx, end_tx, generation)
            .await?;
        Ok(Some(crate::transport::ToolStreamHandle {
            stream_id: handle.stream_id,
            chunks: chunk_rx,
            end: end_rx,
        }))
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.inner.shutdown.cancel();
        let mut close_outcome = self.inner.close_outcome.lock().await;
        if let Some(outcome) = close_outcome.as_ref() {
            return outcome.clone().map_err(TransportError::Io);
        }
        self.inner.fail_pending("plugin transport closed");
        // A spawn already past its closed check may still be installing a
        // child. Wait for that transaction, then take exactly the published
        // generation. A spawn sleeping in restart backoff observes `closed`
        // after waking and exits without resurrecting the transport.
        self.inner
            .fail_active_streams(PluginError::internal("plugin transport closed"))
            .await;
        let _spawn_guard = self.inner.spawn_lock.lock().await;
        // Keep cleanup observable if the caller cancels this close future.
        // Callback tasks may still need handles, so never hold that lock
        // while awaiting their supervisor.
        let finished = self
            .inner
            .handles
            .lock()
            .await
            .as_ref()
            .map(|handles| handles.finished.clone());
        let cleanup = if let Some(finished) = finished {
            match tokio::time::timeout(lifecycle::CLOSE_TIMEOUT, finished).await {
                Ok(result) => result,
                Err(error) => Err(format!("timed out closing stdio plugin: {error}")),
            }
        } else {
            Ok(())
        };
        self.inner.handles.lock().await.take();
        self.inner
            .fail_active_streams(PluginError::internal("plugin transport closed"))
            .await;
        *close_outcome = Some(cleanup.clone());
        if let Err(error) = cleanup {
            self.inner.record_spawn_failure(&error);
            return Err(TransportError::Io(error));
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
        let already_closed = self.inner.closed.swap(true, Ordering::SeqCst);
        self.inner.shutdown.cancel();
        self.inner.fail_pending("plugin transport dropped");
        if !already_closed {
            self.inner
                .record_status(|sink, plugin_id| sink.record_stopped(plugin_id));
        }
    }
}
