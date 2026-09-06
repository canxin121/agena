//! HTTP transport — POSTs JSON-RPC envelopes to a remote plugin server.

use portable_atomic::AtomicI64;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt as _;
use reqwest::Client;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::config::HttpAuth;
use crate::error::TransportError;
use crate::sdk::PluginError;
use crate::sdk::rpc::{JsonRpcVersion, Request, RequestId, Response, ResponsePayload, method};
use crate::sdk::{ToolInvokeInput, ToolInvokeStreamHandle};
use crate::transport::{PluginTransport, ToolStreamHandle};

mod hosted;
mod streams;
use streams::{StreamEvent, StreamRegistry};

const MAX_HTTP_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Plugin transport over HTTP callbacks.
pub struct HttpTransport {
    client: Client,
    url: Url,
    auth_header: Option<String>,
    stream_callbacks: AtomicBool,
    binding: std::sync::Mutex<Option<hosted::HttpBinding>>,
    handoff: tokio::sync::Mutex<()>,
    next_id: AtomicI64,
    streams: Arc<StreamRegistry>,
    shutdown: CancellationToken,
    instance_id: String,
    expected_revision: std::sync::Mutex<Option<String>>,
}

impl HttpTransport {
    pub fn new(
        url: Url,
        auth: HttpAuth,
        env_lookup: &(dyn Fn(&str) -> Option<String> + Send + Sync),
        stream_callbacks: bool,
    ) -> Self {
        let auth_header = match auth {
            HttpAuth::None => None,
            HttpAuth::Bearer { token, token_env } => {
                let resolved = token.or_else(|| token_env.as_deref().and_then(env_lookup));
                resolved.map(|t| format!("Bearer {t}"))
            }
            HttpAuth::Basic {
                username,
                password,
                password_env,
            } => {
                let pwd = password.or_else(|| password_env.as_deref().and_then(env_lookup));
                let pwd = pwd.unwrap_or_default();
                use std::fmt::Write;
                let mut creds = String::new();
                let _ = write!(creds, "{username}:{pwd}");
                let encoded = base64_encode(creds.as_bytes());
                Some(format!("Basic {encoded}"))
            }
        };
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("plugin HTTP client with static configuration should build"),
            url,
            auth_header,
            stream_callbacks: AtomicBool::new(stream_callbacks),
            binding: std::sync::Mutex::new(None),
            handoff: tokio::sync::Mutex::new(()),
            next_id: AtomicI64::new(1),
            streams: Arc::new(StreamRegistry::default()),
            shutdown: CancellationToken::new(),
            instance_id: uuid::Uuid::new_v4().simple().to_string(),
            expected_revision: std::sync::Mutex::new(None),
        }
    }

    async fn send(&self, req: &Request) -> Result<Response, TransportError> {
        let mut builder = self
            .client
            .post(self.url.clone())
            .timeout(Duration::from_secs(60))
            .header(
                crate::sdk::drivers::http::INSTANCE_HEADER,
                &self.instance_id,
            )
            .json(req);
        if req.method == method::META_INIT {
            let revision = self
                .expected_revision
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            builder = builder.header(
                crate::sdk::drivers::http::EXPECTED_REVISION_HEADER,
                revision.as_deref().unwrap_or("-"),
            );
        }
        if let Some(h) = &self.auth_header {
            builder = builder.header("authorization", h);
        }
        let request = async {
            let response = builder.send().await.map_err(|error| {
                TransportError::Io(agena_failure::diagnostic::format_error_chain_with_context(
                    "plugin HTTP transport request failed",
                    &error,
                ))
            })?;
            if response.content_length().is_some_and(|length| {
                length > u64::try_from(MAX_HTTP_RESPONSE_BYTES).unwrap_or(u64::MAX)
            }) {
                return Err(TransportError::Rpc(format!(
                    "plugin HTTP response exceeds the {MAX_HTTP_RESPONSE_BYTES}-byte limit"
                )));
            }
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| {
                    TransportError::Rpc(agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to read the plugin HTTP transport response body",
                        &error,
                    ))
                })?;
                if body.len().saturating_add(chunk.len()) > MAX_HTTP_RESPONSE_BYTES {
                    return Err(TransportError::Rpc(format!(
                        "plugin HTTP response exceeds the {MAX_HTTP_RESPONSE_BYTES}-byte limit"
                    )));
                }
                body.extend_from_slice(&chunk);
            }
            let response: Response = serde_json::from_slice(&body)?;
            if response.id != req.id {
                return Err(TransportError::Rpc(
                    "plugin HTTP response ID does not match its request".into(),
                ));
            }
            Ok(response)
        };
        tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => Err(TransportError::disconnected(
                "HTTP plugin transport was shut down while a request was active",
            )),
            result = request => result,
        }
    }

    fn next_request_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

