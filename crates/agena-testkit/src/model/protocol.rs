//! Explicit wire protocols for the local scripted model.
//!
//! Common Reply builders adapt to the incoming endpoint. ModelEvent retains
//! arbitrary provider fields for messages, multimodal blocks, usage, errors,
//! signatures, and future events that do not have a convenience builder.

use anyhow::{Result, bail, ensure};
use bytes::Bytes;
use serde_json::Value;

use super::{ModelRoute, Reply};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelProtocol {
    #[default]
    OpenAiResponses,
    OpenAiChat,
    Anthropic,
    Gemini,
    /// Anthropic InvokeModel / InvokeModelWithResponseStream over AWS EventStream.
    BedrockAnthropic,
    /// Converse / ConverseStream fixtures. Agena has no Converse adapter yet.
    /// Use with ScriptedModel directly, not TestAgena::model_protocol.
    BedrockConverse,
    Ollama,
}

impl ModelProtocol {
    pub fn adapter(self) -> Option<&'static str> {
        match self {
            Self::OpenAiResponses => Some("openai_responses"),
            Self::OpenAiChat => Some("openai_chat_completions"),
            Self::Anthropic => Some("anthropic"),
            Self::Gemini => Some("gemini"),
            Self::BedrockAnthropic => Some("amazon_bedrock"),
            Self::BedrockConverse => None,
            Self::Ollama => Some("ollama"),
        }
    }

    /// A deterministic model ID suitable for the product's selected adapter.
    pub fn model_id(self) -> &'static str {
        match self {
            Self::BedrockAnthropic => "us.anthropic.claude-test-v1:0",
            _ => "fake-model",
        }
    }

    pub fn routes(self) -> Vec<ModelRoute> {
        let routes = match self {
            Self::OpenAiResponses => vec![ModelRoute::post("/v1/responses")],
            Self::OpenAiChat => vec![ModelRoute::post("/v1/chat/completions")],
            Self::Anthropic => vec![ModelRoute::post("/v1/messages")],
            Self::Gemini => vec![ModelRoute::post_prefix("/v1beta/models/")],
            Self::BedrockAnthropic | Self::BedrockConverse => {
                vec![ModelRoute::post_prefix("/model/")]
            }
            Self::Ollama => vec![ModelRoute::post("/api/chat"), ModelRoute::get("/api/tags")],
        };
        routes
            .into_iter()
            .map(|route| route.for_protocol(self))
            .collect()
    }

    pub fn request_path(self, model: &str, streaming: bool) -> String {
        match self {
            Self::OpenAiResponses => "/v1/responses".into(),
            Self::OpenAiChat => "/v1/chat/completions".into(),
            Self::Anthropic => "/v1/messages".into(),
            Self::Gemini => {
                let model = encode_segment(model);
                format!(
                    "/v1beta/models/{model}:{}",
                    if streaming {
                        "streamGenerateContent"
                    } else {
                        "generateContent"
                    }
                )
            }
            Self::BedrockAnthropic | Self::BedrockConverse => {
                let model = encode_segment(model);
                let operation = match (self, streaming) {
                    (Self::BedrockAnthropic, true) => "invoke-with-response-stream",
                    (Self::BedrockAnthropic, false) => "invoke",
                    (_, true) => "converse-stream",
                    (_, false) => "converse",
                };
                format!("/model/{model}/{operation}")
            }
            Self::Ollama => "/api/chat".into(),
        }
    }

    pub(crate) fn from_path(path: &str) -> Option<Self> {
        let path = path.trim_end_matches('/');
        match path {
            "/v1/responses" => Some(Self::OpenAiResponses),
            "/v1/chat/completions" => Some(Self::OpenAiChat),
            "/v1/messages" => Some(Self::Anthropic),
            "/api/chat" => Some(Self::Ollama),
            _ if path.starts_with("/v1beta/models/")
                && (path.ends_with(":generateContent")
                    || path.ends_with(":streamGenerateContent")) =>
            {
                Some(Self::Gemini)
            }
            _ if path.starts_with("/model/")
                && (path.ends_with("/invoke")
                    || path.ends_with("/invoke-with-response-stream")) =>
            {
                Some(Self::BedrockAnthropic)
            }
            _ if path.starts_with("/model/")
                && (path.ends_with("/converse") || path.ends_with("/converse-stream")) =>
            {
                Some(Self::BedrockConverse)
            }
            _ => None,
        }
    }

    /// Send a complete native JSON message, preserving all supplied fields.
    pub fn message(self, body: Value) -> Reply {
        Reply::raw_json(200, body)
    }

    /// Encode exactly these events, without inserting a terminal event. This is
    /// also the escape hatch for server tools, citations, media, ping, errors,
    /// partial arguments, unknown events, and deliberately incomplete streams.
    pub fn events(self, events: impl IntoIterator<Item = ModelEvent>) -> Reply {
        Reply::events(self, events)
    }

    pub(crate) fn stream_content_type(self) -> &'static str {
        match self {
            Self::BedrockAnthropic | Self::BedrockConverse => "application/vnd.amazon.eventstream",
            Self::Ollama => "application/x-ndjson",
            _ => "text/event-stream",
        }
    }
}

