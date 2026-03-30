use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    error::AppError,
    message::{Message, MessageUsage},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ModelProvider, ProviderModel, StreamResumePolicy, sse,
        utils,
    },
    role::Role,
};

const PROVIDER_ID: &str = "openai";

#[derive(Clone)]
pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    default_model: String,
    api_mode: OpenAiApiMode,
    extra_headers: HashMap<String, String>,
    stream_mode: OpenAiStreamMode,
    realtime_ws_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiApiMode {
    Responses,
    Chat,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiStreamMode {
    Sse,
    RealtimeWebSocket,
}

impl OpenAiProvider {
    pub fn new(
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            client,
            api_key: api_key.into(),
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: default_model.into(),
            api_mode: OpenAiApiMode::Responses,
            extra_headers: HashMap::new(),
            stream_mode: OpenAiStreamMode::Sse,
            realtime_ws_url: None,
        }
    }

    pub fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.extra_headers = headers;
        self
    }

    pub fn with_api_mode(mut self, mode: OpenAiApiMode) -> Self {
        self.api_mode = mode;
        self
    }

    pub fn with_stream_mode(mut self, mode: OpenAiStreamMode) -> Self {
        self.stream_mode = mode;
        self
    }

    pub fn with_realtime_ws_url(mut self, ws_url: Option<String>) -> Self {
        self.realtime_ws_url = ws_url.and_then(|value| utils::normalize_optional_text(Some(value)));
        self
    }

    fn model_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn responses_endpoint(&self) -> String {
        format!("{}/responses", self.base_url)
    }

    fn chat_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn realtime_ws_endpoint(&self, model: &str) -> Result<url::Url, AppError> {
        let mut endpoint = if let Some(ws_url) = self.realtime_ws_url.as_ref() {
            url::Url::parse(ws_url).map_err(|err| {
                AppError::Config(format!("openai realtime websocket url is invalid: {err}"))
            })?
        } else {
            let mut url = url::Url::parse(self.base_url.as_str())
                .map_err(|err| AppError::Config(format!("openai base url is invalid: {err}")))?;
            let realtime_path = format!("{}/realtime", url.path().trim_end_matches('/'));
            url.set_path(realtime_path.as_str());
            url
        };

        match endpoint.scheme() {
            "http" => endpoint.set_scheme("ws").map_err(|_| {
                AppError::Config("openai realtime websocket url is invalid".to_owned())
            })?,
            "https" => endpoint.set_scheme("wss").map_err(|_| {
                AppError::Config("openai realtime websocket url is invalid".to_owned())
            })?,
            "ws" | "wss" => {}
            other => {
                return Err(AppError::Config(format!(
                    "openai realtime websocket url has unsupported scheme `{other}`"
                )));
            }
        }

        let existing = endpoint
            .query_pairs()
            .into_owned()
            .filter(|(key, _)| key != "model")
            .collect::<Vec<_>>();

        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in existing {
            serializer.append_pair(key.as_str(), value.as_str());
        }
        serializer.append_pair("model", model);
        let query = serializer.finish();
        endpoint.set_query(Some(query.as_str()));

        Ok(endpoint)
    }

    fn realtime_handshake_request(
        &self,
        endpoint: &url::Url,
    ) -> Result<http::Request<()>, AppError> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let mut request = endpoint.as_str().into_client_request().map_err(|err| {
            AppError::Config(format!(
                "openai realtime websocket handshake invalid: {err}"
            ))
        })?;

        let auth_header_name = http::header::HeaderName::from_static("authorization");
        let auth_header_value =
            http::header::HeaderValue::from_str(format!("Bearer {}", self.api_key).as_str())
                .map_err(|err| {
                    AppError::Config(format!("openai auth header value is invalid: {err}"))
                })?;
        request
            .headers_mut()
            .insert(auth_header_name, auth_header_value);

        if endpoint
            .host_str()
            .map(|host| {
                host.eq_ignore_ascii_case("api.openai.com") || host.ends_with(".openai.com")
            })
            .unwrap_or(false)
        {
            request.headers_mut().insert(
                http::header::HeaderName::from_static("openai-beta"),
                http::header::HeaderValue::from_static("realtime=v1"),
            );
        }

        for (key, value) in &self.extra_headers {
            let header_name =
                http::header::HeaderName::from_bytes(key.as_bytes()).map_err(|err| {
                    AppError::Config(format!(
                        "openai extra header name `{key}` is invalid: {err}"
                    ))
                })?;
            let header_value =
                http::header::HeaderValue::from_str(value.as_str()).map_err(|err| {
                    AppError::Config(format!(
                        "openai extra header `{key}` value is invalid: {err}"
                    ))
                })?;
            request.headers_mut().insert(header_name, header_value);
        }

        Ok(request)
    }

    fn should_use_responses(&self, model: &str) -> bool {
        match self.api_mode {
            OpenAiApiMode::Responses => true,
            OpenAiApiMode::Chat => false,
            OpenAiApiMode::Auto => {
                model.starts_with("gpt-5") || model.starts_with("o3") || model.starts_with("o4")
            }
        }
    }

    fn responses_endpoint_unsupported(status: reqwest::StatusCode) -> bool {
        matches!(
            status,
            reqwest::StatusCode::NOT_FOUND
                | reqwest::StatusCode::METHOD_NOT_ALLOWED
                | reqwest::StatusCode::NOT_IMPLEMENTED
        )
    }

    async fn complete_with_chat_api(
        &self,
        request: &CompletionRequest,
        model: String,
    ) -> Result<CompletionResponse, AppError> {
        let body = OpenAiChatCompletionRequest {
            model: model.clone(),
            messages: Self::to_chat_messages(request),
            tools: (!request.tools.is_empty()).then(|| Self::chat_tools(request.tools.as_slice())),
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            stream: false,
            stream_options: None,
        };

        let response = self
            .apply_headers(
                self.client
                    .post(self.chat_endpoint())
                    .bearer_auth(&self.api_key)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
            )
            .json(&body)
            .send()
            .await?;

        let payload: OpenAiChatCompletionResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;

        let text = payload
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.content.as_ref())
            .map(extract_chat_content_text)
            .or_else(|| payload.choices.first().and_then(|c| c.text.clone()))
            .unwrap_or_default();

        let finish_reason = CompletionFinishReason::from_provider(
            payload
                .choices
                .first()
                .and_then(|c| c.finish_reason.as_deref()),
        );

        let tool_calls = Self::parse_chat_tool_calls(
            payload
                .choices
                .first()
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.tool_calls.as_ref()),
        )?;

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(
                "openai chat completion payload was empty without finish reason".to_owned(),
            ));
        }

        Ok(CompletionResponse {
            provider_id: PROVIDER_ID.to_owned(),
            model: payload.model.unwrap_or(model),
            text,
            finish_reason,
            tool_calls,
            usage: Self::map_chat_usage(payload.usage),
            provider_metadata: None,
        })
    }

    async fn complete_stream_with_chat_api(
        &self,
        request: &CompletionRequest,
        model: String,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let body = OpenAiChatCompletionRequest {
            model: model.clone(),
            messages: Self::to_chat_messages(request),
            tools: (!request.tools.is_empty()).then(|| Self::chat_tools(request.tools.as_slice())),
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            stream: true,
            stream_options: Some(OpenAiChatStreamOptions {
                include_usage: true,
            }),
        };

        let response = self
            .apply_headers(
                self.client
                    .post(self.chat_endpoint())
                    .bearer_auth(&self.api_key)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

        let mut events = sse::json_events(response);
        let provider_id = PROVIDER_ID.to_owned();
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ChatToolCallState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;

            while let Some(event) = events.next().await {
                let event = event?;

                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(PROVIDER_ID, "chat stream chunk", event)?;
                let choice = chunk.choices.first();

                let delta = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.content.as_ref())
                    .map(extract_chat_content_text)
                    .or_else(|| choice.and_then(|item| item.text.clone()))
                    .unwrap_or_default();

                if !delta.is_empty() {
                    stream_has_content = true;
                    yield CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                let tool_deltas = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.tool_calls.clone())
                    .unwrap_or_default();

                for raw_tool in tool_deltas {
                    let tool = utils::parse_json_value::<OpenAiChatToolCall>(
                        PROVIDER_ID,
                        "chat stream tool_call delta",
                        raw_tool,
                    )?;
                    let id = utils::normalize_optional_text(tool.id.clone());
                    let key = tool
                        .index
                        .map(|idx| format!("idx:{idx}"))
                        .or_else(|| id.as_ref().map(|value| format!("id:{value}")))
                        .ok_or_else(|| {
                            AppError::Provider(
                                "openai chat stream tool_call delta missing index/id".to_owned(),
                            )
                        })?;

                    let state = pending_tool_calls.entry(key.clone()).or_default();
                    if let Some(id) = id {
                        state.id = Some(id);
                    }
                    if let Some(function) = tool.function {
                        if let Some(name) = utils::normalize_optional_text(function.name) {
                            state.name = Some(name);
                        }
                        if let Some(args) = function.arguments {
                            if !args.is_empty() {
                                state.arguments.push_str(args.as_str());
                                stream_has_content = true;
                                yield CompletionStreamEvent::ToolCallDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    stream_key: key.clone(),
                                    id: state.id.clone(),
                                    name: state.name.clone(),
                                    arguments_delta: args,
                                };
                            }
                        }
                    }
                }

                if let Some(raw_usage) = chunk.usage {
                    let usage = utils::parse_json_value::<OpenAiChatUsage>(
                        PROVIDER_ID,
                        "chat stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Some(
                        MessageUsage {
                            input_tokens: usage.prompt_tokens.unwrap_or_default(),
                            output_tokens: usage.completion_tokens.unwrap_or_default(),
                            reasoning_tokens: 0,
                            cache_write_tokens: 0,
                            cache_read_tokens: 0,
                            total_cost: 0.0,
                        }
                        .into(),
                    );
                }

                let finish_reason = choice
                    .and_then(|item| item.finish_reason.as_deref())
                    .filter(|value| !value.is_empty() && *value != "null")
                    .map(ToOwned::to_owned);

                if stream_finish_reason.is_none() {
                    stream_finish_reason = finish_reason;
                }
            }

            if stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some() {
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage,
                    provider_metadata: None,
                };
            }
        };

        Ok(Box::pin(stream))
    }

    async fn complete_stream_with_realtime_ws(
        &self,
        request: &CompletionRequest,
        model: String,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let ws_endpoint = self.realtime_ws_endpoint(model.as_str())?;
        let handshake = self.realtime_handshake_request(&ws_endpoint)?;
        let (ws_stream, _) = tokio_tungstenite::connect_async(handshake)
            .await
            .map_err(|err| {
                AppError::Provider(format!("openai realtime websocket connect failed: {err}"))
            })?;

        let provider_id = PROVIDER_ID.to_owned();
        let model_name = model;
        let input_text = build_realtime_input_text(request.messages.as_slice());
        let response_tools = (!request.tools.is_empty()).then(|| {
            serde_json::to_value(Self::responses_tools(request.tools.as_slice()))
                .expect("realtime tool definitions should serialize")
        });
        let system = request.system.clone();
        let temperature = request.temperature;
        let max_output_tokens = request.max_output_tokens;

        let stream = async_stream::try_stream! {
            let (mut ws_writer, mut ws_reader) = ws_stream.split();

            if let Some(instructions) = system.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                let event = serde_json::json!({
                    "type": "session.update",
                    "session": {
                        "instructions": instructions,
                    }
                });
                ws_writer
                    .send(tokio_tungstenite::tungstenite::Message::Text(event.to_string().into()))
                    .await
                    .map_err(|err| {
                        AppError::Provider(format!(
                            "openai realtime websocket send session.update failed: {err}"
                        ))
                    })?;
            }

            if let Some(text) = input_text.filter(|value| !value.is_empty()) {
                let event = serde_json::json!({
                    "type": "conversation.item.create",
                    "item": {
                        "type": "message",
                        "role": "user",
                        "content": [{
                            "type": "input_text",
                            "text": text,
                        }],
                    }
                });

                ws_writer
                    .send(tokio_tungstenite::tungstenite::Message::Text(event.to_string().into()))
                    .await
                    .map_err(|err| {
                        AppError::Provider(format!(
                            "openai realtime websocket send conversation.item.create failed: {err}"
                        ))
                    })?;
            }

            let mut response = serde_json::json!({
                "modalities": ["text"],
            });
            if let Some(temperature) = temperature {
                response["temperature"] = serde_json::json!(temperature);
            }
            if let Some(max_tokens) = max_output_tokens {
                response["max_output_tokens"] = serde_json::json!(max_tokens);
            }
            if let Some(tools) = response_tools.as_ref() {
                response["tools"] = tools.clone();
            }

            let create_event = serde_json::json!({
                "type": "response.create",
                "response": response,
            });

            ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Text(create_event.to_string().into()))
                .await
                .map_err(|err| {
                    AppError::Provider(format!(
                        "openai realtime websocket send response.create failed: {err}"
                    ))
                })?;

            let mut pending_tool_calls: std::collections::BTreeMap<String, ResponsesToolState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut completed_emitted = false;

            while let Some(message) = ws_reader.next().await {
                let message = message.map_err(|err| {
                    AppError::Provider(format!("openai realtime websocket receive failed: {err}"))
                })?;

                let payload = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        String::from_utf8(bytes.to_vec()).map_err(|err| {
                            AppError::Provider(format!(
                                "openai realtime websocket binary frame is not utf-8: {err}"
                            ))
                        })?
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    tokio_tungstenite::tungstenite::Message::Ping(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Frame(_) => continue,
                };

                let event: serde_json::Value = serde_json::from_str(payload.as_str()).map_err(|err| {
                    AppError::Provider(format!("openai realtime websocket event decode failed: {err}"))
                })?;

                if let Some(err) = utils::responses_stream_error(PROVIDER_ID, &event)? {
                    Err(err)?;
                }

                if let Some(delta) = utils::responses_text_delta(&event) {
                    stream_has_content = true;
                    yield CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                if let Some(tool_event) = utils::responses_tool_event(PROVIDER_ID, &event)? {
                    let key = tool_event.stream_key(PROVIDER_ID)?;

                    let state = pending_tool_calls.entry(key.clone()).or_default();
                    if let Some(id) = tool_event.id.clone() {
                        state.id = Some(id);
                    }
                    if let Some(name) = tool_event.name.clone() {
                        state.name = Some(name);
                    }

                    match tool_event.kind {
                        utils::ResponsesToolEventKind::Delta => {
                            if let Some(arguments_delta) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                state.arguments.push_str(arguments_delta.as_str());
                                stream_has_content = true;
                                yield CompletionStreamEvent::ToolCallDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    stream_key: key.clone(),
                                    id: state.id.clone(),
                                    name: state.name.clone(),
                                    arguments_delta,
                                };
                            }
                        }
                        utils::ResponsesToolEventKind::Added => {
                            if let Some(arguments_snapshot) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
                                {
                                    arguments_snapshot[state.arguments.len()..].to_owned()
                                } else {
                                    arguments_snapshot.clone()
                                };

                                if arguments_snapshot.starts_with(&state.arguments) {
                                    state.arguments.push_str(arguments_delta.as_str());
                                } else {
                                    state.arguments = arguments_snapshot;
                                }

                                if !arguments_delta.is_empty() {
                                    stream_has_content = true;
                                    yield CompletionStreamEvent::ToolCallDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        stream_key: key.clone(),
                                        id: state.id.clone(),
                                        name: state.name.clone(),
                                        arguments_delta,
                                    };
                                }
                            }
                        }
                        utils::ResponsesToolEventKind::Done => {
                            if let Some(arguments_snapshot) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
                                {
                                    arguments_snapshot[state.arguments.len()..].to_owned()
                                } else {
                                    arguments_snapshot.clone()
                                };

                                if arguments_snapshot.starts_with(&state.arguments) {
                                    state.arguments.push_str(arguments_delta.as_str());
                                } else {
                                    state.arguments = arguments_snapshot;
                                }

                                if !arguments_delta.is_empty() {
                                    stream_has_content = true;
                                    yield CompletionStreamEvent::ToolCallDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        stream_key: key.clone(),
                                        id: state.id.clone(),
                                        name: state.name.clone(),
                                        arguments_delta,
                                    };
                                }
                            }

                            pending_tool_calls.remove(key.as_str());
                        }
                    }
                }

                if let Some(raw_usage) = utils::responses_usage_value(&event) {
                    let usage = utils::parse_json_value::<OpenAiUsage>(
                        PROVIDER_ID,
                        "realtime stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Self::map_usage(Some(usage));
                }

                if stream_finish_reason.is_none() {
                    stream_finish_reason = utils::responses_finish_reason(&event);
                }

                if utils::responses_is_completed(&event) {
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason: CompletionFinishReason::from_provider(
                            stream_finish_reason.as_deref(),
                        ),
                        usage: stream_usage.clone(),
                        provider_metadata: None,
                    };
                    completed_emitted = true;
                    break;
                }
            }

            let _ = ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Close(None))
                .await;

            if !completed_emitted
                && (stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some())
            {
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage,
                    provider_metadata: None,
                };
            }
        };

        Ok(Box::pin(stream))
    }

    fn extract_text(response: &OpenAiResponsesResponse) -> String {
        if let Some(text) = response.output_text.as_ref() {
            return text.clone();
        }

        response
            .output
            .iter()
            .flatten()
            .flat_map(|item| item.content.iter().flatten())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("")
    }

    fn map_usage(usage: Option<OpenAiUsage>) -> Option<CompletionUsage> {
        usage.map(|u| {
            MessageUsage {
                input_tokens: u.input_tokens.unwrap_or_default(),
                output_tokens: u.output_tokens.unwrap_or_default(),
                reasoning_tokens: u
                    .output_tokens_details
                    .and_then(|d| d.reasoning_tokens)
                    .unwrap_or_default(),
                cache_write_tokens: 0,
                cache_read_tokens: u
                    .input_tokens_details
                    .and_then(|d| d.cached_tokens)
                    .unwrap_or_default(),
                total_cost: 0.0,
            }
            .into()
        })
    }

    fn map_chat_usage(usage: Option<OpenAiChatUsage>) -> Option<CompletionUsage> {
        usage.map(|u| {
            MessageUsage {
                input_tokens: u.prompt_tokens.unwrap_or_default(),
                output_tokens: u.completion_tokens.unwrap_or_default(),
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 0.0,
            }
            .into()
        })
    }

    fn responses_tools(tools: &[crate::tool::ToolDefinition]) -> Vec<OpenAiResponsesTool> {
        tools
            .iter()
            .map(|tool| OpenAiResponsesTool {
                kind: "function",
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            })
            .collect()
    }

    fn chat_tools(tools: &[crate::tool::ToolDefinition]) -> Vec<OpenAiChatToolDefinition> {
        tools
            .iter()
            .map(|tool| OpenAiChatToolDefinition {
                kind: "function".to_string(),
                function: OpenAiChatFunctionDefinition {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone(),
                },
            })
            .collect()
    }

    fn to_chat_messages(request: &CompletionRequest) -> Vec<OpenAiChatMessage> {
        let mut messages = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            messages.push(OpenAiChatMessage {
                role: "system".to_owned(),
                content: Some(serde_json::Value::String(system.clone())),
                tool_calls: None,
                tool_call_id: None,
                kind: None,
            });
        }

        for message in &request.messages {
            let projected_parts = utils::project_session_parts(message);
            match message.role {
                Role::System => messages.push(OpenAiChatMessage {
                    role: "system".to_owned(),
                    content: Some(serde_json::Value::String(session_text_lossy(
                        message,
                        projected_parts.as_slice(),
                    ))),
                    tool_calls: None,
                    tool_call_id: None,
                    kind: None,
                }),
                Role::User => messages.push(OpenAiChatMessage {
                    role: "user".to_owned(),
                    content: Some(serde_json::Value::String(session_text_lossy(
                        message,
                        projected_parts.as_slice(),
                    ))),
                    tool_calls: None,
                    tool_call_id: None,
                    kind: None,
                }),
                Role::Assistant => {
                    let (content, tool_calls) = Self::assistant_chat_content_and_tool_calls(
                        message,
                        projected_parts.as_slice(),
                    );
                    messages.push(OpenAiChatMessage {
                        role: "assistant".to_owned(),
                        content,
                        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
                        tool_call_id: None,
                        kind: None,
                    });
                }
                Role::Tool => {
                    let tool_messages = Self::tool_chat_messages(projected_parts.as_slice());
                    if tool_messages.is_empty() {
                        messages.push(OpenAiChatMessage {
                            role: "user".to_owned(),
                            content: Some(serde_json::Value::String(session_text_lossy(
                                message,
                                projected_parts.as_slice(),
                            ))),
                            tool_calls: None,
                            tool_call_id: None,
                            kind: None,
                        });
                    } else {
                        messages.extend(tool_messages);
                    }
                }
            }
        }

        messages
    }

    fn to_responses_input(request: &CompletionRequest) -> Vec<OpenAiResponsesInputItem> {
        let mut input = Vec::new();

        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            Self::push_responses_text_message(&mut input, "system", system.clone());
        }

        for message in &request.messages {
            Self::append_responses_items_for_message(&mut input, message);
        }

        input
    }

    fn push_responses_text_message(
        input: &mut Vec<OpenAiResponsesInputItem>,
        role: &str,
        text: String,
    ) {
        if text.trim().is_empty() {
            return;
        }

        input.push(OpenAiResponsesInputItem::Message(OpenAiInputMessage {
            role: role.to_owned(),
            content: vec![OpenAiInputContent {
                kind: "input_text".to_owned(),
                text,
            }],
        }));
    }

    fn append_responses_items_for_message(
        input: &mut Vec<OpenAiResponsesInputItem>,
        message: &Message,
    ) {
        let projected_parts = utils::project_session_parts(message);
        match message.role {
            Role::System => Self::push_responses_text_message(
                input,
                "system",
                session_text_lossy(message, projected_parts.as_slice()),
            ),
            Role::User => Self::push_responses_text_message(
                input,
                "user",
                session_text_lossy(message, projected_parts.as_slice()),
            ),
            Role::Assistant => {
                let mut text_chunks = Vec::new();
                if projected_parts.is_empty() {
                    text_chunks.push(message.as_text_lossy());
                } else {
                    for part in projected_parts {
                        match part {
                            utils::ProjectedSessionPart::Text { text } => text_chunks.push(text),
                            utils::ProjectedSessionPart::ImageUrl { url } => {
                                text_chunks.push(format!("[image:{url}]"));
                            }
                            utils::ProjectedSessionPart::ToolCall {
                                id,
                                name,
                                arguments_json,
                            } => {
                                if !id.trim().is_empty() && !name.trim().is_empty() {
                                    input.push(OpenAiResponsesInputItem::FunctionCall(
                                        OpenAiFunctionCallItem {
                                            kind: "function_call",
                                            call_id: id,
                                            name,
                                            arguments: arguments_json,
                                        },
                                    ));
                                }
                            }
                            utils::ProjectedSessionPart::ToolResult {
                                tool_call_id,
                                output_json,
                            } => {
                                if !tool_call_id.trim().is_empty() {
                                    input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                                        OpenAiFunctionCallOutputItem {
                                            kind: "function_call_output",
                                            call_id: tool_call_id,
                                            output: output_json,
                                        },
                                    ));
                                }
                            }
                        }
                    }
                }

                if !text_chunks.is_empty() {
                    Self::push_responses_text_message(input, "assistant", text_chunks.join(""));
                }
            }
            Role::Tool => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "user", message.as_text_lossy());
                } else {
                    let mut fallback_text = Vec::new();
                    let mut emitted_output = false;
                    for part in projected_parts {
                        match part {
                            utils::ProjectedSessionPart::ToolResult {
                                tool_call_id,
                                output_json,
                            } => {
                                if tool_call_id.trim().is_empty() {
                                    fallback_text.push(output_json);
                                    continue;
                                }
                                emitted_output = true;
                                input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                                    OpenAiFunctionCallOutputItem {
                                        kind: "function_call_output",
                                        call_id: tool_call_id,
                                        output: output_json,
                                    },
                                ));
                            }
                            utils::ProjectedSessionPart::Text { text } => fallback_text.push(text),
                            utils::ProjectedSessionPart::ImageUrl { url } => {
                                fallback_text.push(format!("[image:{url}]"));
                            }
                            utils::ProjectedSessionPart::ToolCall { name, .. } => {
                                fallback_text.push(format!("[tool_call:{name}]"));
                            }
                        }
                    }

                    if !emitted_output || !fallback_text.is_empty() {
                        Self::push_responses_text_message(input, "user", fallback_text.join(""));
                    }
                }
            }
        }
    }

    fn assistant_chat_content_and_tool_calls(
        message: &Message,
        projected_parts: &[utils::ProjectedSessionPart],
    ) -> (Option<serde_json::Value>, Vec<OpenAiChatToolCall>) {
        if projected_parts.is_empty() {
            return (
                Some(serde_json::Value::String(message.as_text_lossy())),
                Vec::new(),
            );
        }

        let mut text_chunks = Vec::new();
        let mut tool_calls = Vec::new();

        for part in projected_parts {
            match part {
                utils::ProjectedSessionPart::Text { text } => text_chunks.push(text.clone()),
                utils::ProjectedSessionPart::ImageUrl { url } => {
                    text_chunks.push(format!("[image:{url}]"));
                }
                utils::ProjectedSessionPart::ToolCall {
                    id,
                    name,
                    arguments_json,
                } => {
                    tool_calls.push(OpenAiChatToolCall {
                        index: None,
                        id: Some(id.clone()),
                        kind: Some("function".to_owned()),
                        function: Some(OpenAiChatFunctionCall {
                            name: Some(name.clone()),
                            arguments: Some(arguments_json.clone()),
                        }),
                    });
                }
                utils::ProjectedSessionPart::ToolResult { tool_call_id, .. } => {
                    text_chunks.push(format!("[tool_result:{tool_call_id}]"));
                }
            }
        }

        let content =
            (!text_chunks.is_empty()).then(|| serde_json::Value::String(text_chunks.join("")));
        (content, tool_calls)
    }

    fn tool_chat_messages(parts: &[utils::ProjectedSessionPart]) -> Vec<OpenAiChatMessage> {
        parts
            .iter()
            .filter_map(|part| match part {
                utils::ProjectedSessionPart::ToolResult {
                    tool_call_id,
                    output_json,
                } => {
                    if tool_call_id.trim().is_empty() {
                        None
                    } else {
                        Some(OpenAiChatMessage {
                            role: "tool".to_owned(),
                            content: Some(serde_json::Value::String(output_json.clone())),
                            tool_calls: None,
                            tool_call_id: Some(tool_call_id.clone()),
                            kind: None,
                        })
                    }
                }
                _ => None,
            })
            .collect()
    }

    fn parse_chat_tool_calls(
        calls: Option<&Vec<OpenAiChatToolCall>>,
    ) -> Result<Vec<CompletionToolCall>, AppError> {
        calls
            .into_iter()
            .flatten()
            .map(|c| {
                let id = utils::normalize_optional_text(c.id.clone()).ok_or_else(|| {
                    AppError::Provider(
                        "openai chat completion returned tool_call without id".to_owned(),
                    )
                })?;

                let function = c.function.as_ref().ok_or_else(|| {
                    AppError::Provider(
                        "openai chat completion returned tool_call without function".to_owned(),
                    )
                })?;

                let name =
                    utils::normalize_optional_text(function.name.clone()).ok_or_else(|| {
                        AppError::Provider(
                            "openai chat completion returned tool_call without function.name"
                                .to_owned(),
                        )
                    })?;

                Ok(CompletionToolCall::Function {
                    id,
                    name,
                    arguments_json: function.arguments.clone().unwrap_or_default(),
                })
            })
            .collect()
    }

    fn parse_responses_tool_calls(
        items: Option<&Vec<OpenAiOutputItem>>,
    ) -> Result<Vec<CompletionToolCall>, AppError> {
        items
            .into_iter()
            .flatten()
            .filter(|item| item.kind.as_deref() == Some("function_call"))
            .map(|item| {
                let id = utils::normalize_optional_text(item.call_id.clone())
                    .or_else(|| utils::normalize_optional_text(item.id.clone()))
                    .ok_or_else(|| {
                        AppError::Provider(
                            "openai responses payload returned function_call without id/call_id"
                                .to_owned(),
                        )
                    })?;

                let name = utils::normalize_optional_text(item.name.clone()).ok_or_else(|| {
                    AppError::Provider(
                        "openai responses payload returned function_call without name".to_owned(),
                    )
                })?;

                Ok(CompletionToolCall::Function {
                    id,
                    name,
                    arguments_json: item.arguments.clone().unwrap_or_default(),
                })
            })
            .collect()
    }

    async fn send_json<R>(
        &self,
        endpoint: String,
        body: Option<&impl Serialize>,
    ) -> Result<R, AppError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let mut request = self.apply_headers(
            self.client
                .post(endpoint)
                .bearer_auth(&self.api_key)
                .header(reqwest::header::CONTENT_TYPE, "application/json"),
        );

        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request.send().await?;
        utils::parse_json_response(PROVIDER_ID, response).await
    }

    fn apply_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        utils::apply_extra_headers(req, &self.extra_headers)
    }
}

