//! Concrete AWS Smithy event-stream decoding for Amazon Bedrock Anthropic.
//!
//! This leaf converts a successful Bedrock HTTP response into decoded JSON
//! events. Product-specific Anthropic event projection, request construction,
//! logging, and error presentation remain with the Runtime provider adapter.

use std::{error::Error as StdError, fmt, pin::Pin};

use async_stream::stream;
use aws_smithy_eventstream::{
    error::Error as EventStreamError,
    frame::{UnmarshallMessage, UnmarshalledMessage, read_message_from},
};
use aws_smithy_types::event_stream::{HeaderValue, Message as EventStreamMessage};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::Value;

static FRAME_DECODERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
const INLINE_BYTES: usize = 64 * 1024;
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_BATCH_EVENTS: usize = 64;

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

#[derive(Default)]
struct FrameBuffer {
    pending: BytesMut,
    ended: bool,
}

impl FrameBuffer {
    /// Pure framing/CRC/JSON/base64 work. Limit each batch, retain fragmented
    /// frames, and let the async owner fetch network chunks between batches.
    fn feed(
        &mut self,
        chunk: Option<Bytes>,
    ) -> Vec<Result<Value, BedrockAnthropicStreamDecodeError>> {
        let mut events = Vec::new();
        if let Some(chunk) = chunk {
            if chunk.len() > MAX_FRAME_BYTES
                || self.pending.len().saturating_add(chunk.len()) > 2 * MAX_FRAME_BYTES
            {
                self.ended = true;
                events.push(Err(BedrockAnthropicStreamDecodeError::Decode(
                    "Bedrock event-stream buffer exceeds its size limit".to_owned(),
                )));
                return events;
            }
            self.pending.extend_from_slice(&chunk);
        }
        while events.len() < MAX_BATCH_EVENTS && self.pending.len() >= 12 {
            let frame_len =
                u32::from_be_bytes(self.pending[..4].try_into().expect("four bytes")) as usize;
            let headers_len =
                u32::from_be_bytes(self.pending[4..8].try_into().expect("four bytes")) as usize;
            if !(16..=MAX_FRAME_BYTES).contains(&frame_len) || headers_len > frame_len - 16 {
                self.ended = true;
                events.push(Err(BedrockAnthropicStreamDecodeError::Decode(
                    "Bedrock event-stream frame has an invalid or oversized length".to_owned(),
                )));
                break;
            }
            if self.pending.len() < frame_len {
                break;
            }
            let frame = self.pending.split_to(frame_len).freeze();
            // Smithy still validates the prelude and message checksums and
            // header layout; only its synchronous execution boundary changes.
            let event = read_message_from(frame)
                .and_then(|message| BedrockAnthropicStreamUnmarshaller.unmarshall(&message));
            let event = match event {
                Ok(UnmarshalledMessage::Event(event)) => Ok(event),
                Ok(UnmarshalledMessage::Error(service)) => {
                    Err(BedrockAnthropicStreamDecodeError::Service(service))
                }
                Err(error) => Err(BedrockAnthropicStreamDecodeError::Decode(error.to_string())),
            };
            self.ended = event.is_err();
            events.push(event);
            if self.ended {
                break;
            }
        }
        events
    }
}

/// Decode a successful Bedrock response. Network I/O stays async; large frame
/// accumulation, CRC validation and payload decoding use bounded workers.
pub fn decode_response(
    response: reqwest::Response,
) -> Pin<Box<dyn Stream<Item = Result<Value, BedrockAnthropicStreamDecodeError>> + Send>> {
    Box::pin(stream! {
        let mut chunks = response.bytes_stream();
        let mut state = FrameBuffer::default();
        while let Some(chunk) = chunks.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    yield Err(BedrockAnthropicStreamDecodeError::Decode(error.to_string()));
                    return;
                }
            };
            let mut input = Some(chunk);
            loop {
                tokio::task::consume_budget().await;
                let offload = state.pending.len().saturating_add(input.as_ref().map_or(0, Bytes::len)) >= INLINE_BYTES;
                let decode = move || {
                    let events = state.feed(input);
                    (state, events)
                };
                let (next, events) = if offload {
                    match FRAME_DECODERS.run(decode).await {
                        Ok(result) => result,
                        Err(error) => {
                            yield Err(BedrockAnthropicStreamDecodeError::Decode(format!("Bedrock frame worker failed: {error}")));
                            return;
                        }
                    }
                } else {
                    decode()
                };
                state = next;
                let full_batch = events.len() == MAX_BATCH_EVENTS;
                for event in events { yield event; }
                if state.ended { return; }
                if full_batch {
                    input = None;
                } else {
                    break;
                }
            }
        }
        if !state.pending.is_empty() {
            yield Err(BedrockAnthropicStreamDecodeError::Decode("unexpected end of Bedrock event stream".to_owned()));
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