/// A complete provider event. The JSON is never projected onto a smaller schema.
#[derive(Clone, Debug)]
pub enum ModelEvent {
    Json(Value),
    Named {
        name: String,
        data: Value,
    },
    /// An outer AWS message event, rather than an inner Anthropic chunk.
    Bedrock {
        name: String,
        data: Value,
    },
    /// A modeled AWS service error, with :exception-type and message-type exception.
    Exception {
        name: String,
        data: Value,
    },
    /// An unmodeled AWS error, carried in headers with an empty payload.
    Error {
        code: String,
        message: String,
    },
    /// OpenAI's literal SSE sentinel. Not valid for other wire protocols.
    Done,
}

impl ModelEvent {
    pub fn json(data: Value) -> Self {
        Self::Json(data)
    }

    pub fn named(name: impl Into<String>, data: Value) -> Self {
        Self::Named {
            name: name.into(),
            data,
        }
    }

    pub fn bedrock(name: impl Into<String>, data: Value) -> Self {
        Self::Bedrock {
            name: name.into(),
            data,
        }
    }

    pub fn exception(name: impl Into<String>, data: Value) -> Self {
        Self::Exception {
            name: name.into(),
            data,
        }
    }

    /// Preserve an unmodeled AWS error code and message in their native headers.
    /// Encoding this event with a non-Bedrock protocol is an explicit error.
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<Value> for ModelEvent {
    fn from(data: Value) -> Self {
        Self::Json(data)
    }
}

pub(super) fn encode_events(protocol: ModelProtocol, events: &[ModelEvent]) -> Result<Vec<Bytes>> {
    events
        .iter()
        .map(|event| encode_event(protocol, event))
        .collect()
}

pub(super) fn encode_event(protocol: ModelProtocol, event: &ModelEvent) -> Result<Bytes> {
    let (name, data) = match event {
        ModelEvent::Error { code, message } => {
            ensure!(
                matches!(
                    protocol,
                    ModelProtocol::BedrockAnthropic | ModelProtocol::BedrockConverse
                ),
                "AWS error events require a Bedrock protocol"
            );
            return super::wire::bedrock_error_frame(code, message);
        }
        ModelEvent::Done => {
            ensure!(
                matches!(
                    protocol,
                    ModelProtocol::OpenAiChat | ModelProtocol::OpenAiResponses
                ),
                "the [DONE] sentinel is only valid for OpenAI SSE"
            );
            return Ok(Bytes::from_static(b"data: [DONE]\n\n"));
        }
        ModelEvent::Bedrock { name, data } | ModelEvent::Exception { name, data } => {
            ensure!(
                matches!(
                    protocol,
                    ModelProtocol::BedrockAnthropic | ModelProtocol::BedrockConverse
                ),
                "AWS envelope events require a Bedrock protocol"
            );
            return super::wire::bedrock_frame(
                name,
                data,
                matches!(event, ModelEvent::Exception { .. }),
            );
        }
        ModelEvent::Named { name, data } => (Some(name.as_str()), data),
        ModelEvent::Json(data) => (data.get("type").and_then(Value::as_str), data),
    };
    match protocol {
        ModelProtocol::BedrockAnthropic => super::wire::anthropic_frame(data),
        ModelProtocol::BedrockConverse => {
            if let Some(name) = name {
                super::wire::bedrock_frame(name, data, false)
            } else if let Some(object) = data.as_object().filter(|object| object.len() == 1) {
                let (name, payload) = object.iter().next().expect("one event member");
                super::wire::bedrock_frame(name, payload, false)
            } else {
                bail!("Converse events require a name or one named event member")
            }
        }
        ModelProtocol::Ollama => Ok(Bytes::from(format!("{}\n", serde_json::to_string(data)?))),
        _ => {
            let named = matches!(protocol, ModelProtocol::Anthropic)
                || matches!(event, ModelEvent::Named { .. });
            let mut frame = String::new();
            if named && let Some(name) = name {
                ensure!(
                    !name.is_empty() && !name.contains(['\r', '\n', '\0']),
                    "SSE event names must be non-empty single lines"
                );
                frame.push_str("event: ");
                frame.push_str(name);
                frame.push('\n');
            }
            frame.push_str("data: ");
            frame.push_str(&serde_json::to_string(data)?);
            frame.push_str("\n\n");
            Ok(Bytes::from(frame))
        }
    }
}

fn encode_segment(model: &str) -> String {
    let mut url = url::Url::parse("http://fixture.invalid").expect("constant fixture URL");
    url.path_segments_mut().expect("HTTP path").push(model);
    url.path().trim_start_matches('/').to_owned()
}

/// Decode only a model path segment, without applying form '+' conversion.
pub(super) fn decode_model(model: &str) -> String {
    let mut bytes = Vec::with_capacity(model.len());
    let mut input = model.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let a = input.next();
            let b = input.next();
            if let (Some(a), Some(b)) = (a, b) {
                let digit = |x: u8| (x as char).to_digit(16).map(|x| x as u8);
                if let (Some(a), Some(b)) = (digit(a), digit(b)) {
                    bytes.push((a << 4) | b);
                    continue;
                }
                bytes.extend_from_slice(&[b'%', a, b]);
            } else {
                bytes.push(b'%');
                bytes.extend(a);
                bytes.extend(b);
            }
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| model.to_owned())
}