#[async_trait]
impl ModelProvider for OpenAiProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let response = self
            .apply_headers(
                self.client
                    .get(self.model_endpoint())
                    .bearer_auth(&self.api_key),
            )
            .send()
            .await?;

        let payload: OpenAiModelListResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;
        Ok(payload
            .data
            .into_iter()
            .map(|m| ProviderModel {
                provider_id: PROVIDER_ID.to_owned(),
                id: m.id,
                display_name: None,
            })
            .collect())
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = if request.model.trim().is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        if !self.should_use_responses(model.as_str()) {
            return self.complete_with_chat_api(&request, model).await;
        }

        let input = Self::to_responses_input(&request);

        let body = OpenAiResponsesRequest {
            model: model.clone(),
            input,
            tools: Self::responses_tools(request.tools.as_slice()),
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            stream: false,
        };

        let response: OpenAiResponsesResponse =
            match self.send_json(self.responses_endpoint(), Some(&body)).await {
                Ok(payload) => payload,
                Err(AppError::HttpStatus { status, .. })
                    if Self::responses_endpoint_unsupported(status) =>
                {
                    return self.complete_with_chat_api(&request, model).await;
                }
                Err(err) => return Err(err),
            };

        let response_model = response.model.clone().unwrap_or(model);
        let text = Self::extract_text(&response);
        let finish_reason = CompletionFinishReason::from_provider(response.stop_reason.as_deref());
        let tool_calls = Self::parse_responses_tool_calls(response.output.as_ref())?;

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(
                "openai responses payload was empty without finish reason".to_owned(),
            ));
        }

        let usage = Self::map_usage(response.usage);

        Ok(CompletionResponse {
            provider_id: PROVIDER_ID.to_owned(),
            model: response_model,
            text,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: None,
        })
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = if request.model.trim().is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        if matches!(self.stream_mode, OpenAiStreamMode::RealtimeWebSocket) {
            return self.complete_stream_with_realtime_ws(&request, model).await;
        }

        if !self.should_use_responses(model.as_str()) {
            return self.complete_stream_with_chat_api(&request, model).await;
        }

        let input = Self::to_responses_input(&request);

        let body = OpenAiResponsesRequest {
            model: model.clone(),
            input,
            tools: Self::responses_tools(request.tools.as_slice()),
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            stream: true,
        };

        let response = self
            .apply_headers(
                self.client
                    .post(self.responses_endpoint())
                    .bearer_auth(&self.api_key)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            if Self::responses_endpoint_unsupported(response.status()) {
                return self.complete_stream_with_chat_api(&request, model).await;
            }
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

        let mut events = sse::json_events(response);
        let provider_id = PROVIDER_ID.to_owned();
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ResponsesToolState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut completed_emitted = false;

            while let Some(event) = events.next().await {
                let event = event?;

                if let Some(err) = utils::responses_stream_error(PROVIDER_ID, &event)? {
                    Err(err)?;
                }

                if let Some(delta) = utils::responses_text_delta(&event) {
                    stream_has_content = true;
                    yield CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                if let Some(tool_event) = utils::responses_tool_event(PROVIDER_ID, &event)? {
                    let key = tool_event.stream_key(PROVIDER_ID)?;

                    let state = pending_tool_calls.entry(key.clone()).or_default();
                    if let Some(id) = tool_event.id.clone() {
                        state.id = Some(id);
                    }
                    if let Some(name) = tool_event.name.clone() {
                        state.name = Some(name);
                    }

                    match tool_event.kind {
                        utils::ResponsesToolEventKind::Delta => {
                            if let Some(arguments_delta) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                state.arguments.push_str(arguments_delta.as_str());
                                stream_has_content = true;
                                yield CompletionStreamEvent::ToolCallDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    stream_key: key.clone(),
                                    id: state.id.clone(),
                                    name: state.name.clone(),
                                    arguments_delta,
                                };
                            }
                        }
                        utils::ResponsesToolEventKind::Added => {
                            if let Some(arguments_snapshot) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
                                {
                                    arguments_snapshot[state.arguments.len()..].to_owned()
                                } else {
                                    arguments_snapshot.clone()
                                };

                                if arguments_snapshot.starts_with(&state.arguments) {
                                    state.arguments.push_str(arguments_delta.as_str());
                                } else {
                                    state.arguments = arguments_snapshot;
                                }

                                if !arguments_delta.is_empty() {
                                    stream_has_content = true;
                                    yield CompletionStreamEvent::ToolCallDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        stream_key: key.clone(),
                                        id: state.id.clone(),
                                        name: state.name.clone(),
                                        arguments_delta,
                                    };
                                }
                            }
                        }
                        utils::ResponsesToolEventKind::Done => {
                            if let Some(arguments_snapshot) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
                                {
                                    arguments_snapshot[state.arguments.len()..].to_owned()
                                } else {
                                    arguments_snapshot.clone()
                                };

                                if arguments_snapshot.starts_with(&state.arguments) {
                                    state.arguments.push_str(arguments_delta.as_str());
                                } else {
                                    state.arguments = arguments_snapshot;
                                }

                                if !arguments_delta.is_empty() {
                                    stream_has_content = true;
                                    yield CompletionStreamEvent::ToolCallDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        stream_key: key.clone(),
                                        id: state.id.clone(),
                                        name: state.name.clone(),
                                        arguments_delta,
                                    };
                                }
                            }

                            pending_tool_calls.remove(key.as_str());
                        }
                    }
                }

                if let Some(raw_usage) = utils::responses_usage_value(&event) {
                    let usage = utils::parse_json_value::<OpenAiUsage>(
                        PROVIDER_ID,
                        "responses stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Some(
                        MessageUsage {
                            input_tokens: usage.input_tokens.unwrap_or_default(),
                            output_tokens: usage.output_tokens.unwrap_or_default(),
                            reasoning_tokens: usage
                                .output_tokens_details
                                .and_then(|d| d.reasoning_tokens)
                                .unwrap_or_default(),
                            cache_write_tokens: 0,
                            cache_read_tokens: usage
                                .input_tokens_details
                                .and_then(|d| d.cached_tokens)
                                .unwrap_or_default(),
                            total_cost: 0.0,
                        }
                        .into(),
                    );
                }

                if stream_finish_reason.is_none() {
                    stream_finish_reason = utils::responses_finish_reason(&event);
                }

                if utils::responses_is_completed(&event) {
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason: CompletionFinishReason::from_provider(
                            stream_finish_reason.as_deref(),
                        ),
                        usage: stream_usage.clone(),
                        provider_metadata: None,
                    };
                    completed_emitted = true;
                    break;
                }
            }

            if !completed_emitted
                && (stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some())
            {
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage,
                    provider_metadata: None,
                };
            }
        };

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesRequest {
    model: String,
    input: Vec<OpenAiResponsesInputItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiResponsesTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesTool {
    #[serde(rename = "type")]
    kind: &'static str,
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct OpenAiInputMessage {
    role: String,
    content: Vec<OpenAiInputContent>,
}

#[derive(Debug, Serialize)]
struct OpenAiInputContent {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAiResponsesInputItem {
    Message(OpenAiInputMessage),
    FunctionCall(OpenAiFunctionCallItem),
    FunctionCallOutput(OpenAiFunctionCallOutputItem),
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionCallItem {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionCallOutputItem {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: String,
    output: String,
}

#[derive(Debug, Default)]
struct ResponsesToolState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Default)]
struct ChatToolCallState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelListResponse {
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Option<Vec<OpenAiOutputItem>>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputItem {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    content: Option<Vec<OpenAiOutputContent>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputContent {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    output_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    input_tokens_details: Option<OpenAiInputTokenDetails>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputTokenDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OpenAiInputTokenDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
struct OpenAiChatCompletionRequest {
    model: String,
    messages: Vec<OpenAiChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiChatToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<OpenAiChatStreamOptions>,
}

#[derive(Debug, Serialize)]
struct OpenAiChatStreamOptions {
    #[serde(rename = "include_usage")]
    include_usage: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiChatMessage {
    role: String,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiChatToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiChatToolCall {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default)]
    function: Option<OpenAiChatFunctionCall>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiChatFunctionCall {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenAiChatToolDefinition {
    #[serde(rename = "type")]
    kind: String,
    function: OpenAiChatFunctionDefinition,
}

#[derive(Debug, Serialize)]
struct OpenAiChatFunctionDefinition {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatCompletionResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<OpenAiChatChoice>,
    #[serde(default)]
    usage: Option<OpenAiChatUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatChoice {
    #[serde(default)]
    message: Option<OpenAiChatMessage>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}

fn extract_chat_content_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn build_realtime_input_text(messages: &[Message]) -> Option<String> {
    let normalized = messages
        .iter()
        .filter_map(|message| {
            let text = utils::project_session_text_lossy(message);
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some((message.role, trimmed.to_owned()))
            }
        })
        .collect::<Vec<_>>();

    if normalized.is_empty() {
        return None;
    }

    if normalized.len() == 1 && matches!(normalized[0].0, Role::User) {
        return Some(normalized[0].1.clone());
    }

    Some(
        normalized
            .into_iter()
            .map(|(role, text)| format!("{role}: {text}"))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

fn session_text_lossy(
    message: &Message,
    projected_parts: &[utils::ProjectedSessionPart],
) -> String {
    if projected_parts.is_empty() {
        message.as_text_lossy()
    } else {
        utils::projected_parts_text_lossy(projected_parts)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use futures_util::StreamExt;
    use tokio::net::TcpListener;

    use crate::message::Message;
    use crate::tool::{ToolBehavior, ToolDefinition};

    fn sample_tool_definition() -> ToolDefinition {
        ToolDefinition::plugin(
            "project_search",
            "Search project files.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
            ToolBehavior::ReadOnly,
            "fixture",
        )
    }

    #[tokio::test]
    async fn complete_falls_back_to_chat_when_responses_unsupported() {
        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": { "message": "not found" }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let _chat = server
            .mock("POST", "/chat/completions")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-5",
                    "choices": [{
                        "finish_reason": "stop",
                        "message": { "role": "assistant", "content": "fallback chat response" }
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider =
            OpenAiProvider::new(reqwest::Client::new(), "sk-test", server.url(), "gpt-5");

        let response = provider
            .complete(CompletionRequest {
                model: "gpt-5".to_owned(),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("responses 404 should fall back to chat");

        assert_eq!(response.text, "fallback chat response");
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::Stop)
        ));
    }

    #[tokio::test]
    async fn complete_chat_supports_legacy_choice_text_payload() {
        let mut server = mockito::Server::new_async().await;
        let _chat = server
            .mock("POST", "/chat/completions")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "text-davinci-003",
                    "choices": [{
                        "text": "legacy chat-compatible text",
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiProvider::new(
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "text-davinci-003",
        );

        let response = provider
            .complete(CompletionRequest {
                model: "text-davinci-003".to_owned(),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("legacy text payload should parse");

        assert_eq!(response.text, "legacy chat-compatible text");
    }

    #[tokio::test]
    async fn complete_responses_parses_function_tool_calls() {
        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-5",
                    "output_text": "",
                    "stop_reason": "tool_calls",
                    "output": [{
                        "type": "function_call",
                        "call_id": "call_1",
                        "name": "search",
                        "arguments": "{\"q\":\"rust\"}"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider =
            OpenAiProvider::new(reqwest::Client::new(), "sk-test", server.url(), "gpt-5");

        let response = provider
            .complete(CompletionRequest {
                model: "gpt-5".to_owned(),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("responses payload should parse tool calls");

        assert_eq!(response.tool_calls.len(), 1);
        match &response.tool_calls[0] {
            CompletionToolCall::Function {
                id,
                name,
                arguments_json,
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "search");
                assert_eq!(arguments_json, "{\"q\":\"rust\"}");
            }
        }
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::ToolCalls)
        ));
    }

    #[tokio::test]
    async fn complete_stream_chat_preserves_usage_emitted_after_finish_reason() {
        let mut server = mockito::Server::new_async().await;
        let _chat = server
            .mock("POST", "/chat/completions")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
                "data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2}}\n\n",
                "data: [DONE]\n\n"
            ))
            .create_async()
            .await;

        let mut provider = OpenAiProvider::new(
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4.1-mini",
        );
        provider.api_mode = OpenAiApiMode::Chat;

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: "gpt-4.1-mini".to_owned(),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("chat stream should start");

        let mut saw_completed = false;
        while let Some(item) = stream.next().await {
            if let CompletionStreamEvent::Completed {
                finish_reason,
                usage,
                ..
            } = item.expect("event should parse")
            {
                assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                let usage = usage.expect("usage should be present");
                assert_eq!(usage.input_tokens, 4);
                assert_eq!(usage.output_tokens, 2);
                saw_completed = true;
            }
        }

        assert!(saw_completed);
    }

    #[tokio::test]
    async fn complete_responses_sends_tool_result_as_function_call_output() {
        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .match_body(mockito::Matcher::Regex(
                "\\\"type\\\":\\\"function_call_output\\\"".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"call_id\\\":\\\"call_1\\\"".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-5",
                    "output_text": "ok",
                    "stop_reason": "stop"
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider =
            OpenAiProvider::new(reqwest::Client::new(), "sk-test", server.url(), "gpt-5");

        let response = provider
            .complete(CompletionRequest {
                model: "gpt-5".to_owned(),
                system: None,
                messages: vec![Message::prompt_tool_result("call_1", "{\"ok\":true}")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("responses request should include function_call_output");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test]
    async fn complete_stream_responses_emits_tool_call_delta() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"search\",\"arguments\":\"\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"part1\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"part2\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Done\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"stop_reason\":\"tool_calls\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n",
            "data: [DONE]\n\n"
        );

        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider =
            OpenAiProvider::new(reqwest::Client::new(), "sk-test", server.url(), "gpt-5");

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: "gpt-5".to_owned(),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
            })
            .await
            .expect("responses stream should start");

        let mut text = String::new();
        let mut tool = String::new();
        let mut completed = false;

        while let Some(item) = stream.next().await {
            match item.expect("stream item should parse") {
                CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(delta.as_str()),
                CompletionStreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                    ..
                } => {
                    assert_eq!(id.as_deref(), Some("call_1"));
                    assert_eq!(name.as_deref(), Some("search"));
                    tool.push_str(arguments_delta.as_str());
                }
                CompletionStreamEvent::Completed {
                    finish_reason,
                    usage,
                    ..
                } => {
                    assert!(matches!(
                        finish_reason,
                        Some(CompletionFinishReason::ToolCalls)
                    ));
                    let usage = usage.expect("usage should be present");
                    assert_eq!(usage.input_tokens, 3);
                    assert_eq!(usage.output_tokens, 2);
                    completed = true;
                }
            }
        }

        assert_eq!(text, "Done");
        assert_eq!(tool, "part1part2");
        assert!(completed);
    }

    #[test]
    fn realtime_ws_endpoint_uses_ws_scheme_and_model_query() {
        let provider = OpenAiProvider::new(
            reqwest::Client::new(),
            "sk-test",
            "https://api.openai.com/v1",
            "gpt-4o-realtime-preview",
        )
        .with_stream_mode(OpenAiStreamMode::RealtimeWebSocket);

        let endpoint = provider
            .realtime_ws_endpoint("gpt-4o-realtime-preview")
            .expect("endpoint should derive");

        assert_eq!(endpoint.scheme(), "wss");
        assert_eq!(endpoint.path(), "/v1/realtime");
        assert_eq!(
            endpoint
                .query_pairs()
                .find(|(key, _)| key == "model")
                .map(|(_, value)| value.into_owned()),
            Some("gpt-4o-realtime-preview".to_owned())
        );
    }

    #[tokio::test]
    async fn complete_stream_realtime_ws_emits_text_tool_delta_and_completed() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let auth_header = Arc::new(Mutex::new(None::<String>));
        let auth_header_server = auth_header.clone();

        let server = tokio::spawn(async move {
            let (tcp, _) = listener
                .accept()
                .await
                .expect("connection should be accepted");

            let ws_stream = tokio_tungstenite::accept_hdr_async(
                tcp,
                move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                      response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    let value = request
                        .headers()
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(ToOwned::to_owned);
                    *auth_header_server
                        .lock()
                        .expect("auth header lock should succeed") = value;
                    Ok(response)
                },
            )
            .await
            .expect("websocket upgrade should succeed");

            let (mut writer, mut reader) = ws_stream.split();

            let mut saw_response_create = false;
            while let Some(message) = reader.next().await {
                let message = message.expect("request message should parse");
                let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
                    continue;
                };
                let value: serde_json::Value =
                    serde_json::from_str(text.as_str()).expect("request event should be json");
                if value.get("type").and_then(|v| v.as_str()) == Some("response.create") {
                    assert_eq!(value["response"]["tools"][0]["name"], "project_search");
                    assert_eq!(
                        value["response"]["tools"][0]["parameters"]["properties"]["query"]["type"],
                        "string"
                    );
                    saw_response_create = true;
                    break;
                }
            }

            assert!(saw_response_create);

            writer
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "type": "response.output_text.delta",
                        "delta": "Hel"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("text delta should send");

            writer
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "type": "response.function_call_arguments.delta",
                        "output_index": 0,
                        "call_id": "call_1",
                        "name": "search",
                        "delta": "{"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("tool delta should send");

            writer
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "type": "response.function_call_arguments.done",
                        "output_index": 0,
                        "call_id": "call_1",
                        "name": "search",
                        "arguments": "{\"q\":\"rust\"}"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("tool done should send");

            writer
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "type": "response.done",
                        "response": {
                            "status": "completed",
                            "usage": {
                                "input_tokens": 5,
                                "output_tokens": 3
                            }
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("response done should send");
        });

        let provider = OpenAiProvider::new(
            reqwest::Client::new(),
            "sk-test",
            format!("http://{addr}"),
            "gpt-4o-realtime-preview",
        )
        .with_stream_mode(OpenAiStreamMode::RealtimeWebSocket)
        .with_realtime_ws_url(Some(format!("ws://{addr}/realtime")));

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: "gpt-4o-realtime-preview".to_owned(),
                system: Some("you are helpful".to_owned()),
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: vec![sample_tool_definition()],
                temperature: Some(0.2),
                max_output_tokens: Some(64),
            })
            .await
            .expect("stream should start");

        let mut saw_text = false;
        let mut saw_tool = false;
        let mut saw_completed = false;

        while let Some(item) = stream.next().await {
            match item.expect("event should parse") {
                CompletionStreamEvent::TextDelta { delta, .. } => {
                    if delta == "Hel" {
                        saw_text = true;
                    }
                }
                CompletionStreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                    ..
                } => {
                    if id.as_deref() == Some("call_1")
                        && name.as_deref() == Some("search")
                        && !arguments_delta.is_empty()
                    {
                        saw_tool = true;
                    }
                }
                CompletionStreamEvent::Completed {
                    finish_reason,
                    usage,
                    ..
                } => {
                    assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                    let usage = usage.expect("usage should be present");
                    assert_eq!(usage.input_tokens, 5);
                    assert_eq!(usage.output_tokens, 3);
                    saw_completed = true;
                }
            }
        }

        assert!(saw_text);
        assert!(saw_tool);
        assert!(saw_completed);

        server.await.expect("server task should finish");
        assert_eq!(
            auth_header
                .lock()
                .expect("auth header lock should succeed")
                .as_deref(),
            Some("Bearer sk-test")
        );
    }
}
