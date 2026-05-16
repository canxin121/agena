use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

use crate::{
    error::AppError,
    message::{AttachmentItem, AttachmentKind, AttachmentSource, Message, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CapabilityFamily, CompletionFinishReason, CompletionRequest, CompletionResponse,
        CompletionStreamEvent, CompletionToolCall, CompletionUsage, ManagedCredential,
        ModelProvider, ProviderModel, StreamResumePolicy,
        auth::AuthData,
        chat_wire::{
            self, ChatCompletionRequest, ChatCompletionResponse, ChatStreamOptions,
            request_to_chat_messages, tools_to_chat_definitions,
        },
        prompt_cache,
        remote_model_catalog_cache::{RemoteModelCatalogCache, RemoteModelCatalogSource},
        sse, utils, wire_message,
    },
    role::Role,
};

const CHATGPT_CODEX_ORIGINATOR: &str = "agena";
const CHATGPT_CODEX_USER_AGENT: &str = concat!("agena/", env!("CARGO_PKG_VERSION"));
const DEFAULT_COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";

#[derive(Clone)]
pub struct OpenAiProvider {
    id: String,
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    backend: OpenAiBackend,
    auth_data: Option<Arc<Mutex<AuthData>>>,
    api_mode: OpenAiApiMode,
    api_mode_explicit: bool,
    profile: OpenAiProfile,
    models_url: Option<String>,
    auth_header: String,
    auth_scheme: Option<String>,
    capability_family: CapabilityFamily,
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
pub enum OpenAiBackend {
    Api,
    ChatgptCodex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiProfile {
    Standard,
    GithubCopilot,
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
        Self::new_managed(
            client,
            ManagedCredential::static_value("openai api key", api_key.into()),
            base_url,
            default_model,
        )
    }

    pub fn new_with_id(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_managed_with_id(
            id,
            client,
            ManagedCredential::static_value("openai api key", api_key.into()),
            base_url,
            default_model,
        )
    }

    pub fn new_managed(
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_managed_with_id("openai", client, api_key, base_url, default_model)
    }

    pub fn new_managed_with_id(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        let id = id.into();
        Self {
            id,
            client,
            api_key,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            backend: OpenAiBackend::Api,
            auth_data: None,
            api_mode: OpenAiApiMode::Responses,
            api_mode_explicit: false,
            profile: OpenAiProfile::Standard,
            models_url: None,
            auth_header: "authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            capability_family: CapabilityFamily::OpenAi,
            extra_headers: HashMap::new(),
            stream_mode: OpenAiStreamMode::Sse,
            realtime_ws_url: None,
        }
    }

    pub fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.extra_headers = headers;
        self
    }

    pub fn with_backend(mut self, backend: OpenAiBackend) -> Self {
        self.backend = backend;
        self
    }

    pub fn with_auth_data(mut self, auth_data: Arc<Mutex<AuthData>>) -> Self {
        self.auth_data = Some(auth_data);
        self
    }

    pub fn with_api_mode(mut self, mode: OpenAiApiMode) -> Self {
        self.api_mode = mode;
        self
    }

    pub fn with_api_mode_explicit(mut self, explicit: bool) -> Self {
        self.api_mode_explicit = explicit;
        self
    }

    pub fn with_profile(mut self, profile: OpenAiProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn with_models_url(mut self, models_url: Option<String>) -> Self {
        self.models_url = models_url.and_then(|value| utils::normalize_optional_text(Some(value)));
        self
    }

    pub fn with_auth_header(
        mut self,
        header: impl Into<String>,
        scheme: Option<impl Into<String>>,
    ) -> Self {
        self.auth_header = header.into();
        self.auth_scheme = scheme.map(|value| value.into());
        self
    }

    pub fn with_capability_family(mut self, family: CapabilityFamily) -> Self {
        self.capability_family = family;
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

    fn configured_public_copilot_base_url(&self) -> bool {
        self.base_url.trim_end_matches('/') == DEFAULT_COPILOT_BASE_URL
    }

    fn resolved_base_url(&self) -> Result<String, AppError> {
        if self.profile != OpenAiProfile::GithubCopilot
            || !self.configured_public_copilot_base_url()
        {
            return Ok(self.base_url.clone());
        }

        let Some(auth_data) = self.auth_data.as_ref() else {
            return Ok(self.base_url.clone());
        };

        let domain = auth_data
            .try_lock()
            .ok()
            .as_deref()
            .and_then(AuthData::enterprise_url)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                AppError::Config("enterprise_url missing for enterprise copilot auth".to_owned())
            })?;

        Ok(format!("https://copilot-api.{}", normalize_domain(&domain)))
    }

    fn prompt_cache_base_url(&self) -> String {
        self.resolved_base_url()
            .unwrap_or_else(|_| self.base_url.clone())
    }

    fn model_endpoint(&self) -> Result<String, AppError> {
        Ok(self.models_url.clone().unwrap_or_else(|| {
            format!(
                "{}/models",
                self.prompt_cache_base_url().trim_end_matches('/')
            )
        }))
    }

