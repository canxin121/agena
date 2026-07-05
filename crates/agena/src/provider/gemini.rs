use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    config::{ProviderNativeToolKind, ProviderNativeToolRoute},
    error::AppError,
    message::{AttachmentItem, Message, MessageUsage},
    model::{ModelId, ModelMetadata, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        ManagedCredential, ModelRuntime, ProviderModel, ReasoningEffort, ResponseFormat,
        ThinkingRequest, should_retry_credential, sse, utils, wire_message,
    },
    role::Role,
};

const PROVIDER_ID: &str = "google";
const ADAPTER_KIND: &str = "gemini";

#[derive(Clone)]
pub struct GeminiAdapter {
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    auth_mode: GeminiAuthMode,
    extra_headers: HashMap<String, String>,
    stream_mode: GeminiStreamMode,
    realtime_ws_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GeminiAuthMode {
    QueryParameter {
        name: String,
    },
    Header {
        name: String,
        scheme: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiStreamMode {
    Sse,
    RealtimeWebSocket,
}

impl GeminiAdapter {
    pub fn new(
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_managed(
            client,
            ManagedCredential::static_value("gemini api key", api_key.into()),
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
        let default_model = ModelId::new(default_model);
        let user_agent = crate::provider::gemini_cli_user_agent(default_model.as_str());
        Self {
            client,
            api_key,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model,
            auth_mode: GeminiAuthMode::Header {
                name: "x-goog-api-key".to_owned(),
                scheme: None,
            },
            extra_headers: HashMap::from([(
                reqwest::header::USER_AGENT.as_str().to_owned(),
                user_agent,
            )]),
            stream_mode: GeminiStreamMode::Sse,
            realtime_ws_url: None,
        }
    }

    pub fn with_auth_header(
        mut self,
        header: impl Into<String>,
        scheme: Option<impl Into<String>>,
    ) -> Self {
        self.auth_mode = GeminiAuthMode::Header {
            name: header.into(),
            scheme: scheme.map(|value| value.into()),
        };
        self
    }

    pub fn with_auth_query_parameter(mut self, name: impl Into<String>) -> Self {
        self.auth_mode = GeminiAuthMode::QueryParameter { name: name.into() };
        self
    }

    pub fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        if headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()))
        {
            self.extra_headers
                .retain(|key, _| !key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()));
        }
        self.extra_headers.extend(headers);
        self
    }

    pub fn with_stream_mode(mut self, mode: GeminiStreamMode) -> Self {
        self.stream_mode = mode;
        self
    }

    pub fn with_realtime_ws_url(mut self, ws_url: Option<String>) -> Self {
        self.realtime_ws_url = ws_url.and_then(|value| utils::normalize_optional_text(Some(value)));
        self
    }

    fn list_models_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn generate_endpoint(&self, model: &str) -> String {
        let model_name = if model.starts_with("models/") {
            model.to_owned()
        } else {
            format!("models/{model}")
        };
        format!("{}/{}:generateContent", self.base_url, model_name)
    }

    fn stream_generate_endpoint(&self, model: &str) -> String {
        let model_name = if model.starts_with("models/") {
            model.to_owned()
        } else {
            format!("models/{model}")
        };
        format!(
            "{}/{}:streamGenerateContent?alt=sse",
            self.base_url, model_name
        )
    }

    fn map_response_format(
        fmt: Option<&ResponseFormat>,
    ) -> (Option<String>, Option<serde_json::Value>) {
        match fmt {
            Some(ResponseFormat::JsonObject) => (Some("application/json".to_owned()), None),
            Some(ResponseFormat::JsonSchema { schema, .. }) => {
                (Some("application/json".to_owned()), Some(schema.clone()))
            }
            _ => (None, None),
        }
    }

    fn endpoint_with_auth(&self, endpoint: String, api_key: &str) -> String {
        match &self.auth_mode {
            GeminiAuthMode::QueryParameter { name } => {
                if let Ok(mut url) = url::Url::parse(endpoint.as_str()) {
                    url.query_pairs_mut().append_pair(name.as_str(), api_key);
                    return url.to_string();
                }

                let query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair(name.as_str(), api_key)
                    .finish();
                let separator = if endpoint.contains('?') { '&' } else { '?' };
                format!("{endpoint}{separator}{query}")
            }
            GeminiAuthMode::Header { .. } => endpoint,
        }
    }

