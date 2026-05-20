//! HTTP-based MCP transport.
//!
//! Two server styles are in the wild:
//!
//! * **SSE** (legacy):  client `GET /sse` → server pushes an `endpoint`
//!   event holding the URL to POST messages to; subsequent server→client
//!   frames arrive as SSE `message` events on that GET.
//! * **Streamable HTTP** (current):  every client→server message is a
//!   `POST /` whose response is either a single JSON body or an SSE stream
//!   of frames.  A separate `GET /` SSE channel can be opened for
//!   server-initiated requests/notifications.
//!
//! Both modes are supported; the variant is chosen at construction.
//!
//! NOTE: this is intentionally a thin implementation — no resumable
//! streams, no session id management.  Sufficient for typical "remote
//! MCP server" deployments behind a reverse proxy.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::Client;
use reqwest::StatusCode;
use serde_json::Value;
use tokio::sync::{Mutex, OnceCell, mpsc};
use url::Url;

use crate::error::{McpError, McpResult};
use crate::protocol::InboundMessage;
use crate::transport::McpTransport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpTransportMode {
    /// Legacy spec — `GET /sse` opens the read channel, POST endpoint
    /// announced via `endpoint` event.
    Sse,
    /// Current spec — every POST may stream frames; a parallel GET SSE
    /// can be opened for server-initiated traffic.
    StreamableHttp,
}

pub struct HttpTransport {
    inner: Arc<Inner>,
}

struct Inner {
    client: Client,
    base_url: Url,
    mode: HttpTransportMode,
    headers: HashMap<String, String>,
    /// In SSE mode: the URL announced in the `endpoint` SSE event.
    /// In Streamable mode: same as base_url.
    post_url: OnceCell<Url>,
    inbox: Mutex<mpsc::UnboundedReceiver<McpResult<InboundMessage>>>,
    inbox_tx: mpsc::UnboundedSender<McpResult<InboundMessage>>,
}

impl HttpTransport {
    pub async fn connect(
        base_url: Url,
        mode: HttpTransportMode,
        headers: HashMap<String, String>,
    ) -> McpResult<Self> {
        let client = Client::builder()
            .build()
            .map_err(|e| McpError::Http(e.to_string()))?;
        let (tx, rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            client,
            base_url: base_url.clone(),
            mode,
            headers,
            post_url: OnceCell::new(),
            inbox: Mutex::new(rx),
            inbox_tx: tx,
        });