    fn responses_endpoint(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}/responses",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    fn chat_endpoint(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}/chat/completions",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    fn backend_key(&self) -> &'static str {
        match self.backend {
            OpenAiBackend::Api => "api",
            OpenAiBackend::ChatgptCodex => "chatgpt_codex",
        }
    }

    fn can_fallback_to_chat(&self) -> bool {
        matches!(self.backend, OpenAiBackend::Api)
    }

    fn chatgpt_account_id(&self) -> Option<String> {
        self.auth_data
            .as_ref()
            .and_then(|auth| auth.try_lock().ok())
            .as_deref()
            .and_then(AuthData::account_id)
            .map(ToOwned::to_owned)
            .and_then(|value| utils::normalize_optional_text(Some(value)))
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
        api_key: &str,
    ) -> Result<http::Request<()>, AppError> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let mut request = endpoint.as_str().into_client_request().map_err(|err| {
            AppError::Config(format!(
                "openai realtime websocket handshake invalid: {err}"
            ))
        })?;

        let auth_header_name = http::header::HeaderName::from_bytes(self.auth_header.as_bytes())
            .map_err(|err| {
                AppError::Config(format!("openai auth header name is invalid: {err}"))
            })?;
        let auth_header_value = http::header::HeaderValue::from_str(
            utils::auth_header_value(self.auth_scheme.as_deref(), api_key).as_str(),
        )
        .map_err(|err| AppError::Config(format!("openai auth header value is invalid: {err}")))?;
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
        if matches!(self.profile, OpenAiProfile::GithubCopilot)
            && !self.api_mode_explicit
            && matches!(self.api_mode, OpenAiApiMode::Responses)
        {
            return Self::copilot_should_use_responses(model);
        }

        if matches!(self.backend, OpenAiBackend::ChatgptCodex) {
            return true;
        }

        match self.api_mode {
            OpenAiApiMode::Responses => true,
            OpenAiApiMode::Chat => false,
            OpenAiApiMode::Auto => {
                model.starts_with("gpt-5") || model.starts_with("o3") || model.starts_with("o4")
            }
        }
    }

    fn copilot_should_use_responses(model: &str) -> bool {
        let is_gpt5 = model
            .strip_prefix("gpt-")
            .and_then(|x| x.split('-').next())
            .and_then(|major| major.parse::<u32>().ok())
            .map(|major| major >= 5)
            .unwrap_or(false);
        is_gpt5 && !model.starts_with("gpt-5-mini")
    }

    fn api_mode_key(&self) -> &'static str {
        match self.api_mode {
            OpenAiApiMode::Responses => "responses",
            OpenAiApiMode::Chat => "chat",
            OpenAiApiMode::Auto => "auto",
        }
    }

    fn stream_mode_key(&self) -> &'static str {
        match self.stream_mode {
            OpenAiStreamMode::Sse => "sse",
            OpenAiStreamMode::RealtimeWebSocket => "realtime_websocket",
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
        let body = ChatCompletionRequest {
            model: model.clone(),
            messages: self.chat_messages_for_request(request),
            tools: (!request.tools.is_empty())
                .then(|| tools_to_chat_definitions(request.tools.as_slice())),
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            cache_control: None,
            prompt_cache_key: None,
            prompt_cache_key_camel_case: None,
            stream: false,
            stream_options: None,
            stop: request.stop_sequences.clone(),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
        };

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let auth_value = utils::auth_header_value(self.auth_scheme.as_deref(), api_key);
            self.apply_headers(
                self.client
                    .post(self.chat_endpoint().expect("chat endpoint should resolve"))
                    .header(self.auth_header.as_str(), auth_value)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
                RequestHeaderContext::from_request(request),
            )
            .json(&body)
        })
        .await?;

        let payload: ChatCompletionResponse =
            utils::parse_json_response(self.id.as_str(), response).await?;

        chat_wire::parse_completion_response(self.id.as_str(), model.as_str(), payload)
    }

    async fn complete_stream_with_chat_api(
        &self,
        request: &CompletionRequest,
        model: String,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let body = ChatCompletionRequest {
            model: model.clone(),
            messages: self.chat_messages_for_request(request),
            tools: (!request.tools.is_empty())
                .then(|| tools_to_chat_definitions(request.tools.as_slice())),
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            cache_control: None,
            prompt_cache_key: None,
            prompt_cache_key_camel_case: None,
            stream: true,
            stream_options: Some(ChatStreamOptions {
                include_usage: true,
            }),
            stop: request.stop_sequences.clone(),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
        };

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let auth_value = utils::auth_header_value(self.auth_scheme.as_deref(), api_key);
            self.apply_headers(
                self.client
                    .post(self.chat_endpoint().expect("chat endpoint should resolve"))
                    .header(self.auth_header.as_str(), auth_value)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
                RequestHeaderContext::from_request(request),
            )
            .json(&body)
        })
        .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(self.id.as_str(), response).await);
        }

        let provider_name = self.id.clone();
        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(provider_name.as_str());
        let model_name = ModelId::new(model);

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, chat_wire::ChatToolCallStreamState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;

            while let Some(event) = events.next().await {
                let event = event?;

                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(provider_name.as_str(), "chat stream chunk", event)?;
                let choice = chunk.choices.first();

                let delta = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.content.as_ref())
                    .map(chat_wire::extract_text_from_content)
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
                    let tool = utils::parse_json_value::<chat_wire::ChatToolCallWire>(
                        provider_name.as_str(),
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
                    let mut emitted_any = false;
                    if let Some(function) = tool.function {
                        if let Some(name) = utils::normalize_optional_text(function.name) {
                            state.name = Some(name);
                        }
                        if let Some(args) = function.arguments
                            && !args.is_empty() {
                                state.arguments.push_str(args.as_str());
                                stream_has_content = true;
                                emitted_any = true;
                                state.announced = true;
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
                    // Register the call with the aggregator the first
                    // time we have its name available, even if no
                    // arguments arrived this chunk — a parameterless
                    // call may never carry args.
                    if !state.announced && !emitted_any && state.name.is_some() {
                        state.announced = true;
                        stream_has_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: key.clone(),
                            id: state.id.clone(),
                            name: state.name.clone(),
                            arguments_delta: String::new(),
                        };
                    }
                }

                if let Some(raw_usage) = chunk.usage {
                    let usage = utils::parse_json_value::<chat_wire::ChatUsage>(
                        provider_name.as_str(),
                        "chat stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Some(chat_wire::chat_usage_to_completion(usage));
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
        let api_key = self.api_key.resolve().await?;
        let handshake = self.realtime_handshake_request(&ws_endpoint, api_key.as_str())?;
        let (ws_stream, _) = tokio_tungstenite::connect_async(handshake)
            .await
            .map_err(|err| {
                AppError::Provider(format!("openai realtime websocket connect failed: {err}"))
            })?;

        let provider_name = self.id.clone();
        let provider_id = ProviderId::new(provider_name.as_str());
        let model_name = ModelId::new(model);
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

                if let Some(err) = utils::responses_stream_error(provider_name.as_str(), &event)? {
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

                if let Some(tool_event) = utils::responses_tool_event(provider_name.as_str(), &event)? {
                    let key = tool_event.stream_key(provider_name.as_str())?;

                    let is_added = matches!(tool_event.kind, utils::ResponsesToolEventKind::Added);
                    let was_new = !pending_tool_calls.contains_key(&key);
                    let state = pending_tool_calls.entry(key.clone()).or_default();
                    if let Some(id) = tool_event.id.clone() {
                        state.id = Some(id);
                    }
                    if let Some(name) = tool_event.name.clone() {
                        state.name = Some(name);
                    }

                    if is_added && was_new {
                        // Register the call with the aggregator so a
                        // parameterless tool call (no Delta events) is
                        // not silently dropped.
                        stream_has_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: key.clone(),
                            id: state.id.clone(),
                            name: state.name.clone(),
                            arguments_delta: String::new(),
                        };
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
                        provider_name.as_str(),
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
            .filter(|item| item.kind.as_deref() != Some("reasoning"))
            .flat_map(|item| item.content.iter().flatten())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("")
    }

    fn extract_reasoning_text(response: &OpenAiResponsesResponse) -> Option<String> {
        let text: String = response
            .output
            .iter()
            .flatten()
            .filter(|item| item.kind.as_deref() == Some("reasoning"))
            .flat_map(|item| item.content.iter().flatten())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("");
        (!text.is_empty()).then_some(text)
    }

    async fn complete_by_aggregating_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let fallback_model = request.model.clone();
        let stream = ModelProvider::complete_stream(self, request).await?;
        utils::aggregate_stream(self.id.as_str(), fallback_model, stream).await
    }

    fn map_usage(usage: Option<OpenAiUsage>) -> Option<CompletionUsage> {
        usage.map(|u| {
            let input_tokens_raw = u.input_tokens.unwrap_or_default();
            let cache_read_tokens = u
                .input_tokens_details
                .and_then(|d| d.cached_tokens)
                .unwrap_or_default();
            // Match Anthropic's convention: `input_tokens` is the uncached
            // portion only. OpenAI's `input_tokens` is inclusive of cache.
            let input_tokens = input_tokens_raw.saturating_sub(cache_read_tokens);
            MessageUsage {
                input_tokens,
                output_tokens: u.output_tokens.unwrap_or_default(),
                reasoning_tokens: u
                    .output_tokens_details
                    .and_then(|d| d.reasoning_tokens)
                    .unwrap_or_default(),
                cache_write_tokens: 0,
                cache_read_tokens,
                total_cost: 0.0,
            }
            .into()
        })
    }

    fn responses_tools(tools: &[crate::plugin::registry::PluginEntry]) -> Vec<OpenAiResponsesTool> {
        tools
            .iter()
            .map(|tool| OpenAiResponsesTool {
                kind: "function",
                name: tool.exposed_name.clone(),
                description: tool.description_text().to_string(),
                parameters: tool.sanitized_input_schema(),
                strict: tool.decl.strict,
            })
            .collect()
    }

    fn is_vision_request(request: &CompletionRequest) -> bool {
        request.messages.iter().any(|message| {
            wire_message::project(message).iter().any(|part| {
                matches!(
                    part,
                    wire_message::WirePart::Attachment { item }
                        if item.kind == AttachmentKind::Image
                )
            })
        })
    }

    fn initiator(request: &CompletionRequest) -> &'static str {
        match request.messages.last().map(|m| m.role) {
            Some(Role::User) => "user",
            _ => "agent",
        }
    }

    fn chat_messages_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Vec<chat_wire::ChatMessage> {
        let mut messages = request_to_chat_messages(request);
        if matches!(self.profile, OpenAiProfile::GithubCopilot) {
            apply_chat_prompt_cache_hints(messages.as_mut_slice());
        }
        messages
    }

    fn responses_input_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Vec<OpenAiResponsesInputItem> {
        let mut input = Self::to_responses_input(request);
        if !matches!(self.profile, OpenAiProfile::GithubCopilot) {
            clear_responses_prompt_cache_hints(input.as_mut_slice());
        }
        input
    }

    fn to_responses_input(request: &CompletionRequest) -> Vec<OpenAiResponsesInputItem> {
        let mut input = Vec::new();

        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            Self::push_responses_text_message(&mut input, "system", system.clone());
        }

        for message in &request.messages {
            Self::append_responses_items_for_message(&mut input, message);
        }

        apply_responses_prompt_cache_hints(input.as_mut_slice());
        input
    }

    fn attachment_upload_name(item: &AttachmentItem) -> String {
        wire_message::filename(item)
            .map(str::to_owned)
            .unwrap_or_else(|| item.summary_label())
    }

    fn responses_file_content(item: &AttachmentItem) -> Option<OpenAiInputContent> {
        let filename = Some(Self::attachment_upload_name(item));
        match &item.source {
            AttachmentSource::Base64 { .. } | AttachmentSource::DataUrl { .. } => {
                wire_message::data_url(item).map(|file_data| OpenAiInputContent::File {
                    file_data: Some(file_data),
                    file_id: None,
                    file_url: None,
                    filename,
                })
            }
            AttachmentSource::FileId { file_id } => {
                let file_id = file_id.trim();
                (!file_id.is_empty()).then(|| OpenAiInputContent::File {
                    file_data: None,
                    file_id: Some(file_id.to_owned()),
                    file_url: None,
                    filename,
                })
            }
            AttachmentSource::Url { url } => {
                let file_url = url.trim();
                (!file_url.is_empty()).then(|| OpenAiInputContent::File {
                    file_data: None,
                    file_id: None,
                    file_url: Some(file_url.to_owned()),
                    filename,
                })
            }
            AttachmentSource::LocalPath { .. } => None,
        }
    }

    fn responses_content_from_attachment(item: &AttachmentItem) -> OpenAiInputContent {
        match item.kind {
            AttachmentKind::Image => wire_message::media_url(item)
                .map(|image_url| OpenAiInputContent::Image { image_url })
                .unwrap_or_else(|| OpenAiInputContent::Text {
                    text: wire_message::hint_text(item),
                }),
            AttachmentKind::Audio
            | AttachmentKind::Video
            | AttachmentKind::Pdf
            | AttachmentKind::File => {
                Self::responses_file_content(item).unwrap_or_else(|| OpenAiInputContent::Text {
                    text: wire_message::hint_text(item),
                })
            }
        }
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
            content: vec![OpenAiInputContent::Text { text }],
            copilot_cache_control: None,
        }));
    }

    fn flush_assistant_responses_text(
        input: &mut Vec<OpenAiResponsesInputItem>,
        text_chunks: &mut Vec<String>,
    ) {
        if text_chunks.is_empty() {
            return;
        }

        let text = text_chunks.join("");
        text_chunks.clear();
        Self::push_responses_text_message(input, "assistant", text);
    }

    fn push_responses_message_from_parts(
        input: &mut Vec<OpenAiResponsesInputItem>,
        role: &str,
        parts: &[wire_message::WirePart],
    ) {
        let content = Self::responses_input_contents_from_parts(parts);
        if content.is_empty() {
            return;
        }

        input.push(OpenAiResponsesInputItem::Message(OpenAiInputMessage {
            role: role.to_owned(),
            content,
            copilot_cache_control: None,
        }));
    }

    fn append_responses_items_for_message(
        input: &mut Vec<OpenAiResponsesInputItem>,
        message: &Message,
    ) {
        let projected_parts = wire_message::project(message);
        match message.role {
            Role::System => Self::push_responses_text_message(
                input,
                "system",
                session_text_lossy(message, projected_parts.as_slice()),
            ),
            Role::User => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "user", message.as_text_lossy());
                } else {
                    Self::push_responses_message_from_parts(
                        input,
                        "user",
                        projected_parts.as_slice(),
                    );
                }
            }
            Role::Assistant => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "assistant", message.as_text_lossy());
                } else {
                    let mut text_chunks = Vec::new();
                    for part in projected_parts {
                        match part {
                            wire_message::WirePart::Text { text } => text_chunks.push(text),
                            wire_message::WirePart::Attachment { item } => {
                                text_chunks.push(wire_message::hint_text(&item));
                            }
                            wire_message::WirePart::ToolCall {
                                id,
                                name,
                                arguments_json,
                            } => {
                                Self::flush_assistant_responses_text(input, &mut text_chunks);
                                if !id.trim().is_empty() && !name.trim().is_empty() {
                                    input.push(OpenAiResponsesInputItem::FunctionCall(
                                        OpenAiFunctionCallItem {
                                            kind: "function_call",
                                            call_id: id,
                                            name,
                                            arguments: arguments_json,
                                            copilot_cache_control: None,
                                        },
                                    ));
                                }
                            }
                            wire_message::WirePart::ToolResult {
                                tool_call_id,
                                output_json,
                                ..
                            } => {
                                Self::flush_assistant_responses_text(input, &mut text_chunks);
                                if !tool_call_id.trim().is_empty() {
                                    input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                                        OpenAiFunctionCallOutputItem {
                                            kind: "function_call_output",
                                            call_id: tool_call_id,
                                            output: serde_json::Value::String(output_json),
                                            copilot_cache_control: None,
                                        },
                                    ));
                                }
                            }
                        }
                    }
                    Self::flush_assistant_responses_text(input, &mut text_chunks);
                }
            }
            Role::Tool => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "user", message.as_text_lossy());
                } else {
                    let tool_results = wire_message::tool_results(projected_parts.as_slice());
                    let extra_parts =
                        wire_message::non_tool_result_parts(projected_parts.as_slice());

                    if tool_results.len() > 1 {
                        let mut buffered_parts = Vec::new();
                        for part in projected_parts {
                            match part {
                                wire_message::WirePart::ToolResult {
                                    tool_call_id,
                                    output_json,
                                    ..
                                } => {
                                    if !buffered_parts.is_empty() {
                                        Self::push_responses_message_from_parts(
                                            input,
                                            "user",
                                            buffered_parts.as_slice(),
                                        );
                                        buffered_parts.clear();
                                    }

                                    if tool_call_id.trim().is_empty() {
                                        buffered_parts.push(wire_message::WirePart::Text {
                                            text: output_json,
                                        });
                                    } else {
                                        input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                                            OpenAiFunctionCallOutputItem {
                                                kind: "function_call_output",
                                                call_id: tool_call_id,
                                                output: serde_json::Value::String(output_json),
                                                copilot_cache_control: None,
                                            },
                                        ));
                                    }
                                }
                                other => buffered_parts.push(other),
                            }
                        }

                        if !buffered_parts.is_empty() {
                            Self::push_responses_message_from_parts(
                                input,
                                "user",
                                buffered_parts.as_slice(),
                            );
                        }
                    } else if let Some((tool_call_id, output_json)) =
                        tool_results.into_iter().next()
                    {
                        if tool_call_id.trim().is_empty() {
                            let mut fallback_parts =
                                vec![wire_message::WirePart::Text { text: output_json }];
                            fallback_parts.extend(extra_parts);
                            Self::push_responses_message_from_parts(
                                input,
                                "user",
                                fallback_parts.as_slice(),
                            );
                        } else {
                            input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                                OpenAiFunctionCallOutputItem {
                                    kind: "function_call_output",
                                    call_id: tool_call_id,
                                    output: Self::multimodal_function_output_value(
                                        output_json.as_str(),
                                        extra_parts.as_slice(),
                                    ),
                                    copilot_cache_control: None,
                                },
                            ));
                        }
                    } else {
                        Self::push_responses_message_from_parts(
                            input,
                            "user",
                            projected_parts.as_slice(),
                        );
                    }
                }
            }
        }
    }

    fn responses_input_contents_from_parts(
        parts: &[wire_message::WirePart],
    ) -> Vec<OpenAiInputContent> {
        parts
            .iter()
            .map(|part| match part {
                wire_message::WirePart::Text { text } => {
                    OpenAiInputContent::Text { text: text.clone() }
                }
                wire_message::WirePart::Attachment { item } => {
                    Self::responses_content_from_attachment(item)
                }
                wire_message::WirePart::ToolCall { name, .. } => OpenAiInputContent::Text {
                    text: format!("[tool_call:{name}]"),
                },
                wire_message::WirePart::ToolResult { tool_call_id, .. } => {
                    OpenAiInputContent::Text {
                        text: format!("[tool_result:{tool_call_id}]"),
                    }
                }
            })
            .collect()
    }

    fn multimodal_function_output_value(
        output_json: &str,
        extra_parts: &[wire_message::WirePart],
    ) -> serde_json::Value {
        if extra_parts.is_empty() {
            return serde_json::Value::String(output_json.to_owned());
        }

        let mut content = Vec::new();
        if !output_json.trim().is_empty() {
            content.push(OpenAiInputContent::Text {
                text: output_json.to_owned(),
            });
        }
        content.extend(Self::responses_input_contents_from_parts(extra_parts));
        serde_json::to_value(content).expect("openai function_call_output content should serialize")
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
        context: RequestHeaderContext<'_>,
    ) -> Result<R, AppError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let auth_value = utils::auth_header_value(self.auth_scheme.as_deref(), api_key);
            let mut request = self.apply_headers(
                self.client
                    .post(endpoint.clone())
                    .header(self.auth_header.as_str(), auth_value)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
                context,
            );

            if let Some(body) = body {
                request = request.json(body);
            }

            request
        })
        .await?;
        utils::parse_json_response(self.id.as_str(), response).await
    }

    fn apply_headers(
        &self,
        req: reqwest::RequestBuilder,
        context: RequestHeaderContext<'_>,
    ) -> reqwest::RequestBuilder {
        let mut headers = self.extra_headers.clone();

        if matches!(self.backend, OpenAiBackend::ChatgptCodex) {
            headers
                .entry("originator".to_owned())
                .or_insert_with(|| CHATGPT_CODEX_ORIGINATOR.to_owned());
            headers
                .entry(reqwest::header::USER_AGENT.as_str().to_owned())
                .or_insert_with(|| CHATGPT_CODEX_USER_AGENT.to_owned());

            if let Some(account_id) = self.chatgpt_account_id() {
                headers.insert("ChatGPT-Account-Id".to_owned(), account_id);
            }

            if let Some(window_id) = context.window_id_header() {
                headers.insert("x-codex-window-id".to_owned(), window_id);
            }
        }

        if matches!(self.profile, OpenAiProfile::GithubCopilot) {
            headers
                .entry(reqwest::header::USER_AGENT.as_str().to_owned())
                .or_insert_with(|| "agena/0.1.0".to_owned());
            headers
                .entry("Openai-Intent".to_owned())
                .or_insert_with(|| "conversation-edits".to_owned());
            headers.insert(
                "x-initiator".to_owned(),
                context.initiator_header().to_owned(),
            );
            if context.vision_request {
                headers.insert("Copilot-Vision-Request".to_owned(), "true".to_owned());
            }
        }

        utils::apply_request_headers(self.id.as_str(), req, &headers)
    }
}

fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    utils::response_id_metadata(response_id)
}

#[derive(Clone, Copy, Default)]
struct RequestHeaderContext<'a> {
    prompt_cache_key: Option<&'a str>,
    prompt_window_generation: Option<u64>,
    initiator: Option<&'a str>,
    vision_request: bool,
}

impl<'a> RequestHeaderContext<'a> {
    fn from_request(request: &'a CompletionRequest) -> Self {
        Self {
            prompt_cache_key: request.prompt_cache_key.as_deref(),
            prompt_window_generation: request.prompt_window_generation,
            initiator: Some(OpenAiProvider::initiator(request)),
            vision_request: OpenAiProvider::is_vision_request(request),
        }
    }

    fn none() -> Self {
        Self::default()
    }

    fn window_id_header(&self) -> Option<String> {
        self.prompt_cache_key.map(|prompt_cache_key| {
            format!(
                "{}:{}",
                prompt_cache_key,
                self.prompt_window_generation.unwrap_or_default()
            )
        })
    }

    fn initiator_header(&self) -> &str {
        self.initiator.unwrap_or("agent")
    }
}

#[async_trait]
impl ModelProvider for OpenAiProvider {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(self.capability_family)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        matches!(self.stream_mode, OpenAiStreamMode::Sse)
            && self.should_use_responses(model.as_str())
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        Some(
            crate::provider::PromptCacheShape::new(self.id.as_str())
                .with_string("auth_scope", self.api_key.prompt_cache_scope())
                .with_string("backend", self.backend_key())
                .with_string("base_url", self.prompt_cache_base_url().as_str())
                .with_string("api_mode", self.api_mode_key())
                .with_string("stream_mode", self.stream_mode_key())
                .with_optional_string("models_url", self.models_url.as_deref())
                .with_string("auth_header", self.auth_header.as_str())
                .with_optional_string("auth_scheme", self.auth_scheme.as_deref())
                .with_string(
                    "profile",
                    match self.profile {
                        OpenAiProfile::Standard => "standard",
                        OpenAiProfile::GithubCopilot => "github_copilot",
                    },
                )
                .with_string(
                    "capability_family",
                    match self.capability_family {
                        CapabilityFamily::OpenAi => "openai",
                        CapabilityFamily::OpenAiCompatible => "openai_compatible",
                        CapabilityFamily::Anthropic => "anthropic",
                        CapabilityFamily::Gemini => "gemini",
                        CapabilityFamily::Bedrock => "bedrock",
                        CapabilityFamily::Gitlab => "gitlab",
                    },
                )
                .with_optional_string("auth_account_id", self.chatgpt_account_id())
                .with_bool("uses_responses", self.should_use_responses(model.as_str()))
                .with_optional_string("realtime_ws_url", self.realtime_ws_url.as_deref())
                .with_json(
                    "extra_headers",
                    &utils::prompt_cache_header_entries(&self.extra_headers),
                ),
        )
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let mut source = RemoteModelCatalogSource::new(
            self.id.as_str(),
            self.model_endpoint()?,
            self.api_key.prompt_cache_scope(),
        );
        source = match self.capability_family {
            CapabilityFamily::OpenAi => source
                .with_catalog_provider_id("openai")
                .with_catalog_visible_model_prefix("openai"),
            CapabilityFamily::OpenAiCompatible => source
                .with_catalog_provider_id("openai")
                .with_catalog_visible_model_prefix("openai"),
            CapabilityFamily::Anthropic => source.with_catalog_provider_id("anthropic"),
            CapabilityFamily::Gemini => source.with_catalog_provider_id("gemini"),
            CapabilityFamily::Bedrock => source.with_catalog_provider_id("bedrock"),
            CapabilityFamily::Gitlab => source.with_catalog_provider_id("gitlab"),
        };
        RemoteModelCatalogCache::default()
            .get_or_fetch(&source, || async {
                let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
                    let auth_value = utils::auth_header_value(self.auth_scheme.as_deref(), api_key);
                    self.apply_headers(
                        self.client
                            .get(
                                self.model_endpoint()
                                    .expect("model endpoint should resolve"),
                            )
                            .header(self.auth_header.as_str(), auth_value),
                        RequestHeaderContext::none(),
                    )
                })
                .await?;