    fn auth_transport_key(&self) -> &'static str {
        match self.auth_mode {
            GeminiAuthMode::QueryParameter { .. } => "query_parameter",
            GeminiAuthMode::Header { .. } => "header",
        }
    }

    fn stream_mode_key(&self) -> &'static str {
        match self.stream_mode {
            GeminiStreamMode::Sse => "sse",
            GeminiStreamMode::RealtimeWebSocket => "realtime_websocket",
        }
    }

    fn request_system_and_contents(
        request: &CompletionRequest,
    ) -> (Vec<String>, Vec<GeminiContent>) {
        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(system.clone());
        }

        let mut contents = Vec::new();
        for msg in &request.messages {
            match msg.role {
                Role::System => system_chunks.push(msg.as_text_lossy()),
                Role::Assistant => contents.extend(Self::assistant_contents(msg)),
                Role::User => contents.push(GeminiContent {
                    role: Some("user".to_owned()),
                    parts: Self::message_parts(msg),
                }),
                Role::Tool => contents.extend(Self::tool_contents(msg)),
            }
        }

        (system_chunks, contents)
    }

    fn generation_config(
        model: &str,
        request: &CompletionRequest,
        response_modalities: Option<Vec<String>>,
    ) -> GeminiGenerationConfig {
        let (response_mime_type, response_schema) =
            Self::map_response_format(request.response_format.as_ref());
        GeminiGenerationConfig {
            temperature: request.temperature,
            max_output_tokens: request.max_output_tokens,
            top_p: request.top_p,
            top_k: request.top_k,
            stop_sequences: request.stop_sequences.clone(),
            response_mime_type,
            response_schema,
            thinking_config: gemini_thinking_config(model, request.thinking.as_ref()),
            response_modalities,
        }
    }

    fn generate_request(
        &self,
        request: &CompletionRequest,
        stream: Option<bool>,
    ) -> Result<GeminiGenerateRequest, AppError> {
        let (system_chunks, contents) = Self::request_system_and_contents(request);
        Ok(GeminiGenerateRequest {
            system_instruction: (!system_chunks.is_empty()).then(|| GeminiInstruction {
                parts: vec![GeminiPart::text(system_chunks.join("\n\n"))],
            }),
            contents,
            generation_config: Self::generation_config(request.model.as_str(), request, None),
            stream,
            tools: build_gemini_tools(request)?,
            tool_config: None,
        })
    }

    fn request_contains_tool_results(request: &CompletionRequest) -> bool {
        request.messages.iter().any(|message| {
            wire_message::project(message)
                .iter()
                .any(|part| matches!(part, wire_message::WirePart::ToolResult { .. }))
        })
    }

    fn realtime_ws_endpoint(&self) -> Result<url::Url, AppError> {
        let mut endpoint = if let Some(ws_url) = self.realtime_ws_url.as_ref() {
            url::Url::parse(ws_url).map_err(|err| {
                AppError::Config(format!("gemini realtime websocket url is invalid: {err}"))
            })?
        } else {
            let mut url = url::Url::parse(self.base_url.as_str())
                .map_err(|err| AppError::Config(format!("gemini base url is invalid: {err}")))?;
            let path = url.path().trim_end_matches('/').trim();
            let segments = path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>();
            let Some(version) = segments
                .last()
                .copied()
                .filter(|segment| segment.starts_with('v'))
            else {
                return Err(AppError::Config(
                    "gemini realtime websocket endpoint could not be derived from base_url; configure `realtime_ws_url` explicitly".to_owned(),
                ));
            };
            let prefix = if segments.len() > 1 {
                format!("/{}", segments[..segments.len() - 1].join("/"))
            } else {
                String::new()
            };
            let realtime_path = format!(
                "{}/ws/google.ai.generativelanguage.{}.GenerativeService.BidiGenerateContent",
                prefix, version
            );
            url.set_path(realtime_path.as_str());
            url
        };

        match endpoint.scheme() {
            "http" => endpoint.set_scheme("ws").map_err(|_| {
                AppError::Config("gemini realtime websocket url is invalid".to_owned())
            })?,
            "https" => endpoint.set_scheme("wss").map_err(|_| {
                AppError::Config("gemini realtime websocket url is invalid".to_owned())
            })?,
            "ws" | "wss" => {}
            other => {
                return Err(AppError::Config(format!(
                    "gemini realtime websocket url has unsupported scheme `{other}`"
                )));
            }
        }

        Ok(endpoint)
    }

    fn realtime_handshake_request(
        &self,
        endpoint: &url::Url,
        api_key: &str,
        request_headers: &HashMap<String, String>,
    ) -> Result<http::Request<()>, AppError> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let endpoint = self.endpoint_with_auth(endpoint.to_string(), api_key);
        let mut request = endpoint.into_client_request().map_err(|err| {
            AppError::Config(format!(
                "gemini realtime websocket handshake invalid: {err}"
            ))
        })?;

        let uses_first_party_api_key_header = matches!(
            &self.auth_mode,
            GeminiAuthMode::Header { name, scheme }
                if name.eq_ignore_ascii_case("x-goog-api-key") && scheme.is_none()
        ) && request
            .uri()
            .host()
            .is_some_and(|host| host.eq_ignore_ascii_case("generativelanguage.googleapis.com"));

        if uses_first_party_api_key_header {
            let mut url = url::Url::parse(request.uri().to_string().as_str()).map_err(|err| {
                AppError::Config(format!(
                    "gemini realtime websocket handshake url is invalid: {err}"
                ))
            })?;
            url.query_pairs_mut().append_pair("key", api_key);
            *request.uri_mut() = url.to_string().parse().map_err(|err| {
                AppError::Config(format!(
                    "gemini realtime websocket handshake uri is invalid: {err}"
                ))
            })?;
        } else if let GeminiAuthMode::Header { name, scheme } = &self.auth_mode {
            let auth_header_name =
                http::header::HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
                    AppError::Config(format!("gemini auth header name is invalid: {err}"))
                })?;
            let auth_header_value = http::header::HeaderValue::from_str(
                utils::auth_header_value(scheme.as_deref(), api_key).as_str(),
            )
            .map_err(|err| {
                AppError::Config(format!("gemini auth header value is invalid: {err}"))
            })?;
            request
                .headers_mut()
                .insert(auth_header_name, auth_header_value);
        }

        for (key, value) in utils::resolved_request_headers(PROVIDER_ID, request_headers) {
            let header_name =
                http::header::HeaderName::from_bytes(key.as_bytes()).map_err(|err| {
                    AppError::Config(format!(
                        "gemini extra header name `{key}` is invalid: {err}"
                    ))
                })?;
            let header_value =
                http::header::HeaderValue::from_str(value.as_str()).map_err(|err| {
                    AppError::Config(format!(
                        "gemini extra header `{key}` value is invalid: {err}"
                    ))
                })?;
            request.headers_mut().insert(header_name, header_value);
        }

        Ok(request)
    }

    fn message_parts(message: &Message) -> Vec<GeminiPart> {
        let projected_parts = wire_message::project(message);
        Self::parts_from_projected_parts(message, projected_parts.as_slice())
    }

    fn parts_from_projected_parts(
        message: &Message,
        projected_parts: &[wire_message::WirePart],
    ) -> Vec<GeminiPart> {
        if projected_parts.is_empty() {
            let text = message.as_text_lossy();
            return if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![GeminiPart::text(text)]
            };
        }

        projected_parts
            .iter()
            .map(Self::part_from_wire_part)
            .collect()
    }

    fn part_from_wire_part(part: &wire_message::WirePart) -> GeminiPart {
        match part {
            wire_message::WirePart::Text { text } => GeminiPart::text(text.clone()),
            wire_message::WirePart::Attachment { item } => Self::attachment_part(item),
            wire_message::WirePart::ToolCall {
                name,
                arguments_json,
                ..
            } => GeminiPart::function_call(
                gemini_wire_tool_name(name),
                parse_json_or_object(arguments_json),
            ),
            wire_message::WirePart::ToolResult {
                tool_name,
                output_json,
                ..
            } => GeminiPart::function_response(
                gemini_tool_response_name(tool_name),
                parse_json_or_string_object(output_json),
            ),
        }
    }

    fn assistant_contents(message: &Message) -> Vec<GeminiContent> {
        let projected = wire_message::project(message);
        if !projected
            .iter()
            .any(|part| matches!(part, wire_message::WirePart::ToolResult { .. }))
        {
            return vec![GeminiContent {
                role: Some("model".to_owned()),
                parts: Self::parts_from_projected_parts(message, projected.as_slice()),
            }];
        }

        let mut contents = Vec::new();
        let mut buffered = Vec::<wire_message::WirePart>::new();
        for part in &projected {
            match part {
                wire_message::WirePart::ToolResult {
                    tool_name,
                    output_json,
                    ..
                } => {
                    Self::flush_model_content(message, &mut contents, &mut buffered);
                    contents.push(GeminiContent {
                        role: Some("user".to_owned()),
                        parts: vec![GeminiPart::function_response(
                            gemini_tool_response_name(tool_name),
                            parse_json_or_string_object(output_json),
                        )],
                    });
                }
                other => buffered.push(other.clone()),
            }
        }
        Self::flush_model_content(message, &mut contents, &mut buffered);

        contents
    }

    fn tool_contents(message: &Message) -> Vec<GeminiContent> {
        wire_message::project(message)
            .into_iter()
            .filter_map(|part| match part {
                wire_message::WirePart::ToolResult {
                    tool_name,
                    output_json,
                    ..
                } => Some(GeminiContent {
                    role: Some("user".to_owned()),
                    parts: vec![GeminiPart::function_response(
                        gemini_tool_response_name(tool_name.as_str()),
                        parse_json_or_string_object(output_json.as_str()),
                    )],
                }),
                _ => None,
            })
            .collect()
    }

    fn flush_model_content(
        message: &Message,
        contents: &mut Vec<GeminiContent>,
        buffered: &mut Vec<wire_message::WirePart>,
    ) {
        if buffered.is_empty() {
            return;
        }
        let parts = Self::parts_from_projected_parts(message, buffered.as_slice());
        buffered.clear();
        if parts.is_empty() {
            return;
        }
        contents.push(GeminiContent {
            role: Some("model".to_owned()),
            parts,
        });
    }

    fn attachment_part(item: &AttachmentItem) -> GeminiPart {
        wire_message::base64_with_mime(item)
            .map(|(mime_type, data)| GeminiPart::inline_data(mime_type, data))
            .unwrap_or_else(|| GeminiPart::text(wire_message::hint_text(item)))
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

    async fn complete_stream_with_realtime_ws(
        &self,
        request: &CompletionRequest,
        model: ModelId,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let ws_endpoint = self.realtime_ws_endpoint()?;
        let api_key = self.api_key.resolve().await?;
        let request_headers =
            utils::merged_request_headers(&self.extra_headers, &request.request_override.headers);
        let handshake =
            self.realtime_handshake_request(&ws_endpoint, api_key.as_str(), &request_headers)?;
        let (ws_stream, _) = tokio_tungstenite::connect_async(handshake)
            .await
            .map_err(|err| {
                AppError::Provider(format!("gemini realtime websocket connect failed: {err}"))
            })?;

        let (system_chunks, contents) = Self::request_system_and_contents(request);
        let live_request = GeminiLiveConversationRequest {
            setup: GeminiLiveSetup {
                model: if model.as_str().starts_with("models/") {
                    model.to_string()
                } else {
                    format!("models/{model}")
                },
                generation_config: Self::generation_config(
                    model.as_str(),
                    request,
                    Some(vec!["TEXT".to_owned()]),
                ),
                system_instruction: (!system_chunks.is_empty()).then(|| GeminiInstruction {
                    parts: vec![GeminiPart::text(system_chunks.join("\n\n"))],
                }),
                tools: build_gemini_tools(request)?,
            },
            client_content: GeminiLiveClientContent {
                turns: contents,
                turn_complete: Some(true),
            },
        };
        let live_json = utils::serialize_request_body_with_patch(
            &live_request,
            &request.request_override.body_patch,
        )?;
        let setup = live_json.get("setup").cloned().ok_or_else(|| {
            AppError::Config(
                "gemini realtime request patch removed `setup`; restore it or disable the patch"
                    .to_owned(),
            )
        })?;
        let client_content = live_json.get("clientContent").cloned().ok_or_else(|| {
            AppError::Config(
                "gemini realtime request patch removed `clientContent`; restore it or disable the patch"
                    .to_owned(),
            )
        })?;

        let provider_id = ProviderId::new(PROVIDER_ID);
        let model_name = model;

        let stream = async_stream::try_stream! {
            let (mut ws_writer, mut ws_reader) = ws_stream.split();

            ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({ "setup": setup }).to_string().into(),
                ))
                .await
                .map_err(|err| {
                    AppError::Provider(format!(
                        "gemini realtime websocket send setup failed: {err}"
                    ))
                })?;

            let mut setup_complete = false;
            while let Some(message) = ws_reader.next().await {
                let message = message.map_err(|err| {
                    AppError::Provider(format!(
                        "gemini realtime websocket receive failed before setup complete: {err}"
                    ))
                })?;
                let text = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        String::from_utf8(bytes.to_vec()).map_err(|err| {
                            AppError::Provider(format!(
                                "gemini realtime websocket setup message was not utf-8: {err}"
                            ))
                        })?
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    tokio_tungstenite::tungstenite::Message::Ping(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Frame(_) => continue,
                };

                let payload: GeminiLiveServerMessage = utils::parse_json_value(
                    PROVIDER_ID,
                    "realtime setup message",
                    serde_json::from_str::<serde_json::Value>(text.as_str()).map_err(|err| {
                        AppError::Provider(format!(
                            "gemini realtime websocket returned invalid setup json: {err}"
                        ))
                    })?,
                )?;
                if payload.setup_complete.is_some() {
                    setup_complete = true;
                    break;
                }
            }

            if !setup_complete {
                Err(AppError::Provider(
                    "gemini realtime websocket closed before setup completed".to_owned(),
                ))?;
            }

            ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({ "clientContent": client_content })
                        .to_string()
                        .into(),
                ))
                .await
                .map_err(|err| {
                    AppError::Provider(format!(
                        "gemini realtime websocket send clientContent failed: {err}"
                    ))
                })?;

            let mut emitted = String::new();
            let mut emitted_reasoning = String::new();
            let mut emitted_tool_calls = std::collections::BTreeSet::new();
            let mut saw_content = false;
            let mut tool_call_seen = false;
            let mut completed_emitted = false;
            let mut fallback_usage: Option<crate::provider::CompletionUsage> = None;
            let mut fallback_provider_metadata: Option<serde_json::Value> = None;

            while let Some(message) = ws_reader.next().await {
                let message = message.map_err(|err| {
                    AppError::Provider(format!(
                        "gemini realtime websocket receive failed: {err}"
                    ))
                })?;

                let text = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        String::from_utf8(bytes.to_vec()).map_err(|err| {
                            AppError::Provider(format!(
                                "gemini realtime websocket message was not utf-8: {err}"
                            ))
                        })?
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    tokio_tungstenite::tungstenite::Message::Ping(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Frame(_) => continue,
                };

                let payload: GeminiLiveServerMessage = utils::parse_json_value(
                    PROVIDER_ID,
                    "realtime stream message",
                    serde_json::from_str::<serde_json::Value>(text.as_str()).map_err(|err| {
                        AppError::Provider(format!(
                            "gemini realtime websocket returned invalid json: {err}"
                        ))
                    })?,
                )?;

                if let Some(usage) = payload.usage_metadata.clone().map(map_gemini_usage) {
                    fallback_usage = Some(usage);
                }

                if let Some(server_content) = payload.server_content {
                    if let Some(metadata) = server_content.provider_metadata() {
                        fallback_provider_metadata = Some(metadata);
                    }

                    if let Some(model_turn) = server_content.model_turn {
                        let full_reasoning =
                            gemini_reasoning_text_from_content(&model_turn).unwrap_or_default();
                        if full_reasoning.starts_with(emitted_reasoning.as_str()) {
                            let delta = full_reasoning[emitted_reasoning.len()..].to_owned();
                            if !delta.is_empty() {
                                emitted_reasoning = full_reasoning;
                                saw_content = true;
                                yield CompletionStreamEvent::ThinkingDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    delta,
                                };
                            }
                        } else if !full_reasoning.is_empty() {
                            emitted_reasoning = full_reasoning.clone();
                            saw_content = true;
                            yield CompletionStreamEvent::ThinkingDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta: full_reasoning,
                            };
                        }

                        let full_text = gemini_text_from_content(&model_turn);
                        if full_text.starts_with(emitted.as_str()) {
                            let delta = full_text[emitted.len()..].to_owned();
                            if !delta.is_empty() {
                                emitted = full_text;
                                saw_content = true;
                                yield CompletionStreamEvent::TextDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    delta,
                                };
                            }
                        } else if !full_text.is_empty() {
                            emitted = full_text.clone();
                            saw_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta: full_text,
                            };
                        }
                    }

                    if server_content.turn_complete.unwrap_or(false) {
                        completed_emitted = true;
                        yield CompletionStreamEvent::Completed {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            finish_reason: Some(if tool_call_seen {
                                CompletionFinishReason::ToolCalls
                            } else {
                                CompletionFinishReason::Stop
                            }),
                            usage: fallback_usage.clone(),
                            provider_metadata: fallback_provider_metadata.clone(),
                        };
                        break;
                    }
                }

                if let Some(tool_call) = payload.tool_call {
                    for call in tool_call.function_calls {
                        let dedupe_key = call
                            .id
                            .clone()
                            .unwrap_or_else(|| utils::request_shape_fingerprint(&call));
                        if !emitted_tool_calls.insert(dedupe_key.clone()) {
                            continue;
                        }

                        tool_call_seen = true;
                        saw_content = true;
                        let arguments_json = if call.args.is_null() {
                            "{}".to_owned()
                        } else {
                            serde_json::to_string(&call.args)
                                .unwrap_or_else(|_| "{}".to_owned())
                        };
                        let id = call.id.unwrap_or(dedupe_key);
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: id.clone(),
                            id: Some(id),
                            name: Some(call.name),
                            arguments_delta: arguments_json,
                        };
                    }
                }
            }

            let _ = ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Close(None))
                .await;

            if !completed_emitted {
                if saw_content || fallback_usage.is_some() || fallback_provider_metadata.is_some() {
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason: Some(if tool_call_seen {
                            CompletionFinishReason::ToolCalls
                        } else {
                            CompletionFinishReason::Stop
                        }),
                        usage: fallback_usage,
                        provider_metadata: fallback_provider_metadata,
                    };
                } else {
                    Err(AppError::Provider(
                        "gemini realtime websocket closed before returning any content".to_owned(),
                    ))?;
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn complete_by_aggregating_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let fallback_model = request.model.clone();
        let stream = ModelRuntime::complete_stream(self, request).await?;
        utils::aggregate_stream(PROVIDER_ID, fallback_model, stream).await
    }

    #[allow(dead_code)]
    fn completion_response_stream(
        response: CompletionResponse,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>> {
        let provider_id = response.provider_id.clone();
        let model = response.model.clone();
        let mut events = Vec::new();
        if !response.text.is_empty() {
            events.push(Ok(CompletionStreamEvent::TextDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: response.text,
            }));
        }
        if let Some(reasoning) = response.reasoning_text
            && !reasoning.is_empty()
        {
            events.push(Ok(CompletionStreamEvent::ThinkingDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: reasoning,
            }));
        }
        for call in response.tool_calls {
            let crate::provider::CompletionToolCall::Function {
                id,
                name,
                arguments_json,
            } = call;
            events.push(Ok(CompletionStreamEvent::ToolCallSnapshot {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key: id.clone(),
                id: Some(id),
                name: Some(name),
                arguments_json,
            }));
        }
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id,
            model,
            finish_reason: response.finish_reason,
            usage: response.usage,
            provider_metadata: response.provider_metadata,
        }));
        Box::pin(futures_util::stream::iter(events))
    }
}

