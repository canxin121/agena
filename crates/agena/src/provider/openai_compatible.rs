use std::collections::HashMap;

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::AppError,
    message::{AttachmentItem, AttachmentKind, AttachmentSource, Message, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ManagedCredential, ModelProvider, ProviderModel,
        StreamResumePolicy, prompt_cache, should_retry_credential, sse, utils,
    },
    role::Role,
};

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    id: String,
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    auth_header: String,
    auth_scheme: Option<String>,
    extra_headers: HashMap<String, String>,
    stream_mode: OpenAiCompatibleStreamMode,
    realtime_ws_url: Option<String>,
    top_level_prompt_cache_override: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiCompatibleStreamMode {
    Sse,
    RealtimeWebSocket,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_managed(
            id,
            client,
            ManagedCredential::static_value("openai-compatible api key", api_key.into()),
            base_url,
            default_model,
        )
    }

    pub fn new_managed(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            client,
            api_key,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            auth_header: "authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            extra_headers: HashMap::new(),
            stream_mode: OpenAiCompatibleStreamMode::Sse,
            realtime_ws_url: None,
            top_level_prompt_cache_override: None,
        }
    }

    pub fn with_auth_header(
        mut self,
        header: impl Into<String>,
        scheme: Option<impl Into<String>>,
    ) -> Self {
        self.auth_header = header.into();
        self.auth_scheme = scheme.map(|v| v.into());
        self
    }

    pub fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.extra_headers = headers;
        self
    }

    pub fn with_stream_mode(mut self, mode: OpenAiCompatibleStreamMode) -> Self {
        self.stream_mode = mode;
        self
    }

    pub fn with_realtime_ws_url(mut self, ws_url: Option<String>) -> Self {
        self.realtime_ws_url = ws_url.and_then(|v| utils::normalize_optional_text(Some(v)));
        self
    }

    pub fn with_top_level_prompt_cache(mut self, enabled: bool) -> Self {
        self.top_level_prompt_cache_override = Some(enabled);
        self
    }

    fn models_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn completions_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn supports_top_level_prompt_cache(&self) -> bool {
        if let Some(enabled) = self.top_level_prompt_cache_override {
            return enabled;
        }
        matches!(
            self.id.as_str(),
            "openrouter" | "zenmux" | "kilo" | "opencode" | "opencode-go"
        )
    }

    fn stream_mode_key(&self) -> &'static str {
        match self.stream_mode {
            OpenAiCompatibleStreamMode::Sse => "sse",
            OpenAiCompatibleStreamMode::RealtimeWebSocket => "realtime_websocket",
        }
    }

    fn realtime_ws_endpoint(&self, model: &str) -> Result<url::Url, AppError> {
        let mut endpoint = if let Some(ws_url) = self.realtime_ws_url.as_ref() {
            url::Url::parse(ws_url).map_err(|err| {
                AppError::Config(format!(
                    "{} realtime websocket url is invalid: {err}",
                    self.id
                ))
            })?
        } else {
            let mut url = url::Url::parse(self.base_url.as_str()).map_err(|err| {
                AppError::Config(format!("{} base url is invalid: {err}", self.id))
            })?;
            let realtime_path = format!("{}/realtime", url.path().trim_end_matches('/'));
            url.set_path(realtime_path.as_str());
            url
        };

        match endpoint.scheme() {
            "http" => endpoint.set_scheme("ws").map_err(|_| {
                AppError::Config(format!("{} realtime websocket url is invalid", self.id))
            })?,
            "https" => endpoint.set_scheme("wss").map_err(|_| {
                AppError::Config(format!("{} realtime websocket url is invalid", self.id))
            })?,
            "ws" | "wss" => {}
            other => {
                return Err(AppError::Config(format!(
                    "{} realtime websocket url has unsupported scheme `{other}`",
                    self.id
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
        api_key: &str,
        session_affinity: Option<&str>,
    ) -> Result<http::Request<()>, AppError> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let mut request = endpoint.as_str().into_client_request().map_err(|err| {
            AppError::Config(format!(
                "{} realtime websocket handshake request invalid: {err}",
                self.id
            ))
        })?;

        let auth_header_name = http::header::HeaderName::from_bytes(self.auth_header.as_bytes())
            .map_err(|err| {
                AppError::Config(format!("{} auth header name is invalid: {err}", self.id))
            })?;
        let auth_value = utils::auth_header_value(self.auth_scheme.as_deref(), api_key);
        let auth_header_value =
            http::header::HeaderValue::from_str(auth_value.as_str()).map_err(|err| {
                AppError::Config(format!("{} auth header value is invalid: {err}", self.id))
            })?;
        request
            .headers_mut()
            .insert(auth_header_name, auth_header_value);

        if let Some(session_affinity) = session_affinity.filter(|value| !value.trim().is_empty()) {
            request.headers_mut().insert(
                http::header::HeaderName::from_static("x-session-affinity"),
                http::header::HeaderValue::from_str(session_affinity).map_err(|err| {
                    AppError::Config(format!(
                        "{} session affinity header value is invalid: {err}",
                        self.id
                    ))
                })?,
            );
        }

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
                        "{} extra header name `{key}` is invalid: {err}",
                        self.id
                    ))
                })?;
            let header_value =
                http::header::HeaderValue::from_str(value.as_str()).map_err(|err| {
                    AppError::Config(format!(
                        "{} extra header `{key}` value is invalid: {err}",
                        self.id
                    ))
                })?;
            request.headers_mut().insert(header_name, header_value);
        }

        Ok(request)
    }

    fn apply_auth_headers(
        &self,
        req: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        self.apply_request_headers(req, api_key, None)
    }

    fn apply_request_headers(
        &self,
        mut req: reqwest::RequestBuilder,
        api_key: &str,
        session_affinity: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let auth_value = utils::auth_header_value(self.auth_scheme.as_deref(), api_key);

        req = req.header(self.auth_header.as_str(), auth_value);
        if let Some(session_affinity) = session_affinity.filter(|value| !value.trim().is_empty()) {
            req = req.header("x-session-affinity", session_affinity);
        }
        utils::apply_request_headers(self.id.as_str(), req, &self.extra_headers)
    }

    fn parse_models(&self, payload: Value) -> Result<Vec<ProviderModel>, AppError> {
        let parsed: OpenAiCompatibleModelList =
            utils::parse_json_value(self.id.as_str(), "models list", payload)?;
        let models = match parsed {
            OpenAiCompatibleModelList::Object { data } => data,
            OpenAiCompatibleModelList::Array(data) => data,
        };

        Ok(models
            .into_iter()
            .map(|model| {
                let mut entry = ProviderModel::new(self.id.clone(), model.id);
                let capabilities = self.model_capabilities(&entry.id);
                entry = entry.with_capabilities(capabilities);
                entry.display_name = model.display_name.or(model.name);
                entry
            })
            .collect())
    }

    fn chat_tools(tools: &[crate::tool::ToolDefinition]) -> Vec<ChatToolDefinition> {
        tools
            .iter()
            .map(|tool| ChatToolDefinition {
                kind: "function".to_owned(),
                function: ChatFunctionDefinition {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone(),
                },
            })
            .collect()
    }

    fn realtime_tools(tools: &[crate::tool::ToolDefinition]) -> Vec<RealtimeToolDefinition> {
        tools
            .iter()
            .map(|tool| RealtimeToolDefinition {
                kind: "function".to_owned(),
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            })
            .collect()
    }

    fn convert_messages(system: Option<String>, messages: Vec<Message>) -> Vec<ChatMessage> {
        let mut result = Vec::new();

        if let Some(system) = system.filter(|s| !s.trim().is_empty()) {
            result.push(ChatMessage {
                role: "system".to_owned(),
                content: Some(Value::String(system)),
                tool_call_id: None,
                tool_calls: None,
            });
        }

        for message in messages {
            let projected_parts = utils::project_session_parts(&message);
            match message.role {
                Role::System => {
                    result.push(ChatMessage {
                        role: "system".to_owned(),
                        content: Some(Value::String(session_text_lossy(
                            &message,
                            &projected_parts,
                        ))),
                        tool_call_id: None,
                        tool_calls: None,
                    });
                }
                Role::User => {
                    result.push(ChatMessage {
                        role: "user".to_owned(),
                        content: Some(provider_message_to_openai_value(&message, &projected_parts)),
                        tool_call_id: None,
                        tool_calls: None,
                    });
                }
                Role::Assistant => {
                    let (content, tool_calls) =
                        assistant_content_and_tool_calls(&message, &projected_parts);
                    result.push(ChatMessage {
                        role: "assistant".to_owned(),
                        content,
                        tool_call_id: None,
                        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
                    });
                }
                Role::Tool => {
                    let ordered_messages =
                        ordered_tool_and_user_messages_from_parts(projected_parts.as_slice());
                    if ordered_messages.is_empty() {
                        result.push(ChatMessage {
                            role: "tool".to_owned(),
                            content: Some(Value::String(session_text_lossy(
                                &message,
                                &projected_parts,
                            ))),
                            tool_call_id: Some("tool".to_owned()),
                            tool_calls: None,
                        });
                    } else {
                        result.extend(ordered_messages);
                    }
                }
            }
        }

        result
    }

    fn parse_completion(
        &self,
        payload: ChatCompletionResponse,
    ) -> Result<CompletionResponse, AppError> {
        let text = payload
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.content.as_ref())
            .map(extract_text_from_content)
            .or_else(|| {
                payload
                    .choices
                    .first()
                    .and_then(|c| c.delta.as_ref())
                    .and_then(|d| d.content.as_ref())
                    .map(extract_text_from_content)
            })
            .or_else(|| payload.choices.first().and_then(|c| c.text.clone()))
            .unwrap_or_default();

        let finish_reason = CompletionFinishReason::from_provider(
            payload
                .choices
                .first()
                .and_then(|c| c.finish_reason.as_deref()),
        );

        let tool_calls = parse_tool_calls(
            self.id.as_str(),
            payload
                .choices
                .first()
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.tool_calls.as_ref()),
        )?;

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(format!(
                "{} returned empty completion payload without finish reason",
                self.id
            )));
        }

        let usage = payload.usage.map(usage_to_completion_usage);
        let response_id = payload.id.clone();

        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id.clone()),
            model: ModelId::new(
                payload
                    .model
                    .unwrap_or_else(|| self.default_model.to_string()),
            ),
            text,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: response_id_metadata(response_id),
        })
    }

    async fn complete_stream_with_realtime_ws(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = request.model.clone();
        let prompt_cache_key = request.prompt_cache_key.clone();

        let ws_endpoint = self.realtime_ws_endpoint(model.as_str())?;
        let api_key = self.api_key.resolve().await?;
        let handshake = self.realtime_handshake_request(
            &ws_endpoint,
            api_key.as_str(),
            prompt_cache_key.as_deref(),
        )?;
        let (ws_stream, _) = tokio_tungstenite::connect_async(handshake)
            .await
            .map_err(|err| {
                AppError::Provider(format!(
                    "{} realtime websocket connect failed: {err}",
                    self.id
                ))
            })?;

        let provider_id = ProviderId::new(self.id.clone());
        let model_name = model;
        let input_text = build_realtime_input_text(request.messages.as_slice());
        let response_tools = (!request.tools.is_empty()).then(|| {
            serde_json::to_value(Self::realtime_tools(request.tools.as_slice()))
                .expect("realtime tool definitions should serialize")
        });
        let system = request.system;
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
                            "{} realtime websocket send session.update failed: {err}",
                            provider_id
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
                            "{} realtime websocket send conversation.item.create failed: {err}",
                            provider_id
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
                        "{} realtime websocket send response.create failed: {err}",
                        provider_id
                    ))
                })?;

            let mut pending_tool_calls: std::collections::BTreeMap<String, ToolCallState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut completed_emitted = false;
            let mut response_id: Option<String> = None;

            while let Some(message) = ws_reader.next().await {
                let message = message.map_err(|err| {
                    AppError::Provider(format!(
                        "{} realtime websocket receive failed: {err}",
                        provider_id
                    ))
                })?;

                let payload = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        String::from_utf8(bytes.to_vec()).map_err(|err| {
                            AppError::Provider(format!(
                                "{} realtime websocket binary frame is not utf-8: {err}",
                                provider_id
                            ))
                        })?
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    tokio_tungstenite::tungstenite::Message::Ping(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Frame(_) => continue,
                };

                let event: Value = serde_json::from_str(payload.as_str()).map_err(|err| {
                    AppError::Provider(format!(
                        "{} realtime websocket event decode failed: {err}",
                        provider_id
                    ))
                })?;

                if let Some(err) = utils::responses_stream_error(provider_id.as_str(), &event)? {
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

                if let Some(tool_event) = utils::responses_tool_event(provider_id.as_str(), &event)? {
                    let key = tool_event.stream_key(provider_id.as_str())?;

                    let state = pending_tool_calls.entry(key.clone()).or_default();
                    if let Some(id) = tool_event.id.clone() {
                        state.id = Some(id);
                    }
                    if let Some(name) = tool_event.name.clone() {
                        state.name = Some(name);
                    }

                    match tool_event.kind {
                        utils::ResponsesToolEventKind::Delta => {
                            if let Some(arguments_delta) = tool_event.arguments.filter(|s| !s.is_empty()) {
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
                            if let Some(arguments_snapshot) = tool_event.arguments.filter(|s| !s.is_empty()) {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments) {
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
                            if let Some(arguments_snapshot) = tool_event.arguments.filter(|s| !s.is_empty()) {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments) {
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
                    let usage = utils::parse_json_value::<ChatUsage>(
                        provider_id.as_str(),
                        "realtime stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Some(usage_to_completion_usage(usage));
                }

                if stream_finish_reason.is_none() {
                    stream_finish_reason = utils::responses_finish_reason(&event);
                }

                if let Some(next_response_id) = utils::responses_response_id(&event) {
                    response_id = Some(next_response_id);
                }

                if utils::responses_is_completed(&event) {
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason: CompletionFinishReason::from_provider(
                            stream_finish_reason.as_deref(),
                        ),
                        usage: stream_usage.clone(),
                        provider_metadata: response_id_metadata(response_id.clone()),
                    };
                    completed_emitted = true;
                    break;
                }
            }

            let _ = ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Close(None))
                .await;

            if !completed_emitted && (stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some()) {
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage,
                    provider_metadata: response_id_metadata(response_id),
                };
            }
        };

        Ok(Box::pin(stream))
    }

    async fn send_request<F>(&self, mut build: F) -> Result<reqwest::Response, AppError>
    where
        F: FnMut(&str) -> reqwest::RequestBuilder,
    {
        let mut force_refresh = false;

        loop {
            let api_key = if force_refresh {
                self.api_key.force_refresh().await?
            } else {
                self.api_key.resolve().await?
            };

            let response = build(api_key.as_str()).send().await?;
            if !force_refresh && should_retry_credential(response.status()) {
                force_refresh = true;
                continue;
            }

            return Ok(response);
        }
    }
}

fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    response_id.map(|response_id| serde_json::json!({ "response_id": response_id }))
}