#[async_trait]
impl PluginTransport for HttpTransport {
    async fn bind_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
    ) -> Result<(), TransportError> {
        self.bind_initial_host(host, scope).await
    }

    async fn initialize(
        &self,
        initialization: super::initialization::PluginInitialization,
    ) -> Result<crate::sdk::InitOutcome, TransportError> {
        let state = self
            .dispatch(method::META_HTTP_STATE, serde_json::json!({}))
            .await?;
        if !state.is_object() {
            return Err(TransportError::Rpc(
                "HTTP instance state must be an object".into(),
            ));
        }
        let state: crate::sdk::drivers::http::HttpInstanceState = serde_json::from_value(state)?;
        if state.version != crate::sdk::drivers::http::HTTP_INSTANCE_VERSION {
            return Err(TransportError::Rpc(
                "HTTP plugin instance protocol is incompatible".into(),
            ));
        }
        *self
            .expected_revision
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = state.revision;
        let outcome = initialization.initialize(self).await?;
        if let Some(binding) = self.binding().as_mut() {
            binding.initialized = true;
        }
        Ok(outcome)
    }

    async fn can_reuse(&self, owner: &Arc<crate::effect_scope::PluginEffectScope>) -> bool {
        self.reusable_binding(owner).is_some()
    }

    async fn try_rebind_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
        previous_scope: Arc<crate::effect_scope::PluginEffectScope>,
        _: super::initialization::PluginInitialization,
    ) -> Result<bool, TransportError> {
        tokio::time::timeout(
            Duration::from_secs(30),
            self.rebind_host(host, scope, previous_scope),
        )
        .await
        .map_err(|error| {
            TransportError::timeout_error("HTTP plugin host handoff exceeded 30 seconds", &error)
        })?
    }

    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: RequestId::Num(self.next_request_id()),
            method: method.to_string(),
            params: Some(params),
            context: crate::sdk::host_api::current_host_callback_context(),
        };
        let resp = self.run_bound(&req, self.send(&req)).await?;
        match resp.payload {
            ResponsePayload::Ok { result } => Ok(result),
            ResponsePayload::Err { error } => {
                let pe = super::plugin_error_from_rpc(error, "decode HTTP plugin dispatch error");
                Err(TransportError::Plugin(pe))
            }
        }
    }

    async fn invoke_stream(
        &self,
        input: ToolInvokeInput,
    ) -> Result<Option<ToolStreamHandle>, TransportError> {
        if !self.stream_callbacks.load(Ordering::Acquire) {
            return Ok(None);
        }
        let context = crate::sdk::host_api::current_host_callback_context().unwrap_or_default();
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: RequestId::Num(self.next_request_id()),
            method: method::HOOK_TOOL_INVOKE_STREAM.to_string(),
            params: Some(serde_json::to_value(&input)?),
            context: Some(context),
        };
        self.run_bound(&req, async {
            let pending = self
                .streams
                .begin(req.context.clone().unwrap_or_default())?;
            let resp = self.send(&req).await?;
            let handle: ToolInvokeStreamHandle = match resp.payload {
                ResponsePayload::Ok { result } => {
                    serde_json::from_value(result).map_err(TransportError::from)?
                }
                ResponsePayload::Err { error } => {
                    let pe = super::plugin_error_from_rpc(
                        error,
                        "decode HTTP plugin streaming-dispatch error",
                    );
                    return Err(TransportError::Plugin(pe));
                }
            };

            Ok(Some(pending.register(handle.stream_id)?))
        })
        .await
    }

    async fn ingest_stream_event(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<bool, TransportError> {
        if !matches!(
            method,
            method::TOOL_STREAM_CHUNK | method::TOOL_STREAM_END | method::TOOL_STREAM_ERROR
        ) {
            return Ok(false);
        }
        let context = params
            .get("context")
            .filter(|context| context.is_object())
            .ok_or_else(|| {
                PluginError::invalid_params(
                    "HTTP plugin stream callback context must be a JSON object",
                )
            })?;
        let context = serde_json::from_value(context.clone())?;
        let event = match method {
            method::TOOL_STREAM_CHUNK => StreamEvent::Chunk(serde_json::from_value(params)?),
            method::TOOL_STREAM_END => StreamEvent::End(serde_json::from_value(params)?),
            method::TOOL_STREAM_ERROR => StreamEvent::Error(serde_json::from_value(params)?),
            _ => unreachable!("stream method was checked above"),
        };
        self.streams.ingest(context, event)?;
        Ok(true)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.close_local("plugin transport closed");
        Ok(())
    }
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        self.close_local("plugin transport dropped");
    }
}

// minimal base64 (RFC 4648) to avoid pulling another dep
fn base64_encode(data: &[u8]) -> String {
    const ALPH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(ALPH[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPH[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPH[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPH[(n & 0x3F) as usize] as char);
        i += 3;
    }
    if i < data.len() {
        let rem = data.len() - i;
        let mut n: u32 = (data[i] as u32) << 16;
        if rem == 2 {
            n |= (data[i + 1] as u32) << 8;
        }
        out.push(ALPH[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPH[((n >> 12) & 0x3F) as usize] as char);
        if rem == 2 {
            out.push(ALPH[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}
