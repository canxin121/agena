use agena_api::error::ApiError;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("decode: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("server returned error: {0}")]
    Api(ApiError),
    #[error("subscription closed")]
    SubscriptionClosed,
    #[error("websocket protocol violation: {0}")]
    Protocol(String),
}

impl From<reqwest::Error> for ClientError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err.to_string())
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for ClientError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::Transport(err.to_string())
    }
}
