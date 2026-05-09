//! Stdio JSON-RPC driver. Plugin author calls `serve_stdio(plugin).await`
//! from their `main` (or use `export_stdio!`).
//!
//! Frames are LSP-style Content-Length newline-delimited.
//!
//! Architecture:
//!   - one reader task drains stdin → demuxes frames
//!   - one writer task drains an mpsc channel → flushes stdout
//!   - dispatcher runs on the tokio runtime; responses go back via the channel

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::drivers::dispatch::PluginDispatcher;
use crate::error::{PluginError, PluginErrorCode};
use crate::hooks::{
    EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision, ToolInvokeOutput,
    ToolInvokeStreamHandle, ToolStreamError,
};
use crate::host_api::{
    AskUserRequest, AskUserResponse, EventSubscription, HostAgentListResponse,
    HostAgentRegisterRequest, HostAgentRemoveRequest, HostAgentRemoveResponse, HostClient,
    HostEnterPlanModeRequest, HostEnterWorktreeRequest, HostEntryListResponse,
    HostEntryMutationResponse, HostEntryRegisterRequest, HostEntryRemoveRequest,
    HostEntryUpdateRequest, HostExitPlanModeRequest, HostExitWorktreeRequest, HostHookListResponse,
    HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse, HostLspListServersResponse,
    HostMcpAddServerRequest, HostMcpListServersResponse, HostMcpRemoveServerRequest,
    HostMcpRemoveServerResponse, HostPlanGetRequest, HostPlanGetResponse, HostPlanListResponse,
    HostPluginStatusGetRequest, HostPluginStatusGetResponse, HostPluginStatusListResponse,
    HostSchedulerCreateRequest, HostSchedulerCreateResponse, HostSchedulerDeleteRequest,
    HostSchedulerDeleteResponse, HostSchedulerListResponse, HostSecretDeleteRequest,
    HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest,
    HostStatuslineContributeRequest, HostStatuslineListResponse, HostStatuslineRemoveRequest,
    HostStatuslineRemoveResponse, HostStorageDeleteRequest, HostStorageGetRequest,
    HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse, HostStorageSetRequest,
    HostThemeListResponse, HostThemeRegisterRequest, HostThemeRemoveRequest,
    HostThemeRemoveResponse, HostTodoWriteRequest, HostWorktreeListResponse, LogLevel,
    MonitorHandle, MonitorReadRequest, MonitorReadResponse, MonitorStartRequest,
    MonitorStopRequest, SpawnSubtaskRequest, SpawnSubtaskResponse, ToolDescriptor,
};
use crate::plugin::Plugin;
use crate::rpc::{
    ErrorObject, Frame, JsonRpcVersion, Notification, Request, RequestId, Response,
    ResponsePayload, codes, method,
};

/// Run a plugin to completion as a stdio JSON-RPC server. Returns when stdin
/// closes.
pub async fn serve_stdio<P: Plugin>(plugin: P) -> std::io::Result<()> {
    let dispatcher = Arc::new(PluginDispatcher::new(plugin));

    let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<Response>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let next_id = Arc::new(Mutex::new(1i64));

    let host: Arc<dyn HostClient> = Arc::new(StdioHostClient {
        tx: tx.clone(),
        pending: Arc::clone(&pending),
        next_id: Arc::clone(&next_id),
    });
    dispatcher.set_host(host).await;

    // Writer task: owns stdout, drains channel.
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        let mut rx = rx;
        while let Some(payload) = rx.recv().await {
            let header = format!("Content-Length: {}\r\n\r\n", payload.len());
            if stdout.write_all(header.as_bytes()).await.is_err() {
                break;
            }
            if stdout.write_all(&payload).await.is_err() {
                break;
            }
            let _ = stdout.flush().await;
        }
    });

    // Reader task: owns stdin, demuxes frames.
    let mut reader = BufReader::new(tokio::io::stdin());
    loop {
        // Read header lines until blank line; capture Content-Length.
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                drop(tx);
                let _ = writer.await;
                return Ok(());
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
                content_length = rest.trim().parse().ok();
            }
        }
        let len = match content_length {
            Some(v) => v,
            None => continue,
        };
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).await?;

        let frame: Frame = match serde_json::from_slice(&body) {
            Ok(f) => f,
            Err(_) => continue,
        };

        match frame {
            Frame::Request(req) => {
                let dispatcher = Arc::clone(&dispatcher);
                let tx = tx.clone();
                tokio::spawn(async move {
                    let id = req.id.clone();
                    let params = req.params.unwrap_or(serde_json::Value::Null);
                    if req.method == method::HOOK_TOOL_INVOKE_STREAM {
                        match serde_json::from_value(params) {
                            Ok(input) => {
                                let mut handle = dispatcher.dispatch_stream(input);
                                let stream_id = handle.stream_id.clone();
                                send_response(
                                    &tx,
                                    Response {
                                        jsonrpc: JsonRpcVersion,
                                        id,
                                        payload: ResponsePayload::Ok {
                                            result: serde_json::to_value(ToolInvokeStreamHandle {
                                                stream_id: stream_id.clone(),
                                                title: None,
                                            })
                                            .expect("stream handle serialize"),
                                        },
                                    },
                                );
                                let tx = tx.clone();
                                tokio::spawn(async move {
                                    while let Some(chunk) = handle.chunks.recv().await {
                                        send_notification(&tx, method::TOOL_STREAM_CHUNK, &chunk);
                                    }
                                    match handle.end.await {
                                        Ok(Ok(end)) => {
                                            send_notification(&tx, method::TOOL_STREAM_END, &end);
                                        }
                                        Ok(Err(error)) => {
                                            send_notification(
                                                &tx,
                                                method::TOOL_STREAM_ERROR,
                                                &ToolStreamError { stream_id, error },
                                            );
                                        }
                                        Err(_) => {
                                            send_notification(
                                                &tx,
                                                method::TOOL_STREAM_ERROR,
                                                &ToolStreamError {
                                                    stream_id,
                                                    error: PluginError::new(
                                                        "stream terminated before sending final frame",
                                                    ),
                                                },
                                            );
                                        }
                                    }
                                });
                            }
                            Err(err) => send_response(
                                &tx,
                                Response {
                                    jsonrpc: JsonRpcVersion,
                                    id,
                                    payload: ResponsePayload::Err {
                                        error: error_object_from(PluginError::invalid_params(
                                            err.to_string(),
                                        )),
                                    },
                                },
                            ),
                        }
                        return;
                    }
                    let resp = match dispatcher.dispatch(&req.method, params).await {
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
                    send_response(&tx, resp);
                });
            }
            Frame::Notification(notif) => {
                // Plugins can receive notifications (like `hooks/event`).
                let dispatcher = Arc::clone(&dispatcher);
                tokio::spawn(async move {
                    let _ = dispatcher
                        .dispatch(
                            &notif.method,
                            notif.params.unwrap_or(serde_json::Value::Null),
                        )
                        .await;
                });
            }
            Frame::Response(resp) => {
                if let Some(slot) = pending.lock().await.remove(&resp.id) {
                    let _ = slot.send(resp);
                }
            }
        }
    }
}