#[async_trait]
impl ModelRuntime for GeminiAdapter {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(crate::provider::CapabilityFamily::Gemini)
    }

    fn validate_native_tools_request(
        &self,
        _adapter_id: Option<&crate::model::AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), AppError> {
        build_gemini_tools(request).map(|_| ())
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        Some(
            crate::provider::PromptCacheShape::new(PROVIDER_ID)
                .with_string("auth_scope", self.api_key.prompt_cache_scope())
                .with_string("base_url", self.base_url.as_str())
                .with_string("default_model", self.default_model.as_str())
                .with_string("auth_transport", self.auth_transport_key())
                .with_string("stream_mode", self.stream_mode_key())
                .with_optional_string(
                    "auth_query_param",
                    match &self.auth_mode {
                        GeminiAuthMode::QueryParameter { name } => Some(name.as_str()),
                        GeminiAuthMode::Header { .. } => None,
                    },
                )
                .with_optional_string(
                    "auth_header",
                    match &self.auth_mode {
                        GeminiAuthMode::Header { name, .. } => Some(name.as_str()),
                        GeminiAuthMode::QueryParameter { .. } => None,
                    },
                )
                .with_optional_string(
                    "auth_scheme",
                    match &self.auth_mode {
                        GeminiAuthMode::Header { scheme, .. } => scheme.as_deref(),
                        GeminiAuthMode::QueryParameter { .. } => None,
                    },
                )
                .with_optional_string("realtime_ws_url", self.realtime_ws_url.as_deref())
                .with_json(
                    "extra_headers",
                    &utils::prompt_cache_header_entries(&self.extra_headers),
                ),
        )
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let endpoint = self.list_models_endpoint();
        let response = self
            .send_request(|api_key| {
                let endpoint = self.endpoint_with_auth(endpoint.clone(), api_key);
                let mut headers = utils::resolved_request_headers(PROVIDER_ID, &self.extra_headers);
                if let GeminiAuthMode::Header { name, scheme } = &self.auth_mode {
                    headers.insert(
                        name.clone(),
                        utils::auth_header_value(scheme.as_deref(), api_key),
                    );
                }
                utils::adapter_log_http_request_json(
                    PROVIDER_ID,
                    ADAPTER_KIND,
                    "list_models",
                    "GET",
                    endpoint.as_str(),
                    headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                    None,
                );
                utils::apply_resolved_request_headers(self.client.get(endpoint), &headers)
            })
            .await?;

        let payload: GeminiModelListResponse =
            utils::parse_json_response_logged(PROVIDER_ID, ADAPTER_KIND, "list_models", response)
                .await?;
        Ok(payload
            .models
            .into_iter()
            .map(|m| {
                let metadata = m.metadata();
                let id = m.name.trim_start_matches("models/").to_owned();
                let mut model = ProviderModel::new(PROVIDER_ID, id);
                let capabilities = self.model_capabilities(&model.id);
                model = model.with_capabilities(capabilities);
                if !metadata.is_empty() {
                    model = model.with_metadata(metadata);
                }
                model.display_name = m.display_name;
                model
            })
            .collect())
    }

    #[tracing::instrument(skip_all, fields(provider = "gemini", model = %request.model))]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = request.model.clone();
        let stream_fallback_request = request.clone();
        let body = self.generate_request(&request, None)?;
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;
        let request_headers =
            utils::merged_request_headers(&self.extra_headers, &request.request_override.headers);

        let response = self
            .send_request(|api_key| {
                let endpoint =
                    self.endpoint_with_auth(self.generate_endpoint(model.as_str()), api_key);
                let mut headers = utils::resolved_request_headers(PROVIDER_ID, &request_headers);
                if let GeminiAuthMode::Header { name, scheme } = &self.auth_mode {
                    headers.insert(
                        name.clone(),
                        utils::auth_header_value(scheme.as_deref(), api_key),
                    );
                }
                headers.insert(
                    reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                    "application/json".to_owned(),
                );
                utils::adapter_log_http_request_json(
                    PROVIDER_ID,
                    ADAPTER_KIND,
                    "complete.generate_content",
                    "POST",
                    endpoint.as_str(),
                    headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                    Some(&body_json),
                );
                utils::apply_resolved_request_headers(self.client.post(endpoint), &headers)
                    .json(&body_json)
            })
            .await?;

        let payload: GeminiGenerateResponse = utils::parse_json_response_logged(
            PROVIDER_ID,
            ADAPTER_KIND,
            "complete.generate_content",
            response,
        )
        .await?;
        let candidate = payload.candidates.first();
        let text = candidate.map(GeminiCandidate::text).unwrap_or_default();
        let reasoning_text = candidate.and_then(GeminiCandidate::reasoning_text);
        let tool_calls = candidate
            .map(GeminiCandidate::function_calls)
            .unwrap_or_default();
        let finish_reason = CompletionFinishReason::from_provider(
            candidate.and_then(|c| c.finish_reason.as_deref()),
        );
        let usage = payload.usage_metadata.map(map_gemini_usage);

        if text.is_empty() && tool_calls.is_empty() && reasoning_text.is_none() {
            return self
                .complete_by_aggregating_stream(stream_fallback_request)
                .await;
        }

        Ok(CompletionResponse {
            provider_id: ProviderId::new(PROVIDER_ID),
            model,
            text,
            reasoning_text,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: payload
                .candidates
                .first()
                .and_then(GeminiCandidate::provider_metadata),
        })
    }

    #[tracing::instrument(skip_all, fields(provider = "gemini", model = %request.model))]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = request.model.clone();

        if matches!(self.stream_mode, GeminiStreamMode::RealtimeWebSocket)
            && !Self::request_contains_tool_results(&request)
        {
            return self
                .complete_stream_with_realtime_ws(&request, model.clone())
                .await;
        }

        let body = self.generate_request(&request, Some(true))?;
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;
        let request_headers =
            utils::merged_request_headers(&self.extra_headers, &request.request_override.headers);

        let response = self
            .send_request(|api_key| {
                let endpoint =
                    self.endpoint_with_auth(self.stream_generate_endpoint(model.as_str()), api_key);
                let mut headers = utils::resolved_request_headers(PROVIDER_ID, &request_headers);
                if let GeminiAuthMode::Header { name, scheme } = &self.auth_mode {
                    headers.insert(
                        name.clone(),
                        utils::auth_header_value(scheme.as_deref(), api_key),
                    );
                }
                headers.insert(
                    reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                    "application/json".to_owned(),
                );
                utils::adapter_log_http_request_json(
                    PROVIDER_ID,
                    ADAPTER_KIND,
                    "complete_stream.generate_content",
                    "POST",
                    endpoint.as_str(),
                    headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                    Some(&body_json),
                );
                utils::apply_resolved_request_headers(self.client.post(endpoint), &headers)
                    .json(&body_json)
            })
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response_logged(
                PROVIDER_ID,
                ADAPTER_KIND,
                "complete_stream.generate_content",
                response,
            )
            .await);
        }

        utils::adapter_log_http_response_open(
            PROVIDER_ID,
            ADAPTER_KIND,
            "complete_stream.generate_content",
            response.status(),
            response.headers(),
        );
        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(PROVIDER_ID);
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut emitted = String::new();
            let mut emitted_reasoning = String::new();
            let mut emitted_tool_calls: usize = 0;
            let mut saw_content = false;
            let mut fallback_usage: Option<crate::provider::CompletionUsage> = None;
            let mut fallback_provider_metadata: Option<serde_json::Value> = None;
            let mut completed_emitted = false;
            let mut tool_call_seen = false;

            while let Some(event) = events.next().await {
                let event = event?;
                utils::adapter_log_stream_event(
                    provider_id.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.generate_content",
                    &event,
                );

                let chunk: GeminiGenerateResponse =
                    utils::parse_json_value(provider_id.as_str(), "stream chunk", event)?;
                if let Some(usage) = chunk.usage_metadata.as_ref().cloned().map(map_gemini_usage) {
                    fallback_usage = Some(usage);
                }
                if let Some(metadata) = chunk
                    .candidates
                    .first()
                    .and_then(GeminiCandidate::provider_metadata)
                {
                    fallback_provider_metadata = Some(metadata);
                }
                let mut done = false;

                for stream_event in GeminiStreamEvent::from_chunk(
                    chunk,
                    &mut emitted,
                    &mut emitted_reasoning,
                    &mut emitted_tool_calls,
                ) {
                    match stream_event {
                        GeminiStreamEvent::TextDelta(delta) => {
                            saw_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta,
                            };
                        }
                        GeminiStreamEvent::ThinkingDelta(delta) => {
                            saw_content = true;
                            yield CompletionStreamEvent::ThinkingDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta,
                            };
                        }
                        GeminiStreamEvent::ToolCall(call) => {
                            saw_content = true;
                            tool_call_seen = true;
                            let crate::provider::CompletionToolCall::Function {
                                id, name, arguments_json,
                            } = call;
                            yield CompletionStreamEvent::ToolCallDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                stream_key: id.clone(),
                                id: Some(id),
                                name: Some(name),
                                arguments_delta: arguments_json,
                            };
                        }
                        GeminiStreamEvent::Completed {
                            finish_reason,
                            usage,
                            provider_metadata,
                        } => {
                            completed_emitted = true;
                            let resolved_finish_reason = CompletionFinishReason::from_provider(
                                Some(finish_reason.as_str()),
                            )
                            .or_else(|| {
                                tool_call_seen.then_some(CompletionFinishReason::ToolCalls)
                            });
                            yield CompletionStreamEvent::Completed {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                finish_reason: resolved_finish_reason,
                                usage: usage.or_else(|| fallback_usage.clone()),
                                provider_metadata: provider_metadata
                                    .or_else(|| fallback_provider_metadata.clone()),
                            };
                            done = true;
                            break;
                        }
                    }
                }

                if done {
                    break;
                }
            }

            if !completed_emitted
                && (saw_content || fallback_usage.is_some() || fallback_provider_metadata.is_some())
            {
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: tool_call_seen.then_some(CompletionFinishReason::ToolCalls),
                    usage: fallback_usage,
                    provider_metadata: fallback_provider_metadata,
                };
            }
        };

        Ok(Box::pin(stream))
    }
}