fn usage_to_completion_usage(usage: ChatUsage) -> CompletionUsage {
    MessageUsage {
        input_tokens: usage.prompt_tokens.unwrap_or_default(),
        output_tokens: usage.completion_tokens.unwrap_or_default(),
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
    .into()
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

fn provider_message_to_openai_value(
    message: &Message,
    parts: &[utils::ProjectedSessionPart],
) -> Value {
    if parts.is_empty() {
        return Value::String(message.as_text_lossy());
    }

    projected_parts_to_openai_value(parts)
}

fn attachment_upload_name(item: &AttachmentItem) -> String {
    utils::attachment_filename(item)
        .map(str::to_owned)
        .unwrap_or_else(|| item.summary_label())
}

fn attachment_file_content_value(item: &AttachmentItem) -> Option<Value> {
    let filename = attachment_upload_name(item);
    match &item.source {
        AttachmentSource::Base64 { .. } | AttachmentSource::DataUrl { .. } => {
            utils::attachment_data_url(item).map(|file_data| {
                serde_json::json!({
                    "type": "file",
                    "file": {
                        "file_data": file_data,
                        "filename": filename,
                    }
                })
            })
        }
        AttachmentSource::FileId { file_id } => {
            let file_id = file_id.trim();
            (!file_id.is_empty()).then(|| {
                serde_json::json!({
                    "type": "file",
                    "file": {
                        "file_id": file_id,
                        "filename": filename,
                    }
                })
            })
        }
        AttachmentSource::Url { .. } | AttachmentSource::LocalPath { .. } => None,
    }
}

fn attachment_content_value(item: &AttachmentItem) -> Value {
    match item.kind {
        AttachmentKind::Image => utils::attachment_media_url(item)
            .map(|url| {
                serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": url }
                })
            })
            .unwrap_or_else(|| {
                serde_json::json!({
                    "type": "text",
                    "text": utils::attachment_hint_text(item),
                })
            }),
        AttachmentKind::Audio
        | AttachmentKind::Video
        | AttachmentKind::Pdf
        | AttachmentKind::File => attachment_file_content_value(item).unwrap_or_else(|| {
            serde_json::json!({
                "type": "text",
                "text": utils::attachment_hint_text(item),
            })
        }),
    }
}