fn send_response(tx: &mpsc::UnboundedSender<Vec<u8>>, response: Response) {
    if let Ok(body) = serde_json::to_vec(&response) {
        let _ = tx.send(body);
    }
}

fn send_notification<T: serde::Serialize>(
    tx: &mpsc::UnboundedSender<Vec<u8>>,
    method_name: &str,
    params: &T,
) {
    let params = match serde_json::to_value(params) {
        Ok(params) => params,
        Err(_) => return,
    };
    let notification = Notification {
        jsonrpc: JsonRpcVersion,
        method: method_name.to_string(),
        params: Some(params),
    };
    if let Ok(body) = serde_json::to_vec(&notification) {
        let _ = tx.send(body);
    }
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
    tx: mpsc::UnboundedSender<Vec<u8>>,
    pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<Response>>>>,
    next_id: Arc<Mutex<i64>>,
}

impl StdioHostClient {
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> crate::error::Result<T> {
        let id = {
            let mut g = self.next_id.lock().await;
            let v = *g;
            *g += 1;
            v
        };
        let req_id = RequestId::Num(id);
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: req_id.clone(),
            method: method.to_string(),
            params: Some(params),
        };
        let body =
            serde_json::to_vec(&req).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let (slot_tx, slot_rx) = oneshot::channel();
        self.pending.lock().await.insert(req_id, slot_tx);
        self.tx.send(body).map_err(|_| PluginError {
            code: PluginErrorCode::Disconnected,
            message: "host writer closed".into(),
            hook: None,
            plugin: None,
            data: None,
        })?;
        let resp = slot_rx.await.map_err(|_| PluginError {
            code: PluginErrorCode::Disconnected,
            message: "host closed connection".into(),
            hook: None,
            plugin: None,
            data: None,
        })?;
        match resp.payload {
            ResponsePayload::Ok { result } => serde_json::from_value(result)
                .map_err(|e| PluginError::invalid_params(e.to_string())),
            ResponsePayload::Err { error } => Err(PluginError {
                code: PluginErrorCode::Generic,
                message: error.message,
                hook: None,
                plugin: None,
                data: error.data,
            }),
        }
    }

    fn notify(&self, method: &str, params: serde_json::Value) {
        let n = Notification {
            jsonrpc: JsonRpcVersion,
            method: method.to_string(),
            params: Some(params),
        };
        if let Ok(body) = serde_json::to_vec(&n) {
            let _ = self.tx.send(body);
        }
    }
}

