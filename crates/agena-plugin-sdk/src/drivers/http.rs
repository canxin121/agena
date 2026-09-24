//! HTTP driver. Plugin author serves an axum router on their own port; one
//! POST endpoint receives JSON-RPC envelopes and forwards them to dispatch.

use portable_atomic::AtomicI64;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
use tokio::sync::{RwLock, Semaphore};

use crate::drivers::dispatch::PluginDispatcher;
use crate::error::{PluginError, PluginErrorKind};
use crate::hooks::{
    EventEnvelope, EventFilter, ToolInvokeInput, ToolInvokeOutput, ToolInvokeStreamHandle,
    ToolStreamError,
};
use crate::host_api::{
    EventSubscription, HostClient, HostConfigReloadRequestResponse, HostConfigReloadStatusRequest,
    HostConfigReloadStatusResponse, HostImageExecuteRequest, HostImageExecuteResponse, LogLevel,
};
use crate::plugin::{InitContext, Plugin};
use crate::rpc::{
    ErrorObject, JsonRpcVersion, Request, RequestId, Response, ResponsePayload, codes, method,
};

const MAX_CALLBACK_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[cfg(test)]
mod handoff_tests;
mod host_proxy;
mod instance;
pub use instance::{
    EXPECTED_REVISION_HEADER, HTTP_INSTANCE_VERSION, HttpInstanceState, INSTANCE_HEADER,
};
#[cfg(test)]
mod stream_context_tests;

struct HttpDriverState<P: Plugin> {
    factory: Arc<dyn Fn() -> P + Send + Sync>,
    first: std::sync::Mutex<Option<P>>,
    manifest: crate::PluginManifest,
    fallback_host: Arc<dyn HostClient>,
    dispatch_slots: Arc<Semaphore>,
    instances: instance::Instances<P>,
}

impl<P: Plugin> HttpDriverState<P> {
    fn new(factory: impl Fn() -> P + Send + Sync + 'static, host: Arc<dyn HostClient>) -> Self {
        let first = factory();
        let manifest = first.manifest();
        Self {
            factory: Arc::new(factory),
            first: std::sync::Mutex::new(Some(first)),
            manifest,
            fallback_host: host,
            dispatch_slots: Arc::new(Semaphore::new(64)),
            instances: instance::Instances::default(),
        }
    }

    fn prepare_plugin(&self) -> crate::Result<P> {
        let (plugin, manifest) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let first = self
                .first
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            let plugin = first.unwrap_or_else(|| (self.factory)());
            let manifest = plugin.manifest();
            (plugin, manifest)
        }))
        .map_err(|payload| {
            let diagnostic = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            PluginError::from_kind(
                PluginErrorKind::Panicked,
                format!("HTTP plugin construction panicked: {diagnostic}"),
            )
        })?;
        if manifest != self.manifest {
            return Err(PluginError::invalid_params(
                "HTTP plugin factory changed its manifest",
            ));
        }
        Ok(plugin)
    }
}

struct HttpCallbackHostClient {
    client: reqwest::Client,
    binding: std::sync::RwLock<HttpCallbackBinding>,
    next_id: AtomicI64,
}

/// Callback destination installed in one HTTP plugin instance.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpCallbackBinding {
    pub url: Option<String>,
    pub token: Option<String>,
}

impl HttpCallbackBinding {
    fn validate(&self) -> crate::Result<()> {
        if self.url.is_none() && self.token.is_some() {
            return Err(PluginError::invalid_params(
                "HTTP callback token requires a callback URL",
            ));
        }
        if let Some(url) = self.url.as_deref() {
            let parsed = reqwest::Url::parse(url)
                .map_err(|error| PluginError::invalid_params_error(&error))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(PluginError::invalid_params(
                    "HTTP callback URL must use HTTP or HTTPS",
                ));
            }
        }
        if let Some(token) = self.token.as_deref()
            && (token.is_empty()
                || axum::http::HeaderValue::from_str(&format!("Bearer {token}")).is_err())
        {
            return Err(PluginError::invalid_params(
                "invalid HTTP callback bearer token",
            ));
        }
        Ok(())
    }
}

/// Compare-and-replace operation for a retained HTTP HostClient. The plugin
/// keeps its original Arc; each callback snapshots one complete destination.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpCallbackRebind {
    pub expected: HttpCallbackBinding,
    pub next: HttpCallbackBinding,
}

