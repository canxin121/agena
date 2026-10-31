//! Stdio JSON-RPC driver. Plugin author calls `serve_stdio(plugin).await`
//! from their `main` (or use `export_stdio!`).
//!
//! Frames are LSP-style Content-Length newline-delimited.
//!
//! Architecture:
//!   - one reader task drains stdin → demuxes frames
//!   - one writer task drains an mpsc channel → flushes stdout
//!   - dispatcher runs on the tokio runtime; responses go back via the channel

use portable_atomic::AtomicI64;
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex as StdMutex, atomic::Ordering};
use std::time::Duration;

use agena_stdio_codec::ContentLengthCodec;
use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::JoinSet;
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::drivers::dispatch::{PluginDispatcher, StreamHandle};
use crate::error::{PluginError, PluginErrorKind};
use crate::hooks::{EventEnvelope, EventFilter, ToolInvokeStreamHandle, ToolStreamError};
use crate::host_api::{
    AskUserRequest, AskUserResponse, EventSubscription, HostClient, HostCommandMutationResponse,
    HostCommandRegisterRequest, HostCommandRemoveRequest, HostCommandUpdateRequest,
    HostContextStatusRequest, HostContextStatusResponse, HostDisplayContributeRequest,
    HostDisplayRemoveRequest, HostDisplayRemoveResponse, HostHookListResponse,
    HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse, HostLspListServersResponse,
    HostMcpAddServerRequest, HostMcpListServersResponse, HostMcpRemoveServerRequest,
    HostMcpRemoveServerResponse, HostPluginStatusGetRequest, HostPluginStatusGetResponse,
    HostPluginStatusListResponse, HostRegisteredCommandListResponse,
    HostRegisteredToolListResponse, HostSchedulerCreateRequest, HostSchedulerCreateResponse,
    HostSchedulerDeleteRequest, HostSchedulerDeleteResponse, HostSchedulerListResponse,
    HostSecretDeleteRequest, HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse,
    HostSecretSetRequest, HostSetSessionModelRequest, HostSetSessionModelResponse,
    HostStorageDeleteRequest, HostStorageGetRequest, HostStorageGetResponse,
    HostStorageListRequest, HostStorageListResponse, HostStorageSetRequest, HostThemeListResponse,
    HostThemeRegisterRequest, HostThemeRemoveRequest, HostThemeRemoveResponse,
    HostToolMutationResponse, HostToolRegisterRequest, HostToolRemoveRequest,
    HostToolUpdateRequest, LogLevel, MonitorHandle, MonitorReadRequest, MonitorReadResponse,
    MonitorStartRequest, MonitorStopRequest, PluginNotifyRequest, ToolDescriptor,
};
use crate::plugin::Plugin;
use crate::rpc::{
    ErrorObject, Frame, JsonRpcVersion, Notification, Request, RequestId, Response,
    ResponsePayload, codes, method,
};

const STDIO_WRITER_QUEUE_CAPACITY: usize = 256;
const STDIO_DISPATCH_CONCURRENCY: usize = 64;
const STDIO_WRITER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const STDIO_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Run a plugin to completion as a stdio JSON-RPC server. Returns when stdin
/// closes.
pub async fn serve_stdio<P: Plugin>(plugin: P) -> std::io::Result<()> {
    serve_stdio_io(
        plugin,
        tokio::io::stdin(),
        tokio::io::stdout(),
        Arc::new(Semaphore::new(STDIO_DISPATCH_CONCURRENCY)),
    )
    .await
}