                let payload: OpenAiModelListResponse =
                    utils::parse_json_response(self.id.as_str(), response).await?;
                Ok(payload
                    .into_items()
                    .into_iter()
                    .map(|m| {
                        let model = ProviderModel::new(self.id.as_str(), m.id);
                        let capabilities = self.model_capabilities(&model.id);
                        let model = model.with_capabilities(capabilities);
                        if let Some(name) = m.name {
                            model.with_display_name(name)
                        } else {
                            model
                        }
                    })
                    .collect())
            })
            .await
    }

    #[tracing::instrument(
        skip_all,
        fields(provider = tracing::field::Empty, model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        tracing::Span::current().record("provider", &tracing::field::display(self.id.as_str()));
        let model = request.model.clone();

        if !self.should_use_responses(model.as_str()) {
            return self
                .complete_with_chat_api(&request, model.to_string())
                .await;
        }

        let input = self.responses_input_for_request(&request);

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            input,
            tools: Self::responses_tools(request.tools.as_slice()),
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: request.previous_response_id.clone(),
            prompt_window_generation: request.prompt_window_generation,
            stream: false,
            stop: (!request.stop_sequences.is_empty()).then(|| request.stop_sequences.clone()),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
        };

        let response: OpenAiResponsesResponse = match self
            .send_json(
                self.responses_endpoint()?,
                Some(&body),
                RequestHeaderContext::from_request(&request),
            )
            .await
        {
            Ok(payload) => payload,
            Err(AppError::HttpStatus { status, .. })
                if self.can_fallback_to_chat() && Self::responses_endpoint_unsupported(status) =>
            {
                return self
                    .complete_with_chat_api(&request, model.to_string())
                    .await;
            }
            Err(err) => return Err(err),
        };

        let response_model =
            ModelId::new(response.model.clone().unwrap_or_else(|| model.to_string()));
        let text = Self::extract_text(&response);
        let reasoning_text = Self::extract_reasoning_text(&response);
        let finish_reason = CompletionFinishReason::from_provider(response.stop_reason.as_deref());
        let tool_calls = Self::parse_responses_tool_calls(response.output.as_ref())?;

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return self.complete_by_aggregating_stream(request).await;
        }

        let usage = Self::map_usage(response.usage);

        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id.as_str()),
            model: response_model,
            text,
            reasoning_text,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: response_id_metadata(response.id),
        })
    }

    #[tracing::instrument(
        skip_all,
        fields(provider = tracing::field::Empty, model = %request.model)
    )]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        tracing::Span::current().record("provider", &tracing::field::display(self.id.as_str()));
        let model = request.model.clone();

        if matches!(self.stream_mode, OpenAiStreamMode::RealtimeWebSocket) {
            return self
                .complete_stream_with_realtime_ws(&request, model.to_string())
                .await;
        }

        if !self.should_use_responses(model.as_str()) {
            return self
                .complete_stream_with_chat_api(&request, model.to_string())
                .await;
        }

        let input = self.responses_input_for_request(&request);

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            input,
            tools: Self::responses_tools(request.tools.as_slice()),
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: request.previous_response_id.clone(),
            prompt_window_generation: request.prompt_window_generation,
            stream: true,
            stop: (!request.stop_sequences.is_empty()).then(|| request.stop_sequences.clone()),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
        };

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let auth_value = utils::auth_header_value(self.auth_scheme.as_deref(), api_key);
            self.apply_headers(
                self.client
                    .post(
                        self.responses_endpoint()
                            .expect("responses endpoint should resolve"),
                    )
                    .header(self.auth_header.as_str(), auth_value)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
                RequestHeaderContext::from_request(&request),
            )
            .json(&body)
        })
        .await?;

        if !response.status().is_success() {
            if self.can_fallback_to_chat()
                && Self::responses_endpoint_unsupported(response.status())
            {
                return self
                    .complete_stream_with_chat_api(&request, model.to_string())
                    .await;
            }
            return Err(utils::http_status_error_from_response(self.id.as_str(), response).await);
        }

        utils::ensure_response_content_type(self.id.as_str(), &response, "text/event-stream")?;
        let provider_name = self.id.clone();
        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(provider_name.as_str());
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ResponsesToolState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut completed_emitted = false;
            let mut response_id: Option<String> = None;

            while let Some(event) = events.next().await {
                let event = event?;

                if let Some(err) = utils::responses_stream_error(provider_name.as_str(), &event)? {
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

                if let Some(delta) = responses_reasoning_delta(&event) {
                    stream_has_content = true;
                    yield CompletionStreamEvent::ThinkingDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                if let Some(tool_event) = utils::responses_tool_event(provider_name.as_str(), &event)? {
                    let key = tool_event.stream_key(provider_name.as_str())?;

                    let is_added = matches!(tool_event.kind, utils::ResponsesToolEventKind::Added);
                    let was_new = !pending_tool_calls.contains_key(&key);
                    let state = pending_tool_calls.entry(key.clone()).or_default();
                    if let Some(id) = tool_event.id.clone() {
                        state.id = Some(id);
                    }
                    if let Some(name) = tool_event.name.clone() {
                        state.name = Some(name);
                    }

                    if is_added && was_new {
                        // Register the call with the aggregator so a
                        // parameterless tool call (no Delta events) is
                        // not silently dropped.
                        stream_has_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: key.clone(),
                            id: state.id.clone(),
                            name: state.name.clone(),
                            arguments_delta: String::new(),
                        };
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
                        provider_name.as_str(),
                        "responses stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Self::map_usage(Some(usage));
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
                    provider_metadata: response_id_metadata(response_id),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    #[serde(skip)]
    prompt_window_generation: Option<u64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<chat_wire::ChatResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

impl OpenAiResponsesRequest {
    #[cfg(test)]
    fn window_id_header(&self) -> Option<String> {
        self.prompt_cache_key.as_ref().map(|prompt_cache_key| {
            format!(
                "{}:{}",
                prompt_cache_key,
                self.prompt_window_generation.unwrap_or_default()
            )
        })
    }
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesTool {
    #[serde(rename = "type")]
    kind: &'static str,
    name: String,
    description: String,
    parameters: serde_json::Value,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    strict: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiInputMessage {
    role: String,
    content: Vec<OpenAiInputContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    copilot_cache_control: Option<prompt_cache::PromptCacheControl>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum OpenAiInputContent {
    #[serde(rename = "input_text")]
    Text { text: String },
    #[serde(rename = "input_image")]
    Image { image_url: String },
    #[serde(rename = "input_file")]
    File {
        #[serde(skip_serializing_if = "Option::is_none")]
        file_data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAiResponsesInputItem {
    Message(OpenAiInputMessage),
    FunctionCall(OpenAiFunctionCallItem),
    FunctionCallOutput(OpenAiFunctionCallOutputItem),
}

impl OpenAiResponsesInputItem {
    fn is_system(&self) -> bool {
        matches!(
            self,
            Self::Message(OpenAiInputMessage { role, .. }) if role == "system"
        )
    }

    fn set_copilot_cache_control(&mut self, cache_control: prompt_cache::PromptCacheControl) {
        match self {
            Self::Message(message) => message.copilot_cache_control = Some(cache_control),
            Self::FunctionCall(item) => item.copilot_cache_control = Some(cache_control),
            Self::FunctionCallOutput(item) => item.copilot_cache_control = Some(cache_control),
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionCallItem {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: String,
    name: String,
    arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    copilot_cache_control: Option<prompt_cache::PromptCacheControl>,
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionCallOutputItem {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: String,
    output: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    copilot_cache_control: Option<prompt_cache::PromptCacheControl>,
}

#[derive(Debug, Default)]
struct ResponsesToolState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAiModelListResponse {
    Wrapped { data: Vec<OpenAiModel> },
    Bare(Vec<OpenAiModel>),
}

impl OpenAiModelListResponse {
    fn into_items(self) -> Vec<OpenAiModel> {
        match self {
            Self::Wrapped { data } => data,
            Self::Bare(data) => data,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesResponse {
    #[serde(default)]
    id: Option<String>,
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

fn responses_reasoning_delta(event: &serde_json::Value) -> Option<String> {
    let event_type = event.get("type")?.as_str()?;
    if event_type == "response.reasoning_summary_text.delta"
        || event_type == "response.reasoning.delta"
    {
        return event
            .get("delta")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
    }
    None
}

fn apply_chat_prompt_cache_hints(messages: &mut [chat_wire::ChatMessage]) {
    let flags = messages
        .iter()
        .map(|message| message.role == "system")
        .collect::<Vec<_>>();
    for index in prompt_cache::select_cache_target_indices(flags.as_slice()) {
        if let Some(message) = messages.get_mut(index) {
            message.copilot_cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }
    }
}

fn apply_responses_prompt_cache_hints(input: &mut [OpenAiResponsesInputItem]) {
    let flags = input
        .iter()
        .map(OpenAiResponsesInputItem::is_system)
        .collect::<Vec<_>>();
    for index in prompt_cache::select_cache_target_indices(flags.as_slice()) {
        if let Some(item) = input.get_mut(index) {
            item.set_copilot_cache_control(prompt_cache::PromptCacheControl::ephemeral());
        }
    }
}

fn clear_responses_prompt_cache_hints(input: &mut [OpenAiResponsesInputItem]) {
    for item in input {
        match item {
            OpenAiResponsesInputItem::Message(message) => message.copilot_cache_control = None,
            OpenAiResponsesInputItem::FunctionCall(item) => item.copilot_cache_control = None,
            OpenAiResponsesInputItem::FunctionCallOutput(item) => item.copilot_cache_control = None,
        }
    }
}

fn build_realtime_input_text(messages: &[Message]) -> Option<String> {
    let normalized = messages
        .iter()
        .filter_map(|message| {
            let text = wire_message::project_text_lossy(message);
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

fn session_text_lossy(message: &Message, projected_parts: &[wire_message::WirePart]) -> String {
    if projected_parts.is_empty() {
        message.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(projected_parts)
    }
}

fn normalize_domain(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, OnceLock};

    use super::*;
    use futures_util::StreamExt;
    use tokio::net::TcpListener;

    use crate::message::{
        AttachmentItem, AttachmentKind, AttachmentSource, Message, PartContent, StructuredObject,
        TimeRange, ToolExecutionPart, ToolInvocation, ToolOutput,
    };
    use crate::model::ModelId;
    use crate::plugin::PluginToolDecl;
    use crate::plugin::registry::PluginEntry as RegistryPluginEntry;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: tests serialize env mutation through `env_lock()`.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                // SAFETY: tests serialize env mutation through `env_lock()`.
                unsafe {
                    std::env::set_var(self.key, previous);
                }
            } else {
                // SAFETY: tests serialize env mutation through `env_lock()`.
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn sample_tool_definition() -> RegistryPluginEntry {
        RegistryPluginEntry::new(
            "fixture",
            PluginToolDecl::new(
                "project_search",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            )
            .description("Search project files.")
            .tag(crate::plugin::sdk::ToolTag::ReadOnly)
            .concurrency_safe(true),
        )
    }

    fn sample_png_data_url() -> &'static str {
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO9W7tYAAAAASUVORK5CYII="
    }

    fn sample_pdf_data_url() -> &'static str {
        "data:application/pdf;base64,JVBERi0xLjQKJcOkw7zDtsOCCjEgMCBvYmoKPDwvVHlwZS9DYXRhbG9nPj4KZW5kb2JqCnRyYWlsZXIKPDwvUm9vdCAxIDAgUj4+CiUlRU9G"
    }

    fn tool_result_message_with_image(tool_call_id: &str) -> Message {
        let mut message = Message::prompt_parts(
            crate::role::Role::Tool,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 0,
                    invocation: ToolInvocation {
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

    fn tool_result_message_with_pdf(tool_call_id: &str) -> Message {
        let mut message = Message::prompt_parts(
            crate::role::Role::Tool,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 0,
                    invocation: ToolInvocation {
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
                    kind: AttachmentKind::Pdf,
                    mime: "application/pdf".to_owned(),
                    source: AttachmentSource::DataUrl {
                        url: sample_pdf_data_url().to_owned(),
                    },
                    filename: Some("report.pdf".to_owned()),
                    title: None,
                    size_bytes: Some(108),
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: Some(1),
                }]),
            ],
        );
        if let Some(part) = message.parts.first_mut() {
            part.operation_id = Some(tool_call_id.to_owned());
        }
        message
    }

    fn multi_tool_result_message(tool_call_ids: &[&str]) -> Message {
        let mut parts = Vec::new();
        for (index, _) in tool_call_ids.iter().enumerate() {
            parts.push(PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: index as i64,
                invocation: ToolInvocation {
                    name: format!("tool_{index}"),
                    input: StructuredObject::default(),
                },
                output_text: format!("{{\"result\":{index}}}"),
                blocks: Vec::new(),
                attachments: Vec::new(),
                details: ToolOutput::default(),
                lifecycle: TimeRange::default(),
            }));
        }

        let mut message = Message::prompt_parts(crate::role::Role::Tool, parts);
        for (index, tool_call_id) in tool_call_ids.iter().enumerate() {
            if let Some(part) = message.parts.get_mut(index) {
                part.operation_id = Some((*tool_call_id).to_owned());
            }
        }
        message
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
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
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
                model: ModelId::new("text-davinci-003"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
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
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
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
    async fn complete_responses_serializes_cache_fields_and_returns_response_id_metadata() {
        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .match_body(mockito::Matcher::Regex(
                "\\\"prompt_cache_key\\\":\\\"session-42\\\"".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"previous_response_id\\\":\\\"resp_prev\\\"".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "id": "resp_next",
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
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_string()),
                previous_response_id: Some("resp_prev".to_string()),
                prompt_window_generation: Some(3),
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
            .expect("responses request should include cache fields");

        assert_eq!(
            response
                .provider_metadata
                .as_ref()
                .and_then(|value| value.get("response_id"))
                .and_then(|value| value.as_str()),
            Some("resp_next")
        );
    }

    #[test]
    fn responses_request_builds_window_id_header() {
        let request = OpenAiResponsesRequest {
            model: "gpt-5.3-codex".to_owned(),
            input: Vec::new(),
            tools: vec![OpenAiResponsesTool {
                kind: "function",
                name: "project_search".to_owned(),
                description: "Search project files.".to_owned(),
                parameters: serde_json::json!({"type":"object"}),
                strict: false,
            }],
            max_output_tokens: Some(128),
            temperature: Some(0.2),
            prompt_cache_key: Some("session-42".to_owned()),
            previous_response_id: Some("resp_prev".to_owned()),
            prompt_window_generation: Some(4),
            stream: true,
            stop: None,
            top_p: None,
            seed: None,
            response_format: None,
            reasoning_effort: None,
        };

        let json = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(json["prompt_cache_key"], "session-42");
        assert_eq!(json["previous_response_id"], "resp_prev");
        assert_eq!(request.window_id_header().as_deref(), Some("session-42:4"));
    }

    #[test]
    fn github_copilot_profile_defaults_to_model_based_responses_selection() {
        let provider = OpenAiProvider::new_managed_with_id(
            "github-copilot::openai",
            reqwest::Client::new(),
            crate::provider::ManagedCredential::static_value("copilot bearer", "token"),
            "https://api.githubcopilot.com",
            "gpt-4o-mini",
        )
        .with_profile(OpenAiProfile::GithubCopilot)
        .with_api_mode(OpenAiApiMode::Responses)
        .with_api_mode_explicit(false);

        assert!(!provider.should_use_responses("gpt-4o-mini"));
        assert!(provider.should_use_responses("gpt-5"));
    }

    #[tokio::test]
    async fn github_copilot_profile_chat_request_includes_copilot_headers() {
        let mut server = mockito::Server::new_async().await;
        let _chat = server
            .mock("POST", "/chat/completions")
            .match_header("openai-intent", "conversation-edits")
            .match_header("x-initiator", "user")
            .match_header("copilot-vision-request", "true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "finish_reason": "stop",
                        "message": { "role": "assistant", "content": "ok" }
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiProvider::new_managed_with_id(
            "github-copilot::openai",
            reqwest::Client::new(),
            crate::provider::ManagedCredential::static_value("copilot bearer", "token"),
            server.url(),
            "gpt-4o-mini",
        )
        .with_profile(OpenAiProfile::GithubCopilot)
        .with_api_mode(OpenAiApiMode::Responses)
        .with_api_mode_explicit(false);

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: None,
                messages: vec![Message::prompt_parts(
                    crate::role::Role::User,
                    vec![crate::message::PartContent::attachments(vec![
                        AttachmentItem {
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
                        },
                    ])],
                )],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
            .expect("copilot chat request should succeed");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test]
    async fn complete_chatgpt_codex_backend_sends_account_and_window_headers() {
        let auth_data = Arc::new(tokio::sync::Mutex::new(
            crate::provider::auth::AuthData::OAuth {
                issuer: Some(crate::provider::auth::CredentialIssuer::OpenaiChatgpt),
                refresh: "refresh-token".to_owned(),
                access: "access-token".to_owned(),
                expires_at_ms: 4_102_444_800_000,
                account_id: Some("acct-123".to_owned()),
                enterprise_url: None,
            },
        ));

        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .match_header("chatgpt-account-id", "acct-123")
            .match_header("x-codex-window-id", "session-42:7")
            .match_header("originator", CHATGPT_CODEX_ORIGINATOR)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "id": "resp_next",
                    "model": "gpt-5.3-codex",
                    "output_text": "ok",
                    "stop_reason": "stop"
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiProvider::new_managed_with_id(
            "openai_chatgpt",
            reqwest::Client::new(),
            crate::provider::ManagedCredential::auth_data_shared(
                "openai_chatgpt api key",
                "openai_chatgpt",
                auth_data.clone(),
                crate::provider::AuthSecretSelector::AccessOrApiKey,
                crate::provider::AuthRefreshStrategy::OpenAiOAuth,
            ),
            server.url(),
            "gpt-5.3-codex",
        )
        .with_backend(OpenAiBackend::ChatgptCodex)
        .with_auth_data(auth_data);

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-5.3-codex"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_owned()),
                previous_response_id: Some("resp_prev".to_owned()),
                prompt_window_generation: Some(7),
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
            .expect("chatgpt codex completion should succeed");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test]
    async fn complete_responses_falls_back_to_stream_when_non_stream_payload_is_empty() {
        let mut server = mockito::Server::new_async().await;
        let _responses_empty = server
            .mock("POST", "/responses")
            .expect(1)
            .match_body(mockito::Matcher::Regex("\\\"stream\\\":false".to_owned()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "id": "resp_empty",
                    "model": "gpt-5",
                    "output": [],
                    "usage": {
                        "input_tokens": 4,
                        "output_tokens": 0
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;
        let _responses_stream = server
            .mock("POST", "/responses")
            .expect(1)
            .match_body(mockito::Matcher::Regex("\\\"stream\\\":true".to_owned()))
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(concat!(
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_stream\"}}\n\n",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"fallback \"}\n\n",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"stream response\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_stream\",\"stop_reason\":\"stop\",\"usage\":{\"input_tokens\":4,\"output_tokens\":2}}}\n\n",
                "data: [DONE]\n\n"
            ))
            .create_async()
            .await;

        let provider =
            OpenAiProvider::new(reqwest::Client::new(), "sk-test", server.url(), "gpt-5");

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
            .expect("empty responses payload should fall back to stream aggregation");

        assert_eq!(response.text, "fallback stream response");
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::Stop)
        ));
        let usage = response.usage.expect("usage should be present");
        assert_eq!(usage.input_tokens, 4);
        assert_eq!(usage.output_tokens, 2);
        assert_eq!(
            response
                .provider_metadata
                .as_ref()
                .and_then(|value| value.get("response_id"))
                .and_then(|value| value.as_str()),
            Some("resp_stream")
        );
    }

    #[tokio::test]
    async fn complete_rejects_html_response_body_for_json_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/html; charset=utf-8")
            .with_body("<html><body>not an api</body></html>")
            .create_async()
            .await;

        let provider =
            OpenAiProvider::new(reqwest::Client::new(), "sk-test", server.url(), "gpt-5");

        let err = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
            .expect_err("html payload should be rejected for json endpoint");

        assert!(matches!(
            err,
            AppError::Provider(message)
                if message.contains("unexpected content-type")
                    && message.contains("text/html")
                    && message.contains("application/json")
        ));
    }

    #[tokio::test]
    async fn complete_stream_rejects_html_response_body_for_sse_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/html; charset=utf-8")
            .with_body("<html><body>not an api</body></html>")
            .create_async()
            .await;

        let provider =
            OpenAiProvider::new(reqwest::Client::new(), "sk-test", server.url(), "gpt-5");

        let err = match provider
            .complete_stream(CompletionRequest {
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
        {
            Ok(_) => panic!("html payload should be rejected for sse endpoint"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            AppError::Provider(message)
                if message.contains("unexpected content-type")
                    && message.contains("text/html")
                    && message.contains("text/event-stream")
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
                model: ModelId::new("gpt-4.1-mini"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
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
    async fn complete_stream_chat_records_parameterless_tool_call() {
        // Chat Completions tool_calls deltas may contain only id+name
        // with empty arguments. Without a registration delta the
        // aggregator silently drops the call.
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_x\",\"type\":\"function\",\"function\":{\"name\":\"now\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        let _chat = server
            .mock("POST", "/chat/completions")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider = OpenAiProvider::new(
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: ModelId::new("gpt-4o-mini"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "what time")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
            .expect("stream should start");

        let mut tool_event_seen: Option<(String, String, String)> = None;
        let mut completed = false;
        while let Some(item) = stream.next().await {
            match item.expect("stream item should parse") {
                CompletionStreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                    ..
                } => {
                    let entry = tool_event_seen.get_or_insert((
                        id.clone().unwrap_or_default(),
                        name.clone().unwrap_or_default(),
                        String::new(),
                    ));
                    entry.2.push_str(arguments_delta.as_str());
                }
                CompletionStreamEvent::Completed { finish_reason, .. } => {
                    assert!(matches!(
                        finish_reason,
                        Some(CompletionFinishReason::ToolCalls)
                    ));
                    completed = true;
                }
                _ => {}
            }
        }

        let (id, name, args) = tool_event_seen.expect("tool call should be emitted");
        assert_eq!(id, "call_x");
        assert_eq!(name, "now");
        assert!(args.is_empty() || args == "{}");
        assert!(completed);
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
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_tool_result("call_1", "{\"ok\":true}")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
            .expect("responses request should include function_call_output");

        assert_eq!(response.text, "ok");
    }

    #[test]
    fn responses_input_encodes_tool_result_images_as_multimodal_function_output() {
        let request = CompletionRequest {
            model: ModelId::new("gpt-5"),
            system: None,
            messages: vec![tool_result_message_with_image("call_1")],
            tools: Vec::new(),
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,

            stop_sequences: Vec::new(),

            top_p: None,

            top_k: None,

            seed: None,

            thinking: None,

            response_format: None,
        };

        let input = OpenAiProvider::to_responses_input(&request);
        let json = serde_json::to_value(&input).expect("responses input should serialize");
        let items = json.as_array().expect("responses input should be an array");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "call_1");
        assert!(items[0]["output"].is_array());
        assert_eq!(items[0]["output"][0]["type"], "input_text");
        assert_eq!(items[0]["output"][0]["text"], "{\"ok\":true}");
        assert_eq!(items[0]["output"][1]["type"], "input_image");
        assert_eq!(items[0]["output"][1]["image_url"], sample_png_data_url());
    }

    #[test]
    fn responses_input_encodes_tool_result_files_as_input_file() {
        let request = CompletionRequest {
            model: ModelId::new("gpt-5"),
            system: None,
            messages: vec![tool_result_message_with_pdf("call_1")],
            tools: Vec::new(),
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,

            stop_sequences: Vec::new(),

            top_p: None,

            top_k: None,

            seed: None,

            thinking: None,

            response_format: None,
        };

        let input = OpenAiProvider::to_responses_input(&request);
        let json = serde_json::to_value(&input).expect("responses input should serialize");
        let items = json.as_array().expect("responses input should be an array");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "call_1");
        assert!(items[0]["output"].is_array());
        assert_eq!(items[0]["output"][0]["type"], "input_text");
        assert_eq!(items[0]["output"][0]["text"], "{\"ok\":true}");
        assert_eq!(items[0]["output"][1]["type"], "input_file");
        assert_eq!(items[0]["output"][1]["file_data"], sample_pdf_data_url());
        assert_eq!(items[0]["output"][1]["filename"], "report.pdf");
    }

    #[test]
    fn responses_input_preserves_assistant_part_order_around_tool_calls() {
        let mut assistant = Message::prompt_text(crate::role::Role::Assistant, "Before ");
        assistant.push_part(crate::message::MessagePart::with_content(
            2,
            assistant.id,
            assistant.created_at,
            crate::message::ExecutionStatus::Completed,
            crate::message::PartContent::ToolExecution(
                crate::message::ToolExecutionPart::Completed {
                    call_id: 1,
                    invocation: crate::message::ToolInvocation {
                        name: "search".to_owned(),
                        input: crate::message::StructuredObject::default(),
                    },
                    output_text: String::new(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: crate::message::ToolOutput::default(),
                    lifecycle: crate::message::TimeRange::default(),
                },
            ),
        ));
        if let Some(part) = assistant.parts.last_mut() {
            part.operation_id = Some("call_1".to_owned());
        }
        assistant.push_part(crate::message::MessagePart::with_content(
            3,
            assistant.id,
            assistant.created_at,
            crate::message::ExecutionStatus::Completed,
            crate::message::PartContent::text("After"),
        ));

        let request = CompletionRequest {
            model: ModelId::new("gpt-5"),
            system: None,
            messages: vec![assistant],
            tools: Vec::new(),
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,

            stop_sequences: Vec::new(),

            top_p: None,

            top_k: None,

            seed: None,

            thinking: None,

            response_format: None,
        };

        let input = OpenAiProvider::to_responses_input(&request);
        let json = serde_json::to_value(&input).expect("responses input should serialize");
        let items = json.as_array().expect("responses input should be an array");

        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["role"], "assistant");
        assert_eq!(items[0]["content"][0]["text"], "Before ");
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["call_id"], "call_1");
        assert_eq!(items[1]["name"], "search");
        assert_eq!(items[2]["role"], "assistant");
        assert_eq!(items[2]["content"][0]["text"], "After");
    }

    #[test]
    fn chat_messages_preserve_interleaved_tool_result_and_follow_up_order() {
        let mut message = Message::prompt_parts(
            crate::role::Role::Tool,
            vec![
                PartContent::text("Before"),
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 1,
                    invocation: ToolInvocation {
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
                    invocation: ToolInvocation {
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

        let messages = request_to_chat_messages(&CompletionRequest {
            model: ModelId::new("gpt-4o"),
            system: None,
            messages: vec![message],
            tools: Vec::new(),
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,

            stop_sequences: Vec::new(),

            top_p: None,

            top_k: None,

            seed: None,

            thinking: None,

            response_format: None,
        });
        let json = serde_json::to_value(&messages).expect("chat messages should serialize");
        let items = json.as_array().expect("chat messages should be an array");

        assert_eq!(items.len(), 5);
        assert_eq!(items[0]["role"], "user");
        assert_eq!(items[0]["content"][0]["text"], "Before");
        assert_eq!(items[1]["role"], "tool");
        assert_eq!(items[1]["tool_call_id"], "call_1");
        assert_eq!(items[1]["content"], "{\"result\":1}");
        assert_eq!(items[2]["role"], "user");
        assert_eq!(items[2]["content"][0]["text"], "Middle");
        assert_eq!(items[3]["role"], "tool");
        assert_eq!(items[3]["tool_call_id"], "call_2");
        assert_eq!(items[3]["content"], "{\"result\":2}");
        assert_eq!(items[4]["role"], "user");
        assert_eq!(items[4]["content"][0]["text"], "After");
    }

    #[test]
    fn responses_input_emits_all_tool_results_from_single_tool_message() {
        let request = CompletionRequest {
            model: ModelId::new("gpt-5"),
            system: None,
            messages: vec![multi_tool_result_message(&["call_1", "call_2"])],
            tools: Vec::new(),
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,

            stop_sequences: Vec::new(),

            top_p: None,

            top_k: None,

            seed: None,

            thinking: None,

            response_format: None,
        };

        let input = OpenAiProvider::to_responses_input(&request);
        let json = serde_json::to_value(&input).expect("responses input should serialize");
        let items = json.as_array().expect("responses input should be an array");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "call_1");
        assert_eq!(items[0]["output"], "{\"result\":0}");
        assert_eq!(items[1]["type"], "function_call_output");
        assert_eq!(items[1]["call_id"], "call_2");
        assert_eq!(items[1]["output"], "{\"result\":1}");
    }

    #[test]
    fn prompt_cache_shape_changes_when_auth_scope_changes() {
        let provider_a = OpenAiProvider::new_managed(
            reqwest::Client::new(),
            crate::provider::ManagedCredential::environment(
                "openai env",
                "openai",
                "api_key",
                "OPENAI_API_KEY_A",
            ),
            "https://api.openai.com/v1",
            "gpt-5",
        );
        let provider_b = OpenAiProvider::new_managed(
            reqwest::Client::new(),
            crate::provider::ManagedCredential::environment(
                "openai env",
                "openai",
                "api_key",
                "OPENAI_API_KEY_B",
            ),
            "https://api.openai.com/v1",
            "gpt-5",
        );

        let shape_a = provider_a
            .prompt_cache_shape(&ModelId::new("gpt-5"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&ModelId::new("gpt-5"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn prompt_cache_shape_changes_when_chatgpt_account_id_changes() {
        let auth_data_a = Arc::new(tokio::sync::Mutex::new(
            crate::provider::auth::AuthData::OAuth {
                issuer: Some(crate::provider::auth::CredentialIssuer::OpenaiChatgpt),
                refresh: "refresh-a".to_owned(),
                access: "access-a".to_owned(),
                expires_at_ms: 0,
                account_id: Some("acct-a".to_owned()),
                enterprise_url: None,
            },
        ));
        let provider_a = OpenAiProvider::new_managed_with_id(
            "openai_chatgpt",
            reqwest::Client::new(),
            crate::provider::ManagedCredential::auth_data_shared(
                "openai_chatgpt api key",
                "openai_chatgpt",
                auth_data_a.clone(),
                crate::provider::AuthSecretSelector::AccessOrApiKey,
                crate::provider::AuthRefreshStrategy::OpenAiOAuth,
            ),
            "https://chatgpt.com/backend-api/codex",
            "gpt-5.3-codex",
        )
        .with_backend(OpenAiBackend::ChatgptCodex)
        .with_auth_data(auth_data_a);

        let auth_data_b = Arc::new(tokio::sync::Mutex::new(
            crate::provider::auth::AuthData::OAuth {
                issuer: Some(crate::provider::auth::CredentialIssuer::OpenaiChatgpt),
                refresh: "refresh-b".to_owned(),
                access: "access-b".to_owned(),
                expires_at_ms: 0,
                account_id: Some("acct-b".to_owned()),
                enterprise_url: None,
            },
        ));
        let provider_b = OpenAiProvider::new_managed_with_id(
            "openai_chatgpt",
            reqwest::Client::new(),
            crate::provider::ManagedCredential::auth_data_shared(
                "openai_chatgpt api key",
                "openai_chatgpt",
                auth_data_b.clone(),
                crate::provider::AuthSecretSelector::AccessOrApiKey,
                crate::provider::AuthRefreshStrategy::OpenAiOAuth,
            ),
            "https://chatgpt.com/backend-api/codex",
            "gpt-5.3-codex",
        )
        .with_backend(OpenAiBackend::ChatgptCodex)
        .with_auth_data(auth_data_b);

        let shape_a = provider_a
            .prompt_cache_shape(&ModelId::new("gpt-5.3-codex"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&ModelId::new("gpt-5.3-codex"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
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
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
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
                CompletionStreamEvent::ThinkingDelta { .. } => {}
            }
        }

        assert_eq!(text, "Done");
        assert_eq!(tool, "part1part2");
        assert!(completed);
    }

    #[tokio::test]
    async fn complete_stream_responses_records_parameterless_tool_call() {
        // Regression: a tool call with no arguments emits no
        // function_call_arguments.delta events. Ensure the call is still
        // surfaced via the aggregator.
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call_x\",\"name\":\"now\",\"arguments\":\"\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.done\",\"output_index\":0,\"arguments\":\"\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"stop_reason\":\"tool_calls\",\"usage\":{\"input_tokens\":2,\"output_tokens\":1}}}\n\n",
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
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "what time")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
            .expect("stream should start");

        let mut tool_event_seen: Option<(String, String, String)> = None;
        let mut completed = false;
        while let Some(item) = stream.next().await {
            match item.expect("stream item should parse") {
                CompletionStreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                    ..
                } => {
                    let entry = tool_event_seen.get_or_insert((
                        id.clone().unwrap_or_default(),
                        name.clone().unwrap_or_default(),
                        String::new(),
                    ));
                    entry.2.push_str(arguments_delta.as_str());
                }
                CompletionStreamEvent::Completed { finish_reason, .. } => {
                    assert!(matches!(
                        finish_reason,
                        Some(CompletionFinishReason::ToolCalls)
                    ));
                    completed = true;
                }
                _ => {}
            }
        }

        let (id, name, args) = tool_event_seen.expect("tool call delta should be emitted");
        assert_eq!(id, "call_x");
        assert_eq!(name, "now");
        assert!(args.is_empty() || args == "{}");
        assert!(completed);
    }
    #[tokio::test]
    async fn complete_stream_responses_emits_response_id_metadata() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_stream\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_stream\",\"stop_reason\":\"stop\"}}\n\n",
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
                model: ModelId::new("gpt-5"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
                prompt_cache_key: Some("session-42".to_string()),
                previous_response_id: Some("resp_prev".to_string()),
                prompt_window_generation: Some(7),
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
            })
            .await
            .expect("responses stream should start");

        let mut completed_metadata = None;
        while let Some(item) = stream.next().await {
            if let CompletionStreamEvent::Completed {
                provider_metadata, ..
            } = item.expect("stream item should parse")
            {
                completed_metadata = provider_metadata;
            }
        }

        assert_eq!(
            completed_metadata
                .as_ref()
                .and_then(|value| value.get("response_id"))
                .and_then(|value| value.as_str()),
            Some("resp_stream")
        );
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
                model: ModelId::new("gpt-4o-realtime-preview"),
                system: Some("you are helpful".to_owned()),
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: vec![sample_tool_definition()],
                temperature: Some(0.2),
                max_output_tokens: Some(64),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
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
                CompletionStreamEvent::ThinkingDelta { .. } => {}
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

    #[tokio::test]
    async fn list_models_uses_disk_cache_after_first_fetch() {
        let _env_lock = env_lock().lock().expect("env lock should succeed");
        let dir = tempfile::tempdir().expect("tempdir should create");
        let _cache_dir =
            EnvVarGuard::set("AGENA_PROVIDER_MODELS_CACHE_DIR", dir.path().as_os_str());

        let mut server = mockito::Server::new_async().await;
        {
            let _mock = server
                .mock("GET", "/models")
                .match_header("authorization", "Bearer sk-test")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(
                    serde_json::json!({
                        "data": [{ "id": "gpt-5" }]
                    })
                    .to_string(),
                )
                .expect(1)
                .create_async()
                .await;

            let provider =
                OpenAiProvider::new(reqwest::Client::new(), "sk-test", server.url(), "gpt-5");
            let models = provider
                .list_models()
                .await
                .expect("initial list_models should succeed");
            assert_eq!(models.len(), 1);
            assert_eq!(models[0].id.as_str(), "gpt-5");
        }

        let provider =
            OpenAiProvider::new(reqwest::Client::new(), "sk-test", server.url(), "gpt-5");
        let models = provider
            .list_models()
            .await
            .expect("cached list_models should succeed");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id.as_str(), "gpt-5");
    }

    #[tokio::test]
    async fn chatgpt_codex_list_models_uses_live_models_endpoint() {
        let _env_lock = env_lock().lock().expect("env lock should succeed");
        let dir = tempfile::tempdir().expect("tempdir should create");
        let _cache_dir =
            EnvVarGuard::set("AGENA_PROVIDER_MODELS_CACHE_DIR", dir.path().as_os_str());

        let auth_data = Arc::new(tokio::sync::Mutex::new(
            crate::provider::auth::AuthData::OAuth {
                issuer: Some(crate::provider::auth::CredentialIssuer::OpenaiChatgpt),
                refresh: "refresh".to_owned(),
                access: "access-token".to_owned(),
                expires_at_ms: 0,
                account_id: Some("acct-123".to_owned()),
                enterprise_url: None,
            },
        ));

        let mut server = mockito::Server::new_async().await;
        let _models = server
            .mock("GET", "/models")
            .match_header("authorization", "Bearer access-token")
            .match_header("chatgpt-account-id", "acct-123")
            .match_header("originator", CHATGPT_CODEX_ORIGINATOR)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "data": [
                        { "id": "gpt-5.3-codex", "name": "GPT-5.3 Codex" },
                        { "id": "gpt-5-codex-mini", "name": "GPT-5 Codex Mini" }
                    ]
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;

        let provider = OpenAiProvider::new_managed_with_id(
            "openai_chatgpt",
            reqwest::Client::new(),
            crate::provider::ManagedCredential::auth_data_shared(
                "openai_chatgpt api key",
                "openai_chatgpt",
                auth_data.clone(),
                crate::provider::AuthSecretSelector::AccessOrApiKey,
                crate::provider::AuthRefreshStrategy::OpenAiOAuth,
            ),
            server.url(),
            "gpt-5.3-codex",
        )
        .with_backend(OpenAiBackend::ChatgptCodex)
        .with_auth_data(auth_data);

        let models = provider
            .list_models()
            .await
            .expect("chatgpt codex list_models should succeed");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id.as_str(), "gpt-5.3-codex");
        assert_eq!(models[0].display_name.as_deref(), Some("GPT-5.3 Codex"));
        assert_eq!(models[1].id.as_str(), "gpt-5-codex-mini");
        assert_eq!(models[1].display_name.as_deref(), Some("GPT-5 Codex Mini"));
    }
}
