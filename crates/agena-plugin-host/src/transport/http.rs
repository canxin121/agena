//! HTTP transport — POSTs JSON-RPC envelopes to a remote plugin server.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt as _;
use reqwest::Client;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::config::HttpAuth;
use crate::error::TransportError;
use crate::sdk::PluginError;
use crate::sdk::rpc::{JsonRpcVersion, Request, RequestId, Response, ResponsePayload, method};
use crate::sdk::{
    ToolInvokeInput, ToolInvokeStreamHandle, ToolStreamChunk, ToolStreamEnd, ToolStreamError,
};
use crate::transport::{PluginTransport, ToolStreamHandle};

const MAX_HTTP_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_BUFFERED_STREAMS: usize = 128;
const MAX_BUFFERED_STREAM_EVENTS: usize = 64;

/// Plugin transport over HTTP callbacks.
pub struct HttpTransport {
    client: Client,
    url: Url,
    auth_header: Option<String>,
    stream_callbacks: bool,
    next_id: AtomicI64,
    active_streams: Arc<Mutex<HashMap<String, ActiveStreamState>>>,
    buffered_streams: Arc<Mutex<HashMap<String, Vec<BufferedStreamEvent>>>>,
    shutdown: CancellationToken,
}

struct ActiveStreamState {
    chunks: mpsc::Sender<ToolStreamChunk>,
    end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
    monitor_stop: CancellationToken,
}

