//! HTTP driver. Plugin author serves an axum router on their own port; one
//! POST endpoint receives JSON-RPC envelopes and forwards them to dispatch.

use portable_atomic::AtomicI64;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::{Json, Router, extract::State, routing::post};
use tokio::sync::{RwLock, Semaphore};

use crate::drivers::dispatch::PluginDispatcher;
use crate::error::{PluginError, PluginErrorKind};
use crate::hooks::{
    EventEnvelope, EventFilter, ToolInvokeInput, ToolInvokeOutput, ToolInvokeStreamHandle,
    ToolStreamError,
};
use crate::host_api::{
    EventSubscription, HostClient, HostConfigReloadResponse, HostImageExecuteRequest,
    HostImageExecuteResponse, LogLevel,
};
use crate::plugin::{InitContext, Plugin};
use crate::rpc::{
    ErrorObject, JsonRpcVersion, Request, RequestId, Response, ResponsePayload, codes, method,
};

const MAX_CALLBACK_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

struct HttpDriverState<P: Plugin> {
    dispatcher: Arc<PluginDispatcher<P>>,
    callback_client: RwLock<Option<Arc<HttpCallbackHostClient>>>,
    dispatch_slots: Arc<Semaphore>,
}

struct HttpCallbackHostClient {
    client: reqwest::Client,
    url: String,
    auth_header: Option<String>,
    next_id: AtomicI64,
}