fn gemini_tool_response_name(tool_name: &str) -> String {
    let trimmed = tool_name.trim();
    if trimmed.is_empty() {
        "tool_result".to_owned()
    } else {
        gemini_wire_tool_name(trimmed)
    }
}

fn gemini_wire_tool_name(tool_name: &str) -> String {
    if gemini_native_tool_name_is_exact(tool_name) {
        tool_name.trim().to_owned()
    } else {
        crate::tool::model_safe_tool_name(tool_name)
    }
}

fn gemini_native_tool_name_is_exact(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return false;
    }
    let mut bytes = trimmed.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn gemini_thinking_config(
    model: &str,
    thinking: Option<&ThinkingRequest>,
) -> Option<GeminiThinkingConfig> {
    let thinking = thinking?;
    let normalized = model.to_ascii_lowercase();

    if normalized.contains("gemini-2.5") {
        let thinking_budget = match thinking {
            ThinkingRequest::Budget { budget_tokens } => Some(*budget_tokens),
            ThinkingRequest::Adaptive { effort, .. } => Some(match effort {
                Some(ReasoningEffort::Minimal) => 1_024,
                Some(ReasoningEffort::Low) => 4_096,
                Some(ReasoningEffort::Medium) => 10_240,
                Some(ReasoningEffort::High) | None => 16_384,
                Some(ReasoningEffort::Xhigh) | Some(ReasoningEffort::Max) => {
                    if normalized.contains("pro") && !normalized.contains("flash") {
                        32_768
                    } else {
                        24_576
                    }
                }
            }),
            ThinkingRequest::Effort { effort } => Some(match effort {
                ReasoningEffort::Minimal => 1_024,
                ReasoningEffort::Low => 4_096,
                ReasoningEffort::Medium => 10_240,
                ReasoningEffort::High => 16_384,
                ReasoningEffort::Xhigh | ReasoningEffort::Max => {
                    if normalized.contains("pro") && !normalized.contains("flash") {
                        32_768
                    } else {
                        24_576
                    }
                }
            }),
            ThinkingRequest::Disabled => Some(0),
        };
        return Some(GeminiThinkingConfig {
            thinking_budget,
            thinking_level: None,
            include_thoughts: Some(true),
        });
    }

    if normalized.contains("gemini-3") {
        let thinking_level = match thinking {
            ThinkingRequest::Budget { budget_tokens } => {
                if *budget_tokens == 0 {
                    None
                } else if *budget_tokens >= 12_000 {
                    Some("HIGH")
                } else {
                    Some("LOW")
                }
            }
            ThinkingRequest::Adaptive { effort, .. } => Some(match effort {
                Some(ReasoningEffort::High)
                | Some(ReasoningEffort::Xhigh)
                | Some(ReasoningEffort::Max)
                | None => "HIGH",
                Some(ReasoningEffort::Minimal)
                | Some(ReasoningEffort::Low)
                | Some(ReasoningEffort::Medium) => "LOW",
            }),
            ThinkingRequest::Effort { effort } => Some(match effort {
                ReasoningEffort::Minimal | ReasoningEffort::Low | ReasoningEffort::Medium => "LOW",
                ReasoningEffort::High | ReasoningEffort::Xhigh | ReasoningEffort::Max => "HIGH",
            }),
            ThinkingRequest::Disabled => None,
        };
        return Some(GeminiThinkingConfig {
            thinking_budget: None,
            thinking_level,
            include_thoughts: Some(true),
        });
    }

    None
}