fn projected_parts_to_openai_value(parts: &[utils::ProjectedSessionPart]) -> Value {
    let items = parts
        .iter()
        .map(|part| match part {
            utils::ProjectedSessionPart::Text { text } => {
                serde_json::json!({ "type": "text", "text": text })
            }
            utils::ProjectedSessionPart::Attachment { item } => attachment_content_value(item),
            utils::ProjectedSessionPart::ToolCall { name, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_call:{name}]") })
            }
            utils::ProjectedSessionPart::ToolResult { tool_call_id, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_result:{tool_call_id}]") })
            }
        })
        .collect::<Vec<_>>();
    Value::Array(items)
}

fn assistant_content_and_tool_calls(
    message: &Message,
    parts: &[utils::ProjectedSessionPart],
) -> (Option<Value>, Vec<ChatToolCallRequest>) {
    if parts.is_empty() {
        return (Some(Value::String(message.as_text_lossy())), Vec::new());
    }

    let mut text_chunks = Vec::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        match part {
            utils::ProjectedSessionPart::Text { text } => text_chunks.push(text.clone()),
            utils::ProjectedSessionPart::ToolCall {
                id,
                name,
                arguments_json,
            } => {
                tool_calls.push(ChatToolCallRequest {
                    kind: "function".to_owned(),
                    id: id.clone(),
                    function: ChatFunctionCallRequest {
                        name: name.clone(),
                        arguments: arguments_json.clone(),
                    },
                });
            }
            utils::ProjectedSessionPart::Attachment { item } => {
                text_chunks.push(utils::attachment_hint_text(item));
            }
            utils::ProjectedSessionPart::ToolResult { tool_call_id, .. } => {
                text_chunks.push(format!("[tool_result:{tool_call_id}]"));
            }
        }
    }
    let content = (!text_chunks.is_empty()).then(|| Value::String(text_chunks.join("")));
    (content, tool_calls)
}