enum BufferedStreamEvent {
    Chunk(ToolStreamChunk),
    End(ToolStreamEnd),
    Error(ToolStreamError),
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
            stream_callbacks,
            next_id: AtomicI64::new(1),
            active_streams: Arc::new(Mutex::new(HashMap::new())),
            buffered_streams: Arc::new(Mutex::new(HashMap::new())),
            shutdown: CancellationToken::new(),
        }
    }

    async fn send(&self, req: &Request) -> Result<Response, TransportError> {
        let mut builder = self
            .client
            .post(self.url.clone())
            .timeout(Duration::from_secs(60))
            .json(req);
        if let Some(h) = &self.auth_header {
            builder = builder.header("authorization", h);
        }
        let request = async {
            let response = builder
                .send()
                .await
                .map_err(|error| TransportError::Io(error.to_string()))?;
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
                let chunk = chunk.map_err(|error| TransportError::Rpc(error.to_string()))?;
                if body.len().saturating_add(chunk.len()) > MAX_HTTP_RESPONSE_BYTES {
                    return Err(TransportError::Rpc(format!(
                        "plugin HTTP response exceeds the {MAX_HTTP_RESPONSE_BYTES}-byte limit"
                    )));
                }
                body.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&body).map_err(TransportError::from)
        };
        tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => Err(TransportError::Disconnected),
            result = request => result,
        }
    }

    fn next_request_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
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
                    let state = self.active_streams.lock().await.remove(stream_id.as_str());
                    if let Some(state) = state {
                        state.monitor_stop.cancel();
                        let _ = state.end.send(Err(PluginError::internal(
                            "plugin stream consumer exceeded the 64-chunk buffer",
                        )));
                    }
                }
            }
            return;
        }
        let mut buffered = self.buffered_streams.lock().await;
        if !buffered.contains_key(stream_id.as_str()) && buffered.len() >= MAX_BUFFERED_STREAMS {
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
            let _ = state.end.send(result);
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
            return;
        }
        let events = buffered.entry(stream_id).or_default();
        if events.len() >= MAX_BUFFERED_STREAM_EVENTS {
            events.clear();
        }
        events.push(event);
    }

    async fn register_stream(
        &self,
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
        let weak_active = Arc::downgrade(&self.active_streams);
        let weak_buffered = Arc::downgrade(&self.buffered_streams);
        let shutdown = self.shutdown.clone();
        let abandoned_stream_id = stream_id.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = monitor_stop.cancelled() => {}
                _ = shutdown.cancelled() => {}
                _ = chunks.closed() => {
                    if let Some(active) = weak_active.upgrade()
                        && let Some(state) = active.lock().await.remove(abandoned_stream_id.as_str())
                    {
                        state.monitor_stop.cancel();
                    }
                    if let Some(buffered) = weak_buffered.upgrade() {
                        buffered.lock().await.remove(abandoned_stream_id.as_str());
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

    async fn drain_buffered_stream_events(&self, stream_id: String) {
        let events = {
            let mut buffered = self.buffered_streams.lock().await;
            buffered.remove(&stream_id).unwrap_or_default()
        };
        for event in events {
            match event {
                BufferedStreamEvent::Chunk(chunk) => self.deliver_stream_chunk(chunk).await,
                BufferedStreamEvent::End(end) => {
                    self.finish_stream(stream_id.clone(), Ok(end)).await
                }
                BufferedStreamEvent::Error(err) => {
                    self.finish_stream(stream_id.clone(), Err(err.error)).await;
                }
            }
        }
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
            let _ = state.end.send(Err(error.clone()));
        }
    }
}

#[async_trait]
impl PluginTransport for HttpTransport {
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
        };
        let resp = self.send(&req).await?;
        match resp.payload {
            ResponsePayload::Ok { result } => Ok(result),
            ResponsePayload::Err { error } => {
                let pe: Option<PluginError> = error
                    .data
                    .as_ref()
                    .and_then(|d| serde_json::from_value(d.clone()).ok());
                let pe = pe.unwrap_or_else(|| {
                    let mut plugin_error = PluginError::internal(error.message);
                    plugin_error.diagnostic.data = error.data;
                    plugin_error
                });
                Err(TransportError::Plugin(pe))
            }
        }
    }

    async fn invoke_stream(
        &self,
        input: ToolInvokeInput,
    ) -> Result<Option<ToolStreamHandle>, TransportError> {
        if !self.stream_callbacks {
            return Ok(None);
        }
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: RequestId::Num(self.next_request_id()),
            method: method::HOOK_TOOL_INVOKE_STREAM.to_string(),
            params: Some(serde_json::to_value(&input)?),
        };
        let resp = self.send(&req).await?;
        let handle: ToolInvokeStreamHandle = match resp.payload {
            ResponsePayload::Ok { result } => {
                serde_json::from_value(result).map_err(|e| TransportError::Rpc(e.to_string()))?
            }
            ResponsePayload::Err { error } => {
                let pe: Option<PluginError> = error
                    .data
                    .as_ref()
                    .and_then(|d| serde_json::from_value(d.clone()).ok());
                let pe = pe.unwrap_or_else(|| {
                    let mut plugin_error = PluginError::internal(error.message);
                    plugin_error.diagnostic.data = error.data;
                    plugin_error
                });
                return Err(TransportError::Plugin(pe));
            }
        };

        let (chunk_tx, chunk_rx) = mpsc::channel::<ToolStreamChunk>(64);
        let (end_tx, end_rx) = oneshot::channel::<Result<ToolStreamEnd, PluginError>>();
        self.register_stream(handle.stream_id.clone(), chunk_tx, end_tx)
            .await;
        Ok(Some(ToolStreamHandle {
            stream_id: handle.stream_id,
            chunks: chunk_rx,
            end: end_rx,
        }))
    }

    async fn ingest_stream_event(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<bool, TransportError> {
        match method {
            method::TOOL_STREAM_CHUNK => {
                let chunk: ToolStreamChunk = serde_json::from_value(params)
                    .map_err(|e| TransportError::Rpc(e.to_string()))?;
                self.deliver_stream_chunk(chunk).await;
                Ok(true)
            }
            method::TOOL_STREAM_END => {
                let end: ToolStreamEnd = serde_json::from_value(params)
                    .map_err(|e| TransportError::Rpc(e.to_string()))?;
                self.finish_stream(end.stream_id.clone(), Ok(end)).await;
                Ok(true)
            }
            method::TOOL_STREAM_ERROR => {
                let err: ToolStreamError = serde_json::from_value(params)
                    .map_err(|e| TransportError::Rpc(e.to_string()))?;
                self.finish_stream(err.stream_id.clone(), Err(err.error))
                    .await;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.shutdown.cancel();
        self.fail_active_streams(PluginError::internal("plugin transport closed"))
            .await;
        Ok(())
    }
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        self.shutdown.cancel();
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::{HttpTransport, PluginTransport, ToolStreamChunk};
    use crate::config::HttpAuth;

    fn transport() -> HttpTransport {
        HttpTransport::new(
            "http://127.0.0.1:1/rpc".parse().expect("test URL"),
            HttpAuth::None,
            &|_| None,
            true,
        )
    }

    #[tokio::test]
    async fn slow_stream_consumer_fails_instead_of_blocking_event_ingress() {
        let transport = transport();
        let (chunks, _chunk_rx) = tokio::sync::mpsc::channel(64);
        let (end, end_rx) = tokio::sync::oneshot::channel();
        transport
            .register_stream("stream-1".to_string(), chunks, end)
            .await;

        for index in 0..=64 {
            transport
                .deliver_stream_chunk(ToolStreamChunk {
                    stream_id: "stream-1".to_string(),
                    text_delta: Some(format!("chunk-{index}")),
                    metadata: BTreeMap::new(),
                })
                .await;
        }

        let result = tokio::time::timeout(Duration::from_secs(1), end_rx)
            .await
            .expect("overflow is reported promptly")
            .expect("terminal sender remains available");
        assert!(result.is_err());
        assert!(transport.active_streams.lock().await.is_empty());
    }

    #[tokio::test]
    async fn dropping_stream_receivers_reclaims_registration() {
        let transport = transport();
        let (chunks, chunk_rx) = tokio::sync::mpsc::channel(64);
        let (end, end_rx) = tokio::sync::oneshot::channel();
        transport
            .register_stream("stream-2".to_string(), chunks, end)
            .await;

        drop(chunk_rx);
        drop(end_rx);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !transport.active_streams.lock().await.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("abandoned stream registration is reclaimed");
        transport.close().await.expect("transport closes");
    }
}
