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
    fn from_init_context(ctx: &InitContext) -> crate::error::Result<Option<Self>> {
        let Some(url) = ctx.host_callback_url.clone() else {
            return Ok(None);
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| {
                PluginError::internal_error(&error).with_hook(method::META_INIT.to_owned())
            })?;
        Ok(Some(Self {
            client,
            url,
            auth_header: ctx
                .host_callback_token
                .as_ref()
                .map(|token| format!("Bearer {token}")),
            next_id: AtomicI64::new(1),
        }))
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
            context: None,
        };
        let mut builder = self.client.post(&self.url).json(&req);
        if let Some(header) = &self.auth_header {
            builder = builder.header("authorization", header);
        }
        let mut resp = builder.send().await.map_err(|error| {
            PluginError::from_kind(
                PluginErrorKind::Disconnected,
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("failed to send HTTP host callback `{method_name}`"),
                    &error,
                ),
            )
            .with_hook(method_name)
        })?;
        let status = resp.status();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|error| {
            PluginError::from_kind(
                PluginErrorKind::Disconnected,
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("failed to read HTTP host callback `{method_name}` response body"),
                    &error,
                ),
            )
            .with_hook(method_name)
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
            PluginError::from_kind(
                PluginErrorKind::Disconnected,
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("HTTP host callback `{method_name}` returned a non-UTF-8 body"),
                    &error,
                ),
            )
            .with_hook(method_name)
        })?;
        let resp: Response = serde_json::from_str(&body).map_err(|error| {
            PluginError::from_kind(
                PluginErrorKind::Disconnected,
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!(
                        "HTTP host callback `{method_name}` returned status {status} with an invalid JSON-RPC response"
                    ),
                    &error,
                ),
            )
            .with_hook(method_name)
        })?;
        match resp.payload {
            ResponsePayload::Ok { result } => serde_json::from_value(result)
                .map_err(|error| PluginError::invalid_params_error(&error)),
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
        if let Err(error) = self
            .fire(
                method::HOST_LOG,
                params_with_current_context(serde_json::json!({
                    "level": level,
                    "message": message,
                    "fields": fields,
                })),
            )
            .await
        {
            tracing::warn!(
                diagnostic = %error.diagnostic_message(),
                "failed to deliver a plugin log record to the HTTP callback host"
            );
        }
    }

    async fn publish_event(&self, env: EventEnvelope) -> crate::error::Result<()> {
        self.fire(
            method::HOST_EVENT_PUBLISH,
            params_with_current_context(
                serde_json::to_value(env).map_err(|e| PluginError::invalid_params_error(&e))?,
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
    let callback_context = req.context;
    let dispatch_slot = Arc::clone(&state.dispatch_slots).try_acquire_owned();
    let Ok(dispatch_slot) = dispatch_slot else {
        return Json(error_response(
            id,
            PluginError::internal("http plugin dispatch capacity exhausted"),
        ));
    };

    if req.method == method::META_INIT
        && let Ok(ctx) = serde_json::from_value::<InitContext>(params.clone())
    {
        match HttpCallbackHostClient::from_init_context(&ctx) {
            Ok(Some(callback_client)) => {
                let callback_client = Arc::new(callback_client);
                let host: Arc<dyn HostClient> = callback_client.clone();
                state.dispatcher.set_host(host).await;
                *state.callback_client.write().await = Some(callback_client);
            }
            Ok(None) => {}
            Err(error) => return Json(error_response(id, error)),
        }
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
                return Json(error_response(id, PluginError::invalid_params_error(&err)));
            }
        };
        let mut handle = if let Some(context) = callback_context.clone() {
            crate::host_api::run_in_host_callback_context(context, async {
                state.dispatcher.dispatch_stream(input.clone())
            })
            .await
        } else {
            state.dispatcher.dispatch_stream(input.clone())
        };
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
                let chunk = match serde_json::to_value(&chunk) {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        tracing::error!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to serialize an HTTP plugin stream chunk callback",
                                &error,
                            ),
                            "HTTP plugin stream callback stopped"
                        );
                        return;
                    }
                };
                if let Err(error) = callback_client
                    .fire(
                        method::TOOL_STREAM_CHUNK,
                        attach_host_context(chunk, callback_context.clone()),
                    )
                    .await
                {
                    tracing::warn!(
                        diagnostic = %error.diagnostic_message(),
                        "failed to deliver an HTTP plugin stream chunk callback"
                    );
                    return;
                }
            }
            match handle.end.await {
                Ok(Ok(end)) => {
                    let end = match serde_json::to_value(&end) {
                        Ok(end) => end,
                        Err(error) => {
                            tracing::error!(
                                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                    "failed to serialize an HTTP plugin stream terminal callback",
                                    &error,
                                ),
                                "HTTP plugin stream terminal callback was not sent"
                            );
                            return;
                        }
                    };
                    if let Err(error) = callback_client
                        .fire(
                            method::TOOL_STREAM_END,
                            attach_host_context(end, callback_context),
                        )
                        .await
                    {
                        tracing::warn!(
                            diagnostic = %error.diagnostic_message(),
                            "failed to deliver an HTTP plugin stream terminal callback"
                        );
                    }
                }
                Ok(Err(error)) => {
                    let stream_error = match serde_json::to_value(ToolStreamError {
                        stream_id,
                        error,
                    }) {
                        Ok(stream_error) => stream_error,
                        Err(error) => {
                            tracing::error!(
                                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                    "failed to serialize an HTTP plugin stream error callback",
                                    &error,
                                ),
                                "HTTP plugin stream error callback was not sent"
                            );
                            return;
                        }
                    };
                    if let Err(error) = callback_client
                        .fire(
                            method::TOOL_STREAM_ERROR,
                            attach_host_context(stream_error, callback_context),
                        )
                        .await
                    {
                        tracing::warn!(
                            diagnostic = %error.diagnostic_message(),
                            "failed to deliver an HTTP plugin stream error callback"
                        );
                    }
                }
                Err(receive_error) => {
                    let stream_error = ToolStreamError {
                        stream_id,
                        error: PluginError::internal(
                            agena_failure::diagnostic::format_error_chain_with_context(
                                "tool stream terminated before sending its final frame",
                                &receive_error,
                            ),
                        ),
                    };
                    let stream_error = match serde_json::to_value(stream_error) {
                        Ok(stream_error) => stream_error,
                        Err(error) => {
                            tracing::error!(
                                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                    "failed to serialize an HTTP plugin premature-stream callback",
                                    &error,
                                ),
                                "HTTP plugin premature-stream callback was not sent"
                            );
                            return;
                        }
                    };
                    if let Err(error) = callback_client
                        .fire(
                            method::TOOL_STREAM_ERROR,
                            attach_host_context(stream_error, callback_context),
                        )
                        .await
                    {
                        tracing::warn!(
                            diagnostic = %error.diagnostic_message(),
                            "failed to deliver an HTTP plugin premature-stream callback"
                        );
                    }
                }
            }
        });
        let handle_resource = match serde_json::to_value(ToolInvokeStreamHandle {
            stream_id: handle.stream_id,
            title: None,
        }) {
            Ok(handle) => handle,
            Err(error) => {
                return Json(error_response(
                    id,
                    PluginError::internal_error(&error)
                        .with_hook(method::HOOK_TOOL_INVOKE_STREAM.to_owned()),
                ));
            }
        };
        return Json(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok {
                result: handle_resource,
            },
        });
    }

    let dispatch = state.dispatcher.dispatch(&req.method, params);
    let result = if let Some(context) = callback_context {
        crate::host_api::run_in_host_callback_context(context, dispatch).await
    } else {
        dispatch.await
    };
    match result {
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
        match serde_json::to_value(context) {
            Ok(context) => {
                object.insert("context".to_string(), context);
            }
            Err(error) => {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "serialize HTTP plugin host callback context",
                        &error,
                    ),
                    "HTTP plugin callback is missing its unserializable host context"
                );
            }
        }
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
                data: error.rpc_error_data(),
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