#[async_trait::async_trait]
impl HostClient for StdioHostClient {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        self.notify(
            method::HOST_LOG,
            serde_json::json!({ "level": level, "message": message, "fields": fields }),
        );
    }

    async fn publish_event(&self, env: EventEnvelope) -> crate::error::Result<()> {
        self.notify(
            method::HOST_EVENT_PUBLISH,
            serde_json::to_value(env).map_err(|e| PluginError::invalid_params(e.to_string()))?,
        );
        Ok(())
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

    async fn ask_permission(
        &self,
        req: PermissionAskInput,
    ) -> crate::error::Result<PermissionDecision> {
        self.call(
            method::HOST_PERMISSION_ASK,
            serde_json::to_value(req).map_err(|e| PluginError::invalid_params(e.to_string()))?,
        )
        .await
    }

    async fn read_config(&self, path: Option<String>) -> crate::error::Result<serde_json::Value> {
        self.call(
            method::HOST_CONFIG_READ,
            serde_json::json!({ "path": path }),
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

    async fn spawn_subtask(
        &self,
        req: SpawnSubtaskRequest,
    ) -> crate::error::Result<SpawnSubtaskResponse> {
        self.call(
            method::HOST_SUBTASK_SPAWN,
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

    async fn todo_write(
        &self,
        req: HostTodoWriteRequest,
    ) -> crate::error::Result<ToolInvokeOutput> {
        self.call(
            method::HOST_TODO_WRITE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn enter_plan_mode(
        &self,
        req: HostEnterPlanModeRequest,
    ) -> crate::error::Result<ToolInvokeOutput> {
        self.call(
            method::HOST_PLAN_ENTER,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn exit_plan_mode(
        &self,
        req: HostExitPlanModeRequest,
    ) -> crate::error::Result<ToolInvokeOutput> {
        self.call(
            method::HOST_PLAN_EXIT,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn enter_worktree(
        &self,
        req: HostEnterWorktreeRequest,
    ) -> crate::error::Result<ToolInvokeOutput> {
        self.call(
            method::HOST_WORKTREE_ENTER,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn exit_worktree(
        &self,
        req: HostExitWorktreeRequest,
    ) -> crate::error::Result<ToolInvokeOutput> {
        self.call(
            method::HOST_WORKTREE_EXIT,
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

    async fn entry_register(
        &self,
        req: HostEntryRegisterRequest,
    ) -> crate::error::Result<HostEntryMutationResponse> {
        self.call(
            method::HOST_ENTRY_REGISTER,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn entry_update(
        &self,
        req: HostEntryUpdateRequest,
    ) -> crate::error::Result<HostEntryMutationResponse> {
        self.call(
            method::HOST_ENTRY_UPDATE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn entry_remove(
        &self,
        req: HostEntryRemoveRequest,
    ) -> crate::error::Result<HostEntryMutationResponse> {
        self.call(
            method::HOST_ENTRY_REMOVE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn entry_list(&self) -> crate::error::Result<HostEntryListResponse> {
        self.call(
            method::HOST_ENTRY_LIST,
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

    async fn plan_list(&self) -> crate::error::Result<HostPlanListResponse> {
        self.call(
            method::HOST_PLAN_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn plan_get(&self, req: HostPlanGetRequest) -> crate::error::Result<HostPlanGetResponse> {
        self.call(
            method::HOST_PLAN_GET,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn worktree_list(&self) -> crate::error::Result<HostWorktreeListResponse> {
        self.call(
            method::HOST_WORKTREE_LIST,
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

    async fn agent_register(&self, req: HostAgentRegisterRequest) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_AGENT_REGISTER,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn agent_remove(
        &self,
        req: HostAgentRemoveRequest,
    ) -> crate::error::Result<HostAgentRemoveResponse> {
        self.call(
            method::HOST_AGENT_REMOVE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn agent_list(&self) -> crate::error::Result<HostAgentListResponse> {
        self.call(
            method::HOST_AGENT_LIST,
            serde_json::json!({
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

    async fn ui_statusline_contribute(
        &self,
        req: HostStatuslineContributeRequest,
    ) -> crate::error::Result<()> {
        let _: serde_json::Value = self
            .call(
                method::HOST_UI_STATUSLINE_CONTRIBUTE,
                serde_json::json!({
                    "request": req,
                    "context": crate::host_api::current_host_callback_context(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn ui_statusline_list(&self) -> crate::error::Result<HostStatuslineListResponse> {
        self.call(
            method::HOST_UI_STATUSLINE_LIST,
            serde_json::json!({
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn ui_statusline_remove(
        &self,
        req: HostStatuslineRemoveRequest,
    ) -> crate::error::Result<HostStatuslineRemoveResponse> {
        self.call(
            method::HOST_UI_STATUSLINE_REMOVE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
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
    let code = match e.code {
        PluginErrorCode::Generic => codes::PLUGIN_GENERIC,
        PluginErrorCode::NotImplemented => codes::PLUGIN_NOT_IMPLEMENTED,
        PluginErrorCode::InvalidParams => codes::PLUGIN_INVALID_PARAMS,
        PluginErrorCode::Timeout => codes::PLUGIN_TIMEOUT,
        PluginErrorCode::Disconnected => codes::PLUGIN_DISCONNECTED,
        PluginErrorCode::Panicked => codes::PLUGIN_PANICKED,
        PluginErrorCode::HostUnavailable => codes::HOST_UNAVAILABLE,
    };
    ErrorObject {
        code,
        message: e.message.clone(),
        data: serde_json::to_value(&e).ok(),
    }
}
