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

use crate::drivers::dispatch::PluginDispatcher;
use crate::error::{PluginError, PluginErrorKind};
use crate::hooks::{
    EventEnvelope, EventFilter, ToolInvokeOutput, ToolInvokeStreamHandle, ToolStreamError,
};
use crate::host_api::{
    AskUserRequest, AskUserResponse, CancelSubtaskRequest, EventSubscription, HostClient,
    HostConfigReloadResponse, HostContextStatusRequest, HostContextStatusResponse,
    HostDisplayContributeRequest, HostDisplayRemoveRequest, HostDisplayRemoveResponse,
    HostEnterSnapshotRequest, HostExitSnapshotRequest, HostHookListResponse,
    HostImageExecuteRequest, HostImageExecuteResponse, HostLspListDiagnosticsRequest,
    HostLspListDiagnosticsResponse, HostLspListServersResponse, HostMcpAddServerRequest,
    HostMcpListServersResponse, HostMcpRemoveServerRequest, HostMcpRemoveServerResponse,
    HostPluginStatusGetRequest, HostPluginStatusGetResponse, HostPluginStatusListResponse,
    HostRegisteredToolListResponse, HostSchedulerCreateRequest, HostSchedulerCreateResponse,
    HostSchedulerDeleteRequest, HostSchedulerDeleteResponse, HostSchedulerListResponse,
    HostSecretDeleteRequest, HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse,
    HostSecretSetRequest, HostSetSessionModelRequest, HostSetSessionModelResponse,
    HostSnapshotListResponse, HostStorageDeleteRequest, HostStorageGetRequest,
    HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse, HostStorageSetRequest,
    HostThemeListResponse, HostThemeRegisterRequest, HostThemeRemoveRequest,
    HostThemeRemoveResponse, HostToolMutationResponse, HostToolRegisterRequest,
    HostToolRemoveRequest, HostToolUpdateRequest, LogLevel, MessageSubtaskRequest, MonitorHandle,
    MonitorReadRequest, MonitorReadResponse, MonitorStartRequest, MonitorStopRequest,
    PluginNotifyRequest, ReadSubtaskOutputRequest, ReadSubtaskOutputResponse, RunSubtaskRequest,
    RunSubtaskResponse, SubtaskControlResponse, ToolDescriptor,
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
        let mut stdout = FramedWrite::new(
            tokio::io::stdout(),
            ContentLengthCodec::new(STDIO_MAX_FRAME_BYTES),
        );
        let mut rx = rx;
        while let Some(payload) = rx.recv().await {
            stdout.send(payload).await.map_err(io::Error::other)?;
        }
        Ok::<(), io::Error>(())
    });

    // The reader must never wait for a dispatch slot: responses to plugin ->
    // host callbacks share this pipe and must always be demultiplexed. Excess
    // host requests receive an explicit overload response instead of spawning
    // an unbounded number of tasks.
    let dispatch_slots = Arc::new(Semaphore::new(STDIO_DISPATCH_CONCURRENCY));
    let mut dispatch_tasks = JoinSet::new();

    // Reader loop: owns stdin, demuxes frames.
    let mut reader = FramedRead::new(
        tokio::io::stdin(),
        ContentLengthCodec::new(STDIO_MAX_FRAME_BYTES),
    );
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
                        let dispatch_slot = Arc::clone(&dispatch_slots).try_acquire_owned();
                        let Ok(dispatch_slot) = dispatch_slot else {
                            if !send_response(
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
                            )
                            .await
                            {
                                return Ok(());
                            }
                            continue;
                        };
                        let dispatcher = Arc::clone(&dispatcher);
                        let tx = tx.clone();
                        dispatch_tasks.spawn(async move {
                        let _dispatch_slot = dispatch_slot;
                        let id = req.id.clone();
                        let params = req.params.unwrap_or(serde_json::Value::Null);
                        let callback_context = req.context;
                        if req.method == method::HOOK_TOOL_INVOKE_STREAM {
                            match serde_json::from_value(params) {
                                Ok(input) => {
                                    let mut handle = if let Some(context) = callback_context {
                                        crate::host_api::run_in_host_callback_context(
                                            context,
                                            async { dispatcher.dispatch_stream(input) },
                                        )
                                        .await
                                    } else {
                                        dispatcher.dispatch_stream(input)
                                    };
                                    let stream_id = handle.stream_id.clone();
                                    let stream_handle = match serde_json::to_value(
                                        ToolInvokeStreamHandle {
                                            stream_id: stream_id.clone(),
                                            title: None,
                                        },
                                    ) {
                                        Ok(value) => value,
                                        Err(error) => {
                                            let _sent = send_response(
                                                &tx,
                                                Response {
                                                    jsonrpc: JsonRpcVersion,
                                                    id,
                                                    payload: ResponsePayload::Err {
                                                        error: error_object_from(
                                                            PluginError::internal_error(&error),
                                                        ),
                                                    },
                                                },
                                            )
                                            .await;
                                            return;
                                        }
                                    };
                                    if !send_response(
                                        &tx,
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
                                        if !send_notification(
                                            &tx,
                                            method::TOOL_STREAM_CHUNK,
                                            &chunk,
                                        )
                                        .await
                                        {
                                            return;
                                        }
                                    }
                                    match handle.end.await {
                                        Ok(Ok(end)) => {
                                            if !send_notification(
                                                &tx,
                                                method::TOOL_STREAM_END,
                                                &end,
                                            )
                                            .await
                                            {
                                                return;
                                            }
                                        }
                                        Ok(Err(error)) => {
                                            if !send_notification(
                                                &tx,
                                                method::TOOL_STREAM_ERROR,
                                                &ToolStreamError { stream_id, error },
                                            )
                                            .await
                                            {
                                                return;
                                            }
                                        }
                                        Err(error) => {
                                            if !send_notification(
                                                &tx,
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
                                            .await
                                            {
                                                return;
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    let _sent = send_response(
                                        &tx,
                                        Response {
                                            jsonrpc: JsonRpcVersion,
                                            id,
                                            payload: ResponsePayload::Err {
                                                error: error_object_from(
                                                    PluginError::invalid_params_error(&err),
                                                ),
                                            },
                                        },
                                    )
                                    .await;
                                }
                            }
                            return;
                        }
                        let dispatch = dispatcher.dispatch(&req.method, params);
                        let result = if let Some(context) = callback_context {
                            crate::host_api::run_in_host_callback_context(context, dispatch).await
                        } else {
                            dispatch.await
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
                                    error: error_object_from(e),
                                },
                            },
                        };
                        let _sent = send_response(&tx, resp).await;
                    });
                    }
                    Frame::Notification(notif) => {
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
            ResponsePayload::Err { error } => {
                let mut plugin_error =
                    PluginError::from_kind(PluginErrorKind::Internal, error.message);
                plugin_error.diagnostic.data = error.data;
                Err(plugin_error)
            }
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

#[async_trait::async_trait]
impl HostClient for StdioHostClient {
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

    async fn reload_config(&self) -> crate::error::Result<HostConfigReloadResponse> {
        self.call(
            method::HOST_CONFIG_RELOAD,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> crate::error::Result<ToolInvokeOutput> {
        self.call(
            method::HOST_TOOL_INVOKE,
            serde_json::json!({
                "tool": tool,
                "input": input,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn invoke_service(
        &self,
        req: crate::PluginServiceInvokeInput,
    ) -> crate::error::Result<crate::PluginServiceInvokeOutput> {
        self.call(
            method::HOST_SERVICE_INVOKE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
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

    async fn run_subtask(
        &self,
        req: RunSubtaskRequest,
    ) -> crate::error::Result<RunSubtaskResponse> {
        self.call(
            method::HOST_SUBTASK_RUN,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn cancel_subtask(
        &self,
        req: CancelSubtaskRequest,
    ) -> crate::error::Result<SubtaskControlResponse> {
        self.call(
            method::HOST_SUBTASK_CANCEL,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn message_subtask(
        &self,
        req: MessageSubtaskRequest,
    ) -> crate::error::Result<SubtaskControlResponse> {
        self.call(
            method::HOST_SUBTASK_MESSAGE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn read_subtask_output(
        &self,
        req: ReadSubtaskOutputRequest,
    ) -> crate::error::Result<ReadSubtaskOutputResponse> {
        self.call(
            method::HOST_SUBTASK_OUTPUT,
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

    async fn image_execute(
        &self,
        req: HostImageExecuteRequest,
    ) -> crate::error::Result<HostImageExecuteResponse> {
        self.call(
            method::HOST_IMAGE_EXECUTE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn enter_snapshot(
        &self,
        req: HostEnterSnapshotRequest,
    ) -> crate::error::Result<ToolInvokeOutput> {
        self.call(
            method::HOST_SNAPSHOT_ENTER,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn exit_snapshot(
        &self,
        req: HostExitSnapshotRequest,
    ) -> crate::error::Result<ToolInvokeOutput> {
        self.call(
            method::HOST_SNAPSHOT_EXIT,
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

    async fn snapshot_list(&self) -> crate::error::Result<HostSnapshotListResponse> {
        self.call(
            method::HOST_SNAPSHOT_LIST,
            serde_json::json!({
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
}

fn error_object_from(e: PluginError) -> ErrorObject {
    let code = match e.kind {
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
mod authorization_error_code_tests {
    use super::error_object_from;
    use crate::{PluginError, PluginErrorKind, rpc::codes};

    #[test]
    fn authorization_and_availability_outcomes_keep_distinct_json_rpc_codes() {
        for (kind, expected) in [
            (PluginErrorKind::PolicyDenied, codes::POLICY_DENIED),
            (PluginErrorKind::UserDeclined, codes::USER_DECLINED),
            (
                PluginErrorKind::CapabilityUnavailable,
                codes::CAPABILITY_UNAVAILABLE,
            ),
            (PluginErrorKind::ToolUnavailable, codes::TOOL_UNAVAILABLE),
        ] {
            let error = PluginError::from_kind(kind, "structured non-execution outcome");
            assert_eq!(error_object_from(error).code, expected);
        }
    }
}