impl HttpCallbackHostClient {
    fn from_init_context(ctx: &InitContext) -> crate::error::Result<Option<Self>> {
        let binding = HttpCallbackBinding {
            url: ctx.host_callback_url.clone(),
            token: ctx.host_callback_token.clone(),
        };
        binding.validate()?;
        if binding.url.is_none() {
            return Ok(None);
        }
        Self::from_binding(binding).map(Some)
    }

    fn from_binding(binding: HttpCallbackBinding) -> crate::Result<Self> {
        binding.validate()?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| {
                PluginError::internal_error(&error).with_hook(method::META_INIT.to_owned())
            })?;
        Ok(Self {
            client,
            binding: std::sync::RwLock::new(binding),
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
            context: None,
        };
        let binding = self
            .binding
            .read()
            .map_err(|_| PluginError::internal("HTTP callback binding lock is poisoned"))?
            .clone();
        let url = binding.url.ok_or_else(|| {
            PluginError::from_kind(
                PluginErrorKind::HostUnavailable,
                "HTTP host callbacks are disabled",
            )
        })?;
        let mut builder = self.client.post(url).json(&req);
        if let Some(token) = binding.token {
            builder = builder.bearer_auth(token);
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
        if resp.id != RequestId::Num(id) {
            return Err(PluginError::from_kind(
                PluginErrorKind::Disconnected,
                "HTTP host callback response ID does not match its request",
            )
            .with_hook(method_name));
        }
        match resp.payload {
            ResponsePayload::Ok { result } => serde_json::from_value(result)
                .map_err(|error| PluginError::invalid_params_error(&error)),
            ResponsePayload::Err { error } => Err(PluginError::from_rpc_error(error, method_name)),
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

    async fn request_config_reload(&self) -> crate::Result<HostConfigReloadRequestResponse> {
        self.call(
            method::HOST_CONFIG_RELOAD_REQUEST,
            params_with_current_context(serde_json::json!({})),
        )
        .await
    }

    async fn config_reload_status(
        &self,
        request: HostConfigReloadStatusRequest,
    ) -> crate::Result<HostConfigReloadStatusResponse> {
        self.call(
            method::HOST_CONFIG_RELOAD_STATUS,
            params_with_current_context(serde_json::json!({"request": request})),
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

/// Serve independently constructed plugin instances. The factory runs once to
/// obtain the immutable manifest and first object, then again for each new init.
/// Use `router(MyPlugin::default, host)` or a closure returning a fresh object.
/// Constructors should be brief; asynchronous setup belongs in `Plugin::init`.
pub fn router<P: Plugin>(
    factory: impl Fn() -> P + Send + Sync + 'static,
    host: Arc<dyn HostClient>,
) -> Router {
    let state = Arc::new(HttpDriverState::new(factory, host));
    Router::new()
        .route("/rpc", post(handle_rpc::<P>))
        .with_state(state)
}

async fn handle_rpc<P: Plugin>(
    State(state): State<Arc<HttpDriverState<P>>>,
    headers: HeaderMap,
    Json(req): Json<Request>,
) -> Json<Response> {
    let id = req.id.clone();
    let params = req.params.unwrap_or(serde_json::Value::Null);
    let callback_context = req.context;
    if matches!(
        req.method.as_str(),
        method::META_INIT
            | method::META_SHUTDOWN
            | method::META_HOST_REBIND
            | method::META_HTTP_STATE
    ) {
        return Json(rpc_response(
            id,
            instance::control(state, &headers, &req.method, params, callback_context).await,
        ));
    }
    let dispatch_slot = Arc::clone(&state.dispatch_slots).try_acquire_owned();
    let Ok(dispatch_slot) = dispatch_slot else {
        return Json(error_response(
            id,
            PluginError::internal("http plugin dispatch capacity exhausted"),
        ));
    };

    if matches!(
        req.method.as_str(),
        method::META_MANIFEST | method::META_PING
    ) {
        return Json(rpc_response(
            id,
            if req.method == method::META_MANIFEST {
                serde_json::to_value(&state.manifest).map_err(PluginError::from)
            } else {
                Ok(serde_json::json!({"ok": true}))
            },
        ));
    }
    let admission = match state.instances.admit(&headers).await {
        Ok(admission) => admission,
        Err(error) => return Json(error_response(id, error)),
    };

    if req.method == method::HOOK_TOOL_INVOKE_STREAM {
        let Some(callback_client) = admission.instance.callback_client.read().await.clone() else {
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
        // Capture the request authority before leaving its context. Both the
        // plugin task and the event-forwarding task must echo the same owner;
        // the router's ambient context belongs to middleware, not this call.
        let callback_context = crate::host_api::HostCallbackContext {
            session_id: Some(input.session_id),
            call_id: Some(input.call_id),
            workspace_root: Some(input.workspace_root.clone()),
            tool_name: Some(input.tool_name.clone()),
            ..callback_context.unwrap_or_default()
        };
        let mut handle = crate::host_api::run_in_isolated_host_callback_context(
            callback_context.clone(),
            async {
                admission
                    .dispatcher
                    .dispatch_stream_with_lifetime(input, admission.clone())
            },
        )
        .await;
        let stream_id = handle.stream_id.clone();
        tokio::spawn(async move {
            let stop = admission.stop.clone();
            let forward = async move {
                let _admission = admission;
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
            };
            tokio::select! {
                biased;
                _ = stop.cancelled() => {},
                () = forward => {},
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

    let dispatch = admission.dispatcher.dispatch(&req.method, params);
    let result = admission
        .run(crate::host_api::run_in_isolated_host_callback_context(
            callback_context.unwrap_or_default(),
            dispatch,
        ))
        .await;
    match result {
        Ok(v) => Json(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok { result: v },
        }),
        Err(e) => Json(error_response(id, e)),
    }
}

async fn rebind_callback_client<P: Plugin>(
    state: &HttpDriverState<P>,
    instance: &instance::Instance<P>,
    params: serde_json::Value,
) -> crate::Result<()> {
    if !params.is_object()
        || !params
            .get("expected")
            .is_some_and(serde_json::Value::is_object)
        || !params.get("next").is_some_and(serde_json::Value::is_object)
    {
        return Err(PluginError::invalid_params(
            "HTTP callback rebind requires expected and next binding objects",
        ));
    }
    let rebind: HttpCallbackRebind = serde_json::from_value(params)?;
    rebind.expected.validate()?;
    rebind.next.validate()?;
    // This control operation needs no ordinary dispatch permit or plugin code.
    // A long-running plugin stream must not starve its own callback handoff.
    let mut current = instance.callback_client.write().await;
    if let Some(client) = current.as_ref() {
        let mut binding = client
            .binding
            .write()
            .map_err(|_| PluginError::internal("HTTP callback binding lock is poisoned"))?;
        if *binding != rebind.expected {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "HTTP callback binding changed before handoff",
            ));
        }
        *binding = rebind.next.clone();
    } else {
        if rebind.expected.url.is_some() || rebind.expected.token.is_some() {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "HTTP callback binding changed before handoff",
            ));
        }
        if rebind.next.url.is_some() {
            let client = Arc::new(HttpCallbackHostClient::from_binding(rebind.next.clone())?);
            instance.host_proxy.replace(client.clone());
            *current = Some(client);
        }
    }
    if rebind.next.url.is_none() {
        instance.host_proxy.replace(state.fallback_host.clone());
        *current = None;
    }
    Ok(())
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

fn rpc_response(id: RequestId, result: crate::Result<serde_json::Value>) -> Response {
    match result {
        Ok(result) => Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok { result },
        },
        Err(error) => error_response(id, error),
    }
}

#[macro_export]
/// Export an HTTP plugin server. The plugin expression is evaluated for each
/// new instance; construct fresh plugin state rather than consuming one object.
macro_rules! export_http {
    ($plugin_expr:expr, $bind_addr:expr) => {
        fn main() -> std::io::Result<()> {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(async move {
                let host: ::std::sync::Arc<dyn $crate::host_api::HostClient> =
                    ::std::sync::Arc::new($crate::host_api::NoopHostClient);
                let app = $crate::drivers::http::router(move || $plugin_expr, host);
                let listener = tokio::net::TcpListener::bind($bind_addr).await?;
                axum::serve(listener, app).await
            })
        }
    };
}
