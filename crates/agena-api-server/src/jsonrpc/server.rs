use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws::Message},
    response::IntoResponse,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use thiserror::Error;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt},
    sync::broadcast,
};

use super::protocol::{
    self, AppServerNotification, CancelRunParams, CancelRunResult, CreateSessionParams,
    CreateSessionResult, InboundMessage, JsonRpcError, JsonRpcRequest, JsonRpcResponse,
    ListSessionsParams, ListSessionsResult, PermissionReplyParams, PermissionReplyResult,
    ReadPartsParams, ReadPartsResult, SubmitRunParams, SubmitRunResult,
};

#[derive(Debug, Error)]
/// Error from the JSON-RPC app server.
pub enum AppServerError {
    #[error("invalid params: {0}")]
    InvalidParams(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[async_trait]
/// Backend implementing JSON-RPC methods.
pub trait AppServerBackend: Send + Sync + 'static {
    async fn create_session(
        &self,
        params: CreateSessionParams,
    ) -> Result<CreateSessionResult, AppServerError>;
    async fn submit_message(
        &self,
        params: SubmitRunParams,
    ) -> Result<SubmitRunResult, AppServerError>;
    async fn reply_permission(
        &self,
        params: PermissionReplyParams,
    ) -> Result<PermissionReplyResult, AppServerError>;
    async fn list_sessions(
        &self,
        params: ListSessionsParams,
    ) -> Result<ListSessionsResult, AppServerError>;
    async fn read_messages(
        &self,
        params: ReadPartsParams,
    ) -> Result<ReadPartsResult, AppServerError>;
    async fn cancel_run(&self, params: CancelRunParams) -> Result<CancelRunResult, AppServerError>;
}

#[derive(Clone)]
/// Broadcaster of server notifications to subscribers.
pub struct EventBroadcaster {
    sender: broadcast::Sender<AppServerNotification>,
}

impl EventBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppServerNotification> {
        self.sender.subscribe()
    }

    pub fn publish(&self, notification: AppServerNotification) {
        let _ = self.sender.send(notification);
    }
}

#[derive(Clone)]
/// JSON-RPC application server.
pub struct AppServer<B> {
    backend: Arc<B>,
    events: EventBroadcaster,
}

impl<B> AppServer<B>
where
    B: AppServerBackend,
{
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            events: EventBroadcaster::new(1024),
        }
    }

    pub fn events(&self) -> EventBroadcaster {
        self.events.clone()
    }

    pub async fn serve_stdio<R, W>(&self, reader: R, writer: W) -> Result<(), AppServerError>
    where
        R: AsyncBufRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut lines = reader.lines();
        let mut writer = writer;
        while let Some(line) = lines.next_line().await? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line)?;
            if let Some(response) = self.handle_value(value).await? {
                let mut encoded = serde_json::to_vec(&response)?;
                encoded.push(b'\n');
                writer.write_all(&encoded).await?;
                writer.flush().await?;
            }
        }
        Ok(())
    }

    pub async fn handle_value(
        &self,
        value: Value,
    ) -> Result<Option<JsonRpcResponse>, AppServerError> {
        match InboundMessage::from_value(value)? {
            InboundMessage::Request(request) => Ok(Some(self.handle_request(request).await)),
            InboundMessage::Notification(_) | InboundMessage::Response(_) => Ok(None),
        }
    }

    async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let result = match request.method.as_str() {
            protocol::method::SESSION_CREATE => {
                self.dispatch::<CreateSessionParams, CreateSessionResult, _>(
                    request.params,
                    |params| async move { self.backend.create_session(params).await },
                )
                .await
            }
            protocol::method::MESSAGE_SUBMIT => {
                self.dispatch::<SubmitRunParams, SubmitRunResult, _>(
                    request.params,
                    |params| async move {
                        let result = self.backend.submit_message(params).await?;
                        self.events
                            .publish(AppServerNotification::SessionStateChanged {
                                session_id: result.session_id,
                                // The run marker (first part of the returned
                                // run) mirrors the run/reply status.
                                status: result
                                    .parts
                                    .first()
                                    .map(|part| part.state.clone())
                                    .unwrap_or_else(|| "submitted".to_owned()),
                            });
                        // Deliver the accepted parts as v2 part patches so live
                        // clients can reconcile without re-reading the session.
                        for part in &result.parts {
                            self.events.publish(AppServerNotification::PartAdded {
                                session_id: result.session_id,
                                part: Box::new(part.clone()),
                            });
                        }
                        Ok(result)
                    },
                )
                .await
            }
            protocol::method::PERMISSION_REPLY => {
                self.dispatch::<PermissionReplyParams, PermissionReplyResult, _>(
                    request.params,
                    |params| async move { self.backend.reply_permission(params).await },
                )
                .await
            }
            protocol::method::SESSIONS_LIST => {
                self.dispatch::<ListSessionsParams, ListSessionsResult, _>(
                    request.params,
                    |params| async move { self.backend.list_sessions(params).await },
                )
                .await
            }
            protocol::method::MESSAGES_LIST => {
                self.dispatch::<ReadPartsParams, ReadPartsResult, _>(
                    request.params,
                    |params| async move { self.backend.read_messages(params).await },
                )
                .await
            }
            protocol::method::RUN_CANCEL => {
                self.dispatch::<CancelRunParams, CancelRunResult, _>(
                    request.params,
                    |params| async move { self.backend.cancel_run(params).await },
                )
                .await
            }
            _ => Err(JsonRpcError {
                code: -32601,
                message: format!("method not found: {}", request.method),
                data: None,
            }),
        };
        match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: protocol::JSONRPC_VERSION.to_owned(),
                id: request.id,
                result: Some(value),
                error: None,
            },
            Err(error) => JsonRpcResponse {
                jsonrpc: protocol::JSONRPC_VERSION.to_owned(),
                id: request.id,
                result: None,
                error: Some(error),
            },
        }
    }

    async fn dispatch<P, R, Fut>(
        &self,
        params: Option<Value>,
        f: impl FnOnce(P) -> Fut,
    ) -> Result<Value, JsonRpcError>
    where
        P: serde::de::DeserializeOwned,
        R: serde::Serialize,
        Fut: std::future::Future<Output = Result<R, AppServerError>>,
    {
        let params = decode_params::<P>(params)?;
        let value = f(params).await.map_err(to_json_rpc_error)?;
        serialize_result(value)
    }
}