async fn serve_stdio_io<P, R, W>(
    plugin: P,
    input: R,
    output: W,
    dispatch_slots: Arc<Semaphore>,
) -> io::Result<()>
where
    P: Plugin,
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let dispatcher = Arc::new(PluginDispatcher::new(plugin));

    let (tx, rx) = mpsc::channel::<Bytes>(STDIO_WRITER_QUEUE_CAPACITY);
    let pending: Arc<StdMutex<HashMap<RequestId, oneshot::Sender<Response>>>> =
        Arc::new(StdMutex::new(HashMap::new()));
    let next_id = Arc::new(AtomicI64::new(1));

    let host: Arc<dyn HostClient> = Arc::new(StdioHostClient {
        tx: tx.clone(),
        pending: Arc::clone(&pending),
        next_id: Arc::clone(&next_id),
    });
    dispatcher.set_host(host).await;

    // Writer task: owns stdout, drains channel.
    let mut writer = tokio::spawn(async move {
        let mut stdout = FramedWrite::new(output, ContentLengthCodec::new(STDIO_MAX_FRAME_BYTES));
        let mut rx = rx;
        while let Some(payload) = rx.recv().await {
            stdout.send(payload).await.map_err(io::Error::other)?;
        }
        Ok::<(), io::Error>(())
    });

    // The reader must never wait for dispatch capacity or writer backpressure:
    // cancellation and responses to plugin -> host callbacks share this pipe.
    // If a control/error response cannot be queued, disconnect and stop work.
    let mut dispatch_tasks = JoinSet::new();

    // Reader loop: owns stdin, demuxes frames.
    let mut reader = FramedRead::new(input, ContentLengthCodec::new(STDIO_MAX_FRAME_BYTES));
    let (read_result, writer_finished) = {
        let read_loop = async {
            loop {
                while let Some(result) = dispatch_tasks.try_join_next() {
                    if let Err(error) = result {
                        tracing::error!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "a stdio plugin dispatch task failed",
                                &error,
                            ),
                            "stdio plugin dispatch task did not complete normally"
                        );
                    }
                }

                let Some(body) = reader.next().await else {
                    return Ok(());
                };
                let body = body.map_err(std::io::Error::other)?;

                let frame: Frame = match serde_json::from_slice(&body) {
                    Ok(f) => f,
                    Err(error) => {
                        tracing::warn!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to decode an inbound stdio plugin JSON-RPC frame",
                                &error,
                            ),
                            frame_bytes = body.len(),
                            "ignoring malformed stdio plugin frame"
                        );
                        continue;
                    }
                };

                match frame {
                    Frame::Request(req) => {
                        if req.method == method::CANCEL_REQUEST {
                            let result = dispatcher
                                .dispatch(
                                    &req.method,
                                    req.params.unwrap_or(serde_json::Value::Null),
                                )
                                .await;
                            let payload = match result {
                                Ok(result) => ResponsePayload::Ok { result },
                                Err(error) => ResponsePayload::Err {
                                    error: error_object_from(error),
                                },
                            };
                            try_send_response(
                                &tx,
                                Response {
                                    jsonrpc: JsonRpcVersion,
                                    id: req.id,
                                    payload,
                                },
                            )?;
                            continue;
                        }
                        let dispatch_slot = Arc::clone(&dispatch_slots).try_acquire_owned();
                        let Ok(dispatch_slot) = dispatch_slot else {
                            try_send_response(
                                &tx,
                                Response {
                                    jsonrpc: JsonRpcVersion,
                                    id: req.id,
                                    payload: ResponsePayload::Err {
                                        error: ErrorObject {
                                            code: codes::INTERNAL_ERROR,
                                            message: "plugin dispatch capacity exhausted"
                                                .to_string(),
                                            data: None,
                                        },
                                    },
                                },
                            )?;
                            continue;
                        };
                        let request_scope = match dispatcher.begin_request(req.id.clone()) {
                            Ok(scope) => scope,
                            Err(error) => {
                                try_send_response(
                                    &tx,
                                    Response {
                                        jsonrpc: JsonRpcVersion,
                                        id: req.id,
                                        payload: ResponsePayload::Err {
                                            error: error_object_from(error),
                                        },
                                    },
                                )?;
                                continue;
                            }
                        };
                        let dispatcher = Arc::clone(&dispatcher);
                        let tx = tx.clone();
                        dispatch_tasks.spawn(async move {
                            let _dispatch_slot = dispatch_slot;
                            let id = req.id.clone();
                            let params = req.params.unwrap_or(serde_json::Value::Null);
                            let callback_context = req.context;
                            if req.method == method::HOOK_TOOL_INVOKE_STREAM {
                                let _ = request_scope
                                    .run(async {
                                        match serde_json::from_value(params) {
                                            Ok(input) => {
                                                let handle = if let Some(context) = callback_context
                                                {
                                                    crate::host_api::run_in_host_callback_context(
                                                        context,
                                                        async {
                                                            dispatcher.dispatch_stream_request(
                                                                input,
                                                                &request_scope,
                                                                (),
                                                            )
                                                        },
                                                    )
                                                    .await
                                                } else {
                                                    dispatcher.dispatch_stream_request(
                                                        input,
                                                        &request_scope,
                                                        (),
                                                    )
                                                };
                                                forward_tool_stream(&tx, id, handle).await;
                                            }
                                            Err(err) => {
                                                let _sent = send_response(
                                                    &tx,
                                                    Response {
                                                        jsonrpc: JsonRpcVersion,
                                                        id,
                                                        payload: ResponsePayload::Err {
                                                            error: error_object_from(
                                                                PluginError::invalid_params_error(
                                                                    &err,
                                                                ),
                                                            ),
                                                        },
                                                    },
                                                )
                                                .await;
                                            }
                                        }
                                        Ok(())
                                    })
                                    .await;
                                return;
                            }
                            let result = request_scope
                                .run(async {
                                    let dispatch = dispatcher.dispatch(&req.method, params);
                                    let result = if let Some(context) = callback_context {
                                        crate::host_api::run_in_host_callback_context(
                                            context, dispatch,
                                        )
                                        .await
                                    } else {
                                        dispatch.await
                                    };
                                    let payload = match result {
                                        Ok(result) => ResponsePayload::Ok { result },
                                        Err(error) => ResponsePayload::Err {
                                            error: error_object_from(error),
                                        },
                                    };
                                    let _sent = send_response(
                                        &tx,
                                        Response {
                                            jsonrpc: JsonRpcVersion,
                                            id: id.clone(),
                                            payload,
                                        },
                                    )
                                    .await;
                                    Ok(())
                                })
                                .await;
                            if let Err(error) = result {
                                // Cancellation must release this worker even
                                // when the host is no longer draining stdout.
                                let _ = try_send_response(
                                    &tx,
                                    Response {
                                        jsonrpc: JsonRpcVersion,
                                        id,
                                        payload: ResponsePayload::Err {
                                            error: error_object_from(error),
                                        },
                                    },
                                );
                            }
                        });
                    }
                    Frame::Notification(notif) => {
                        if notif.method == method::CANCEL_REQUEST {
                            match serde_json::from_value::<crate::rpc::CancelRequest>(
                                notif.params.unwrap_or(serde_json::Value::Null),
                            ) {
                                Ok(request) => {
                                    dispatcher.cancel_request(&request.id);
                                }
                                Err(error) => {
                                    tracing::warn!(%error, "invalid plugin cancellation notification")
                                }
                            }
                            continue;
                        }
                        // Plugins can receive notifications (like `hooks/event`).
                        let Ok(dispatch_slot) = Arc::clone(&dispatch_slots).try_acquire_owned()
                        else {
                            tracing::warn!(
                                method = %notif.method,
                                "stdio plugin notification was dropped because the dispatch limit was exhausted"
                            );
                            continue;
                        };
                        let dispatcher = Arc::clone(&dispatcher);
                        dispatch_tasks.spawn(async move {
                            let _dispatch_slot = dispatch_slot;
                            if let Err(error) = dispatcher
                                .dispatch(
                                    &notif.method,
                                    notif.params.unwrap_or(serde_json::Value::Null),
                                )
                                .await
                            {
                                tracing::warn!(
                                    method = %notif.method,
                                    diagnostic = %error.diagnostic.message,
                                    "stdio plugin notification dispatch failed"
                                );
                            }
                        });
                    }
                    Frame::Response(resp) => {
                        let mut pending = pending.lock().unwrap_or_else(|error| {
                            tracing::error!(
                                diagnostic = %error,
                                "recovering the poisoned stdio plugin response registry"
                            );
                            error.into_inner()
                        });
                        if let Some(slot) = pending.remove(&resp.id)
                            && slot.send(resp).is_err()
                        {
                            tracing::debug!(
                                "stdio plugin response arrived after its receiver was dropped"
                            );
                        }
                    }
                }
            }
        };
        tokio::pin!(read_loop);
        let mut writer_finished = false;
        let read_result = tokio::select! {
            result = &mut read_loop => result,
            result = &mut writer => {
                writer_finished = true;
                match result {
                    Ok(Ok(())) => Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "stdio plugin writer stopped while the reader was still active",
                    )),
                    Ok(Err(error)) => Err(io::Error::other(
                        agena_failure::diagnostic::format_error_chain_with_context(
                            "stdio plugin writer failed while the reader was still active",
                            &error,
                        ),
                    )),
                    Err(error) => Err(io::Error::other(
                        agena_failure::diagnostic::format_error_chain_with_context(
                            "stdio plugin writer task terminated while the reader was still active",
                            &error,
                        ),
                    )),
                }
            }
        };
        (read_result, writer_finished)
    };

    dispatch_tasks.abort_all();
    while let Some(result) = dispatch_tasks.join_next().await {
        if let Err(error) = result
            && !error.is_cancelled()
        {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "a stdio plugin dispatch task failed during shutdown",
                    &error,
                ),
                "stdio plugin dispatch shutdown failed"
            );
        }
    }
    drop(dispatcher);
    drop(tx);

    let writer_result = if writer_finished {
        Ok(())
    } else {
        match tokio::time::timeout(STDIO_WRITER_SHUTDOWN_TIMEOUT, &mut writer).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(io::Error::other(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "stdio plugin writer task failed",
                    &error,
                ),
            )),
            Err(timeout_error) => {
                writer.abort();
                match writer.await {
                    Err(join_error) if join_error.is_cancelled() => {}
                    Err(join_error) => tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "stdio plugin writer task failed while being aborted after a shutdown timeout",
                            &join_error,
                        ),
                        "stdio plugin writer abort failed"
                    ),
                    Ok(Err(writer_error)) => tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "stdio plugin writer also returned an I/O error after its shutdown timed out",
                            &writer_error,
                        ),
                        "stdio plugin writer timed out and failed"
                    ),
                    Ok(Ok(())) => {}
                }
                Err(io::Error::other(
                    agena_failure::diagnostic::format_error_chain_with_context(
                        format!(
                            "stdio plugin writer did not stop within {}ms",
                            STDIO_WRITER_SHUTDOWN_TIMEOUT.as_millis()
                        ),
                        &timeout_error,
                    ),
                ))
            }
        }
    };

    match (read_result, writer_result) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(read_error), Err(writer_error)) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "stdio plugin writer also failed while the reader was failing",
                    &writer_error,
                ),
                "returning the primary stdio plugin reader failure"
            );
            Err(read_error)
        }
    }
}