        match mode {
            HttpTransportMode::Sse => {
                Self::start_sse_reader(inner.clone()).await?;
            }
            HttpTransportMode::StreamableHttp => {
                inner.post_url.set(base_url).ok();
                Self::start_streamable_get_reader(inner.clone()).await?;
            }
        }
        Ok(Self { inner })
    }

    async fn start_sse_reader(inner: Arc<Inner>) -> McpResult<()> {
        let mut req = inner
            .client
            .get(inner.base_url.clone())
            .header("Accept", "text/event-stream");
        for (k, v) in &inner.headers {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(McpError::Http(format!("SSE GET status {}", resp.status())));
        }
        let mut stream = resp.bytes_stream().eventsource();
        let inner_for_task = inner.clone();
        tokio::spawn(async move {
            while let Some(event) = stream.next().await {
                match event {
                    Ok(ev) => match ev.event.as_str() {
                        "endpoint" => {
                            // The data field is the URL (relative or absolute)
                            // to which the client should POST messages.
                            let raw = ev.data.trim();
                            let url = if let Ok(u) = Url::parse(raw) {
                                u
                            } else {
                                match inner_for_task.base_url.join(raw) {
                                    Ok(u) => u,
                                    Err(e) => {
                                        let _ =
                                            inner_for_task.inbox_tx.send(Err(McpError::Malformed(
                                                format!("invalid endpoint URL '{raw}': {e}"),
                                            )));
                                        continue;
                                    }
                                }
                            };
                            let _ = inner_for_task.post_url.set(url);
                        }
                        // "message" or empty (SSE default) — payload is JSON
                        _ => {
                            if ev.data.is_empty() {
                                continue;
                            }
                            let value: Value = match serde_json::from_str(&ev.data) {
                                Ok(v) => v,
                                Err(e) => {
                                    let _ = inner_for_task.inbox_tx.send(Err(McpError::Malformed(
                                        format!("invalid SSE JSON frame: {e}"),
                                    )));
                                    continue;
                                }
                            };
                            let frame = InboundMessage::from_value(value)
                                .map_err(|e| McpError::Malformed(e.to_string()));
                            if inner_for_task.inbox_tx.send(frame).is_err() {
                                break;
                            }
                        }
                    },
                    Err(e) => {
                        let _ = inner_for_task
                            .inbox_tx
                            .send(Err(McpError::Http(e.to_string())));
                        break;
                    }
                }
            }
            let _ = inner_for_task.inbox_tx.send(Err(McpError::TransportClosed));
        });
        Ok(())
    }

    async fn start_streamable_get_reader(inner: Arc<Inner>) -> McpResult<()> {
        let mut req = inner
            .client
            .get(inner.base_url.clone())
            .header("Accept", "text/event-stream");
        for (k, v) in &inner.headers {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        match resp.status() {
            status if status.is_success() => {}
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
                tracing::debug!(
                    target: "agena_mcp_client::http",
                    status = %resp.status(),
                    url = %inner.base_url,
                    "streamable HTTP server does not expose optional GET event stream"
                );
                return Ok(());
            }
            status => {
                return Err(McpError::Http(format!(
                    "streamable HTTP GET status {status}"
                )));
            }
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !content_type.starts_with("text/event-stream") {
            return Err(McpError::Http(format!(
                "streamable HTTP GET returned unexpected content-type '{content_type}'"
            )));
        }

        let mut stream = resp.bytes_stream().eventsource();
        let inner_for_task = inner.clone();
        tokio::spawn(async move {
            while let Some(event) = stream.next().await {
                match event {
                    Ok(ev) if ev.data.is_empty() => continue,
                    Ok(ev) => {
                        let value: Value = match serde_json::from_str(&ev.data) {
                            Ok(value) => value,
                            Err(err) => {
                                tracing::warn!(
                                    target: "agena_mcp_client::http",
                                    url = %inner_for_task.base_url,
                                    "invalid streamable HTTP SSE frame: {err}"
                                );
                                continue;
                            }
                        };
                        let frame = InboundMessage::from_value(value)
                            .map_err(|err| McpError::Malformed(err.to_string()));
                        if inner_for_task.inbox_tx.send(frame).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            target: "agena_mcp_client::http",
                            url = %inner_for_task.base_url,
                            "streamable HTTP GET reader ended: {err}"
                        );
                        break;
                    }
                }
            }
        });
        Ok(())
    }

    async fn post_url(&self) -> McpResult<Url> {
        // Wait briefly for the SSE endpoint event to land (Sse mode),
        // or just return the base URL (Streamable mode).
        for _ in 0..50 {
            if let Some(u) = self.inner.post_url.get() {
                return Ok(u.clone());
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Err(McpError::Transport(
            "MCP server did not advertise a POST endpoint".to_string(),
        ))
    }

    async fn handle_streamable_response(&self, resp: reqwest::Response) -> McpResult<()> {
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();
        if ct.starts_with("text/event-stream") {
            // Parse the streamed frames until the stream ends.
            let mut stream = resp.bytes_stream().eventsource();
            while let Some(event) = stream.next().await {
                match event {
                    Ok(ev) if !ev.data.is_empty() => {
                        let value: Value = match serde_json::from_str(&ev.data) {
                            Ok(v) => v,
                            Err(e) => {
                                let _ = self
                                    .inner
                                    .inbox_tx
                                    .send(Err(McpError::Malformed(e.to_string())));
                                continue;
                            }
                        };
                        let frame = InboundMessage::from_value(value)
                            .map_err(|e| McpError::Malformed(e.to_string()));
                        if self.inner.inbox_tx.send(frame).is_err() {
                            return Ok(());
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let _ = self.inner.inbox_tx.send(Err(McpError::Http(e.to_string())));
                        return Ok(());
                    }
                }
            }
        } else {
            // Single JSON body (or empty 202).
            let bytes = resp.bytes().await?;
            if bytes.is_empty() {
                return Ok(());
            }
            let value: Value = serde_json::from_slice(&bytes)?;
            let frame =
                InboundMessage::from_value(value).map_err(|e| McpError::Malformed(e.to_string()));
            let _ = self.inner.inbox_tx.send(frame);
        }
        Ok(())
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn send(&self, payload: Value) -> McpResult<()> {
        let url = self.post_url().await?;
        let mut req = self
            .inner
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        for (k, v) in &self.inner.headers {
            req = req.header(k, v);
        }
        let resp = req.json(&payload).send().await?;
        if !resp.status().is_success() {
            return Err(McpError::Http(format!("POST status {}", resp.status())));
        }
        match self.inner.mode {
            HttpTransportMode::Sse => {
                // In SSE mode the server replies via the long-lived GET; the
                // POST response body is irrelevant (often "Accepted").
                drop(resp);
                Ok(())
            }
            HttpTransportMode::StreamableHttp => self.handle_streamable_response(resp).await,
        }
    }

    async fn recv(&self) -> McpResult<InboundMessage> {
        let mut guard = self.inner.inbox.lock().await;
        guard.recv().await.unwrap_or(Err(McpError::TransportClosed))
    }

    async fn close(&self) -> McpResult<()> {
        // Drain inbox so any waiters wake.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::convert::Infallible;

    use axum::{
        Json, Router,
        http::StatusCode as AxumStatusCode,
        response::sse::{Event, Sse},
        routing::get,
    };
    use futures_util::stream;
    use serde_json::json;
    use tokio::net::TcpListener;

    async fn spawn_app(app: Router) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind http test listener");
        let addr = listener.local_addr().expect("listener addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve http app");
        });
        Url::parse(format!("http://{addr}/").as_str()).expect("parse app url")
    }

    #[tokio::test]
    async fn streamable_http_works_without_optional_get_support() {
        let app = Router::new().route(
            "/",
            get(|| async { AxumStatusCode::METHOD_NOT_ALLOWED }).post(|| async {
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "ok": true }
                }))
            }),
        );
        let url = spawn_app(app).await;
        let transport =
            HttpTransport::connect(url, HttpTransportMode::StreamableHttp, HashMap::new())
                .await
                .expect("connect streamable http transport");

        transport
            .send(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "ping",
                "params": {}
            }))
            .await
            .expect("send streamable http request");
        let frame = transport.recv().await.expect("recv http response");
        let InboundMessage::Response(response) = frame else {
            panic!("expected response frame");
        };
        assert!(matches!(response.id, crate::protocol::RequestId::Number(1)));
        assert_eq!(response.result.expect("response result")["ok"], true);
    }

    #[tokio::test]
    async fn streamable_http_receives_optional_get_events() {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/tools/list_changed",
            "params": {}
        })
        .to_string();
        let app = Router::new().route(
            "/",
            get({
                let notification = notification.clone();
                move || async move {
                    let stream = stream::once(async move {
                        Ok::<Event, Infallible>(Event::default().data(notification))
                    });
                    Sse::new(stream)
                }
            })
            .post(|| async {
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {}
                }))
            }),
        );
        let url = spawn_app(app).await;
        let transport =
            HttpTransport::connect(url, HttpTransportMode::StreamableHttp, HashMap::new())
                .await
                .expect("connect streamable http transport");

        let frame = transport.recv().await.expect("recv optional get event");
        let InboundMessage::Notification(notification) = frame else {
            panic!("expected notification frame");
        };
        assert_eq!(notification.method, "notifications/tools/list_changed");
    }
}