pub async fn serve_stdio<B>(backend: B) -> Result<(), AppServerError>
where
    B: AppServerBackend,
{
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();
    AppServer::new(backend).serve_stdio(stdin, stdout).await
}

pub fn websocket_router(events: EventBroadcaster) -> Router {
    Router::new()
        .route("/events", get(websocket_events))
        .with_state(events)
}

async fn websocket_events(
    State(events): State<EventBroadcaster>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        let (mut sender, _) = socket.split();
        let mut rx = events.subscribe();
        while let Ok(notification) = rx.recv().await {
            let Ok(text) = serde_json::to_string(&notification) else {
                continue;
            };
            if sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    })
}

fn decode_params<T>(params: Option<Value>) -> Result<T, JsonRpcError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(params.unwrap_or_else(|| serde_json::json!({}))).map_err(|err| {
        let error = agena_api::ApiError::bad_request("The request parameters are invalid.");
        tracing::warn!(failure_id = %error.problem.id, diagnostic = %err, "invalid JSON-RPC parameters");
        json_rpc_problem(-32602, error)
    })
}

fn serialize_result<T>(value: T) -> Result<Value, JsonRpcError>
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|err| {
        let error = agena_api::ApiError::internal(err.to_string());
        tracing::error!(failure_id = %error.problem.id, diagnostic = %err, "failed to serialize JSON-RPC result");
        json_rpc_problem(-32603, error)
    })
}

fn to_json_rpc_error(error: AppServerError) -> JsonRpcError {
    match error {
        AppServerError::InvalidParams(message) => {
            tracing::warn!(diagnostic = %message, "invalid JSON-RPC request");
            json_rpc_problem(
                -32602,
                agena_api::ApiError::bad_request("The request parameters are invalid."),
            )
        }
        AppServerError::NotFound(message) => {
            tracing::warn!(diagnostic = %message, "JSON-RPC resource not found");
            json_rpc_problem(
                -32004,
                agena_api::ApiError::not_found("The requested resource was not found."),
            )
        }
        other => {
            let diagnostic = other.to_string();
            let error = agena_api::ApiError::internal(diagnostic.as_str());
            tracing::error!(failure_id = %error.problem.id, diagnostic = %diagnostic, "JSON-RPC backend failed");
            json_rpc_problem(-32603, error)
        }
    }
}

fn json_rpc_problem(code: i64, error: agena_api::ApiError) -> JsonRpcError {
    JsonRpcError {
        code,
        message: error.to_string(),
        data: serde_json::to_value(error).ok(),
    }
}