async fn forward_tool_stream(tx: &mpsc::Sender<Bytes>, id: RequestId, mut handle: StreamHandle) {
    let stream_id = handle.stream_id.clone();
    let stream_handle = match serde_json::to_value(ToolInvokeStreamHandle {
        stream_id: stream_id.clone(),
        title: None,
    }) {
        Ok(value) => value,
        Err(error) => {
            let _sent = send_response(
                tx,
                Response {
                    jsonrpc: JsonRpcVersion,
                    id,
                    payload: ResponsePayload::Err {
                        error: error_object_from(PluginError::internal_error(&error)),
                    },
                },
            )
            .await;
            return;
        }
    };
    if !send_response(
        tx,
        Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok {
                result: stream_handle,
            },
        },
    )
    .await
    {
        return;
    }
    while let Some(chunk) = handle.chunks.recv().await {
        if !send_notification(tx, method::TOOL_STREAM_CHUNK, &chunk).await {
            return;
        }
    }
    match handle.end.await {
        Ok(Ok(end)) => {
            let _sent = send_notification(tx, method::TOOL_STREAM_END, &end).await;
        }
        Ok(Err(error)) => {
            let _sent = send_notification(
                tx,
                method::TOOL_STREAM_ERROR,
                &ToolStreamError { stream_id, error },
            )
            .await;
        }
        Err(error) => {
            let _sent = send_notification(
                tx,
                method::TOOL_STREAM_ERROR,
                &ToolStreamError {
                    stream_id,
                    error: PluginError::internal(
                        agena_failure::diagnostic::format_error_chain_with_context(
                            "tool stream terminated before sending its final frame",
                            &error,
                        ),
                    ),
                },
            )
            .await;
        }
    }
}

fn try_send_response(tx: &mpsc::Sender<Bytes>, response: Response) -> io::Result<()> {
    let body = serde_json::to_vec(&response).map_err(io::Error::other)?;
    tx.try_send(body.into()).map_err(|error| match error {
        mpsc::error::TrySendError::Full(_) => io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "stdio plugin writer queue is full; cannot enqueue a response",
        ),
        mpsc::error::TrySendError::Closed(_) => io::Error::new(
            io::ErrorKind::BrokenPipe,
            "stdio plugin writer closed while enqueueing a response",
        ),
    })
}

async fn send_response(tx: &mpsc::Sender<Bytes>, response: Response) -> bool {
    let body = match serde_json::to_vec(&response) {
        Ok(body) => body,
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to serialize a stdio plugin JSON-RPC response",
                    &error,
                ),
                "stdio plugin response was not sent"
            );
            return false;
        }
    };
    if let Err(error) = tx.send(body.into()).await {
        tracing::debug!(
            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                "failed to enqueue a stdio plugin JSON-RPC response because the writer was closed",
                &error,
            ),
            "stdio plugin response receiver is unavailable"
        );
        return false;
    }
    true
}

async fn send_notification<T: serde::Serialize>(
    tx: &mpsc::Sender<Bytes>,
    method_name: &str,
    params: &T,
) -> bool {
    let params = match serde_json::to_value(params) {
        Ok(params) => params,
        Err(error) => {
            tracing::error!(
                method = method_name,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to serialize stdio plugin notification parameters",
                    &error,
                ),
                "stdio plugin notification was not sent"
            );
            return false;
        }
    };
    let notification = Notification {
        jsonrpc: JsonRpcVersion,
        method: method_name.to_string(),
        params: Some(params),
    };
    let body = match serde_json::to_vec(&notification) {
        Ok(body) => body,
        Err(error) => {
            tracing::error!(
                method = method_name,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to serialize a stdio plugin notification envelope",
                    &error,
                ),
                "stdio plugin notification was not sent"
            );
            return false;
        }
    };
    if let Err(error) = tx.send(body.into()).await {
        tracing::debug!(
            method = method_name,
            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                "failed to enqueue a stdio plugin notification because the writer was closed",
                &error,
            ),
            "stdio plugin notification receiver is unavailable"
        );
        return false;
    }
    true
}

#[macro_export]
macro_rules! export_stdio {
    ($plugin_expr:expr) => {
        fn main() -> std::io::Result<()> {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on($crate::drivers::stdio::serve_stdio($plugin_expr))
        }
    };
}

