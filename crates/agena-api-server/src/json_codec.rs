//! Bound JSON CPU work at transport boundaries, independently of database work.

use serde::Serialize;
#[cfg(feature = "ws")]
use serde::de::DeserializeOwned;

static ENCODERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
#[cfg(feature = "ws")]
static DECODERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
#[cfg(feature = "ws")]
const INLINE_DECODE_BYTES: usize = 64 * 1024;

#[cfg(any(feature = "ws", feature = "sse"))]
#[derive(Debug, thiserror::Error)]
pub(crate) enum CodecError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("transport JSON worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
    #[cfg(feature = "ipc")]
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
}

#[cfg(any(feature = "ws", feature = "sse"))]
pub(crate) async fn encode<T: Serialize + Send + 'static>(value: T) -> Result<String, CodecError> {
    Ok(ENCODERS
        .run(move || serde_json::to_string(&value))
        .await??)
}

#[cfg(feature = "sse")]
pub(crate) async fn encode_with<T: Serialize + Send + 'static>(
    project: impl FnOnce() -> T + Send + 'static,
) -> Result<String, CodecError> {
    Ok(ENCODERS
        .run(move || serde_json::to_string(&project()))
        .await??)
}

#[cfg(feature = "ws")]
pub(crate) async fn decode<T: DeserializeOwned + Send + 'static>(
    input: impl std::ops::Deref<Target = str> + Send + 'static,
) -> Result<T, CodecError> {
    if input.len() < INLINE_DECODE_BYTES {
        return Ok(serde_json::from_str(&input)?);
    }
    Ok(DECODERS.run(move || serde_json::from_str(&input)).await??)
}

#[cfg(feature = "ipc")]
pub(crate) async fn decode_line<T: DeserializeOwned + Send + 'static>(
    bytes: Vec<u8>,
) -> Result<Option<T>, CodecError> {
    let inline = bytes.len() < INLINE_DECODE_BYTES;
    let parse = move || {
        let line = String::from_utf8(bytes)?;
        if line.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&line)?))
    };
    if inline {
        return parse();
    }
    DECODERS.run(parse).await?
}

#[cfg(feature = "http")]
pub(crate) async fn response<T: Serialize + Send + 'static>(
    value: T,
) -> Result<axum::response::Response, crate::error::ServerError> {
    use axum::response::IntoResponse as _;

    let bytes = ENCODERS
        .run(move || serde_json::to_vec(&value))
        .await
        .map_err(|error| crate::error::ServerError::internal_error(&error))?
        .map_err(|error| crate::error::ServerError::internal_error(&error))?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        bytes,
    )
        .into_response())
}
