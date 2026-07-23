//! Concrete AWS Smithy event-stream decoding for Amazon Bedrock Anthropic.
//!
//! This leaf converts a successful Bedrock HTTP response into decoded JSON
//! events. Product-specific Anthropic event projection, request construction,
//! logging, and error presentation remain with the Runtime provider adapter.

use std::{error::Error as StdError, fmt, pin::Pin};

use async_stream::stream;
use aws_smithy_eventstream::{
    error::Error as EventStreamError,
    frame::{UnmarshallMessage, UnmarshalledMessage},
};
use aws_smithy_http::event_stream::Receiver;
use aws_smithy_types::{
    body::SdkBody,
    event_stream::{HeaderValue, Message as EventStreamMessage},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use futures_core::Stream;
use futures_util::TryStreamExt;
use http_body::Frame;
use serde_json::Value;

/// Bedrock service failure carried by an AWS event-stream frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockAnthropicStreamServiceError {
    pub event_type: String,
    pub message: String,
    pub retryable: bool,
}

impl BedrockAnthropicStreamServiceError {
    fn from_payload(event_type: &str, payload: Value) -> Self {
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| payload.get("originalMessage").and_then(Value::as_str))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(event_type)
            .to_owned();

        Self {
            event_type: event_type.to_owned(),
            message,
            retryable: matches!(
                event_type,
                "internalServerException"
                    | "modelStreamErrorException"
                    | "throttlingException"
                    | "modelTimeoutException"
                    | "serviceUnavailableException"
            ),
        }
    }
}

impl fmt::Display for BedrockAnthropicStreamServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.event_type, self.message)
    }
}

impl StdError for BedrockAnthropicStreamServiceError {}

/// Failure decoding Bedrock's Smithy event stream.
#[derive(Debug, thiserror::Error)]
pub enum BedrockAnthropicStreamDecodeError {
    #[error("Bedrock service stream error: {0}")]
    Service(BedrockAnthropicStreamServiceError),
    #[error("Bedrock event-stream decoding: {0}")]
    Decode(String),
}

#[derive(Debug)]
struct BedrockAnthropicStreamUnmarshaller;

impl UnmarshallMessage for BedrockAnthropicStreamUnmarshaller {
    type Output = Value;
    type Error = BedrockAnthropicStreamServiceError;

    fn unmarshall(
        &self,
        message: &EventStreamMessage,
    ) -> Result<UnmarshalledMessage<Self::Output, Self::Error>, EventStreamError> {
        let event_type = message
            .headers()
            .iter()
            .find(|header| header.name().as_str() == ":event-type")
            .and_then(|header| match header.value() {
                HeaderValue::String(value) => Some(value.as_str()),
                _ => None,
            })
            .ok_or_else(|| {
                EventStreamError::unmarshalling(
                    "amazon-bedrock stream frame missing :event-type header",
                )
            })?;

        let payload = if message.payload().is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_slice::<Value>(message.payload()).map_err(|error| {
                EventStreamError::unmarshalling(format!(
                    "amazon-bedrock stream frame payload was not valid JSON: {error}"
                ))
            })?
        };

        match event_type {
            "chunk" => {
                let encoded = payload
                    .get("bytes")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        EventStreamError::unmarshalling(
                            "amazon-bedrock chunk event missing base64 `bytes` field",
                        )
                    })?;
                let decoded = BASE64_STANDARD.decode(encoded).map_err(|error| {
                    EventStreamError::unmarshalling(format!(
                        "amazon-bedrock chunk event contained invalid base64 payload: {error}"
                    ))
                })?;
                let event = serde_json::from_slice::<Value>(&decoded).map_err(|error| {
                    EventStreamError::unmarshalling(format!(
                        "amazon-bedrock chunk event contained invalid Anthropic JSON: {error}"
                    ))
                })?;
                Ok(UnmarshalledMessage::Event(event))
            }
            "internalServerException"
            | "modelStreamErrorException"
            | "validationException"
            | "throttlingException"
            | "modelTimeoutException"
            | "serviceUnavailableException" => Ok(UnmarshalledMessage::Error(
                BedrockAnthropicStreamServiceError::from_payload(event_type, payload),
            )),
            other => Err(EventStreamError::unmarshalling(format!(
                "amazon-bedrock stream returned unknown event type `{other}`"
            ))),
        }
    }
}

/// Decode a successful Bedrock response without leaking Smithy frame types to
/// the Runtime adapter.
pub fn decode_response(
    response: reqwest::Response,
) -> Pin<Box<dyn Stream<Item = Result<Value, BedrockAnthropicStreamDecodeError>> + Send>> {
    let response_stream = response.bytes_stream().map_ok(Frame::data);
    let body = SdkBody::from_body_1_x(http_body_util::StreamBody::new(response_stream));
    let receiver = Receiver::<Value, BedrockAnthropicStreamServiceError>::new(
        BedrockAnthropicStreamUnmarshaller,
        body,
    );

    Box::pin(stream! {
        let mut receiver = receiver;
        loop {
            match receiver.recv().await {
                Ok(Some(event)) => yield Ok(event),
                Ok(None) => break,
                Err(error) => {
                    if let Some(service) = error.as_service_error() {
                        yield Err(BedrockAnthropicStreamDecodeError::Service(service.clone()));
                    } else {
                        let source = error
                            .into_source()
                            .map(|source| source.to_string())
                            .unwrap_or_else(|error| error.to_string());
                        yield Err(BedrockAnthropicStreamDecodeError::Decode(source));
                    }
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::BedrockAnthropicStreamServiceError;
    use serde_json::json;

    #[test]
    fn service_error_preserves_message_and_retry_policy() {
        let throttled = BedrockAnthropicStreamServiceError::from_payload(
            "throttlingException",
            json!({"message": "slow down"}),
        );
        assert_eq!(throttled.message, "slow down");
        assert!(throttled.retryable);

        let invalid = BedrockAnthropicStreamServiceError::from_payload(
            "validationException",
            json!({"originalMessage": "unsupported model"}),
        );
        assert_eq!(invalid.message, "unsupported model");
        assert!(!invalid.retryable);
    }
}