// ---------- StdioHostClient: plugin -> host callbacks over the same pipe ----------

struct StdioHostClient {
    tx: mpsc::Sender<Bytes>,
    pending: Arc<StdMutex<HashMap<RequestId, oneshot::Sender<Response>>>>,
    next_id: Arc<AtomicI64>,
}

impl StdioHostClient {
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> crate::error::Result<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req_id = RequestId::Num(id);
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: req_id.clone(),
            method: method.to_string(),
            params: Some(params),
            context: None,
        };
        let body = serde_json::to_vec(&req).map_err(|error| {
            PluginError::invalid_params_error(&error).with_hook(method.to_owned())
        })?;
        let (slot_tx, slot_rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(req_id.clone(), slot_tx);
        let _pending_guard = PendingCallGuard {
            pending: Arc::clone(&self.pending),
            request_id: req_id,
        };
        self.tx.send(body.into()).await.map_err(|error| {
            PluginError::from_kind(
                PluginErrorKind::Disconnected,
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("host writer closed while sending callback `{method}`"),
                    &error,
                ),
            )
            .with_hook(method.to_owned())
        })?;
        let resp = slot_rx.await.map_err(|error| {
            PluginError::from_kind(
                PluginErrorKind::Disconnected,
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("host closed callback `{method}` before returning a response"),
                    &error,
                ),
            )
            .with_hook(method.to_owned())
        })?;
        match resp.payload {
            ResponsePayload::Ok { result } => {
                serde_json::from_value(result).map_err(|e| PluginError::invalid_params_error(&e))
            }
            ResponsePayload::Err { error } => Err(PluginError::from_rpc_error(error, method)),
        }
    }

    async fn notify(&self, method: &str, params: serde_json::Value) -> crate::error::Result<()> {
        let n = Notification {
            jsonrpc: JsonRpcVersion,
            method: method.to_string(),
            params: Some(params),
        };
        let body = serde_json::to_vec(&n).map_err(|error| {
            PluginError::invalid_params_error(&error).with_hook(method.to_owned())
        })?;
        self.tx.send(body.into()).await.map_err(|error| {
            PluginError::from_kind(
                PluginErrorKind::Disconnected,
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("host writer closed while sending notification `{method}`"),
                    &error,
                ),
            )
            .with_hook(method.to_owned())
        })?;
        Ok(())
    }
}

struct PendingCallGuard {
    pending: Arc<StdMutex<HashMap<RequestId, oneshot::Sender<Response>>>>,
    request_id: RequestId,
}

impl Drop for PendingCallGuard {
    fn drop(&mut self) {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.request_id);
    }
}

fn params_with_optional_context(mut params: serde_json::Value) -> serde_json::Value {
    if let Some(object) = params.as_object_mut() {
        object.insert(
            "context".to_owned(),
            serde_json::json!(crate::host_api::current_host_callback_context()),
        );
    }
    params
}

