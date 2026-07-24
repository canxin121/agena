use agena_provider::{CompletionFinishReason, CompletionUsage};
use futures_util::{SinkExt, StreamExt};

use super::{
    AttachmentItem, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    GEMINI_FINAL_PART_SIGNATURE_KEY, GeminiAdapter, GeminiAdapterOptions, GeminiAuthMode,
    GeminiContent, GeminiGenerateRequest, GeminiGenerationConfig, GeminiInstruction,
    GeminiLiveClientContent, GeminiLiveConversationRequest, GeminiLiveServerMessage,
    GeminiLiveSetup, GeminiPart, GeminiStreamMode, HashMap, ManagedCredential, ModelId,
    ModelRuntime, PROVIDER_ID, ProviderError, ProviderId, ResponseFormat, Role, Stream,
    build_gemini_tools, gemini_live_server_content_provider_metadata,
    gemini_reasoning_text_from_content, gemini_text_from_content, gemini_thinking_config,
    gemini_tool_call_arguments_json, gemini_tool_response_name, gemini_usage_to_completion,
    gemini_wire_tool_name, merge_gemini_provider_metadata, parse_json_or_object,
    parse_json_or_string_object, should_retry_credential, utils, wire_message,
};

impl GeminiAdapter {
    pub fn new_managed_with_options(
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        options: GeminiAdapterOptions,
    ) -> Self {
        let default_model = ModelId::new(default_model);
        let user_agent = crate::gemini_cli_user_agent(default_model.as_ref());
        let mut extra_headers =
            HashMap::from([(reqwest::header::USER_AGENT.as_str().to_owned(), user_agent)]);
        if options
            .extra_headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()))
        {
            extra_headers
                .retain(|key, _| !key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()));
        }
        extra_headers.extend(options.extra_headers);
        let auth_mode = if let Some(name) = options.auth_query_parameter {
            GeminiAuthMode::QueryParameter { name }
        } else if let Some((name, scheme)) = options.auth_header {
            GeminiAuthMode::Header { name, scheme }
        } else {
            GeminiAuthMode::Header {
                name: "x-goog-api-key".to_owned(),
                scheme: None,
            }
        };
        Self {
            client,
            api_key,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model,
            auth_mode,
            extra_headers,
            stream_mode: options.stream_mode,
            realtime_ws_url: options
                .realtime_ws_url
                .and_then(|value| utils::normalize_optional_text(Some(value))),
        }
    }

    pub(super) fn list_models_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    pub(super) fn generate_endpoint(&self, model: &str) -> String {
        let model_name = if model.starts_with("models/") {
            model.to_owned()
        } else {
            format!("models/{model}")
        };
        format!("{}/{}:generateContent", self.base_url, model_name)
    }

    pub(super) fn stream_generate_endpoint(&self, model: &str) -> String {
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

    pub(super) fn map_response_format(
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

    pub(super) fn endpoint_with_auth(&self, endpoint: String, api_key: &str) -> String {
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

    pub(super) fn auth_transport_key(&self) -> &'static str {
        match self.auth_mode {
            GeminiAuthMode::QueryParameter { .. } => "query_parameter",
            GeminiAuthMode::Header { .. } => "header",
        }
    }

    pub(super) fn stream_mode_key(&self) -> &'static str {
        match self.stream_mode {
            GeminiStreamMode::Sse => "sse",
            GeminiStreamMode::RealtimeWebSocket => "realtime_websocket",
        }
    }

    pub(super) fn completion_request_headers(
        &self,
        request: &CompletionRequest,
    ) -> HashMap<String, String> {
        let mut headers =
            utils::merged_request_headers(&self.extra_headers, &request.request_override.headers);
        let default_user_agent = crate::gemini_cli_user_agent(self.default_model.as_ref());
        let generated_user_agent = headers.iter().find_map(|(key, value)| {
            (key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str())
                && value == &default_user_agent)
                .then(|| key.clone())
        });
        if let Some(key) = generated_user_agent {
            headers.insert(key, crate::gemini_cli_user_agent(request.model.as_ref()));
        }
        headers
    }

    pub(super) fn request_system_and_contents(
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
                Role::Assistant => {
                    for content in Self::assistant_contents(msg) {
                        Self::push_content(&mut contents, content);
                    }
                }
                Role::User => Self::push_content(
                    &mut contents,
                    GeminiContent {
                        role: Some("user".to_owned()),
                        parts: Self::message_parts(msg),
                    },
                ),
                Role::Tool => {
                    for content in Self::tool_contents(msg) {
                        Self::push_content(&mut contents, content);
                    }
                }
            }
        }

        (system_chunks, contents)
    }

    pub(super) fn push_content(contents: &mut Vec<GeminiContent>, mut content: GeminiContent) {
        if content.parts.is_empty() {
            return;
        }
        if let Some(previous) = contents.last_mut()
            && previous.role == content.role
        {
            previous.parts.append(&mut content.parts);
            return;
        }
        contents.push(content);
    }

    pub(super) fn generation_config(
        model: &str,
        request: &CompletionRequest,
        response_modalities: Option<Vec<String>>,
    ) -> GeminiGenerationConfig {
        let (response_mime_type, response_json_schema) =
            Self::map_response_format(request.response_format.as_ref());
        GeminiGenerationConfig {
            temperature: request.temperature,
            max_output_tokens: request.max_output_tokens,
            top_p: request.top_p,
            top_k: request.top_k,
            stop_sequences: request.stop_sequences.clone(),
            response_mime_type,
            response_json_schema,
            thinking_config: gemini_thinking_config(model, request.thinking.as_ref()),
            response_modalities,
        }
    }

    pub(super) fn generate_request(
        &self,
        request: &CompletionRequest,
    ) -> Result<GeminiGenerateRequest, ProviderError> {
        let (system_chunks, contents) = Self::request_system_and_contents(request);
        Ok(GeminiGenerateRequest {
            system_instruction: (!system_chunks.is_empty()).then(|| GeminiInstruction {
                parts: vec![GeminiPart::text(system_chunks.join("\n\n"))],
            }),
            contents,
            generation_config: Self::generation_config(request.model.as_ref(), request, None),
            tools: build_gemini_tools(request)?,
            tool_config: None,
        })
    }

    pub(super) fn request_contains_tool_results(request: &CompletionRequest) -> bool {
        request.messages.iter().any(|message| {
            wire_message::project(message)
                .iter()
                .any(|part| matches!(part, wire_message::WirePart::ToolResult { .. }))
        })
    }

    pub(super) fn realtime_ws_endpoint(&self) -> Result<url::Url, ProviderError> {
        let mut endpoint = if let Some(ws_url) = self.realtime_ws_url.as_ref() {
            url::Url::parse(ws_url).map_err(|err| {
                ProviderError::Config(format!("gemini realtime websocket url is invalid: {err}"))
            })?
        } else {
            let mut url = url::Url::parse(self.base_url.as_str()).map_err(|err| {
                ProviderError::Config(format!("gemini base url is invalid: {err}"))
            })?;
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
                return Err(ProviderError::Config(
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
                ProviderError::Config("gemini realtime websocket url is invalid".to_owned())
            })?,
            "https" => endpoint.set_scheme("wss").map_err(|_| {
                ProviderError::Config("gemini realtime websocket url is invalid".to_owned())
            })?,
            "ws" | "wss" => {}
            other => {
                return Err(ProviderError::Config(format!(
                    "gemini realtime websocket url has unsupported scheme `{other}`"
                )));
            }
        }

        Ok(endpoint)
    }

    pub(super) fn realtime_handshake_request(
        &self,
        endpoint: &url::Url,
        api_key: &str,
        request_headers: &HashMap<String, String>,
    ) -> Result<http::Request<()>, ProviderError> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let endpoint = self.endpoint_with_auth(endpoint.to_string(), api_key);
        let mut request = endpoint.into_client_request().map_err(|err| {
            ProviderError::Config(format!(
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
                ProviderError::Config(format!(
                    "gemini realtime websocket handshake url is invalid: {err}"
                ))
            })?;
            url.query_pairs_mut().append_pair("key", api_key);
            *request.uri_mut() = url.to_string().parse().map_err(|err| {
                ProviderError::Config(format!(
                    "gemini realtime websocket handshake uri is invalid: {err}"
                ))
            })?;
        } else if let GeminiAuthMode::Header { name, scheme } = &self.auth_mode {
            let auth_header_name =
                http::header::HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
                    ProviderError::Config(format!("gemini auth header name is invalid: {err}"))
                })?;
            let auth_header_value = http::header::HeaderValue::from_str(
                utils::auth_header_value(scheme.as_deref(), api_key).as_str(),
            )
            .map_err(|err| {
                ProviderError::Config(format!("gemini auth header value is invalid: {err}"))
            })?;
            request
                .headers_mut()
                .insert(auth_header_name, auth_header_value);
        }

        for (key, value) in utils::resolved_request_headers(PROVIDER_ID, request_headers) {
            let header_name =
                http::header::HeaderName::from_bytes(key.as_bytes()).map_err(|err| {
                    ProviderError::Config(format!(
                        "gemini extra header name `{key}` is invalid: {err}"
                    ))
                })?;
            let header_value =
                http::header::HeaderValue::from_str(value.as_str()).map_err(|err| {
                    ProviderError::Config(format!(
                        "gemini extra header `{key}` value is invalid: {err}"
                    ))
                })?;
            request.headers_mut().insert(header_name, header_value);
        }

        Ok(request)
    }

    pub(super) fn message_parts(
        message: &agena_provider::CompletionInputMessage,
    ) -> Vec<GeminiPart> {
        let projected_parts = wire_message::project(message);
        Self::parts_from_projected_parts(message, projected_parts.as_slice())
    }

    pub(super) fn parts_from_projected_parts(
        message: &agena_provider::CompletionInputMessage,
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

        let signatures = Some(&message.provider_state.gemini_thought_signatures);
        let has_function_calls = projected_parts
            .iter()
            .any(|part| matches!(part, wire_message::WirePart::ToolCall { .. }));
        let final_part_index = (!has_function_calls)
            .then(|| projected_parts.len().checked_sub(1))
            .flatten();
        let mut first_function_call = true;
        projected_parts
            .iter()
            .enumerate()
            .map(|(index, part)| {
                let thought_signature = match part {
                    wire_message::WirePart::ToolCall { id, .. } => {
                        let signature = signatures
                            .and_then(|signatures| signatures.get(id))
                            .cloned()
                            .or_else(|| {
                                first_function_call
                                    .then(|| "skip_thought_signature_validator".to_owned())
                            });
                        first_function_call = false;
                        signature
                    }
                    _ if final_part_index == Some(index) => signatures
                        .and_then(|signatures| signatures.get(GEMINI_FINAL_PART_SIGNATURE_KEY))
                        .cloned(),
                    _ => None,
                };
                Self::part_from_wire_part(part, thought_signature)
            })
            .collect()
    }

    pub(super) fn part_from_wire_part(
        part: &wire_message::WirePart,
        thought_signature: Option<String>,
    ) -> GeminiPart {
        let mut part = match part {
            wire_message::WirePart::Text { text } => GeminiPart::text(text.clone()),
            wire_message::WirePart::Attachment { item } => Self::attachment_part(item),
            wire_message::WirePart::ToolCall {
                id,
                function,
                arguments_json,
            } => GeminiPart::function_call(
                Some(id.clone()),
                gemini_wire_tool_name(function.function_name()),
                parse_json_or_object(arguments_json),
                thought_signature.clone(),
            ),
            wire_message::WirePart::ToolResult {
                tool_call_id,
                function,
                output_json,
                ..
            } => GeminiPart::function_response(
                Some(tool_call_id.clone()),
                gemini_tool_response_name(function.function_name()),
                parse_json_or_string_object(output_json),
            ),
        };
        if part.function_response.is_none() && part.thought_signature.is_none() {
            part.thought_signature = thought_signature;
        }
        part
    }

    pub(super) fn assistant_contents(
        message: &agena_provider::CompletionInputMessage,
    ) -> Vec<GeminiContent> {
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
                    tool_call_id,
                    function,
                    output_json,
                    ..
                } => {
                    Self::flush_model_content(message, &mut contents, &mut buffered);
                    contents.push(GeminiContent {
                        role: Some("user".to_owned()),
                        parts: vec![GeminiPart::function_response(
                            Some(tool_call_id.clone()),
                            gemini_tool_response_name(function.function_name()),
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

    pub(super) fn tool_contents(
        message: &agena_provider::CompletionInputMessage,
    ) -> Vec<GeminiContent> {
        wire_message::project(message)
            .into_iter()
            .filter_map(|part| match part {
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    function,
                    output_json,
                    ..
                } => Some(GeminiContent {
                    role: Some("user".to_owned()),
                    parts: vec![GeminiPart::function_response(
                        Some(tool_call_id.clone()),
                        gemini_tool_response_name(function.function_name()),
                        parse_json_or_string_object(output_json.as_str()),
                    )],
                }),
                _ => None,
            })
            .collect()
    }

    pub(super) fn flush_model_content(
        message: &agena_provider::CompletionInputMessage,
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

    pub(super) fn attachment_part(item: &AttachmentItem) -> GeminiPart {
        wire_message::base64_with_mime(item)
            .map(|(mime_type, data)| GeminiPart::inline_data(mime_type, data))
            .unwrap_or_else(|| GeminiPart::text(wire_message::hint_text(item)))
    }

    pub(super) async fn send_request<F>(
        &self,
        mut build: F,
    ) -> Result<reqwest::Response, ProviderError>
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

    pub(super) async fn complete_stream_with_realtime_ws(
        &self,
        request: &CompletionRequest,
        model: ModelId,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let ws_endpoint = self.realtime_ws_endpoint()?;
        let api_key = self.api_key.resolve().await?;
        let request_headers = self.completion_request_headers(request);
        let handshake =
            self.realtime_handshake_request(&ws_endpoint, api_key.as_str(), &request_headers)?;
        let (ws_stream, _) = tokio_tungstenite::connect_async(handshake)
            .await
            .map_err(|err| {
                ProviderError::Provider(format!("gemini realtime websocket connect failed: {err}"))
            })?;

        let (system_chunks, contents) = Self::request_system_and_contents(request);
        let live_request = GeminiLiveConversationRequest {
            setup: GeminiLiveSetup {
                model: if model.as_ref().starts_with("models/") {
                    model.to_string()
                } else {
                    format!("models/{model}")
                },
                generation_config: Self::generation_config(
                    model.as_ref(),
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
            ProviderError::Config(
                "gemini realtime request patch removed `setup`; restore it or disable the patch"
                    .to_owned(),
            )
        })?;
        let client_content = live_json.get("clientContent").cloned().ok_or_else(|| {
            ProviderError::Config(
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
                    ProviderError::Provider(format!(
                        "gemini realtime websocket send setup failed: {err}"
                    ))
                })?;

            let mut setup_complete = false;
            while let Some(message) = ws_reader.next().await {
                let message = message.map_err(|err| {
                    ProviderError::Provider(format!(
                        "gemini realtime websocket receive failed before setup complete: {err}"
                    ))
                })?;
                let text = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        String::from_utf8(bytes.to_vec()).map_err(|err| {
                            ProviderError::Provider(format!(
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
                        ProviderError::Provider(format!(
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
                Err(ProviderError::Provider(
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
                    ProviderError::Provider(format!(
                        "gemini realtime websocket send clientContent failed: {err}"
                    ))
                })?;

            let mut emitted_tool_calls = std::collections::BTreeSet::new();
            let mut saw_content = false;
            let mut tool_call_seen = false;
            let mut completed_emitted = false;
            let mut fallback_usage: Option<CompletionUsage> = None;
            let mut fallback_provider_metadata: Option<serde_json::Value> = None;

            while let Some(message) = ws_reader.next().await {
                let message = message.map_err(|err| {
                    ProviderError::Provider(format!(
                        "gemini realtime websocket receive failed: {err}"
                    ))
                })?;

                let text = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        String::from_utf8(bytes.to_vec()).map_err(|err| {
                            ProviderError::Provider(format!(
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
                        ProviderError::Provider(format!(
                            "gemini realtime websocket returned invalid json: {err}"
                        ))
                    })?,
                )?;

                if let Some(usage) = payload
                    .usage_metadata
                    .clone()
                    .map(gemini_usage_to_completion)
                {
                    fallback_usage = Some(usage);
                }

                if let Some(server_content) = payload.server_content {
                    if let Some(metadata) =
                        gemini_live_server_content_provider_metadata(&server_content)
                    {
                        fallback_provider_metadata = merge_gemini_provider_metadata(
                            fallback_provider_metadata.take(),
                            Some(metadata),
                        );
                    }

                    if let Some(model_turn) = server_content.model_turn {
                        let reasoning_delta =
                            gemini_reasoning_text_from_content(&model_turn).unwrap_or_default();
                        if !reasoning_delta.is_empty() {
                            saw_content = true;
                            yield CompletionStreamEvent::ThinkingDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta: reasoning_delta,
                            };
                        }

                        let text_delta = gemini_text_from_content(&model_turn);
                        if !text_delta.is_empty() {
                            saw_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta: text_delta,
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
                        let arguments_json =
                            gemini_tool_call_arguments_json(&call, "realtime")?;
                        let dedupe_key = call
                            .id
                            .clone()
                            .unwrap_or_else(|| utils::request_shape_fingerprint(&call));
                        if !emitted_tool_calls.insert(dedupe_key.clone()) {
                            continue;
                        }

                        tool_call_seen = true;
                        saw_content = true;
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
                    Err(ProviderError::Provider(
                        "gemini realtime websocket closed before returning any content".to_owned(),
                    ))?;
                }
            }
        };

        Ok(Box::pin(stream))
    }

    pub(super) async fn complete_by_aggregating_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let fallback_model = request.model.clone();
        let stream = ModelRuntime::complete_stream(self, request).await?;
        utils::aggregate_stream(PROVIDER_ID, fallback_model, stream).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_runtime_contracts::message::Message;
    use agena_runtime_contracts::message::MessageProviderState;
    use std::collections::BTreeMap;

    #[test]
    fn parallel_calls_replay_only_their_exact_thought_signatures() {
        let mut message = Message::prompt_text(Role::Assistant, "");
        message.provider_state = Some(MessageProviderState {
            gemini_thought_signatures: BTreeMap::from([
                ("first".to_owned(), "first-signature".to_owned()),
                ("second".to_owned(), "second-signature".to_owned()),
            ]),
            ..MessageProviderState::default()
        });
        let projected = vec![
            wire_message::WirePart::ToolCall {
                id: "first".to_owned(),
                function: agena_domain::ToolApiFunction::Call,
                arguments_json: "{}".to_owned(),
            },
            wire_message::WirePart::ToolCall {
                id: "second".to_owned(),
                function: agena_domain::ToolApiFunction::Call,
                arguments_json: "{}".to_owned(),
            },
        ];

        let input = crate::provider::project_completion_input(&message);
        let parts = GeminiAdapter::parts_from_projected_parts(&input, &projected);

        assert_eq!(
            parts[0].thought_signature.as_deref(),
            Some("first-signature")
        );
        assert_eq!(
            parts[1].thought_signature.as_deref(),
            Some("second-signature")
        );
    }

    #[test]
    fn missing_parallel_signature_uses_validator_escape_only_on_first_call() {
        let message = Message::prompt_text(Role::Assistant, "");
        let projected = vec![
            wire_message::WirePart::ToolCall {
                id: "first".to_owned(),
                function: agena_domain::ToolApiFunction::Call,
                arguments_json: "{}".to_owned(),
            },
            wire_message::WirePart::ToolCall {
                id: "second".to_owned(),
                function: agena_domain::ToolApiFunction::Call,
                arguments_json: "{}".to_owned(),
            },
        ];

        let input = crate::provider::project_completion_input(&message);
        let parts = GeminiAdapter::parts_from_projected_parts(&input, &projected);

        assert_eq!(
            parts[0].thought_signature.as_deref(),
            Some("skip_thought_signature_validator")
        );
        assert_eq!(parts[1].thought_signature, None);
    }

    #[test]
    fn final_non_function_part_replays_its_thought_signature() {
        let mut message = Message::prompt_text(Role::Assistant, "");
        message.provider_state = Some(MessageProviderState {
            gemini_thought_signatures: BTreeMap::from([(
                GEMINI_FINAL_PART_SIGNATURE_KEY.to_owned(),
                "final-signature".to_owned(),
            )]),
            ..MessageProviderState::default()
        });
        let projected = vec![wire_message::WirePart::Text {
            text: "answer".to_owned(),
        }];

        let input = crate::provider::project_completion_input(&message);
        let parts = GeminiAdapter::parts_from_projected_parts(&input, &projected);

        assert_eq!(
            parts[0].thought_signature.as_deref(),
            Some("final-signature")
        );
    }
}