#[derive(Debug, Serialize)]
struct GeminiGenerateRequest {
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiInstruction>,
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    tool_config: Option<GeminiToolConfig>,
}

#[derive(Debug, Serialize)]
struct GeminiInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct GeminiPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thought: Option<bool>,
    #[serde(
        default,
        rename = "inlineData",
        skip_serializing_if = "Option::is_none"
    )]
    inline_data: Option<GeminiInlineData>,
    #[serde(
        default,
        rename = "functionCall",
        skip_serializing_if = "Option::is_none"
    )]
    function_call: Option<GeminiFunctionCall>,
    #[serde(
        default,
        rename = "functionResponse",
        skip_serializing_if = "Option::is_none"
    )]
    function_response: Option<GeminiFunctionResponse>,
}

impl GeminiPart {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            ..Self::default()
        }
    }

    fn inline_data(mime_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            inline_data: Some(GeminiInlineData {
                mime_type: mime_type.into(),
                data: data.into(),
            }),
            ..Self::default()
        }
    }

    fn function_call(name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            function_call: Some(GeminiFunctionCall {
                id: None,
                name: name.into(),
                args,
            }),
            ..Self::default()
        }
    }

    fn function_response(name: impl Into<String>, response: serde_json::Value) -> Self {
        Self {
            function_response: Some(GeminiFunctionResponse {
                name: name.into(),
                response,
            }),
            ..Self::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct GeminiFunctionCall {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    args: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiFunctionResponse {
    name: String,
    #[serde(default)]
    response: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct GeminiToolConfig {
    #[serde(rename = "functionCallingConfig")]
    function_calling_config: GeminiFunctionCallingConfig,
}

#[derive(Debug, Serialize)]
struct GeminiFunctionCallingConfig {
    mode: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiInlineData {
    #[serde(rename = "mimeType")]
    mime_type: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(rename = "topK", skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(
        rename = "stopSequences",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    stop_sequences: Vec<String>,
    #[serde(rename = "responseMimeType", skip_serializing_if = "Option::is_none")]
    response_mime_type: Option<String>,
    #[serde(rename = "responseSchema", skip_serializing_if = "Option::is_none")]
    response_schema: Option<serde_json::Value>,
    #[serde(rename = "thinkingConfig", skip_serializing_if = "Option::is_none")]
    thinking_config: Option<GeminiThinkingConfig>,
    #[serde(rename = "responseModalities", skip_serializing_if = "Option::is_none")]
    response_modalities: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct GeminiThinkingConfig {
    #[serde(rename = "thinkingBudget", skip_serializing_if = "Option::is_none")]
    thinking_budget: Option<u32>,
    #[serde(rename = "thinkingLevel", skip_serializing_if = "Option::is_none")]
    thinking_level: Option<&'static str>,
    #[serde(rename = "includeThoughts", skip_serializing_if = "Option::is_none")]
    include_thoughts: Option<bool>,
}

#[derive(Debug, Serialize)]
struct GeminiLiveConversationRequest {
    setup: GeminiLiveSetup,
    #[serde(rename = "clientContent")]
    client_content: GeminiLiveClientContent,
}

#[derive(Debug, Serialize)]
struct GeminiLiveSetup {
    model: String,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize)]
struct GeminiLiveClientContent {
    turns: Vec<GeminiContent>,
    #[serde(rename = "turnComplete", skip_serializing_if = "Option::is_none")]
    turn_complete: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GeminiModelListResponse {
    #[serde(default)]
    models: Vec<GeminiModel>,
}

#[derive(Debug, Deserialize)]
struct GeminiModel {
    name: String,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default, rename = "inputTokenLimit")]
    input_token_limit: Option<u64>,
    #[serde(default, rename = "outputTokenLimit")]
    output_token_limit: Option<u64>,
}

impl GeminiModel {
    fn metadata(&self) -> ModelMetadata {
        let mut metadata = ModelMetadata::default();

        if let Some(input_token_limit) = self.input_token_limit {
            let limit = clamp_u64_to_u32(input_token_limit);
            // Gemini exposes input/output limits rather than a separate
            // context window field, so use the input ceiling as the best
            // available prompt-window budget.
            metadata = metadata
                .with_context_window_tokens(limit)
                .with_max_input_tokens(limit);
        }
        if let Some(output_token_limit) = self.output_token_limit {
            metadata = metadata.with_max_output_tokens(clamp_u64_to_u32(output_token_limit));
        }

        metadata
    }
}

#[derive(Debug, Deserialize)]
struct GeminiGenerateResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
    #[serde(default, rename = "safetyRatings")]
    safety_ratings: Option<serde_json::Value>,
    #[serde(default, rename = "groundingMetadata")]
    grounding_metadata: Option<serde_json::Value>,
}

impl GeminiCandidate {
    fn text(&self) -> String {
        self.content
            .as_ref()
            .map(gemini_text_from_content)
            .unwrap_or_default()
    }

    fn reasoning_text(&self) -> Option<String> {
        self.content
            .as_ref()
            .and_then(gemini_reasoning_text_from_content)
    }

    fn function_calls(&self) -> Vec<crate::provider::CompletionToolCall> {
        self.content
            .as_ref()
            .map(gemini_function_calls_from_content)
            .unwrap_or_default()
    }

    fn provider_metadata(&self) -> Option<serde_json::Value> {
        let mut map = serde_json::Map::new();
        if let Some(s) = self.safety_ratings.clone() {
            map.insert("safety_ratings".to_owned(), s);
        }
        if let Some(g) = self.grounding_metadata.clone() {
            map.insert("grounding_metadata".to_owned(), g);
        }
        (!map.is_empty()).then_some(serde_json::Value::Object(map))
    }
}

fn gemini_text_from_content(content: &GeminiContent) -> String {
    content
        .parts
        .iter()
        .filter(|part| !part.thought.unwrap_or(false))
        .filter_map(|part| part.text.clone())
        .collect::<Vec<_>>()
        .join("")
}

fn gemini_reasoning_text_from_content(content: &GeminiContent) -> Option<String> {
    let text = content
        .parts
        .iter()
        .filter(|part| part.thought.unwrap_or(false))
        .filter_map(|part| part.text.clone())
        .collect::<Vec<_>>()
        .join("");
    (!text.is_empty()).then_some(text)
}

fn gemini_function_calls_from_content(
    content: &GeminiContent,
) -> Vec<crate::provider::CompletionToolCall> {
    content
        .parts
        .iter()
        .enumerate()
        .filter_map(|(idx, part)| {
            let call = part.function_call.as_ref()?;
            let arguments_json = if call.args.is_null() {
                "{}".to_owned()
            } else {
                serde_json::to_string(&call.args).unwrap_or_else(|_| "{}".to_owned())
            };
            Some(crate::provider::CompletionToolCall::Function {
                id: call
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("{}-{idx}", call.name)),
                name: call.name.trim().to_owned(),
                arguments_json,
            })
        })
        .collect()
}

fn parse_json_or_object(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Object(Default::default());
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
}

/// Wrap non-JSON tool output in `{ "result": "<text>" }` because Gemini's
/// `functionResponse.response` field must be a JSON object.
fn parse_json_or_string_object(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Object(Default::default());
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value @ serde_json::Value::Object(_)) => value,
        Ok(other) => serde_json::json!({ "result": other }),
        Err(_) => serde_json::json!({ "result": raw }),
    }
}

/// Strip JSON Schema fields that Gemini's function declaration parser
/// rejects. Gemini accepts an OpenAPI 3.0 subset, so:
/// - drop meta-keys (`$schema`, `$ref`, `definitions`, `$defs`,
///   `additionalProperties`, `title`)
/// - drop unknown `format` values (Gemini accepts `enum` and `date-time`
///   for strings; `float`, `double`, `int32`, `int64` for numbers)
fn sanitize_function_parameters(value: &serde_json::Value) -> Option<serde_json::Value> {
    fn walk(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (key, child) in map {
                    if matches!(
                        key.as_str(),
                        "$schema"
                            | "$ref"
                            | "definitions"
                            | "$defs"
                            | "additionalProperties"
                            | "title"
                    ) {
                        continue;
                    }
                    if key == "format"
                        && !matches!(
                            child.as_str(),
                            Some("enum")
                                | Some("date-time")
                                | Some("float")
                                | Some("double")
                                | Some("int32")
                                | Some("int64")
                        )
                    {
                        continue;
                    }
                    out.insert(key.clone(), walk(child));
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(walk).collect())
            }
            other => other.clone(),
        }
    }

    let cleaned = walk(value);
    if matches!(cleaned, serde_json::Value::Object(ref m) if m.is_empty()) {
        None
    } else {
        Some(cleaned)
    }
}