crate::drivers::impl_callback_host_client!(StdioHostClient, params_with_optional_context, {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        if let Err(error) = self
            .notify(
                method::HOST_LOG,
                serde_json::json!({ "level": level, "message": message, "fields": fields }),
            )
            .await
        {
            tracing::warn!(
                diagnostic = %error.diagnostic_message(),
                "failed to deliver a plugin log record to the stdio host"
            );
        }
    }

    async fn publish_event(&self, env: EventEnvelope) -> crate::error::Result<()> {
        self.notify(
            method::HOST_EVENT_PUBLISH,
            serde_json::to_value(env).map_err(|e| PluginError::invalid_params_error(&e))?,
        )
        .await
    }

    async fn subscribe_events(
        &self,
        filter: EventFilter,
    ) -> crate::error::Result<EventSubscription> {
        let v: serde_json::Value = self
            .call(
                method::HOST_EVENT_SUBSCRIBE,
                serde_json::json!({ "filter": filter }),
            )
            .await?;
        let id = v
            .get("subscription_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        Ok(EventSubscription { id })
    }

    async fn read_config(&self, path: Option<String>) -> crate::error::Result<serde_json::Value> {
        self.call(
            method::HOST_CONFIG_READ,
            serde_json::json!({ "path": path }),
        )
        .await
    }

    async fn require_path_permission(
        &self,
        request: crate::host_api::PathPermissionQuery,
    ) -> crate::error::Result<()> {
        crate::permission_wait::waiting(self.call(method::HOST_PERMISSION_REQUIRE_PATH, serde_json::json!({ "request": request, "context": crate::host_api::current_host_callback_context() }))).await
    }

    async fn require_network_permission(&self, target: String) -> crate::error::Result<()> {
        crate::permission_wait::waiting(self.call(method::HOST_PERMISSION_REQUIRE_NETWORK, serde_json::json!({ "target": target, "context": crate::host_api::current_host_callback_context() }))).await
    }

    async fn ask_user(&self, req: AskUserRequest) -> crate::error::Result<AskUserResponse> {
        self.call(
            method::HOST_ASK_USER,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn list_tools(&self) -> crate::error::Result<Vec<ToolDescriptor>> {
        self.call(
            method::HOST_TOOL_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn get_context_status(
        &self,
        req: HostContextStatusRequest,
    ) -> crate::error::Result<HostContextStatusResponse> {
        self.call(
            method::HOST_CONTEXT_STATUS,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn set_session_model(
        &self,
        req: HostSetSessionModelRequest,
    ) -> crate::error::Result<HostSetSessionModelResponse> {
        self.call(
            method::HOST_SESSION_SET_MODEL,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn monitor_start(&self, req: MonitorStartRequest) -> crate::error::Result<MonitorHandle> {
        self.call(
            method::HOST_MONITOR_START,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn monitor_list(&self) -> crate::error::Result<Vec<MonitorHandle>> {
        self.call(
            method::HOST_MONITOR_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn monitor_read(
        &self,
        req: MonitorReadRequest,
    ) -> crate::error::Result<MonitorReadResponse> {
        self.call(
            method::HOST_MONITOR_READ,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn monitor_stop(&self, req: MonitorStopRequest) -> crate::error::Result<MonitorHandle> {
        self.call(
            method::HOST_MONITOR_STOP,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn register_command(
        &self,
        req: HostCommandRegisterRequest,
    ) -> crate::error::Result<HostCommandMutationResponse> {
        self.call(
            method::HOST_COMMAND_REGISTRY_REGISTER,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn update_command(
        &self,
        req: HostCommandUpdateRequest,
    ) -> crate::error::Result<HostCommandMutationResponse> {
        self.call(
            method::HOST_COMMAND_REGISTRY_UPDATE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn remove_command(
        &self,
        req: HostCommandRemoveRequest,
    ) -> crate::error::Result<HostCommandMutationResponse> {
        self.call(
            method::HOST_COMMAND_REGISTRY_REMOVE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn list_registered_commands(
        &self,
    ) -> crate::error::Result<HostRegisteredCommandListResponse> {
        self.call(
            method::HOST_COMMAND_REGISTRY_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn register_tool(
        &self,
        req: HostToolRegisterRequest,
    ) -> crate::error::Result<HostToolMutationResponse> {
        self.call(
            method::HOST_TOOL_REGISTRY_REGISTER,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn update_tool(
        &self,
        req: HostToolUpdateRequest,
    ) -> crate::error::Result<HostToolMutationResponse> {
        self.call(
            method::HOST_TOOL_REGISTRY_UPDATE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn remove_tool(
        &self,
        req: HostToolRemoveRequest,
    ) -> crate::error::Result<HostToolMutationResponse> {
        self.call(
            method::HOST_TOOL_REGISTRY_REMOVE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn list_registered_tools(&self) -> crate::error::Result<HostRegisteredToolListResponse> {
        self.call(
            method::HOST_TOOL_REGISTRY_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn storage_get(
        &self,
        req: HostStorageGetRequest,
    ) -> crate::error::Result<HostStorageGetResponse> {
        self.call(
            method::HOST_STORAGE_GET,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_STORAGE_SET,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_STORAGE_DELETE,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> crate::error::Result<HostStorageListResponse> {
        self.call(
            method::HOST_STORAGE_LIST,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn secret_get(
        &self,
        req: HostSecretGetRequest,
    ) -> crate::error::Result<HostSecretGetResponse> {
        self.call(
            method::HOST_SECRET_GET,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn secret_set(&self, req: HostSecretSetRequest) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_SECRET_SET,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn secret_delete(&self, req: HostSecretDeleteRequest) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_SECRET_DELETE,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn secret_list(&self) -> crate::error::Result<HostSecretListResponse> {
        self.call(
            method::HOST_SECRET_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn plugin_status_list(&self) -> crate::error::Result<HostPluginStatusListResponse> {
        self.call(
            method::HOST_PLUGIN_STATUS_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn plugin_status_get(
        &self,
        req: HostPluginStatusGetRequest,
    ) -> crate::error::Result<HostPluginStatusGetResponse> {
        self.call(
            method::HOST_PLUGIN_STATUS_GET,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn lsp_list_servers(&self) -> crate::error::Result<HostLspListServersResponse> {
        self.call(
            method::HOST_LSP_LIST_SERVERS,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn lsp_list_diagnostics(
        &self,
        req: HostLspListDiagnosticsRequest,
    ) -> crate::error::Result<HostLspListDiagnosticsResponse> {
        self.call(
            method::HOST_LSP_LIST_DIAGNOSTICS,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn scheduler_list(&self) -> crate::error::Result<HostSchedulerListResponse> {
        self.call(
            method::HOST_SCHEDULER_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn scheduler_create(
        &self,
        req: HostSchedulerCreateRequest,
    ) -> crate::error::Result<HostSchedulerCreateResponse> {
        self.call(
            method::HOST_SCHEDULER_CREATE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn scheduler_delete(
        &self,
        req: HostSchedulerDeleteRequest,
    ) -> crate::error::Result<HostSchedulerDeleteResponse> {
        self.call(
            method::HOST_SCHEDULER_DELETE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn hook_list(&self) -> crate::error::Result<HostHookListResponse> {
        self.call(
            method::HOST_HOOK_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn mcp_list_servers(&self) -> crate::error::Result<HostMcpListServersResponse> {
        self.call(
            method::HOST_MCP_LIST_SERVERS,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn mcp_add_server(&self, req: HostMcpAddServerRequest) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_MCP_ADD_SERVER,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn mcp_remove_server(
        &self,
        req: HostMcpRemoveServerRequest,
    ) -> crate::error::Result<HostMcpRemoveServerResponse> {
        self.call(
            method::HOST_MCP_REMOVE_SERVER,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn display_contribute(
        &self,
        req: HostDisplayContributeRequest,
    ) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_UI_DISPLAY_CONTRIBUTE,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn display_remove(
        &self,
        req: HostDisplayRemoveRequest,
    ) -> crate::error::Result<HostDisplayRemoveResponse> {
        self.call(
            method::HOST_UI_DISPLAY_REMOVE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn notify(&self, req: PluginNotifyRequest) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_NOTIFY,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn ui_theme_register(&self, req: HostThemeRegisterRequest) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_UI_THEME_REGISTER,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn ui_theme_list(&self) -> crate::error::Result<HostThemeListResponse> {
        self.call(
            method::HOST_UI_THEME_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn ui_theme_remove(
        &self,
        req: HostThemeRemoveRequest,
    ) -> crate::error::Result<HostThemeRemoveResponse> {
        self.call(
            method::HOST_UI_THEME_REMOVE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }
});

fn error_object_from(e: PluginError) -> ErrorObject {
    let code = match e.kind {
        PluginErrorKind::Cancelled => codes::REQUEST_CANCELLED,
        PluginErrorKind::Internal => codes::PLUGIN_GENERIC,
        PluginErrorKind::NotImplemented => codes::PLUGIN_NOT_IMPLEMENTED,
        PluginErrorKind::InvalidParams => codes::PLUGIN_INVALID_PARAMS,
        PluginErrorKind::Timeout => codes::PLUGIN_TIMEOUT,
        PluginErrorKind::Disconnected => codes::PLUGIN_DISCONNECTED,
        PluginErrorKind::Panicked => codes::PLUGIN_PANICKED,
        PluginErrorKind::HostUnavailable => codes::HOST_UNAVAILABLE,
        PluginErrorKind::PolicyDenied => codes::POLICY_DENIED,
        PluginErrorKind::UserDeclined => codes::USER_DECLINED,
        PluginErrorKind::CapabilityUnavailable => codes::CAPABILITY_UNAVAILABLE,
        PluginErrorKind::ToolUnavailable => codes::TOOL_UNAVAILABLE,
    };
    ErrorObject {
        code,
        message: e.to_string(),
        data: e.rpc_error_data(),
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};

    struct WaitingPlugin {
        started: mpsc::UnboundedSender<(i64, crate::CancellationToken)>,
    }

    #[async_trait::async_trait]
    impl Plugin for WaitingPlugin {
        fn manifest(&self) -> crate::PluginManifest {
            crate::PluginManifest::new("test", "stdio-cancellation", "1.0.0")
        }

        async fn tool_invoke(
            &self,
            input: crate::ToolInvokeInput,
        ) -> crate::Result<ToolInvokeOutput> {
            self.started
                .send((input.call_id, input.context().cancellation))
                .unwrap();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn cancellation_is_received_when_all_stdio_dispatch_slots_are_busy() {
        let (started, mut signals) = mpsc::unbounded_channel();
        let (host_write, plugin_read) = tokio::io::duplex(256 * 1024);
        let (plugin_write, host_read) = tokio::io::duplex(256 * 1024);
        let server = tokio::spawn(serve_stdio_io(
            WaitingPlugin { started },
            plugin_read,
            plugin_write,
            Arc::new(Semaphore::new(STDIO_DISPATCH_CONCURRENCY)),
        ));
        let mut writer =
            FramedWrite::new(host_write, ContentLengthCodec::new(STDIO_MAX_FRAME_BYTES));
        let mut reader = FramedRead::new(host_read, ContentLengthCodec::new(STDIO_MAX_FRAME_BYTES));
        for id in 0..STDIO_DISPATCH_CONCURRENCY as i64 {
            let request = Request {
                jsonrpc: JsonRpcVersion,
                id: RequestId::Num(id),
                method: method::HOOK_TOOL_INVOKE.into(),
                params: Some(
                    serde_json::to_value(crate::ToolInvokeInput {
                        tool_name: "wait".into(),
                        session_id: 1,
                        call_id: id,
                        workspace_root: "/tmp".into(),
                        input: serde_json::json!({}),
                    })
                    .unwrap(),
                ),
                context: None,
            };
            writer
                .send(Bytes::from(serde_json::to_vec(&request).unwrap()))
                .await
                .unwrap();
        }
        let mut active = HashMap::new();
        for _ in 0..STDIO_DISPATCH_CONCURRENCY {
            let (id, token) = tokio::time::timeout(Duration::from_secs(2), signals.recv())
                .await
                .unwrap()
                .unwrap();
            active.insert(id, token);
        }
        for id in 0..STDIO_DISPATCH_CONCURRENCY as i64 {
            let cancellation = Notification {
                jsonrpc: JsonRpcVersion,
                method: method::CANCEL_REQUEST.into(),
                params: Some(serde_json::json!({"id": id})),
            };
            writer
                .send(Bytes::from(serde_json::to_vec(&cancellation).unwrap()))
                .await
                .unwrap();
        }
        for _ in 0..STDIO_DISPATCH_CONCURRENCY {
            let body = tokio::time::timeout(Duration::from_secs(2), reader.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let response: Response = serde_json::from_slice(&body).unwrap();
            assert!(
                matches!(response.payload, ResponsePayload::Err { error } if error.code == codes::REQUEST_CANCELLED)
            );
        }
        assert!(active.values().all(crate::CancellationToken::is_cancelled));
        drop(writer);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    struct ReturningPlugin {
        finished: mpsc::UnboundedSender<crate::CancellationToken>,
    }

    #[async_trait::async_trait]
    impl Plugin for ReturningPlugin {
        fn manifest(&self) -> crate::PluginManifest {
            crate::PluginManifest::new("test", "returning", "1.0.0")
        }

        async fn tool_invoke(
            &self,
            input: crate::ToolInvokeInput,
        ) -> crate::Result<ToolInvokeOutput> {
            self.finished.send(input.context().cancellation).unwrap();
            Ok(ToolInvokeOutput::text("done"))
        }
    }

    #[tokio::test]
    async fn cancellation_releases_capacity_when_an_ordinary_response_is_blocked() {
        use tokio::io::AsyncReadExt;

        let (finished, mut completions) = mpsc::unbounded_channel();
        let slots = Arc::new(Semaphore::new(STDIO_DISPATCH_CONCURRENCY));
        let (host_write, plugin_read) = tokio::io::duplex(4096);
        let (plugin_write, mut host_read) = tokio::io::duplex(1);
        let server = tokio::spawn(serve_stdio_io(
            ReturningPlugin { finished },
            plugin_read,
            plugin_write,
            slots.clone(),
        ));
        let mut writer =
            FramedWrite::new(host_write, ContentLengthCodec::new(STDIO_MAX_FRAME_BYTES));
        let mut blocked = None;
        // One response blocks in stdout and the next 256 fill its queue.
        // The final request has completed its body but cannot enqueue output.
        for id in 0..STDIO_WRITER_QUEUE_CAPACITY as i64 + 2 {
            let request = Request {
                jsonrpc: JsonRpcVersion,
                id: RequestId::Num(id),
                method: method::HOOK_TOOL_INVOKE.into(),
                params: Some(serde_json::json!({
                    "tool_name":"return", "session_id":1, "call_id":id,
                    "workspace_root":"/tmp", "input":{}
                })),
                context: None,
            };
            writer
                .send(Bytes::from(serde_json::to_vec(&request).unwrap()))
                .await
                .unwrap();
            let signal = tokio::time::timeout(Duration::from_secs(2), completions.recv())
                .await
                .unwrap()
                .unwrap();
            if id == 0 {
                tokio::time::timeout(Duration::from_secs(2), host_read.read_u8())
                    .await
                    .unwrap()
                    .unwrap();
            }
            if id <= STDIO_WRITER_QUEUE_CAPACITY as i64 {
                tokio::time::timeout(Duration::from_secs(2), signal.cancelled())
                    .await
                    .unwrap();
                let idle = tokio::time::timeout(
                    Duration::from_secs(2),
                    slots.acquire_many(STDIO_DISPATCH_CONCURRENCY as u32),
                )
                .await
                .unwrap()
                .unwrap();
                drop(idle);
            } else {
                assert!(!signal.is_cancelled());
                blocked = Some((id, signal));
            }
        }
        assert_eq!(slots.available_permits(), STDIO_DISPATCH_CONCURRENCY - 1);
        let (id, signal) = blocked.unwrap();
        let cancellation = Notification {
            jsonrpc: JsonRpcVersion,
            method: method::CANCEL_REQUEST.into(),
            params: Some(serde_json::json!({"id":id})),
        };
        writer
            .send(Bytes::from(serde_json::to_vec(&cancellation).unwrap()))
            .await
            .unwrap();
        let idle = tokio::time::timeout(
            Duration::from_secs(2),
            slots.acquire_many(STDIO_DISPATCH_CONCURRENCY as u32),
        )
        .await
        .expect("cancelled response delivery must release its dispatch capacity")
        .unwrap();
        assert!(signal.is_cancelled());
        drop(idle);
        drop(writer);
        let mut discarded = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(2),
            host_read.read_to_end(&mut discarded),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    struct BufferedStreamPlugin {
        ready: Arc<tokio::sync::Notify>,
        finished: mpsc::UnboundedSender<crate::CancellationToken>,
        chunks: usize,
    }

    #[async_trait::async_trait]
    impl Plugin for BufferedStreamPlugin {
        fn manifest(&self) -> crate::PluginManifest {
            crate::PluginManifest::new("test", "buffered-stream", "1.0.0")
        }

        async fn tool_invoke_stream(
            &self,
            input: crate::ToolInvokeInput,
            sink: crate::ToolStreamSink,
        ) -> crate::Result<crate::ToolStreamEnd> {
            self.ready.notified().await;
            for _ in 0..self.chunks {
                sink.text("buffered").await;
            }
            self.finished.send(input.context().cancellation).unwrap();
            Ok(crate::ToolStreamEnd::text(sink.stream_id(), "done"))
        }
    }

    #[tokio::test]
    async fn cancellation_releases_stream_capacity_when_chunk_or_terminal_writes_are_blocked() {
        use tokio::io::AsyncReadExt;

        // A blocked setup frame leaves room for exactly the writer queue's
        // chunks. The extra case also fills the stream buffer, so cancellation
        // must interrupt both chunk forwarding and the final terminal write.
        for chunks in [
            STDIO_WRITER_QUEUE_CAPACITY,
            STDIO_WRITER_QUEUE_CAPACITY + 65,
        ] {
            let (finished, mut completions) = mpsc::unbounded_channel();
            let ready = Arc::new(tokio::sync::Notify::new());
            let slots = Arc::new(Semaphore::new(STDIO_DISPATCH_CONCURRENCY));
            let (host_write, plugin_read) = tokio::io::duplex(4096);
            let (plugin_write, mut host_read) = tokio::io::duplex(1);
            let server = tokio::spawn(serve_stdio_io(
                BufferedStreamPlugin {
                    ready: ready.clone(),
                    finished,
                    chunks,
                },
                plugin_read,
                plugin_write,
                slots.clone(),
            ));
            let mut writer =
                FramedWrite::new(host_write, ContentLengthCodec::new(STDIO_MAX_FRAME_BYTES));
            let request = Request {
                jsonrpc: JsonRpcVersion,
                id: RequestId::Num(1),
                method: method::HOOK_TOOL_INVOKE_STREAM.into(),
                params: Some(serde_json::json!({
                    "tool_name":"buffered", "session_id":1, "call_id":1,
                    "workspace_root":"/tmp", "input":{}
                })),
                context: None,
            };
            writer
                .send(Bytes::from(serde_json::to_vec(&request).unwrap()))
                .await
                .unwrap();
            // The writer has removed setup from its queue and is blocked on
            // the one-byte pipe. Keep stdout stalled until after cancellation.
            tokio::time::timeout(Duration::from_secs(2), host_read.read_u8())
                .await
                .unwrap()
                .unwrap();
            ready.notify_one();
            let signal = tokio::time::timeout(Duration::from_secs(2), completions.recv())
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), signal.cancelled())
                .await
                .unwrap();
            assert_eq!(slots.available_permits(), STDIO_DISPATCH_CONCURRENCY - 1);
            let cancellation = Notification {
                jsonrpc: JsonRpcVersion,
                method: method::CANCEL_REQUEST.into(),
                params: Some(serde_json::json!({"id":1})),
            };
            writer
                .send(Bytes::from(serde_json::to_vec(&cancellation).unwrap()))
                .await
                .unwrap();
            let all_slots = tokio::time::timeout(
                Duration::from_secs(2),
                slots.acquire_many(STDIO_DISPATCH_CONCURRENCY as u32),
            )
            .await
            .expect("cancelled stream forwarding must release its dispatch capacity")
            .unwrap();
            drop(all_slots);
            drop(writer);
            let mut discarded = Vec::new();
            tokio::time::timeout(
                Duration::from_secs(2),
                host_read.read_to_end(&mut discarded),
            )
            .await
            .unwrap()
            .unwrap();
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
    }

    struct BodyDrop {
        id: i64,
        stopped: mpsc::UnboundedSender<i64>,
    }

    impl Drop for BodyDrop {
        fn drop(&mut self) {
            let _ = self.stopped.send(self.id);
        }
    }

    struct BackpressurePlugin {
        started: mpsc::UnboundedSender<(i64, crate::CancellationToken)>,
        stopped: mpsc::UnboundedSender<i64>,
        ready: Arc<tokio::sync::Notify>,
        full: mpsc::UnboundedSender<()>,
    }

    impl BackpressurePlugin {
        fn begin(&self, input: &crate::ToolInvokeInput) -> BodyDrop {
            self.started
                .send((input.call_id, input.context().cancellation))
                .unwrap();
            BodyDrop {
                id: input.call_id,
                stopped: self.stopped.clone(),
            }
        }
    }

    #[async_trait::async_trait]
    impl Plugin for BackpressurePlugin {
        fn manifest(&self) -> crate::PluginManifest {
            crate::PluginManifest::new("test", "stdio-backpressure", "1.0.0")
        }

        async fn tool_invoke(
            &self,
            input: crate::ToolInvokeInput,
        ) -> crate::Result<ToolInvokeOutput> {
            let _body = self.begin(&input);
            std::future::pending().await
        }

        async fn tool_invoke_stream(
            &self,
            input: crate::ToolInvokeInput,
            sink: crate::ToolStreamSink,
        ) -> crate::Result<crate::ToolStreamEnd> {
            let _body = self.begin(&input);
            self.ready.notified().await;
            for _ in 0..STDIO_WRITER_QUEUE_CAPACITY + 65 {
                sink.text("buffered").await;
            }
            self.full.send(()).unwrap();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn overload_with_a_full_writer_disconnects_and_drops_all_running_bodies() {
        use tokio::io::AsyncReadExt;

        let (started, mut starts) = mpsc::unbounded_channel();
        let (stopped, mut stops) = mpsc::unbounded_channel();
        let (full, mut filled) = mpsc::unbounded_channel();
        let ready = Arc::new(tokio::sync::Notify::new());
        let slots = Arc::new(Semaphore::new(STDIO_DISPATCH_CONCURRENCY));
        let (host_write, plugin_read) = tokio::io::duplex(256 * 1024);
        let (plugin_write, mut host_read) = tokio::io::duplex(1);
        let server = tokio::spawn(serve_stdio_io(
            BackpressurePlugin {
                started,
                stopped,
                ready: ready.clone(),
                full,
            },
            plugin_read,
            plugin_write,
            slots.clone(),
        ));
        let mut writer =
            FramedWrite::new(host_write, ContentLengthCodec::new(STDIO_MAX_FRAME_BYTES));
        let mut signals = Vec::new();
        for id in 0..=STDIO_DISPATCH_CONCURRENCY as i64 {
            let request = Request {
                jsonrpc: JsonRpcVersion,
                id: RequestId::Num(id),
                method: if id == 0 {
                    method::HOOK_TOOL_INVOKE_STREAM.into()
                } else {
                    method::HOOK_TOOL_INVOKE.into()
                },
                params: Some(serde_json::json!({
                    "tool_name":"wait", "session_id":1, "call_id":id,
                    "workspace_root":"/tmp", "input":{}
                })),
                context: None,
            };
            writer
                .send(Bytes::from(serde_json::to_vec(&request).unwrap()))
                .await
                .unwrap();
            if id == 0 {
                tokio::time::timeout(Duration::from_secs(2), host_read.read_u8())
                    .await
                    .unwrap()
                    .unwrap();
                ready.notify_one();
                tokio::time::timeout(Duration::from_secs(2), filled.recv())
                    .await
                    .unwrap()
                    .unwrap();
            }
            if id == STDIO_DISPATCH_CONCURRENCY as i64 - 1 {
                // Ensure the next frame encounters overload while the writer
                // is already full and all ordinary dispatch slots are busy.
                for _ in 0..STDIO_DISPATCH_CONCURRENCY {
                    let (_, signal) = tokio::time::timeout(Duration::from_secs(2), starts.recv())
                        .await
                        .unwrap()
                        .unwrap();
                    signals.push(signal);
                }
                assert_eq!(slots.available_permits(), 0);
                assert!(signals.iter().all(|signal| !signal.is_cancelled()));
            }
        }
        let mut dropped = std::collections::HashSet::new();
        for _ in 0..STDIO_DISPATCH_CONCURRENCY {
            let id = tokio::time::timeout(Duration::from_secs(2), stops.recv())
                .await
                .expect("disconnect must drop every running plugin body")
                .unwrap();
            assert!(dropped.insert(id));
        }
        let error = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("a full writer cannot leave the reader waiting on its overload response")
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
        assert!(error.to_string().contains("writer queue is full"));
        assert!(signals.iter().all(crate::CancellationToken::is_cancelled));
        assert_eq!(slots.available_permits(), STDIO_DISPATCH_CONCURRENCY);
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;
    use crate::host_api::{
        HostCallbackContext, PathPermissionQuery, PermissionQuery,
        run_in_isolated_host_callback_context,
    };

    #[tokio::test]
    async fn permission_callbacks_send_authority_and_preserve_typed_results() {
        let (tx, mut rx) = mpsc::channel::<Bytes>(4);
        let pending = Arc::new(StdMutex::new(
            HashMap::<RequestId, oneshot::Sender<Response>>::new(),
        ));
        let client = StdioHostClient {
            tx,
            pending: pending.clone(),
            next_id: Arc::new(AtomicI64::new(1)),
        };
        let context = HostCallbackContext {
            session_id: Some(31),
            call_id: Some(47),
            authority_token: Some("opaque-test-authority".into()),
            ..Default::default()
        };
        let expected = serde_json::to_value(&context).unwrap();
        let echo = tokio::spawn(async move {
            let mut methods = Vec::new();
            while let Some(body) = rx.recv().await {
                let request: Request = serde_json::from_slice(&body).unwrap();
                assert_eq!(request.params.as_ref().unwrap()["context"], expected);
                let payload = if request.method.contains(".check_") {
                    ResponsePayload::Ok {
                        result: serde_json::to_value(PermissionQuery::Ask {
                            reason: "needs approval".into(),
                        })
                        .unwrap(),
                    }
                } else {
                    let error =
                        PluginError::from_kind(PluginErrorKind::UserDeclined, "wire decline");
                    ResponsePayload::Err {
                        error: crate::rpc::ErrorObject {
                            code: crate::rpc::codes::USER_DECLINED,
                            message: error.to_string(),
                            data: error.rpc_error_data(),
                        },
                    }
                };
                methods.push(request.method);
                pending
                    .lock()
                    .unwrap()
                    .remove(&request.id)
                    .unwrap()
                    .send(Response {
                        jsonrpc: JsonRpcVersion,
                        id: request.id,
                        payload,
                    })
                    .unwrap();
            }
            methods
        });
        run_in_isolated_host_callback_context(context, async {
            assert!(matches!(
                client
                    .check_path_permission(PathPermissionQuery::read("file"))
                    .await
                    .unwrap(),
                PermissionQuery::Ask { .. }
            ));
            assert!(matches!(
                client
                    .check_network_permission("https://example.org".into())
                    .await
                    .unwrap(),
                PermissionQuery::Ask { .. }
            ));
            assert_eq!(
                client
                    .require_path_permission(PathPermissionQuery::read("file"))
                    .await
                    .unwrap_err()
                    .kind,
                PluginErrorKind::UserDeclined
            );
            assert_eq!(
                client
                    .require_network_permission("https://example.org".into())
                    .await
                    .unwrap_err()
                    .kind,
                PluginErrorKind::UserDeclined
            );
        })
        .await;
        drop(client);
        assert_eq!(
            echo.await.unwrap(),
            [
                method::HOST_PERMISSION_CHECK_PATH,
                method::HOST_PERMISSION_CHECK_NETWORK,
                method::HOST_PERMISSION_REQUIRE_PATH,
                method::HOST_PERMISSION_REQUIRE_NETWORK
            ]
        );
    }
}

#[cfg(test)]
use crate::hooks::ToolInvokeOutput;