fn ordered_tool_and_user_messages_from_parts(
    parts: &[utils::ProjectedSessionPart],
) -> Vec<ChatMessage> {
    let has_tool_message = parts.iter().any(|part| {
        matches!(
            part,
            utils::ProjectedSessionPart::ToolResult { tool_call_id, .. }
                if !tool_call_id.trim().is_empty()
        )
    });
    if !has_tool_message {
        return Vec::new();
    }

    let mut messages = Vec::new();
    let mut buffered_parts = Vec::new();

    for part in parts {
        match part {
            utils::ProjectedSessionPart::ToolResult {
                tool_call_id,
                output_json,
            } if !tool_call_id.trim().is_empty() => {
                if !buffered_parts.is_empty() {
                    messages.push(ChatMessage {
                        role: "user".to_owned(),
                        content: Some(projected_parts_to_openai_value(buffered_parts.as_slice())),
                        tool_call_id: None,
                        tool_calls: None,
                    });
                    buffered_parts.clear();
                }

                messages.push(ChatMessage {
                    role: "tool".to_owned(),
                    content: Some(Value::String(output_json.clone())),
                    tool_call_id: Some(tool_call_id.clone()),
                    tool_calls: None,
                });
            }
            utils::ProjectedSessionPart::ToolResult { output_json, .. } => {
                buffered_parts.push(utils::ProjectedSessionPart::Text {
                    text: output_json.clone(),
                });
            }
            other => buffered_parts.push(other.clone()),
        }
    }

    if !buffered_parts.is_empty() {
        messages.push(ChatMessage {
            role: "user".to_owned(),
            content: Some(projected_parts_to_openai_value(buffered_parts.as_slice())),
            tool_call_id: None,
            tool_calls: None,
        });
    }

    messages
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

fn parse_tool_calls(
    provider_id: &str,
    value: Option<&Vec<ChatToolCall>>,
) -> Result<Vec<CompletionToolCall>, AppError> {
    value
        .into_iter()
        .flatten()
        .map(|item| {
            let id = utils::normalize_optional_text(item.id.clone()).ok_or_else(|| {
                AppError::Provider(format!(
                    "{provider_id} returned tool_call without id in completion response"
                ))
            })?;

            let function = item.function.as_ref().ok_or_else(|| {
                AppError::Provider(format!(
                    "{provider_id} returned tool_call without function payload"
                ))
            })?;

            let name = utils::normalize_optional_text(function.name.clone()).ok_or_else(|| {
                AppError::Provider(format!(
                    "{provider_id} returned tool_call without function.name"
                ))
            })?;

            Ok(CompletionToolCall::Function {
                id,
                name,
                arguments_json: function.arguments.clone().unwrap_or_default(),
            })
        })
        .collect()
}

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn model_capabilities(&self, model: &ModelId) -> crate::provider::ModelCapabilities {
        crate::provider::default_capability_registry().capabilities_for_family(
            crate::provider::CapabilityFamily::OpenAiCompatible,
            model.as_str(),
        )
    }

    fn model_metadata(&self, model: &ModelId) -> crate::provider::ModelMetadata {
        crate::provider::default_model_metadata_registry().metadata_for_family(
            crate::provider::CapabilityFamily::OpenAiCompatible,
            model.as_str(),
        )
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        Some(
            crate::provider::PromptCacheShape::new(self.id.as_str())
                .with_string("auth_scope", self.api_key.prompt_cache_scope())
                .with_string("base_url", self.base_url.as_str())
                .with_string("auth_header", self.auth_header.as_str())
                .with_optional_string("auth_scheme", self.auth_scheme.as_deref())
                .with_string("stream_mode", self.stream_mode_key())
                .with_optional_string("realtime_ws_url", self.realtime_ws_url.as_deref())
                .with_bool(
                    "supports_top_level_prompt_cache",
                    self.supports_top_level_prompt_cache(),
                )
                .with_json(
                    "extra_headers",
                    &utils::prompt_cache_header_entries(&self.extra_headers),
                ),
        )
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let response = self
            .send_request(|api_key| {
                self.apply_auth_headers(self.client.get(self.models_endpoint()), api_key)
            })
            .await?;
        let payload: Value = utils::parse_json_response(self.id.as_str(), response).await?;
        self.parse_models(payload)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = request.model.clone();
        let prompt_cache_key = request.prompt_cache_key.clone();

        let tools = (!request.tools.is_empty()).then(|| Self::chat_tools(request.tools.as_slice()));
        let messages = Self::convert_messages(request.system, request.messages);

        let body = ChatCompletionRequest {
            model: model.to_string(),
            messages,
            tools,
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            cache_control: self
                .supports_top_level_prompt_cache()
                .then(prompt_cache::PromptCacheControl::ephemeral),
            prompt_cache_key: prompt_cache_key.clone(),
            prompt_cache_key_camel_case: prompt_cache_key.clone(),
            stream: false,
            stream_options: None,
        };

        let response = self
            .send_request(|api_key| {
                self.apply_request_headers(
                    self.client.post(self.completions_endpoint()),
                    api_key,
                    prompt_cache_key.as_deref(),
                )
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&body)
            })
            .await?;
        let payload: ChatCompletionResponse =
            utils::parse_json_response(self.id.as_str(), response).await?;
        self.parse_completion(payload)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        if matches!(
            self.stream_mode,
            OpenAiCompatibleStreamMode::RealtimeWebSocket
        ) {
            return self.complete_stream_with_realtime_ws(request).await;
        }

        let model = request.model.clone();
        let prompt_cache_key = request.prompt_cache_key.clone();

        let tools = (!request.tools.is_empty()).then(|| Self::chat_tools(request.tools.as_slice()));
        let messages = Self::convert_messages(request.system, request.messages);

        let body = ChatCompletionRequest {
            model: model.to_string(),
            messages,
            tools,
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            cache_control: self
                .supports_top_level_prompt_cache()
                .then(prompt_cache::PromptCacheControl::ephemeral),
            prompt_cache_key: prompt_cache_key.clone(),
            prompt_cache_key_camel_case: prompt_cache_key.clone(),
            stream: true,
            stream_options: Some(ChatStreamOptions {
                include_usage: true,
            }),
        };

        let response = self
            .send_request(|api_key| {
                self.apply_request_headers(
                    self.client.post(self.completions_endpoint()),
                    api_key,
                    prompt_cache_key.as_deref(),
                )
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&body)
            })
            .await?;
        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(self.id.as_str(), response).await);
        }

        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(self.id.clone());
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ToolCallState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut response_id: Option<String> = None;

            while let Some(event) = events.next().await {
                let event = event?;
                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(provider_id.as_str(), "chat stream chunk", event)?;
                if let Some(next_response_id) = chunk.id.clone() {
                    response_id = Some(next_response_id);
                }
                let choice = chunk.choices.first();

                let delta = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.content.as_ref())
                    .map(extract_text_from_content)
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
                    let tool = utils::parse_json_value::<ChatToolCall>(
                        provider_id.as_str(),
                        "chat stream tool_call delta",
                        raw_tool,
                    )?;

                    let id = utils::normalize_optional_text(tool.id.clone());
                    let key = tool
                        .index
                        .map(|idx| format!("idx:{idx}"))
                        .or_else(|| id.as_ref().map(|value| format!("id:{value}")))
                        .ok_or_else(|| {
                            AppError::Provider(format!(
                                "{} chat stream tool_call delta missing index/id",
                                provider_id
                            ))
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
                    let usage = utils::parse_json_value::<ChatUsage>(
                        provider_id.as_str(),
                        "chat stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Some(usage_to_completion_usage(usage));
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
                    provider_metadata: response_id_metadata(response_id),
                };
            }
        };

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAiCompatibleModelList {
    Object {
        #[serde(default)]
        data: Vec<OpenAiCompatibleModel>,
    },
    Array(Vec<OpenAiCompatibleModel>),
}

