//! HTTP transport — POSTs JSON-RPC envelopes to a remote plugin server.

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use tokio::sync::{Mutex, mpsc, oneshot};
use url::Url;

use crate::config::HttpAuth;
use crate::error::TransportError;
use crate::sdk::PluginError;
use crate::sdk::rpc::{JsonRpcVersion, Request, RequestId, Response, ResponsePayload, method};
use crate::sdk::{
    ToolInvokeInput, ToolInvokeStreamHandle, ToolStreamChunk, ToolStreamEnd, ToolStreamError,
};
use crate::transport::{PluginTransport, ToolStreamHandle};

pub struct HttpTransport {
    client: Client,
    url: Url,
    auth_header: Option<String>,
    stream_callbacks: bool,
    next_id: Mutex<i64>,
    active_streams: Mutex<HashMap<String, ActiveStreamState>>,
    buffered_streams: Mutex<HashMap<String, Vec<BufferedStreamEvent>>>,
}

struct ActiveStreamState {
    chunks: mpsc::Sender<ToolStreamChunk>,
    end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
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
            client: Client::new(),
            url,
            auth_header,
            stream_callbacks,
            next_id: Mutex::new(1),
            active_streams: Mutex::new(HashMap::new()),
            buffered_streams: Mutex::new(HashMap::new()),
        }
    }

    async fn send(&self, req: &Request) -> Result<Response, TransportError> {
        let mut builder = self.client.post(self.url.clone()).json(req);
        if let Some(h) = &self.auth_header {
            builder = builder.header("authorization", h);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let body: Response = resp
            .json()
            .await
            .map_err(|e| TransportError::Rpc(e.to_string()))?;
        Ok(body)
    }

    async fn next_request_id(&self) -> i64 {
        let mut g = self.next_id.lock().await;
        let v = *g;
        *g += 1;
        v
    }

    async fn deliver_stream_chunk(&self, chunk: ToolStreamChunk) {
        let stream_id = chunk.stream_id.clone();
        let sender = {
            let active = self.active_streams.lock().await;
            active.get(&stream_id).map(|state| state.chunks.clone())
        };
        if let Some(sender) = sender {
            let _ = sender.send(chunk).await;
            return;
        }
        let mut buffered = self.buffered_streams.lock().await;
        buffered
            .entry(stream_id)
            .or_default()
            .push(BufferedStreamEvent::Chunk(chunk));
    }

    async fn finish_stream(&self, stream_id: String, result: Result<ToolStreamEnd, PluginError>) {
        let state = {
            let mut active = self.active_streams.lock().await;
            active.remove(&stream_id)
        };
        if let Some(state) = state {
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
        buffered.entry(stream_id).or_default().push(event);
    }

    async fn register_stream(
        &self,
        stream_id: String,
        chunks: mpsc::Sender<ToolStreamChunk>,
        end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
    ) {
        {
            let mut active = self.active_streams.lock().await;
            active.insert(stream_id.clone(), ActiveStreamState { chunks, end });
        }
        self.drain_buffered_stream_events(stream_id).await;
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
            id: RequestId::Num(self.next_request_id().await),
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
            id: RequestId::Num(self.next_request_id().await),
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
        self.fail_active_streams(PluginError::internal("plugin transport closed"))
            .await;
        Ok(())
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
