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
};
use crate::host_api::{
    AskUserRequest, AskUserResponse, BuiltinToolRequest, EventSubscription, HostClient,
    HostEntryListResponse, HostEntryMutationResponse, HostEntryRegisterRequest,
    HostEntryRemoveRequest, HostEntryUpdateRequest, HostPluginStatusGetRequest,
    HostPluginStatusGetResponse, HostPluginStatusListResponse, HostSecretDeleteRequest,
    HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest,
    HostSkillGetRequest, HostSkillGetResponse, HostStorageDeleteRequest, HostStorageGetRequest,
    HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse, HostStorageSetRequest,
    LogLevel, MonitorHandle, MonitorReadRequest, MonitorReadResponse, MonitorStartRequest,
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
                    if let Ok(body) = serde_json::to_vec(&resp) {
                        let _ = tx.send(body);
                    }
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

    async fn execute_builtin_tool(
        &self,
        req: BuiltinToolRequest,
    ) -> crate::error::Result<ToolInvokeOutput> {
        self.call(
            method::HOST_BUILTIN_EXECUTE,
            serde_json::json!({
                "request": req,
                "context": crate::host_api::current_host_callback_context(),
            }),
        )
        .await
    }

    async fn skill_get(
        &self,
        req: HostSkillGetRequest,
    ) -> crate::error::Result<HostSkillGetResponse> {
        self.call(
            method::HOST_SKILL_GET,
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