impl HttpCallbackHostClient {
    fn from_init_context(ctx: &InitContext) -> Option<Self> {
        let url = ctx.host_callback_url.clone()?;
        Some(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .ok()?,
            url,
            auth_header: ctx
                .host_callback_token
                .as_ref()
                .map(|token| format!("Bearer {token}")),
            next_id: AtomicI64::new(1),
        })
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method_name: &str,
        params: serde_json::Value,
    ) -> crate::error::Result<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: RequestId::Num(id),
            method: method_name.to_string(),
            params: Some(params),
        };
        let mut builder = self.client.post(&self.url).json(&req);
        if let Some(header) = &self.auth_header {
            builder = builder.header("authorization", header);
        }
        let mut resp = builder.send().await.map_err(|error| {
            PluginError::from_kind(PluginErrorKind::Disconnected, error).with_hook(method_name)
        })?;
        let status = resp.status();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|error| {
            PluginError::from_kind(PluginErrorKind::Disconnected, error).with_hook(method_name)
        })? {
            if body.len().saturating_add(chunk.len()) > MAX_CALLBACK_RESPONSE_BYTES {
                return Err(PluginError::from_kind(
                    PluginErrorKind::Disconnected,
                    format_args!(
                        "host callback response exceeds the {} MiB limit",
                        MAX_CALLBACK_RESPONSE_BYTES / 1024 / 1024
                    ),
                )
                .with_hook(method_name));
            }
            body.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(body).map_err(|error| {
            PluginError::from_kind(PluginErrorKind::Disconnected, error).with_hook(method_name)
        })?;
        let resp: Response = serde_json::from_str(&body).map_err(|error| {
            PluginError::from_kind(
                PluginErrorKind::Disconnected,
                format_args!("{status}: {error}; body={body}"),
            )
            .with_hook(method_name)
        })?;
        match resp.payload {
            ResponsePayload::Ok { result } => serde_json::from_value(result)
                .map_err(|e| PluginError::invalid_params(e.to_string())),
            ResponsePayload::Err { error } => {
                let mut plugin_error =
                    PluginError::from_kind(PluginErrorKind::Internal, error.message)
                        .with_hook(method_name);
                plugin_error.diagnostic.data = error.data;
                Err(plugin_error)
            }
        }
    }

    async fn fire(&self, method_name: &str, params: serde_json::Value) -> crate::error::Result<()> {
        let _: serde_json::Value = self.call(method_name, params).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl HostClient for HttpCallbackHostClient {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        let _ = self
            .fire(
                method::HOST_LOG,
                params_with_current_context(serde_json::json!({
                    "level": level,
                    "message": message,
                    "fields": fields,
                })),
            )
            .await;
    }

    async fn publish_event(&self, env: EventEnvelope) -> crate::error::Result<()> {
        self.fire(
            method::HOST_EVENT_PUBLISH,
            params_with_current_context(
                serde_json::to_value(env)
                    .map_err(|e| PluginError::invalid_params(e.to_string()))?,
            ),
        )
        .await
    }

    async fn subscribe_events(
        &self,
        filter: EventFilter,
    ) -> crate::error::Result<EventSubscription> {
        let value: serde_json::Value = self
            .call(
                method::HOST_EVENT_SUBSCRIBE,
                params_with_current_context(serde_json::json!({ "filter": filter })),
            )
            .await?;
        let id = value
            .get("subscription_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        Ok(EventSubscription { id })
    }

    async fn read_config(&self, path: Option<String>) -> crate::error::Result<serde_json::Value> {
        self.call(
            method::HOST_CONFIG_READ,
            params_with_current_context(serde_json::json!({ "path": path })),
        )
        .await
    }

    async fn reload_config(&self) -> crate::error::Result<HostConfigReloadResponse> {
        self.call(
            method::HOST_CONFIG_RELOAD,
            params_with_current_context(serde_json::json!({})),
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
            params_with_current_context(serde_json::json!({
                "tool": tool,
                "input": input,
            })),
        )
        .await
    }

    async fn invoke_service(
        &self,
        req: crate::PluginServiceInvokeInput,
    ) -> crate::error::Result<crate::PluginServiceInvokeOutput> {
        self.call(
            method::HOST_SERVICE_INVOKE,
            params_with_current_context(serde_json::json!({ "request": req })),
        )
        .await
    }

    async fn image_execute(
        &self,
        req: HostImageExecuteRequest,
    ) -> crate::error::Result<HostImageExecuteResponse> {
        self.call(
            method::HOST_IMAGE_EXECUTE,
            params_with_current_context(serde_json::json!({ "request": req })),
        )
        .await
    }
}

pub fn router<P: Plugin>(plugin: P, host: Arc<dyn HostClient>) -> Router {
    let dispatcher = Arc::new(PluginDispatcher::with_host(plugin, host));
    let state = Arc::new(HttpDriverState {
        dispatcher,
        callback_client: RwLock::new(None),
        dispatch_slots: Arc::new(Semaphore::new(64)),
    });
    Router::new()
        .route("/rpc", post(handle_rpc::<P>))
        .with_state(state)
}

async fn handle_rpc<P: Plugin>(
    State(state): State<Arc<HttpDriverState<P>>>,
    Json(req): Json<Request>,
) -> Json<Response> {
    let id = req.id.clone();
    let params = req.params.unwrap_or(serde_json::Value::Null);
    let dispatch_slot = Arc::clone(&state.dispatch_slots).try_acquire_owned();
    let Ok(dispatch_slot) = dispatch_slot else {
        return Json(error_response(
            id,
            PluginError::internal("http plugin dispatch capacity exhausted"),
        ));
    };

    if req.method == method::META_INIT
        && let Ok(ctx) = serde_json::from_value::<InitContext>(params.clone())
        && let Some(callback_client) = HttpCallbackHostClient::from_init_context(&ctx)
    {
        let callback_client = Arc::new(callback_client);
        let host: Arc<dyn HostClient> = callback_client.clone();
        state.dispatcher.set_host(host).await;
        *state.callback_client.write().await = Some(callback_client);
    }

    if req.method == method::HOOK_TOOL_INVOKE_STREAM {
        let Some(callback_client) = state.callback_client.read().await.clone() else {
            return Json(error_response(
                id,
                PluginError::internal("http stream callbacks are unavailable"),
            ));
        };
        let input: ToolInvokeInput = match serde_json::from_value(params) {
            Ok(input) => input,
            Err(err) => {
                return Json(error_response(
                    id,
                    PluginError::invalid_params(err.to_string()),
                ));
            }
        };
        let mut handle = state.dispatcher.dispatch_stream(input.clone());
        let stream_id = handle.stream_id.clone();
        let callback_context = crate::host_api::HostCallbackContext {
            session_id: Some(input.session_id),
            call_id: Some(input.call_id),
            workspace_root: Some(input.workspace_root.clone()),
            tool_name: Some(input.tool_name.clone()),
            ..crate::host_api::current_host_callback_context().unwrap_or_default()
        };
        tokio::spawn(async move {
            let _dispatch_slot = dispatch_slot;
            while let Some(chunk) = handle.chunks.recv().await {
                if callback_client
                    .fire(
                        method::TOOL_STREAM_CHUNK,
                        attach_host_context(
                            serde_json::to_value(&chunk).expect("stream chunk serialize"),
                            callback_context.clone(),
                        ),
                    )
                    .await
                    .is_err()
                {
                    return;
                }
            }
            match handle.end.await {
                Ok(Ok(end)) => {
                    let _ = callback_client
                        .fire(
                            method::TOOL_STREAM_END,
                            attach_host_context(
                                serde_json::to_value(&end).expect("stream end serialize"),
                                callback_context,
                            ),
                        )
                        .await;
                }
                Ok(Err(error)) => {
                    let _ = callback_client
                        .fire(
                            method::TOOL_STREAM_ERROR,
                            attach_host_context(
                                serde_json::to_value(ToolStreamError { stream_id, error })
                                    .expect("stream error serialize"),
                                callback_context,
                            ),
                        )
                        .await;
                }
                Err(_) => {
                    let _ = callback_client
                        .fire(
                            method::TOOL_STREAM_ERROR,
                            attach_host_context(
                                serde_json::to_value(ToolStreamError {
                                    stream_id,
                                    error: PluginError::internal(
                                        "stream terminated before sending final frame",
                                    ),
                                })
                                .expect("stream error serialize"),
                                callback_context,
                            ),
                        )
                        .await;
                }
            }
        });
        return Json(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok {
                result: serde_json::to_value(ToolInvokeStreamHandle {
                    stream_id: handle.stream_id,
                    title: None,
                })
                .expect("stream handle serialize"),
            },
        });
    }

    match state.dispatcher.dispatch(&req.method, params).await {
        Ok(v) => Json(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok { result: v },
        }),
        Err(e) => Json(error_response(id, e)),
    }
}

