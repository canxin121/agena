use std::sync::Arc;

use agena_api::{content::ReadContentParams, live::SessionPartsResource, queries::ReadPartsParams};
use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::protocol::{
    self, CancelRunParams, CancelRunResult, CreateSessionParams, CreateSessionResult,
    InboundMessage, JsonRpcError, JsonRpcRequest, JsonRpcResponse, ListSessionsParams,
    ListSessionsResult, PermissionReplyParams, PermissionReplyResult, SubmitRunParams,
    SubmitRunResult,
};

static RPC_CODECS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

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
    #[error("JSON-RPC codec worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
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
    async fn read_parts(
        &self,
        params: ReadPartsParams,
    ) -> Result<SessionPartsResource, AppServerError>;
    async fn read_content(
        &self,
        params: ReadContentParams,
    ) -> Result<agena_domain::ContentPage, AppServerError>;
    async fn read_content_text(
        &self,
        params: agena_api::content::ReadContentTextParams,
    ) -> Result<agena_domain::ContentTextPage, AppServerError>;
    async fn cancel_run(&self, params: CancelRunParams) -> Result<CancelRunResult, AppServerError>;
}

#[derive(Clone)]
/// JSON-RPC application server.
pub struct AppServer<B> {
    backend: Arc<B>,
}

impl<B> AppServer<B>
where
    B: AppServerBackend,
{
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }

    pub async fn serve_stdio<R, W>(&self, reader: R, writer: W) -> Result<(), AppServerError>
    where
        R: AsyncBufRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut reader = reader;
        let mut writer = writer;
        loop {
            tokio::task::consume_budget().await;
            let mut frame = Vec::new();
            let count = (&mut reader)
                .take(MAX_FRAME_BYTES as u64 + 3)
                .read_until(b'\n', &mut frame)
                .await?;
            if count == 0 {
                break;
            }
            if frame.last() == Some(&b'\n') {
                frame.pop();
                if frame.last() == Some(&b'\r') {
                    frame.pop();
                }
            }
            if frame.len() > MAX_FRAME_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "JSON-RPC frame exceeds 64 MiB",
                )
                .into());
            }
            let value: Option<Value> = RPC_CODECS
                .run(move || {
                    if frame.iter().all(u8::is_ascii_whitespace) {
                        return Ok(None);
                    }
                    serde_json::from_slice(&frame).map(Some)
                })
                .await??;
            let Some(value) = value else { continue };
            if let Some(response) = self.handle_value(value).await? {
                let encoded = RPC_CODECS
                    .run(move || {
                        let mut encoded = serde_json::to_vec(&response)?;
                        encoded.push(b'\n');
                        Ok::<_, serde_json::Error>(encoded)
                    })
                    .await??;
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
        match RPC_CODECS
            .run(move || InboundMessage::from_value(value))
            .await??
        {
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
                    |params| async move { self.backend.submit_message(params).await },
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
            protocol::method::PARTS_READ => {
                self.dispatch::<ReadPartsParams, SessionPartsResource, _>(
                    request.params,
                    |params| async move { self.backend.read_parts(params).await },
                )
                .await
            }
            protocol::method::CONTENT_READ => {
                self.dispatch::<ReadContentParams, agena_domain::ContentPage, _>(
                    request.params,
                    |params| async move { self.backend.read_content(params).await },
                )
                .await
            }
            protocol::method::CONTENT_TEXT_READ => {
                self.dispatch::<agena_api::content::ReadContentTextParams, agena_domain::ContentTextPage, _>(request.params,
                    |params| async move { self.backend.read_content_text(params).await }).await
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
        P: serde::de::DeserializeOwned + Send + 'static,
        R: serde::Serialize + Send + 'static,
        Fut: std::future::Future<Output = Result<R, AppServerError>>,
    {
        let params = RPC_CODECS
            .run(move || decode_params::<P>(params))
            .await
            .map_err(|error| to_json_rpc_error(AppServerError::Worker(error)))??;
        let value = f(params).await.map_err(to_json_rpc_error)?;
        RPC_CODECS
            .run(move || serialize_result(value))
            .await
            .map_err(|error| to_json_rpc_error(AppServerError::Worker(error)))?
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
        let diagnostic = agena_failure::diagnostic::format_error_chain(&err);
        let error = agena_api::ApiError::internal(diagnostic.clone());
        tracing::error!(failure_id = %error.problem.id, diagnostic = %diagnostic, "failed to serialize JSON-RPC result");
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
    let failure_id = error.problem.id;
    let message = error.to_string();
    let data = match serde_json::to_value(error) {
        Ok(data) => Some(data),
        Err(serialization_error) => {
            tracing::error!(
                failure_id = %failure_id,
                diagnostic = %agena_failure::diagnostic::format_error_chain(&serialization_error),
                "failed to serialize JSON-RPC error detail"
            );
            None
        }
    };
    JsonRpcError {
        code,
        message,
        data,
    }
}
