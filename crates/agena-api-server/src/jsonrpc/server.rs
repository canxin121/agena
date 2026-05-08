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
    self, AppServerNotification, CancelTurnParams, CancelTurnResult, CreateSessionParams,
    CreateSessionResult, InboundMessage, JsonRpcError, JsonRpcRequest, JsonRpcResponse,
    ListSessionsParams, ListSessionsResult, PermissionReplyParams, PermissionReplyResult,
    ReadMessagesParams, ReadMessagesResult, SubmitTurnParams, SubmitTurnResult,
};

#[derive(Debug, Error)]
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
pub trait AppServerBackend: Send + Sync + 'static {
    async fn create_session(
        &self,
        params: CreateSessionParams,
    ) -> Result<CreateSessionResult, AppServerError>;
    async fn submit_turn(
        &self,
        params: SubmitTurnParams,
    ) -> Result<SubmitTurnResult, AppServerError>;
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
        params: ReadMessagesParams,
    ) -> Result<ReadMessagesResult, AppServerError>;
    async fn cancel_turn(
        &self,
        params: CancelTurnParams,
    ) -> Result<CancelTurnResult, AppServerError>;
}

#[derive(Clone)]
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
            protocol::method::TURN_SUBMIT => {
                self.dispatch::<SubmitTurnParams, SubmitTurnResult, _>(
                    request.params,
                    |params| async move {
                        let result = self.backend.submit_turn(params).await?;
                        self.events
                            .publish(AppServerNotification::SessionStateChanged {
                                session_id: result.session_id,
                                status: result.status.clone(),
                            });
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
                self.dispatch::<ReadMessagesParams, ReadMessagesResult, _>(
                    request.params,
                    |params| async move { self.backend.read_messages(params).await },
                )
                .await
            }
            protocol::method::TURN_CANCEL => {
                self.dispatch::<CancelTurnParams, CancelTurnResult, _>(
                    request.params,
                    |params| async move { self.backend.cancel_turn(params).await },
                )
                .await
            }
            protocol::method::EVENTS_SUBSCRIBE => Ok(serde_json::json!({"subscribed": true})),
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
        JsonRpcError {
            code: -32602,
            message: format!("invalid params: {err}"),
            data: None,
        }
    })
}

fn serialize_result<T>(value: T) -> Result<Value, JsonRpcError>
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|err| JsonRpcError {
        code: -32603,
        message: format!("serialize result: {err}"),
        data: None,
    })
}

fn to_json_rpc_error(error: AppServerError) -> JsonRpcError {
    match error {
        AppServerError::InvalidParams(message) => JsonRpcError {
            code: -32602,
            message,
            data: None,
        },
        AppServerError::NotFound(message) => JsonRpcError {
            code: -32004,
            message,
            data: None,
        },
        other => JsonRpcError {
            code: -32603,
            message: other.to_string(),
            data: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonrpc::protocol::{RequestId, SessionListItem};
    use chrono::Utc;
    use serde_json::json;

    #[derive(Clone)]
    struct TestBackend;

    #[async_trait]
    impl AppServerBackend for TestBackend {
        async fn create_session(
            &self,
            params: CreateSessionParams,
        ) -> Result<CreateSessionResult, AppServerError> {
            Ok(CreateSessionResult {
                session_id: 1,
                title: params.title.unwrap_or_else(|| "Untitled".to_owned()),
            })
        }

        async fn submit_turn(
            &self,
            params: SubmitTurnParams,
        ) -> Result<SubmitTurnResult, AppServerError> {
            Ok(SubmitTurnResult {
                session_id: params.session_id,
                status: "completed".to_owned(),
                text: Some(params.prompt),
            })
        }

        async fn reply_permission(
            &self,
            params: PermissionReplyParams,
        ) -> Result<PermissionReplyResult, AppServerError> {
            Ok(PermissionReplyResult {
                session_id: params.session_id,
                status: "completed".to_owned(),
            })
        }

        async fn list_sessions(
            &self,
            _params: ListSessionsParams,
        ) -> Result<ListSessionsResult, AppServerError> {
            Ok(ListSessionsResult {
                sessions: vec![SessionListItem {
                    session_id: 1,
                    title: "Test".to_owned(),
                    status: "idle".to_owned(),
                    updated_at: Utc::now(),
                }],
            })
        }

        async fn read_messages(
            &self,
            _params: ReadMessagesParams,
        ) -> Result<ReadMessagesResult, AppServerError> {
            Ok(ReadMessagesResult { messages: vec![] })
        }

        async fn cancel_turn(
            &self,
            params: CancelTurnParams,
        ) -> Result<CancelTurnResult, AppServerError> {
            Ok(CancelTurnResult {
                session_id: params.session_id,
                cancelled: true,
            })
        }
    }

    #[tokio::test]
    async fn stdio_handler_creates_session() {
        let server = AppServer::new(TestBackend);
        let response = server
            .handle_value(json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"session/create",
                "params":{"title":"IDE"}
            }))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(response.id, RequestId::Number(1));
        assert_eq!(response.result.unwrap()["session_id"], 1);
    }

    #[tokio::test]
    async fn stdio_handler_submits_turn() {
        let server = AppServer::new(TestBackend);
        let response = server
            .handle_value(json!({
                "jsonrpc":"2.0",
                "id":"turn",
                "method":"turn/submit",
                "params":{"session_id":1,"prompt":"hello"}
            }))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(response.id, RequestId::String("turn".to_owned()));
        assert_eq!(response.result.unwrap()["text"], "hello");
    }
}