fn params_with_current_context(params: serde_json::Value) -> serde_json::Value {
    attach_host_context(
        params,
        crate::host_api::current_host_callback_context().unwrap_or_default(),
    )
}

fn attach_host_context(
    mut params: serde_json::Value,
    context: crate::host_api::HostCallbackContext,
) -> serde_json::Value {
    if let Some(object) = params.as_object_mut() {
        object.insert(
            "context".to_string(),
            serde_json::to_value(context).unwrap_or(serde_json::Value::Object(Default::default())),
        );
    }
    params
}

fn error_response(id: RequestId, error: PluginError) -> Response {
    Response {
        jsonrpc: JsonRpcVersion,
        id,
        payload: ResponsePayload::Err {
            error: ErrorObject {
                code: codes::PLUGIN_GENERIC,
                message: error.to_string(),
                data: serde_json::to_value(&error).ok(),
            },
        },
    }
}

#[macro_export]
macro_rules! export_http {
    ($plugin_expr:expr, $bind_addr:expr) => {
        fn main() -> std::io::Result<()> {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(async move {
                let host: ::std::sync::Arc<dyn $crate::host_api::HostClient> =
                    ::std::sync::Arc::new($crate::host_api::NoopHostClient);
                let app = $crate::drivers::http::router($plugin_expr, host);
                let listener = tokio::net::TcpListener::bind($bind_addr).await?;
                axum::serve(listener, app).await
            })
        }
    };
}