fn merge_gemini_tool_provider_options(
    map: &mut serde_json::Map<String, serde_json::Value>,
    extra: Option<&serde_json::Value>,
    tool_label: &str,
) -> Result<(), AppError> {
    let Some(extra) = extra else {
        return Ok(());
    };
    let extra = extra.as_object().ok_or_else(|| {
        AppError::Config(format!(
            "gemini native tool `{tool_label}` provider_options must be a JSON object"
        ))
    })?;
    for (key, value) in extra {
        map.insert(key.clone(), value.clone());
    }
    Ok(())
}

fn build_gemini_tools(
    request: &CompletionRequest,
) -> Result<Option<Vec<serde_json::Value>>, AppError> {
    let native_bindings = request.native_tools.bindings();
    if native_bindings.is_empty() && request.tools.is_empty() {
        return Ok(None);
    }
    if !native_bindings.is_empty() && !request.tools.is_empty() {
        return Err(AppError::Config(
            "gemini native hosted tools cannot be combined with function tools in the current API; remove plugin tools or disable native hosted tools for this model".to_owned(),
        ));
    }

    let mut tools = Vec::new();
    if !request.tools.is_empty() {
        let function_declarations = request
            .tools
            .iter()
            .map(crate::tool::ModelToolSpec::from_registered_tool)
            .map(|tool| {
                Ok(GeminiFunctionDeclaration {
                    name: gemini_wire_tool_name(tool.model_name.as_str()),
                    description: tool.description,
                    parameters: sanitize_function_parameters(&tool.input_schema),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let mut map = serde_json::Map::new();
        map.insert(
            "functionDeclarations".to_owned(),
            serde_json::to_value(function_declarations)
                .expect("gemini function declarations should serialize"),
        );
        tools.push(serde_json::Value::Object(map));
    }

    for binding in native_bindings {
        if binding.route != ProviderNativeToolRoute::ProviderHosted {
            return Err(AppError::Config(format!(
                "gemini native tool `{}` only supports `provider_hosted` routes in the current runtime",
                binding.tool.config_key()
            )));
        }
        match binding.tool {
            ProviderNativeToolKind::WebSearch => {
                let config = &request.native_tools.hosted.web_search;
                if !config.allowed_domains.is_empty()
                    || !config.blocked_domains.is_empty()
                    || config.freshness.is_some()
                    || !config.user_location.is_empty()
                    || config.max_results.is_some()
                    || config.search_context_size.is_some()
                {
                    return Err(AppError::Config(
                        "gemini native tool `web_search` currently only supports `provider_options`; other hosted web_search fields are not implemented for Gemini".to_owned(),
                    ));
                }
                let mut map = serde_json::Map::new();
                map.insert(
                    "googleSearch".to_owned(),
                    serde_json::Value::Object(Default::default()),
                );
                merge_gemini_tool_provider_options(
                    &mut map,
                    config.provider_options.as_ref(),
                    "web_search",
                )?;
                tools.push(serde_json::Value::Object(map));
            }
            ProviderNativeToolKind::UrlContext => {
                let config = &request.native_tools.hosted.url_context;
                let mut inner = serde_json::Map::new();
                if let Some(max_urls) = config.max_urls {
                    inner.insert(
                        "maxUrls".to_owned(),
                        serde_json::Value::Number(max_urls.into()),
                    );
                }
                let mut map = serde_json::Map::new();
                map.insert("urlContext".to_owned(), serde_json::Value::Object(inner));
                merge_gemini_tool_provider_options(
                    &mut map,
                    config.provider_options.as_ref(),
                    "url_context",
                )?;
                tools.push(serde_json::Value::Object(map));
            }
            ProviderNativeToolKind::CodeExecution => {
                let config = &request.native_tools.hosted.code_execution;
                if !config.container.is_empty() {
                    return Err(AppError::Config(
                        "gemini native tool `code_execution` does not support `hosted.code_execution.container`; use `provider_options` for Gemini-specific overrides instead".to_owned(),
                    ));
                }
                let mut map = serde_json::Map::new();
                map.insert(
                    "codeExecution".to_owned(),
                    serde_json::Value::Object(Default::default()),
                );
                merge_gemini_tool_provider_options(
                    &mut map,
                    config.provider_options.as_ref(),
                    "code_execution",
                )?;
                tools.push(serde_json::Value::Object(map));
            }
            other => {
                return Err(AppError::Config(format!(
                    "gemini native tool `{}` is not supported by the current runtime",
                    other.config_key()
                )));
            }
        }
    }

    Ok(Some(tools))
}

#[allow(dead_code)]
fn build_gemini_native_tools_only(
    request: &CompletionRequest,
) -> Result<Option<Vec<serde_json::Value>>, AppError> {
    let mut native_only = request.clone();
    native_only.tools.clear();
    build_gemini_tools(&native_only)
}

fn map_gemini_usage(u: GeminiUsageMetadata) -> crate::provider::CompletionUsage {
    let prompt_tokens = u.prompt_token_count.unwrap_or_default();
    let cache_read_tokens = u.cached_content_token_count.unwrap_or_default();
    let reasoning_tokens = u.thoughts_token_count.unwrap_or_default();
    // Gemini's `promptTokenCount` is inclusive of cached tokens; the rest
    // of the codebase follows Anthropic's convention where `input_tokens`
    // is just the uncached portion. Subtract to match.
    let input_tokens = prompt_tokens.saturating_sub(cache_read_tokens);
    let output_tokens = u
        .candidates_token_count
        .unwrap_or_default()
        .saturating_sub(reasoning_tokens);
    MessageUsage {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_write_tokens: 0,
        cache_read_tokens,
        total_cost: 0.0,
    }
    .into()
}

#[derive(Debug)]
enum GeminiStreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCall(crate::provider::CompletionToolCall),
    Completed {
        finish_reason: String,
        usage: Option<crate::provider::CompletionUsage>,
        provider_metadata: Option<serde_json::Value>,
    },
}

impl GeminiStreamEvent {
    fn from_chunk(
        chunk: GeminiGenerateResponse,
        emitted: &mut String,
        emitted_reasoning: &mut String,
        emitted_tool_calls: &mut usize,
    ) -> Vec<Self> {
        let mut events = Vec::new();
        let candidate = chunk.candidates.first();

        if let Some(candidate) = candidate {
            let full_reasoning = candidate.reasoning_text().unwrap_or_default();
            if full_reasoning.starts_with(emitted_reasoning.as_str()) {
                let delta = full_reasoning[emitted_reasoning.len()..].to_owned();
                if !delta.is_empty() {
                    *emitted_reasoning = full_reasoning;
                    events.push(Self::ThinkingDelta(delta));
                }
            } else if !full_reasoning.is_empty() {
                *emitted_reasoning = full_reasoning.clone();
                events.push(Self::ThinkingDelta(full_reasoning));
            }

            let full_text = candidate.text();
            if full_text.starts_with(emitted.as_str()) {
                let delta = full_text[emitted.len()..].to_owned();
                if !delta.is_empty() {
                    *emitted = full_text;
                    events.push(Self::TextDelta(delta));
                }
            } else if !full_text.is_empty() {
                *emitted = full_text.clone();
                events.push(Self::TextDelta(full_text));
            }

            // Gemini streams emit each functionCall as a complete part rather
            // than incremental JSON deltas, so we forward them once apiece.
            let calls = candidate.function_calls();
            if calls.len() > *emitted_tool_calls {
                for call in calls.into_iter().skip(*emitted_tool_calls) {
                    events.push(Self::ToolCall(call));
                    *emitted_tool_calls += 1;
                }
            }

            if let Some(finish_reason) = candidate.finish_reason.clone() {
                events.push(Self::Completed {
                    finish_reason,
                    usage: chunk.usage_metadata.map(map_gemini_usage),
                    provider_metadata: candidate.provider_metadata(),
                });
            }
        }

        events
    }
}

#[derive(Debug, Deserialize, Clone)]
struct GeminiUsageMetadata {
    #[serde(default, rename = "promptTokenCount")]
    prompt_token_count: Option<u64>,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_token_count: Option<u64>,
    #[serde(default, rename = "thoughtsTokenCount")]
    thoughts_token_count: Option<u64>,
    #[serde(default, rename = "cachedContentTokenCount")]
    cached_content_token_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GeminiLiveServerMessage {
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default, rename = "setupComplete")]
    setup_complete: Option<serde_json::Value>,
    #[serde(default, rename = "serverContent")]
    server_content: Option<GeminiLiveServerContent>,
    #[serde(default, rename = "toolCall")]
    tool_call: Option<GeminiLiveToolCall>,
}

#[derive(Debug, Deserialize)]
struct GeminiLiveServerContent {
    #[serde(default, rename = "turnComplete")]
    turn_complete: Option<bool>,
    #[serde(default, rename = "groundingMetadata")]
    grounding_metadata: Option<serde_json::Value>,
    #[serde(default, rename = "modelTurn")]
    model_turn: Option<GeminiContent>,
}

impl GeminiLiveServerContent {
    fn provider_metadata(&self) -> Option<serde_json::Value> {
        self.grounding_metadata
            .clone()
            .map(|metadata| serde_json::json!({ "grounding_metadata": metadata }))
    }
}

#[derive(Debug, Deserialize)]
struct GeminiLiveToolCall {
    #[serde(default, rename = "functionCalls")]
    function_calls: Vec<GeminiFunctionCall>,
}

fn clamp_u64_to_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_completion_request(messages: Vec<Message>) -> CompletionRequest {
        CompletionRequest {
            model: ModelId::new("gemini-2.5-pro"),
            system: None,
            messages,
            tools: Vec::new(),
            native_tools: crate::config::ProviderNativeToolsConfig::default(),
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
            verbosity: None,
            response_format: None,
            responses_api_metadata: None,
            request_override: crate::model::ModelSpeedModeRequestOverride::default(),
        }
    }

    fn test_tool(plugin_name: &str, tool_name: &str) -> crate::plugin::registry::RegisteredTool {
        crate::plugin::registry::RegisteredTool::new(
            plugin_name,
            crate::plugin::sdk::PluginToolDecl::new(
                tool_name,
                serde_json::json!({
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"]
                }),
            )
            .description("Run the test tool."),
        )
    }

    #[test]
    fn gemini_model_parses_token_limits_from_list_models() {
        let payload = r#"{
            "models": [
                {
                    "name": "models/gemini-2.5-pro",
                    "displayName": "Gemini 2.5 Pro",
                    "inputTokenLimit": 1048576,
                    "outputTokenLimit": 65536
                }
            ]
        }"#;

        let parsed: GeminiModelListResponse =
            serde_json::from_str(payload).expect("parse gemini model list");
        assert_eq!(parsed.models.len(), 1);

        let metadata = parsed.models[0].metadata();
        assert_eq!(metadata.limits.context_window_tokens, Some(1_048_576));
        assert_eq!(metadata.limits.max_input_tokens, Some(1_048_576));
        assert_eq!(metadata.limits.max_output_tokens, Some(65_536));
    }

    #[test]
    fn gemini_defaults_use_x_goog_api_key_header_and_official_user_agent() {
        let adapter = GeminiAdapter::new_managed(
            reqwest::Client::new(),
            ManagedCredential::static_value("gemini api key", "test".to_owned()),
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-pro",
        )
        .with_extra_headers(std::collections::HashMap::from([(
            "x-test".to_owned(),
            "1".to_owned(),
        )]));

        assert!(matches!(
            &adapter.auth_mode,
            GeminiAuthMode::Header { name, scheme }
                if name == "x-goog-api-key" && scheme.is_none()
        ));
        assert!(
            adapter
                .extra_headers
                .get(reqwest::header::USER_AGENT.as_str())
                .is_some_and(|value| value.starts_with(
                    format!(
                        "{}/gemini-2.5-pro",
                        crate::provider::GEMINI_CLI_USER_AGENT_PREFIX
                    )
                    .as_str()
                ))
        );
        assert_eq!(
            adapter.extra_headers.get("x-test").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn gemini_realtime_first_party_ws_uses_key_query_parameter() {
        let adapter = GeminiAdapter::new_managed(
            reqwest::Client::new(),
            ManagedCredential::static_value("gemini api key", "test".to_owned()),
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-pro",
        );

        let endpoint = adapter
            .realtime_ws_endpoint()
            .expect("realtime websocket endpoint");
        let request = adapter
            .realtime_handshake_request(&endpoint, "test", &adapter.extra_headers)
            .expect("realtime handshake request");

        assert!(
            request.uri().to_string().contains("key=test"),
            "expected api key query parameter in {}",
            request.uri()
        );
        assert!(
            request.headers().get("x-goog-api-key").is_none(),
            "official realtime websocket should not send x-goog-api-key header"
        );
    }

    #[test]
    fn gemini_native_tool_name_rules_accept_dotted_subcommands() {
        assert!(gemini_native_tool_name_is_exact("web.run"));
        assert!(gemini_native_tool_name_is_exact(
            "streaming-fixture.stream_fixture.count"
        ));
        assert!(!gemini_native_tool_name_is_exact("mcp:docs.search"));

        assert!(!gemini_native_tool_name_is_exact("1bad.run"));
        assert!(!gemini_native_tool_name_is_exact(".bad"));
        assert!(!gemini_native_tool_name_is_exact("bad/slash"));
        assert!(!gemini_native_tool_name_is_exact(&format!(
            "tool.{}",
            "x".repeat(65)
        )));
    }

    #[test]
    fn gemini_native_tools_keep_multi_dot_names_when_provider_valid() {
        let adapter = GeminiAdapter::new(
            reqwest::Client::new(),
            "test-token",
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-pro",
        );
        let mut request = test_completion_request(vec![Message::prompt_text(Role::User, "hi")]);
        request.tools = vec![test_tool("streaming-fixture", "stream_fixture.count")];

        let body = adapter
            .generate_request(&request, None)
            .expect("gemini request should build");
        let tools = body.tools.expect("native Gemini tools should be present");

        assert_eq!(
            tools[0]["functionDeclarations"][0]["name"],
            serde_json::json!("streaming-fixture.stream_fixture.count")
        );
        assert!(body.system_instruction.is_none());
    }

    #[test]
    fn gemini_aliases_invalid_native_names_in_function_declarations() {
        let adapter = GeminiAdapter::new(
            reqwest::Client::new(),
            "test-token",
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-pro",
        );
        let mut request = test_completion_request(vec![Message::prompt_text(Role::User, "hi")]);
        request.tools = vec![test_tool(
            "fixture",
            "tool_name_that_is_long_enough_to_exceed_the_gemini_native_limit",
        )];
        let model_name = request.tools[0].model_name.clone();

        let body = adapter
            .generate_request(&request, None)
            .expect("gemini request should build");

        assert!(model_name.len() > 64);
        let tools = body.tools.expect("gemini tools should be present");
        assert_eq!(
            tools[0]["functionDeclarations"][0]["name"],
            serde_json::json!(crate::tool::model_safe_tool_name(model_name.as_str()))
        );
        assert!(body.system_instruction.is_none());
    }

    #[test]
    fn gemini_aliases_names_with_colons_in_function_declarations() {
        let adapter = GeminiAdapter::new(
            reqwest::Client::new(),
            "test-token",
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-pro",
        );
        let mut request = test_completion_request(vec![Message::prompt_text(Role::User, "hi")]);
        let mut tool = test_tool("fixture", "search");
        tool.model_name = "mcp:docs.search".to_owned();
        request.tools = vec![tool];

        let body = adapter
            .generate_request(&request, None)
            .expect("gemini request should build");

        let tools = body.tools.expect("gemini tools should be present");
        assert_eq!(
            tools[0]["functionDeclarations"][0]["name"],
            serde_json::json!(crate::tool::model_safe_tool_name("mcp:docs.search"))
        );
        assert!(body.system_instruction.is_none());
    }
}