#[derive(Debug, Deserialize)]
struct OpenAiCompatibleModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<prompt_cache::PromptCacheControl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    #[serde(rename = "promptCacheKey", skip_serializing_if = "Option::is_none")]
    prompt_cache_key_camel_case: Option<String>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<ChatStreamOptions>,
}

#[derive(Debug, Serialize)]
struct ChatStreamOptions {
    #[serde(rename = "include_usage")]
    include_usage: bool,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ChatToolCallRequest>>,
}

#[derive(Debug, Serialize)]
struct ChatToolCallRequest {
    #[serde(rename = "type")]
    kind: String,
    id: String,
    function: ChatFunctionCallRequest,
}

#[derive(Debug, Serialize)]
struct ChatFunctionCallRequest {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct ChatToolDefinition {
    #[serde(rename = "type")]
    kind: String,
    function: ChatFunctionDefinition,
}

#[derive(Debug, Serialize)]
struct ChatFunctionDefinition {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Serialize)]
struct RealtimeToolDefinition {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatCompletionChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    #[serde(default)]
    message: Option<ChatDeltaOrMessage>,
    #[serde(default)]
    delta: Option<ChatDeltaOrMessage>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatDeltaOrMessage {
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCall {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChatFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct ChatFunctionCall {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    #[serde(default, alias = "input_tokens")]
    prompt_tokens: Option<u64>,
    #[serde(default, alias = "output_tokens")]
    completion_tokens: Option<u64>,
    #[serde(default)]
    output_tokens_details: Option<ChatOutputTokensDetails>,
    #[serde(default)]
    input_tokens_details: Option<ChatInputTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct ChatOutputTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ChatInputTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

fn extract_text_from_content(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

#[derive(Debug, Default)]
struct ToolCallState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::message::{
        AttachmentItem, AttachmentKind, AttachmentSource, Message, PartContent, StructuredObject,
        TimeRange, ToolExecutionPart, ToolInvocation, ToolOutput,
    };
    use crate::model::ModelId;
    use crate::tool::{ToolBehavior, ToolDefinition};
    use tokio::net::TcpListener;

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

    fn sample_png_data_url() -> &'static str {
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO9W7tYAAAAASUVORK5CYII="
    }

    #[test]
    fn prompt_cache_shape_changes_when_env_secret_changes() {
        let key = format!("AGENA_TEST_OPENAI_COMPAT_SCOPE_{}", std::process::id());
        unsafe { std::env::set_var(key.as_str(), "sk-first") };

        let provider = OpenAiCompatibleProvider::new_managed(
            "opencode",
            reqwest::Client::new(),
            ManagedCredential::environment("opencode env", "opencode", "api_key", key.as_str()),
            "https://opencode.ai/zen/v1",
            "gemini-3-pro",
        );
        let shape_a = provider
            .prompt_cache_shape(&ModelId::new("gemini-3-pro"))
            .expect("shape should exist");

        unsafe { std::env::set_var(key.as_str(), "sk-second") };
        let shape_b = provider
            .prompt_cache_shape(&ModelId::new("gemini-3-pro"))
            .expect("shape should exist");
        unsafe { std::env::remove_var(key.as_str()) };

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn prompt_cache_shape_ignores_volatile_or_secret_extra_headers() {
        let provider_a = OpenAiCompatibleProvider::new(
            "opencode",
            reqwest::Client::new(),
            "sk-test",
            "https://opencode.ai/zen/v1",
            "gemini-3-pro",
        )
        .with_extra_headers(HashMap::from([
            ("x-opencode-route".to_owned(), "backend-a".to_owned()),
            ("x-request-id".to_owned(), "req-a".to_owned()),
            ("traceparent".to_owned(), "trace-a".to_owned()),
            ("authorization".to_owned(), "Bearer secret-a".to_owned()),
        ]));
        let provider_b = OpenAiCompatibleProvider::new(
            "opencode",
            reqwest::Client::new(),
            "sk-test",
            "https://opencode.ai/zen/v1",
            "gemini-3-pro",
        )
        .with_extra_headers(HashMap::from([
            ("x-opencode-route".to_owned(), "backend-a".to_owned()),
            ("x-request-id".to_owned(), "req-b".to_owned()),
            ("traceparent".to_owned(), "trace-b".to_owned()),
            ("authorization".to_owned(), "Bearer secret-b".to_owned()),
        ]));

        let shape_a = provider_a
            .prompt_cache_shape(&ModelId::new("gemini-3-pro"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&ModelId::new("gemini-3-pro"))
            .expect("shape should exist");

        assert_eq!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn prompt_cache_shape_changes_when_stable_extra_headers_change() {
        let provider_a = OpenAiCompatibleProvider::new(
            "opencode",
            reqwest::Client::new(),
            "sk-test",
            "https://opencode.ai/zen/v1",
            "gemini-3-pro",
        )
        .with_extra_headers(HashMap::from([(
            "x-opencode-route".to_owned(),
            "backend-a".to_owned(),
        )]));
        let provider_b = OpenAiCompatibleProvider::new(
            "opencode",
            reqwest::Client::new(),
            "sk-test",
            "https://opencode.ai/zen/v1",
            "gemini-3-pro",
        )
        .with_extra_headers(HashMap::from([(
            "x-opencode-route".to_owned(),
            "backend-b".to_owned(),
        )]));

        let shape_a = provider_a
            .prompt_cache_shape(&ModelId::new("gemini-3-pro"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&ModelId::new("gemini-3-pro"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    fn tool_result_message_with_image(tool_call_id: &str) -> Message {
        let mut message = Message::prompt_parts(
            crate::role::Role::Tool,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 0,
                    invocation: ToolInvocation::Custom {
                        name: "tool".to_owned(),
                        input: StructuredObject::default(),
                    },
                    output_text: "{\"ok\":true}".to_owned(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: ToolOutput::default(),
                    lifecycle: TimeRange::default(),
                }),
                PartContent::attachments(vec![AttachmentItem {
                    kind: AttachmentKind::Image,
                    mime: "image/png".to_owned(),
                    source: AttachmentSource::DataUrl {
                        url: sample_png_data_url().to_owned(),
                    },
                    filename: Some("image.png".to_owned()),
                    title: None,
                    size_bytes: Some(68),
                    sha256: None,
                    width: Some(1),
                    height: Some(1),
                    duration_ms: None,
                    page_count: None,
                }]),
            ],
        );
        if let Some(part) = message.parts.first_mut() {
            part.operation_id = Some(tool_call_id.to_owned());
        }
        message
    }

    #[tokio::test]
    async fn complete_parses_text_tool_calls_usage() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": "Need tool",
                            "tool_calls": [{
                                "id": "call_1",
                                "function": {
                                    "name": "search",
                                    "arguments": "{\"q\":\"rust\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 11,
                        "completion_tokens": 7
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(128),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "Need tool");
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::ToolCalls)
        ));
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
        let usage = response.usage.expect("usage should be present");
        assert_eq!(usage.input_tokens, 11);
        assert_eq!(usage.output_tokens, 7);
    }

    #[test]
    fn convert_messages_splits_tool_result_and_image_follow_up() {
        let messages = OpenAiCompatibleProvider::convert_messages(
            None,
            vec![tool_result_message_with_image("call_1")],
        );

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "tool");
        assert_eq!(messages[0].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(
            messages[0].content,
            Some(Value::String("{\"ok\":true}".to_owned()))
        );

        assert_eq!(messages[1].role, "user");
        assert!(messages[1].tool_call_id.is_none());
        let content = messages[1]
            .content
            .as_ref()
            .expect("follow-up user message should have content");
        assert_eq!(content[0]["type"], "image_url");
        assert_eq!(content[0]["image_url"]["url"], sample_png_data_url());
    }

    #[test]
    fn convert_messages_preserves_interleaved_tool_result_and_follow_up_order() {
        let mut message = Message::prompt_parts(
            crate::role::Role::Tool,
            vec![
                PartContent::text("Before"),
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 1,
                    invocation: ToolInvocation::Custom {
                        name: "tool_one".to_owned(),
                        input: StructuredObject::default(),
                    },
                    output_text: "{\"result\":1}".to_owned(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: ToolOutput::default(),
                    lifecycle: TimeRange::default(),
                }),
                PartContent::text("Middle"),
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 2,
                    invocation: ToolInvocation::Custom {
                        name: "tool_two".to_owned(),
                        input: StructuredObject::default(),
                    },
                    output_text: "{\"result\":2}".to_owned(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: ToolOutput::default(),
                    lifecycle: TimeRange::default(),
                }),
                PartContent::text("After"),
            ],
        );
        message.parts[1].operation_id = Some("call_1".to_owned());
        message.parts[3].operation_id = Some("call_2".to_owned());

        let messages = OpenAiCompatibleProvider::convert_messages(None, vec![message]);

        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].role, "user");
        assert_eq!(
            messages[0].content,
            Some(serde_json::json!([{ "type": "text", "text": "Before" }]))
        );
        assert_eq!(messages[1].role, "tool");
        assert_eq!(messages[1].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(
            messages[1].content,
            Some(Value::String("{\"result\":1}".to_owned()))
        );
        assert_eq!(messages[2].role, "user");
        assert_eq!(
            messages[2].content,
            Some(serde_json::json!([{ "type": "text", "text": "Middle" }]))
        );
        assert_eq!(messages[3].role, "tool");
        assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_2"));
        assert_eq!(
            messages[3].content,
            Some(Value::String("{\"result\":2}".to_owned()))
        );
        assert_eq!(messages[4].role, "user");
        assert_eq!(
            messages[4].content,
            Some(serde_json::json!([{ "type": "text", "text": "After" }]))
        );
    }

    #[tokio::test]
    async fn complete_includes_tool_definitions_in_chat_request() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex(
                "\\\"name\\\":\\\"project_search\\\"".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"description\\\":\\\"Search project files\\.\\\"".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"query\\\":\\{\\\"type\\\":\\\"string\\\"\\}".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "message": { "content": "ok" },
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: vec![sample_tool_definition()],
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test]
    async fn generic_requests_include_prompt_cache_key_aliases_and_session_affinity() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .match_header("x-session-affinity", "session-42")
            .match_body(mockito::Matcher::Regex(
                "\\\"prompt_cache_key\\\":\\\"session-42\\\"".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"promptCacheKey\\\":\\\"session-42\\\"".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "message": { "content": "ok" },
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: Some("system".to_string()),
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_string()),
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test]
    async fn openrouter_requests_include_top_level_cache_control_and_prompt_cache_key() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .match_header("x-session-affinity", "session-42")
            .match_body(mockito::Matcher::Regex(
                "\\\"cache_control\\\":\\{\\\"type\\\":\\\"ephemeral\\\"\\}".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"prompt_cache_key\\\":\\\"session-42\\\"".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"promptCacheKey\\\":\\\"session-42\\\"".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "id": "resp_next",
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "message": { "content": "ok" },
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "openrouter",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: Some("system".to_string()),
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_string()),
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "ok");
        assert_eq!(
            response
                .provider_metadata
                .as_ref()
                .and_then(|value| value.get("response_id"))
                .and_then(|value| value.as_str()),
            Some("resp_next")
        );
    }

    #[tokio::test]
    async fn opencode_requests_include_top_level_cache_control_and_prompt_cache_key() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .match_header("x-session-affinity", "session-42")
            .match_body(mockito::Matcher::Regex(
                "\\\"cache_control\\\":\\{\\\"type\\\":\\\"ephemeral\\\"\\}".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"prompt_cache_key\\\":\\\"session-42\\\"".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"promptCacheKey\\\":\\\"session-42\\\"".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "message": { "content": "ok" },
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "opencode",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: Some("system".to_string()),
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_string()),
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test]
    async fn list_models_rejects_invalid_shape() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "data": [{ "name": "missing id" }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let err = provider
            .list_models()
            .await
            .expect_err("invalid model payload should fail");

        assert!(matches!(err, AppError::Provider(_)));
    }

    #[tokio::test]
    async fn complete_stream_emits_text_tool_delta_and_completed() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"id\":\"chatcmpl_stream\",\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"search\",\"arguments\":\"{\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );

        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("stream should start");

        let mut saw_text = false;
        let mut saw_tool_delta = false;
        let mut saw_completed = false;
        let mut completed_metadata = None;

        while let Some(item) = stream.next().await {
            match item.expect("stream event should parse") {
                CompletionStreamEvent::TextDelta { delta, .. } => {
                    if delta == "Hel" || delta == "lo" {
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
                        && (arguments_delta == "{" || arguments_delta == "}")
                    {
                        saw_tool_delta = true;
                    }
                }
                CompletionStreamEvent::Completed {
                    finish_reason,
                    usage,
                    provider_metadata,
                    ..
                } => {
                    assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                    let usage = usage.expect("usage should be present");
                    assert_eq!(usage.input_tokens, 3);
                    assert_eq!(usage.output_tokens, 2);
                    completed_metadata = provider_metadata;
                    saw_completed = true;
                }
            }
        }

        assert!(saw_text);
        assert!(saw_tool_delta);
        assert!(saw_completed);
        assert_eq!(
            completed_metadata
                .as_ref()
                .and_then(|value| value.get("response_id"))
                .and_then(|value| value.as_str()),
            Some("chatcmpl_stream")
        );
    }

    #[test]
    fn realtime_ws_endpoint_uses_ws_scheme_and_model_query() {
        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            "https://api.openai.com/v1",
            "gpt-4o-mini",
        )
        .with_stream_mode(OpenAiCompatibleStreamMode::RealtimeWebSocket);

        let endpoint = provider
            .realtime_ws_endpoint("gpt-4o-mini")
            .expect("endpoint should derive");

        assert_eq!(endpoint.scheme(), "wss");
        assert_eq!(endpoint.path(), "/v1/realtime");
        assert_eq!(
            endpoint
                .query_pairs()
                .find(|(key, _)| key == "model")
                .map(|(_, value)| value.into_owned()),
            Some("gpt-4o-mini".to_owned())
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
        let session_affinity_header = Arc::new(Mutex::new(None::<String>));
        let session_affinity_header_server = session_affinity_header.clone();

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
                    let session_affinity = request
                        .headers()
                        .get("x-session-affinity")
                        .and_then(|v| v.to_str().ok())
                        .map(ToOwned::to_owned);
                    *auth_header_server
                        .lock()
                        .expect("auth header lock should succeed") = value;
                    *session_affinity_header_server
                        .lock()
                        .expect("session affinity header lock should succeed") = session_affinity;
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
                let value: Value =
                    serde_json::from_str(text.as_str()).expect("request event should be json");
                if value.get("type").and_then(|v| v.as_str()) == Some("response.create") {
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
                            "id": "resp_stream",
                            "status": "completed",
                            "usage": {
                                "input_tokens": 3,
                                "output_tokens": 2
                            }
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("response done should send");
        });

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            format!("http://{addr}"),
            "gpt-4o-mini",
        )
        .with_stream_mode(OpenAiCompatibleStreamMode::RealtimeWebSocket)
        .with_realtime_ws_url(Some(format!("ws://{addr}/realtime")));

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: Some("you are helpful".to_owned()),
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: Some(0.2),
                max_output_tokens: Some(64),
                prompt_cache_key: Some("session-42".to_owned()),
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("stream should start");

        let mut saw_text = false;
        let mut saw_tool = false;
        let mut saw_completed = false;
        let mut completed_metadata = None;

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
                    provider_metadata,
                    ..
                } => {
                    assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                    let usage = usage.expect("usage should be present");
                    assert_eq!(usage.input_tokens, 3);
                    assert_eq!(usage.output_tokens, 2);
                    completed_metadata = provider_metadata;
                    saw_completed = true;
                }
            }
        }

        assert!(saw_text);
        assert!(saw_tool);
        assert!(saw_completed);
        assert_eq!(
            completed_metadata
                .as_ref()
                .and_then(|value| value.get("response_id"))
                .and_then(|value| value.as_str()),
            Some("resp_stream")
        );

        server.await.expect("server task should finish");
        assert_eq!(
            auth_header
                .lock()
                .expect("auth header lock should succeed")
                .as_deref(),
            Some("Bearer sk-test")
        );
        assert_eq!(
            session_affinity_header
                .lock()
                .expect("session affinity header lock should succeed")
                .as_deref(),
            Some("session-42")
        );
    }

    #[tokio::test]
    async fn complete_stream_returns_structured_error_details() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": {
                        "message": "invalid request",
                        "type": "invalid_request_error",
                        "code": "bad_request"
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let err = match provider
            .complete_stream(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
        {
            Ok(_) => panic!("stream should fail with provider error"),
            Err(err) => err,
        };

        match err {
            AppError::HttpStatus { body, status, .. } => {
                assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
                assert!(body.contains("invalid request"));
                assert!(body.contains("type=invalid_request_error"));
                assert!(body.contains("code=\"bad_request\""));
            }
            other => panic!("unexpected error type: {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_supports_legacy_choice_text_payload() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "text-davinci-003",
                    "choices": [{
                        "text": "legacy completion text",
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "text-davinci-003",
        );

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("text-davinci-003"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("legacy completion payload should parse");

        assert_eq!(response.text, "legacy completion text");
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::Stop)
        ));
    }

    #[tokio::test]
    async fn complete_stream_sends_include_usage_option() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex("\"include_usage\":true".to_owned()))
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(
                concat!(
                    "data: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
                    "data: [DONE]\n\n"
                ),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("stream should start");

        let mut saw_completed = false;
        while let Some(item) = stream.next().await {
            if let CompletionStreamEvent::Completed { usage, .. } =
                item.expect("event should parse")
            {
                let usage = usage.expect("usage should be present");
                assert_eq!(usage.input_tokens, 1);
                assert_eq!(usage.output_tokens, 1);
                saw_completed = true;
            }
        }

        assert!(saw_completed);
    }

    #[tokio::test]
    async fn complete_stream_preserves_usage_emitted_after_finish_reason() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
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

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("stream should start");

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
}
