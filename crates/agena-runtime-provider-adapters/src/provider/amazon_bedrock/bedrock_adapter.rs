use crate::provider::ModelRuntime;
use agena_domain::Model;
use agena_provider::{
    merge_openai_chat_reasoning_details, openai_chat_extract_reasoning_text,
    openai_chat_extract_text, openai_chat_reasoning_field,
};
use agena_provider_bedrock_streaming::{
    BedrockAnthropicStreamDecodeError, decode_response as decode_bedrock_anthropic_response,
};
use futures_util::StreamExt;

use super::{
    ADAPTER_KIND, AmazonBedrockAdapter, Arc, AttachmentItem, AttachmentKind,
    BEDROCK_ANTHROPIC_VERSION, BTreeMap, BedrockAnthropicBinarySource, BedrockAnthropicMessage,
    BedrockAnthropicMessagesRequest, BedrockAnthropicMessagesResponse, BedrockAnthropicStreamEvent,
    BedrockAnthropicTextBlock, BedrockAnthropicThinkingBlockState, BedrockAnthropicToolCallState,
    BedrockAnthropicToolDefinition, BedrockAnthropicUsage, BedrockAuthMode, ChatCompletionRequest,
    ChatCompletionResponse, ChatMessage, ChatStreamOptions, ChatToolCallWire, ChatUsage,
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    CompletionToolCall, CompletionUsage, Credentials, EVENTSTREAM_CONTENT_TYPE, HashMap,
    JSON_CONTENT_TYPE, ModelId, ModelMetadata, Mutex, OpenAiCompatibleModelList, PROVIDER_ID,
    ProviderError, ProviderId, Role, Sigv4Request, Stream, Value, bedrock_anthropic_metadata,
    bedrock_anthropic_thinking_parts, bedrock_wire_tool_name, json_value_to_string,
    map_bedrock_anthropic_usage, merge_bedrock_anthropic_usage, prefix_bedrock_model, prompt_cache,
    response_id_metadata, sse, strip_cross_region_prefix, utils, wire_message,
};

impl AmazonBedrockAdapter {
    pub fn new_sigv4(
        client: reqwest::Client,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        region: impl Into<String>,
        profile: Option<String>,
        static_credentials: Option<Credentials>,
    ) -> Self {
        Self {
            client,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            region: region.into(),
            auth_mode: BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            },
            resolved_sigv4_shape: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn prompt_cache_sigv4_profile(profile: Option<&str>) -> Option<String> {
        profile
            .map(|value| value.to_owned())
            .and_then(|value| utils::normalize_optional_text(Some(value)))
    }

    pub(super) fn prompt_cache_sigv4_static_credentials_shape(
        credentials: &Credentials,
    ) -> agena_provider::PromptCacheShape {
        let mut shape =
            agena_provider::PromptCacheShape::from_fields(PROVIDER_ID, [("configured", "true")]);

        if let Some(account_id) = credentials
            .account_id()
            .map(|value| value.as_str().trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            shape.insert_string("account_id", account_id);
        }

        if let Some(access_key_id) =
            utils::normalize_optional_text(Some(credentials.access_key_id().to_owned()))
        {
            shape.insert_string(
                "access_key_id_fingerprint",
                utils::request_shape_fingerprint(&access_key_id),
            );
        }

        shape
    }

    pub(super) fn prompt_cache_sigv4_runtime_shape_from_credentials(
        credentials: &Credentials,
    ) -> Option<agena_provider::PromptCacheShape> {
        credentials
            .account_id()
            .map(|value| value.as_str().trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|account_id| {
                agena_provider::PromptCacheShape::from_fields(
                    PROVIDER_ID,
                    [("account_id", account_id)],
                )
            })
    }

    pub(super) fn prompt_cache_sigv4_env_shape() -> Option<agena_provider::PromptCacheShape> {
        let mut shape = agena_provider::PromptCacheShape::new(PROVIDER_ID);
        let mut has_fields = false;

        for (env_key, field_key) in [
            ("AWS_PROFILE", "aws_profile"),
            ("AWS_DEFAULT_PROFILE", "aws_default_profile"),
            ("AWS_ROLE_ARN", "aws_role_arn"),
            ("AWS_WEB_IDENTITY_TOKEN_FILE", "aws_web_identity_token_file"),
            ("AWS_SHARED_CREDENTIALS_FILE", "aws_shared_credentials_file"),
            ("AWS_CONFIG_FILE", "aws_config_file"),
            (
                "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
                "aws_container_credentials_relative_uri",
            ),
            ("AWS_EC2_METADATA_DISABLED", "aws_ec2_metadata_disabled"),
        ] {
            if let Some(value) = std::env::var(env_key)
                .ok()
                .and_then(|value| utils::normalize_optional_text(Some(value)))
            {
                shape.insert_string(field_key, value);
                has_fields = true;
            }
        }

        if let Some(access_key_id) = std::env::var("AWS_ACCESS_KEY_ID")
            .ok()
            .and_then(|value| utils::normalize_optional_text(Some(value)))
        {
            shape.insert_string(
                "aws_access_key_id_fingerprint",
                utils::request_shape_fingerprint(&access_key_id),
            );
            has_fields = true;
        }

        has_fields.then_some(shape)
    }

    pub(super) fn update_prompt_cache_sigv4_runtime_shape(&self, credentials: &Credentials) {
        let mut runtime_shape = match self.resolved_sigv4_shape.lock() {
            Ok(runtime_shape) => runtime_shape,
            Err(error) => {
                tracing::error!(
                    operation = "update Bedrock prompt-cache SigV4 shape",
                    error = %error,
                    "recovering poisoned Bedrock prompt-cache lock"
                );
                error.into_inner()
            }
        };
        *runtime_shape = Self::prompt_cache_sigv4_runtime_shape_from_credentials(credentials);
    }

    pub(super) fn resolve_model(&self, model: &str) -> String {
        let model = if model.trim().is_empty() {
            self.default_model.to_string()
        } else {
            model.trim().to_owned()
        };
        prefix_bedrock_model(self.region.as_str(), model.as_str())
    }

    pub(super) fn models_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    pub(super) fn completions_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    pub(super) fn runtime_endpoint(&self) -> &str {
        self.base_url
            .strip_suffix("/openai/v1")
            .unwrap_or(self.base_url.as_str())
    }

    pub(super) fn native_anthropic_invoke_endpoint(
        &self,
        model: &str,
        stream: bool,
    ) -> Result<String, ProviderError> {
        let mut url = url::Url::parse(self.runtime_endpoint()).map_err(|err| {
            ProviderError::Config(format!(
                "amazon-bedrock invalid runtime base url `{}`: {err}",
                self.runtime_endpoint()
            ))
        })?;
        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                ProviderError::Config(format!(
                    "amazon-bedrock runtime base url cannot accept path segments: {}",
                    self.runtime_endpoint()
                ))
            })?;
            segments.push("model");
            segments.push(model);
            segments.push(if stream {
                "invoke-with-response-stream"
            } else {
                "invoke"
            });
        }
        Ok(url.to_string())
    }

    pub(super) async fn resolve_sigv4_credentials(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
    ) -> Result<Credentials, ProviderError> {
        if let Some(credentials) = static_credentials {
            self.update_prompt_cache_sigv4_runtime_shape(credentials);
            return Ok(credentials.clone());
        }

        let credentials = agena_provider_bedrock_auth::resolve_credentials(
            self.region.as_str(),
            profile,
            static_credentials,
        )
        .await
        .map_err(|error| match error {
            agena_provider_bedrock_auth::BedrockCredentialError::ProviderUnavailable => {
                ProviderError::Config(
                    "amazon-bedrock could not resolve aws credential provider".to_owned(),
                )
            }
            agena_provider_bedrock_auth::BedrockCredentialError::Resolve(error) => {
                ProviderError::Provider(format!(
                    "amazon-bedrock failed to resolve aws credentials from chain: {error}"
                ))
            }
        })?;
        self.update_prompt_cache_sigv4_runtime_shape(&credentials);
        Ok(credentials)
    }

    pub(super) async fn send_sigv4_request(
        &self,
        operation: &str,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request_input: Sigv4Request<'_>,
    ) -> Result<reqwest::Response, ProviderError> {
        let Sigv4Request {
            method,
            url,
            body,
            headers,
            body_debug,
        } = request_input;
        let credentials = self
            .resolve_sigv4_credentials(profile, static_credentials)
            .await?;
        let signing_headers = agena_provider_bedrock_signing::signed_headers(
            method.as_str(),
            url.as_str(),
            body.as_deref().unwrap_or(&[]),
            headers.as_slice(),
            &credentials,
            self.region.as_str(),
        )
        .map_err(|error| match error {
            agena_provider_bedrock_signing::BedrockSigningError::HeaderName { name, error } => {
                ProviderError::Config(format!("bedrock invalid header name `{name}`: {error}"))
            }
            agena_provider_bedrock_signing::BedrockSigningError::HeaderValue { name, error } => {
                ProviderError::Config(format!(
                    "bedrock invalid header value for `{name}`: {error}"
                ))
            }
            agena_provider_bedrock_signing::BedrockSigningError::SigningParameters(error) => {
                ProviderError::Provider(format!("bedrock signing params error: {error}"))
            }
            agena_provider_bedrock_signing::BedrockSigningError::SignableRequest(error) => {
                ProviderError::Provider(format!("bedrock signable request error: {error}"))
            }
            agena_provider_bedrock_signing::BedrockSigningError::Signing(error) => {
                ProviderError::Provider(format!("bedrock signing failed: {error}"))
            }
            agena_provider_bedrock_signing::BedrockSigningError::RequestConstruction(error) => {
                ProviderError::Provider(format!("bedrock signing request build error: {error}"))
            }
        })?;

        let mut request = self.client.request(method.clone(), url.as_str());
        for (name, value) in signing_headers.iter() {
            request = request.header(name, value);
        }
        let plugin_headers: HashMap<String, String> = Default::default();
        let plugin_headers = utils::resolved_request_headers(PROVIDER_ID, &plugin_headers);
        let mut final_headers = signing_headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        final_headers.extend(plugin_headers.clone());
        utils::adapter_log_http_request_json(
            PROVIDER_ID,
            ADAPTER_KIND,
            operation,
            method.as_str(),
            url.as_str(),
            final_headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            body_debug,
        );
        request = utils::apply_resolved_request_headers(request, &plugin_headers);
        if let Some(payload) = body {
            request = request.body(payload);
        }

        request.send().await.map_err(ProviderError::from)
    }

    pub(super) fn parse_models(&self, payload: Value) -> Result<Vec<Model>, ProviderError> {
        let parsed: OpenAiCompatibleModelList =
            utils::parse_json_value(PROVIDER_ID, "models list", payload)?;
        let models = match parsed {
            OpenAiCompatibleModelList::Object { data } => data,
            OpenAiCompatibleModelList::Array(data) => data,
        };

        Ok(models
            .into_iter()
            .map(|model| {
                let model_id = ModelId::new(model.id);
                let capabilities = self.model_capabilities(&model_id);
                Model {
                    provider_id: ProviderId::new(PROVIDER_ID),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name: model.display_name.or(model.name),
                    native_compaction: true,
                    capabilities,
                    metadata: ModelMetadata::default(),
                    thinking_modes: Vec::new(),
                    speed_modes: std::collections::BTreeMap::new(),
                }
            })
            .collect())
    }

    pub(super) async fn list_models_sigv4(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
    ) -> Result<Vec<Model>, ProviderError> {
        let response = self
            .send_sigv4_request(
                "list_models",
                profile,
                static_credentials,
                Sigv4Request {
                    method: reqwest::Method::GET,
                    url: self.models_endpoint(),
                    body: None,
                    headers: Vec::new(),
                    body_debug: None,
                },
            )
            .await?;

        let payload: Value =
            utils::parse_json_response_logged(PROVIDER_ID, ADAPTER_KIND, "list_models", response)
                .await?;
        self.parse_models(payload)
    }

    pub(super) fn parse_completion(
        &self,
        payload: ChatCompletionResponse,
    ) -> Result<CompletionResponse, ProviderError> {
        crate::provider::chat_wire::parse_completion_response_with_required_tool_calls(
            PROVIDER_ID,
            self.default_model.as_ref(),
            payload,
        )
    }

    pub(super) fn parse_anthropic_completion(
        &self,
        payload: BedrockAnthropicMessagesResponse,
        fallback_model: &ModelId,
    ) -> Result<CompletionResponse, ProviderError> {
        let text = payload
            .content
            .iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text.clone())
            .collect::<Vec<_>>()
            .join("");
        let reasoning_text = payload
            .content
            .iter()
            .filter(|block| block.kind == "thinking")
            .filter_map(|block| block.thinking.clone())
            .collect::<Vec<_>>()
            .join("");

        let tool_calls = payload
            .content
            .iter()
            .filter(|block| block.kind == "tool_use")
            .map(|block| {
                let id = utils::normalize_optional_text(block.id.clone()).ok_or_else(|| {
                    ProviderError::Provider(
                        "amazon-bedrock anthropic response returned tool_use without id".to_owned(),
                    )
                })?;

                let name = utils::optional_non_empty(block.name.clone()).ok_or_else(|| {
                    ProviderError::Provider(
                        "amazon-bedrock anthropic response returned tool_use without name"
                            .to_owned(),
                    )
                })?;

                Ok(CompletionToolCall::Function {
                    id,
                    name,
                    arguments_json: block
                        .input
                        .as_ref()
                        .map(json_value_to_string)
                        .unwrap_or_else(|| "{}".to_owned()),
                })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        let finish_reason = CompletionFinishReason::normalize_with_tool_calls(
            CompletionFinishReason::from_provider(payload.stop_reason.as_deref()),
            !tool_calls.is_empty(),
        );

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(ProviderError::Provider(
                "amazon-bedrock anthropic completion payload was empty without finish reason"
                    .to_owned(),
            ));
        }

        let provider_metadata = bedrock_anthropic_metadata(payload.id, payload.content.as_slice());
        Ok(CompletionResponse {
            provider_id: ProviderId::new(PROVIDER_ID),
            model: ModelId::new(payload.model.unwrap_or_else(|| fallback_model.to_string())),
            text,
            reasoning_text: (!reasoning_text.is_empty()).then_some(reasoning_text),
            finish_reason,
            tool_calls,
            usage: payload.usage.map(map_bedrock_anthropic_usage),
            provider_metadata,
        })
    }

    pub(super) fn is_native_anthropic_model(model: &str) -> bool {
        let normalized = strip_cross_region_prefix(model).to_ascii_lowercase();
        normalized.starts_with("anthropic.") || normalized.contains("claude")
    }

    pub(super) fn anthropic_tools(
        tools: &[agena_provider::ToolApiDefinition],
    ) -> Vec<BedrockAnthropicToolDefinition> {
        tools
            .iter()
            .cloned()
            .map(|tool| BedrockAnthropicToolDefinition {
                name: bedrock_wire_tool_name(tool.name.as_str()),
                description: tool.description,
                input_schema: tool.input_schema,
                cache_control: None,
                eager_input_streaming: None,
            })
            .collect()
    }

    pub(super) fn anthropic_content_to_blocks(
        run: &agena_provider::CompletionInputRun,
    ) -> Vec<BedrockAnthropicTextBlock> {
        let projected = wire_message::project(run);
        Self::anthropic_blocks_from_projected_parts(run, projected.as_slice())
    }

    pub(super) fn anthropic_thinking_blocks_from_message(
        run: &agena_provider::CompletionInputRun,
    ) -> Vec<BedrockAnthropicTextBlock> {
        run.provider_state
            .anthropic_thinking_blocks
            .iter()
            .filter_map(|block| {
                let block = match serde_json::from_value::<BedrockAnthropicTextBlock>(block.clone())
                {
                    Ok(block) => block,
                    Err(error) => {
                        tracing::warn!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "decode a persisted Bedrock Anthropic thinking block",
                                &error,
                            ),
                            "malformed Bedrock Anthropic thinking block was not replayed"
                        );
                        return None;
                    }
                };
                match block.kind.as_str() {
                    "thinking"
                        if block
                            .signature
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty()) =>
                    {
                        Some(block)
                    }
                    "redacted_thinking"
                        if block
                            .data
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty()) =>
                    {
                        Some(block)
                    }
                    _ => None,
                }
            })
            .collect()
    }

    pub(super) fn anthropic_blocks_from_projected_parts(
        run: &agena_provider::CompletionInputRun,
        projected: &[wire_message::WirePart],
    ) -> Vec<BedrockAnthropicTextBlock> {
        if projected.is_empty() {
            let text = run.as_text_lossy();
            if text.is_empty() {
                return Vec::new();
            }

            return vec![BedrockAnthropicTextBlock::text(text)];
        }

        let mut blocks = Vec::new();
        for part in projected {
            match part {
                wire_message::WirePart::Text { text } => {
                    blocks.push(BedrockAnthropicTextBlock::text(text.clone()));
                }
                wire_message::WirePart::Reasoning { text } => {
                    blocks.push(BedrockAnthropicTextBlock::text(text.clone()));
                }
                // A system notice (background-operation notification) rides the
                // assistant message as a text block: Bedrock's Anthropic
                // Messages API has no mid-conversation system message.
                wire_message::WirePart::SystemMessage { text } => {
                    blocks.push(BedrockAnthropicTextBlock::text(text.clone()));
                }
                wire_message::WirePart::Attachment { item } => {
                    blocks.extend(Self::anthropic_attachment_blocks(item));
                }
                wire_message::WirePart::ToolCall {
                    id,
                    function,
                    arguments_json,
                } => blocks.push(BedrockAnthropicTextBlock::tool_use(
                    id.clone(),
                    bedrock_wire_tool_name(function.function_name()),
                    arguments_json.clone(),
                )),
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    output_json,
                    ..
                } => blocks.push(BedrockAnthropicTextBlock::tool_result(
                    tool_call_id.clone(),
                    output_json.clone(),
                )),
            }
        }

        blocks
    }

    pub(super) fn anthropic_assistant_messages_from_parts(
        run: &agena_provider::CompletionInputRun,
    ) -> Vec<BedrockAnthropicMessage> {
        let projected = wire_message::project(run);
        if !projected
            .iter()
            .any(|part| matches!(part, wire_message::WirePart::ToolResult { .. }))
        {
            let mut content = Self::anthropic_thinking_blocks_from_message(run);
            content.extend(Self::anthropic_blocks_from_projected_parts(
                run,
                projected.as_slice(),
            ));
            return vec![BedrockAnthropicMessage {
                role: "assistant".to_owned(),
                content,
            }];
        }

        let mut messages = Vec::new();
        let mut buffered = Vec::<wire_message::WirePart>::new();
        for part in &projected {
            match part {
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    output_json,
                    ..
                } if !tool_call_id.trim().is_empty() => {
                    Self::flush_anthropic_assistant_blocks(run, &mut messages, &mut buffered);
                    Self::push_anthropic_request_message(
                        &mut messages,
                        BedrockAnthropicMessage {
                            role: "user".to_owned(),
                            content: vec![BedrockAnthropicTextBlock::tool_result(
                                tool_call_id.clone(),
                                output_json.clone(),
                            )],
                        },
                    );
                }
                wire_message::WirePart::ToolResult { output_json, .. } => {
                    buffered.push(wire_message::WirePart::Text {
                        text: output_json.clone(),
                    });
                }
                other => buffered.push(other.clone()),
            }
        }
        Self::flush_anthropic_assistant_blocks(run, &mut messages, &mut buffered);

        messages
    }

    pub(super) fn anthropic_tool_messages_from_parts(
        run: &agena_provider::CompletionInputRun,
    ) -> Vec<BedrockAnthropicMessage> {
        let content = wire_message::project(run)
            .into_iter()
            .filter_map(|part| match part {
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    output_json,
                    ..
                } if !tool_call_id.trim().is_empty() => Some(
                    BedrockAnthropicTextBlock::tool_result(tool_call_id, output_json),
                ),
                _ => None,
            })
            .collect::<Vec<_>>();
        (!content.is_empty())
            .then(|| BedrockAnthropicMessage {
                role: "user".to_owned(),
                content,
            })
            .into_iter()
            .collect()
    }

    pub(super) fn push_anthropic_request_message(
        messages: &mut Vec<BedrockAnthropicMessage>,
        mut message: BedrockAnthropicMessage,
    ) {
        if message.content.is_empty() {
            return;
        }
        if message.role == "user"
            && let Some(previous) = messages.last_mut()
            && previous.role == "user"
        {
            previous.content.append(&mut message.content);
            return;
        }
        messages.push(message);
    }

    pub(super) fn extend_anthropic_request_messages(
        messages: &mut Vec<BedrockAnthropicMessage>,
        extension: impl IntoIterator<Item = BedrockAnthropicMessage>,
    ) {
        for message in extension {
            Self::push_anthropic_request_message(messages, message);
        }
    }

    pub(super) fn flush_anthropic_assistant_blocks(
        run: &agena_provider::CompletionInputRun,
        messages: &mut Vec<BedrockAnthropicMessage>,
        buffered: &mut Vec<wire_message::WirePart>,
    ) {
        if buffered.is_empty() {
            return;
        }
        let content = Self::anthropic_blocks_from_projected_parts(run, buffered.as_slice());
        buffered.clear();
        if content.is_empty() {
            return;
        }
        let mut content = content;
        if !messages.iter().any(|run| run.role == "assistant") {
            let mut thinking = Self::anthropic_thinking_blocks_from_message(run);
            thinking.append(&mut content);
            content = thinking;
        }
        Self::push_anthropic_request_message(
            messages,
            BedrockAnthropicMessage {
                role: "assistant".to_owned(),
                content,
            },
        );
    }

    pub(super) fn anthropic_attachment_blocks(
        item: &AttachmentItem,
    ) -> Vec<BedrockAnthropicTextBlock> {
        match item.kind {
            AttachmentKind::Image => Self::anthropic_binary_source(item)
                .map(BedrockAnthropicTextBlock::image)
                .into_iter()
                .collect(),
            AttachmentKind::Pdf => Self::anthropic_binary_source(item)
                .map(BedrockAnthropicTextBlock::document)
                .into_iter()
                .collect(),
            AttachmentKind::File => wire_message::attachment_text(item)
                .map(BedrockAnthropicTextBlock::text)
                .into_iter()
                .collect(),
            AttachmentKind::Audio | AttachmentKind::Video => Vec::new(),
        }
        .into_iter()
        .chain(match item.kind {
            AttachmentKind::Audio | AttachmentKind::Video => Some(BedrockAnthropicTextBlock::text(
                wire_message::hint_text(item),
            )),
            AttachmentKind::Image | AttachmentKind::Pdf
                if Self::anthropic_binary_source(item).is_none() =>
            {
                Some(BedrockAnthropicTextBlock::text(wire_message::hint_text(
                    item,
                )))
            }
            AttachmentKind::File if wire_message::attachment_text(item).is_none() => Some(
                BedrockAnthropicTextBlock::text(wire_message::hint_text(item)),
            ),
            _ => None,
        })
        .collect()
    }

    pub(super) fn anthropic_binary_source(
        item: &AttachmentItem,
    ) -> Option<BedrockAnthropicBinarySource> {
        wire_message::base64_with_mime(item)
            .map(|(media_type, data)| BedrockAnthropicBinarySource::base64(media_type, data))
    }

    pub(super) fn apply_anthropic_prompt_cache_hints(
        system: &mut [BedrockAnthropicTextBlock],
        tools: &mut [BedrockAnthropicToolDefinition],
        messages: &mut [BedrockAnthropicMessage],
    ) {
        if let Some(block) = system.last_mut() {
            block.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }

        if let Some(tool) = tools.last_mut() {
            tool.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }

        if let Some(block) = Self::latest_anthropic_user_cache_block(messages) {
            block.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }
    }

    pub(super) fn latest_anthropic_user_cache_block(
        messages: &mut [BedrockAnthropicMessage],
    ) -> Option<&mut BedrockAnthropicTextBlock> {
        messages.iter_mut().rev().find_map(|message| {
            if message.role != "user" || message.content.is_empty() {
                return None;
            }
            let index = message
                .content
                .iter()
                .rposition(|block| block.kind != "tool_result")?;
            message.content.get_mut(index)
        })
    }

    pub(super) fn build_anthropic_request(
        request: CompletionRequest,
    ) -> (ModelId, BedrockAnthropicMessagesRequest) {
        let model = request.model.clone();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(BedrockAnthropicTextBlock::text(system.clone()));
        }
        let mut tools = (!request.tool_api_functions.is_empty())
            .then(|| Self::anthropic_tools(request.tool_api_functions.as_slice()));

        let mut messages = Vec::new();
        for msg in request.turns {
            match msg.role {
                Role::System => {
                    let text = msg.as_text_lossy();
                    if !text.trim().is_empty() {
                        system_chunks.push(BedrockAnthropicTextBlock::text(text));
                    }
                }
                Role::Assistant => Self::extend_anthropic_request_messages(
                    &mut messages,
                    Self::anthropic_assistant_messages_from_parts(&msg),
                ),
                Role::User => Self::push_anthropic_request_message(
                    &mut messages,
                    BedrockAnthropicMessage {
                        role: "user".to_owned(),
                        content: Self::anthropic_content_to_blocks(&msg),
                    },
                ),
                Role::Tool => Self::extend_anthropic_request_messages(
                    &mut messages,
                    Self::anthropic_tool_messages_from_parts(&msg),
                ),
            }
        }

        Self::apply_anthropic_prompt_cache_hints(
            system_chunks.as_mut_slice(),
            tools.as_deref_mut().unwrap_or(&mut []),
            messages.as_mut_slice(),
        );

        let max_tokens = request.max_output_tokens.unwrap_or(4096);
        let thinking_parts =
            bedrock_anthropic_thinking_parts(model.as_ref(), request.thinking.as_ref(), max_tokens);
        let omit_sampling = thinking_parts.include_thinking()
            || !Self::anthropic_model_supports_sampling_parameters(model.as_ref());

        (
            model.clone(),
            BedrockAnthropicMessagesRequest {
                anthropic_version: BEDROCK_ANTHROPIC_VERSION.to_owned(),
                anthropic_beta: thinking_parts.anthropic_beta,
                max_tokens,
                system: (!system_chunks.is_empty()).then_some(system_chunks),
                messages,
                tools,
                thinking: thinking_parts.thinking,
                output_config: thinking_parts.output_config,
                temperature: (!omit_sampling).then_some(request.temperature).flatten(),
                top_p: (!omit_sampling).then_some(request.top_p).flatten(),
                top_k: (!omit_sampling).then_some(request.top_k).flatten(),
                stop_sequences: request.stop_sequences,
            },
        )
    }

    pub(super) fn chat_messages_for_request(request: &CompletionRequest) -> Vec<ChatMessage> {
        let mut messages =
            crate::provider::chat_wire::request_to_chat_messages_with_assistant_reasoning_field(
                request, None,
            );
        for message in &mut messages {
            if let Some(tool_calls) = message.tool_calls.as_mut() {
                for tool_call in tool_calls {
                    tool_call.function.name =
                        bedrock_wire_tool_name(tool_call.function.name.as_str());
                }
            }
        }
        messages
    }

    pub(super) fn anthropic_invoke_headers(stream: bool) -> Vec<(String, String)> {
        let mut headers = vec![
            (
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                JSON_CONTENT_TYPE.to_owned(),
            ),
            (
                reqwest::header::ACCEPT.as_str().to_owned(),
                if stream {
                    EVENTSTREAM_CONTENT_TYPE.to_owned()
                } else {
                    JSON_CONTENT_TYPE.to_owned()
                },
            ),
        ];

        if stream {
            headers.push((
                "x-amzn-bedrock-accept".to_owned(),
                JSON_CONTENT_TYPE.to_owned(),
            ));
        }

        headers
    }

    pub(super) fn anthropic_model_uses_adaptive_thinking(model: &str) -> bool {
        let normalized = model.to_ascii_lowercase();
        normalized.contains("claude-opus-4-7")
            || normalized.contains("claude-opus-4.7")
            || normalized.contains("claude-opus-4-6")
            || normalized.contains("claude-opus-4.6")
            || normalized.contains("claude-sonnet-4-6")
            || normalized.contains("claude-sonnet-4.6")
            || normalized.contains("claude-fable-5")
            || normalized.contains("claude-mythos-5")
            || normalized.contains("claude-mythos-preview")
    }

    pub(super) fn anthropic_model_supports_sampling_parameters(model: &str) -> bool {
        let normalized = model.to_ascii_lowercase();
        !(normalized.contains("claude-fable-5")
            || normalized.contains("claude-mythos-5")
            || normalized.contains("claude-mythos-preview")
            || normalized.contains("claude-opus-4-7")
            || normalized.contains("claude-opus-4.7"))
    }

    pub(super) async fn complete_sigv4_anthropic(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let request_override = request.request_override.clone();
        let (model, body) = Self::build_anthropic_request(request);
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request_override.body_patch)?;
        let mut headers = Self::anthropic_invoke_headers(false);
        headers.extend(
            request_override
                .headers
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );

        let response = self
            .send_sigv4_request(
                "complete.native_anthropic",
                profile,
                static_credentials,
                Sigv4Request {
                    method: reqwest::Method::POST,
                    url: self.native_anthropic_invoke_endpoint(model.as_ref(), false)?,
                    body: Some(serde_json::to_vec(&body_json)?),
                    headers,
                    body_debug: Some(&body_json),
                },
            )
            .await?;

        let payload: BedrockAnthropicMessagesResponse = utils::parse_json_response_logged(
            PROVIDER_ID,
            ADAPTER_KIND,
            "complete.native_anthropic",
            response,
        )
        .await?;
        self.parse_anthropic_completion(payload, &model)
    }

    pub(super) async fn complete_stream_sigv4_anthropic(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let request_override = request.request_override.clone();
        let (model, body) = Self::build_anthropic_request(request);
        let include_thinking = body.thinking.as_ref().is_some_and(|thinking| {
            !matches!(thinking, super::BedrockAnthropicThinkingConfig::Disabled)
        });
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request_override.body_patch)?;
        let mut headers = Self::anthropic_invoke_headers(true);
        headers.extend(
            request_override
                .headers
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        let response = self
            .send_sigv4_request(
                "complete_stream.native_anthropic",
                profile,
                static_credentials,
                Sigv4Request {
                    method: reqwest::Method::POST,
                    url: self.native_anthropic_invoke_endpoint(model.as_ref(), true)?,
                    body: Some(serde_json::to_vec(&body_json)?),
                    headers,
                    body_debug: Some(&body_json),
                },
            )
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response_logged(
                PROVIDER_ID,
                ADAPTER_KIND,
                "complete_stream.native_anthropic",
                response,
            )
            .await);
        }

        utils::adapter_log_http_response_open(
            PROVIDER_ID,
            ADAPTER_KIND,
            "complete_stream.native_anthropic",
            response.status(),
            response.headers(),
        );
        let provider_id = ProviderId::new(PROVIDER_ID);
        let initial_model = model;
        let receiver = decode_bedrock_anthropic_response(response);

        let stream = async_stream::try_stream! {
            let mut receiver = receiver;
            let mut pending_tool_calls: std::collections::HashMap<usize, BedrockAnthropicToolCallState> = std::collections::HashMap::new();
            let mut thinking_blocks = BTreeMap::<usize, BedrockAnthropicThinkingBlockState>::new();
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_usage: Option<BedrockAnthropicUsage> = None;
            let mut stream_has_content = false;
            let mut stream_tool_call_seen = false;
            let mut response_id: Option<String> = None;
            let mut stream_model = initial_model.clone();

            loop {
                let event = match receiver.next().await {
                    Some(Ok(event)) => event,
                    None => break,
                    Some(Err(BedrockAnthropicStreamDecodeError::Service(service))) => {
                        Err(ProviderError::ProviderClassified {
                            provider: PROVIDER_ID.to_owned(),
                            message: service.to_string(),
                            kind: agena_provider::ProviderErrorKind::Unavailable,
                            retryable: service.retryable,
                        })?;
                        unreachable!("stream error branch always returns via `?`");
                    }
                    Some(Err(BedrockAnthropicStreamDecodeError::Decode(source))) => {
                        Err(ProviderError::Provider(format!(
                            "amazon-bedrock anthropic stream decode error: {source}"
                        )))?;
                        unreachable!("stream error branch always returns via `?`");
                    }
                };

                utils::adapter_log_stream_event(
                    PROVIDER_ID,
                    ADAPTER_KIND,
                    "complete_stream.native_anthropic",
                    &event,
                );
                let parsed: BedrockAnthropicStreamEvent = utils::parse_json_value(
                    PROVIDER_ID,
                    "bedrock anthropic stream event",
                    event,
                )?;

                match parsed {
                    BedrockAnthropicStreamEvent::MessageStart { message } => {
                        if response_id.is_none() {
                            response_id = utils::normalize_optional_text(message.id.clone());
                        }
                        if let Some(model) = utils::normalize_optional_text(message.model) {
                            stream_model = ModelId::new(model);
                        }
                        if let Some(usage) = message.usage {
                            stream_usage =
                                Some(merge_bedrock_anthropic_usage(stream_usage.take(), usage));
                        }
                    }
                    BedrockAnthropicStreamEvent::ContentBlockStart {
                        index,
                        content_block,
                    } => {
                        if matches!(content_block.kind.as_str(), "thinking" | "redacted_thinking") {
                            let index = index.ok_or_else(|| {
                                ProviderError::Provider(
                                    "amazon-bedrock anthropic thinking stream event missing content block index"
                                        .to_owned(),
                                )
                            })?;
                            let update = BedrockAnthropicThinkingBlockState {
                                    kind: content_block.kind,
                                    thinking: content_block.thinking.unwrap_or_default(),
                                    signature: content_block
                                        .signature
                                        .filter(|value| !value.trim().is_empty()),
                                    data: content_block
                                        .data
                                        .filter(|value| !value.trim().is_empty()),
                                };
                            match thinking_blocks.entry(index) {
                                std::collections::btree_map::Entry::Occupied(mut entry) => {
                                    entry.get_mut().merge_start(update);
                                }
                                std::collections::btree_map::Entry::Vacant(entry) => {
                                    entry.insert(update);
                                }
                            }
                            continue;
                        }
                        if content_block.kind != "tool_use" {
                            continue;
                        }

                        let index = index.ok_or_else(|| {
                            ProviderError::Provider(
                                "amazon-bedrock anthropic tool_use stream event missing content block index"
                                    .to_owned(),
                            )
                        })?;

                        let id = utils::normalize_optional_text(content_block.id.clone()).ok_or_else(|| {
                            ProviderError::Provider(
                                "amazon-bedrock anthropic tool_use stream event missing tool id"
                                    .to_owned(),
                            )
                        })?;
                        let name = utils::optional_non_empty(content_block.name.clone()).ok_or_else(|| {
                            ProviderError::Provider(
                                "amazon-bedrock anthropic tool_use stream event missing tool name"
                                    .to_owned(),
                            )
                        })?;

                        let state = pending_tool_calls.entry(index).or_default();
                        state.id = id;
                        state.name = name;
                        stream_tool_call_seen = true;

                        let arguments_delta = content_block
                            .input
                            .as_ref()
                            .map(json_value_to_string)
                            .filter(|value| !value.is_empty() && value != "{}")
                            .unwrap_or_default();

                        // Always emit at least one ToolCallDelta so the
                        // shared aggregator records the tool call even
                        // when the model called a parameterless tool.
                        stream_has_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: stream_model.clone(),
                            stream_key: format!("idx:{index}"),
                            id: Some(state.id.clone()),
                            name: Some(state.name.clone()),
                            arguments_delta,
                        };
                    }
                    BedrockAnthropicStreamEvent::ContentBlockDelta { index, delta } => {
                        if let Some(index) = index
                            && let Some(block) = thinking_blocks.get_mut(&index)
                        {
                            if let Some(thinking) = delta.thinking.as_deref() {
                                block.thinking.push_str(thinking);
                            }
                            if let Some(signature) = delta
                                .signature
                                .clone()
                                .filter(|value| !value.trim().is_empty())
                            {
                                block.signature = Some(signature);
                            }
                        }
                        if let Some(text) = delta.text.clone().filter(|value| !value.is_empty()) {
                            stream_has_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: stream_model.clone(),
                                delta: text,
                            };
                        }

                        if include_thinking
                            && let Some(thinking) = delta.thinking.clone().filter(|value| !value.is_empty())
                        {
                            stream_has_content = true;
                            yield CompletionStreamEvent::ThinkingDelta {
                                provider_id: provider_id.clone(),
                                model: stream_model.clone(),
                                delta: thinking,
                            };
                        }

                        if matches!(delta.kind.as_deref(), Some("input_json_delta")) {
                            let Some(arguments_delta) = utils::optional_non_empty(delta.partial_json.clone()) else {
                                continue;
                            };

                            let index = index.ok_or_else(|| {
                                ProviderError::Provider(
                                    "amazon-bedrock anthropic tool delta event missing content block index"
                                        .to_owned(),
                                )
                            })?;

                            let state = pending_tool_calls.get_mut(&index).ok_or_else(|| {
                                ProviderError::Provider(
                                    "amazon-bedrock anthropic tool delta received before tool_use start"
                                        .to_owned(),
                                )
                            })?;

                            stream_has_content = true;
                            yield CompletionStreamEvent::ToolCallDelta {
                                provider_id: provider_id.clone(),
                                model: stream_model.clone(),
                                stream_key: format!("idx:{index}"),
                                id: Some(state.id.clone()),
                                name: Some(state.name.clone()),
                                arguments_delta,
                            };
                        }
                    }
                    BedrockAnthropicStreamEvent::ContentBlockStop { index } => {
                        if let Some(index) = index {
                            pending_tool_calls.remove(&index);
                        }
                    }
                    BedrockAnthropicStreamEvent::MessageDelta {
                        delta,
                        usage,
                        message,
                    } => {
                        if stream_finish_reason.is_none() {
                            stream_finish_reason = utils::normalize_optional_text(delta.stop_reason)
                                .or_else(|| {
                                    utils::normalize_optional_text(
                                        message
                                            .as_ref()
                                            .and_then(|item| item.stop_reason.clone()),
                                    )
                                });
                        }

                        if let Some(message) = message.as_ref() {
                            if response_id.is_none() {
                                response_id = utils::normalize_optional_text(message.id.clone());
                            }
                            if let Some(model) = utils::normalize_optional_text(message.model.clone()) {
                                stream_model = ModelId::new(model);
                            }
                        }

                        if let Some(usage) = usage.or_else(|| message.and_then(|item| item.usage)) {
                            stream_usage =
                                Some(merge_bedrock_anthropic_usage(stream_usage.take(), usage));
                        }
                    }
                    BedrockAnthropicStreamEvent::MessageStop { usage, message } => {
                        if stream_finish_reason.is_none() {
                            stream_finish_reason = utils::normalize_optional_text(
                                message
                                    .as_ref()
                                    .and_then(|item| item.stop_reason.clone()),
                            );
                        }

                        if let Some(message) = message.as_ref() {
                            if response_id.is_none() {
                                response_id = utils::normalize_optional_text(message.id.clone());
                            }
                            if let Some(model) = utils::normalize_optional_text(message.model.clone()) {
                                stream_model = ModelId::new(model);
                            }
                        }

                        if let Some(usage) = usage.or_else(|| message.and_then(|item| item.usage)) {
                            stream_usage =
                                Some(merge_bedrock_anthropic_usage(stream_usage.take(), usage));
                        }

                        break;
                    }
                    BedrockAnthropicStreamEvent::Other => {}
                }
            }

            if stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some() {
                let thinking_blocks = thinking_blocks
                    .into_values()
                    .filter_map(BedrockAnthropicThinkingBlockState::into_value)
                    .collect::<Vec<_>>();
                let mut provider_metadata = response_id_metadata(response_id);
                if !thinking_blocks.is_empty() {
                    let metadata = provider_metadata
                        .get_or_insert_with(|| serde_json::json!({}));
                    metadata["anthropic_thinking_blocks"] = Value::Array(thinking_blocks);
                }
                yield CompletionStreamEvent::Completed {
                    provider_id,
                    model: stream_model,
                    finish_reason: CompletionFinishReason::normalize_with_tool_calls(
                        CompletionFinishReason::from_provider(stream_finish_reason.as_deref()),
                        stream_tool_call_seen,
                    ),
                    usage: stream_usage.map(map_bedrock_anthropic_usage),
                    provider_metadata,
                    end_turn: None,
                };
            }
        };

        Ok(Box::pin(stream))
    }

    pub(super) async fn complete_sigv4(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        if Self::is_native_anthropic_model(request.model.as_ref()) {
            return self
                .complete_sigv4_anthropic(profile, static_credentials, request)
                .await;
        }

        let prompt_cache_key = request.prompt_cache_key.clone();
        let model = self.resolve_model(request.model.as_ref());
        let messages = Self::chat_messages_for_request(&request);
        let body = ChatCompletionRequest {
            model,
            messages,
            tools: (!request.tool_api_functions.is_empty()).then(|| {
                request
                    .tool_api_functions
                    .iter()
                    .cloned()
                    .map(|tool| crate::provider::chat_wire::ChatToolDefinition {
                        kind: "function".to_owned(),
                        function: crate::provider::chat_wire::ChatFunctionDefinition {
                            name: bedrock_wire_tool_name(tool.name.as_str()),
                            description: tool.description,
                            parameters: tool.input_schema,
                            strict: tool.strict,
                        },
                    })
                    .collect()
            }),
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            max_completion_tokens: None,
            cache_control: None,
            stream: false,
            stream_options: None,
            stop: Vec::new(),
            top_p: None,
            seed: None,
            response_format: None,
            reasoning_effort: None,
            verbosity: None,
            prompt_cache_key: prompt_cache_key.clone(),
            parallel_tool_calls: request.request_override.parallel_tool_calls(),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;
        let mut headers = vec![(
            reqwest::header::CONTENT_TYPE.as_str().to_owned(),
            JSON_CONTENT_TYPE.to_owned(),
        )];
        if let Some(session_affinity) = prompt_cache_key {
            headers.push(("x-session-affinity".to_owned(), session_affinity));
        }
        headers.extend(
            request
                .request_override
                .headers
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );

        let response = self
            .send_sigv4_request(
                "complete.chat",
                profile,
                static_credentials,
                Sigv4Request {
                    method: reqwest::Method::POST,
                    url: self.completions_endpoint(),
                    body: Some(serde_json::to_vec(&body_json)?),
                    headers,
                    body_debug: Some(&body_json),
                },
            )
            .await?;

        let payload: ChatCompletionResponse =
            utils::parse_json_response_logged(PROVIDER_ID, ADAPTER_KIND, "complete.chat", response)
                .await?;
        self.parse_completion(payload)
    }

    pub(super) async fn complete_stream_sigv4(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        if Self::is_native_anthropic_model(request.model.as_ref()) {
            return self
                .complete_stream_sigv4_anthropic(profile, static_credentials, request)
                .await;
        }

        let prompt_cache_key = request.prompt_cache_key.clone();
        let model = self.resolve_model(request.model.as_ref());
        let messages = Self::chat_messages_for_request(&request);

        let body = ChatCompletionRequest {
            model: model.clone(),
            messages,
            tools: (!request.tool_api_functions.is_empty()).then(|| {
                request
                    .tool_api_functions
                    .iter()
                    .cloned()
                    .map(|tool| crate::provider::chat_wire::ChatToolDefinition {
                        kind: "function".to_owned(),
                        function: crate::provider::chat_wire::ChatFunctionDefinition {
                            name: bedrock_wire_tool_name(tool.name.as_str()),
                            description: tool.description,
                            parameters: tool.input_schema,
                            strict: tool.strict,
                        },
                    })
                    .collect()
            }),
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            max_completion_tokens: None,
            cache_control: None,
            stream: true,
            stream_options: Some(ChatStreamOptions {
                include_usage: true,
            }),
            stop: Vec::new(),
            top_p: None,
            seed: None,
            response_format: None,
            reasoning_effort: None,
            verbosity: None,
            prompt_cache_key: prompt_cache_key.clone(),
            parallel_tool_calls: request.request_override.parallel_tool_calls(),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;
        let mut headers = vec![(
            reqwest::header::CONTENT_TYPE.as_str().to_owned(),
            JSON_CONTENT_TYPE.to_owned(),
        )];
        if let Some(session_affinity) = prompt_cache_key {
            headers.push(("x-session-affinity".to_owned(), session_affinity));
        }
        headers.extend(
            request
                .request_override
                .headers
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );

        let response = self
            .send_sigv4_request(
                "complete_stream.chat",
                profile,
                static_credentials,
                Sigv4Request {
                    method: reqwest::Method::POST,
                    url: self.completions_endpoint(),
                    body: Some(serde_json::to_vec(&body_json)?),
                    headers,
                    body_debug: Some(&body_json),
                },
            )
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response_logged(
                PROVIDER_ID,
                ADAPTER_KIND,
                "complete_stream.chat",
                response,
            )
            .await);
        }

        utils::adapter_log_http_response_open(
            PROVIDER_ID,
            ADAPTER_KIND,
            "complete_stream.chat",
            response.status(),
            response.headers(),
        );
        let mut events = sse::json_events_with_done(response);
        let provider_id = ProviderId::new(PROVIDER_ID);
        let model_name = ModelId::new(model);

        let stream = async_stream::try_stream! {
            let mut tool_stream = agena_provider::ToolStreamAccumulator::new();
            let mut saw_tool_call = false;
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut done_seen = false;
            let mut assistant_reasoning_field_seen: Option<&'static str> = None;
            let mut reasoning_details_seen: Option<serde_json::Value> = None;
            let mut copilot_reasoning_opaque: Option<String> = None;

            while let Some(event) = events.next().await {
                let event = match event
                    .map_err(|err| utils::json_stream_error(PROVIDER_ID, err))?
                {
                    sse::JsonEventPayload::Event(event) => event,
                    sse::JsonEventPayload::Done => {
                        done_seen = true;
                        break;
                    }
                };
                utils::adapter_log_stream_event(
                    PROVIDER_ID,
                    ADAPTER_KIND,
                    "complete_stream.chat",
                    &event,
                );
                if let Some(err) = utils::chat_stream_error(PROVIDER_ID, &event) {
                    Err(err)?;
                }
                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(PROVIDER_ID, "chat stream chunk", event)?;
                let choice = chunk.choices.first();

                let delta = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.content.as_ref())
                    .map(openai_chat_extract_text)
                    .or_else(|| choice.and_then(|item| item.text.clone()))
                    .unwrap_or_default();

                if !delta.is_empty() {
                    yield CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                let response_delta = choice.and_then(|item| item.delta.as_ref());
                if let Some(delta) = response_delta {
                    if assistant_reasoning_field_seen.is_none() {
                        assistant_reasoning_field_seen =
                            openai_chat_reasoning_field(
                                delta.reasoning_content.as_ref(),
                                delta.reasoning_details.as_ref(),
                            );
                    }
                    if let Some(details) = delta.reasoning_details.as_ref() {
                        merge_openai_chat_reasoning_details(
                            &mut reasoning_details_seen,
                            details,
                        );
                    }
                    if let Some(opaque) = delta
                        .reasoning_opaque
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                    {
                        if copilot_reasoning_opaque
                            .as_deref()
                            .is_some_and(|current| current != opaque)
                        {
                            Err(ProviderError::Provider(format!(
                                "{PROVIDER_ID} returned multiple reasoning_opaque values in one response"
                            )))?;
                        }
                        copilot_reasoning_opaque = Some(opaque.to_owned());
                    }
                }
                let reasoning_delta = response_delta
                    .and_then(|delta| {
                        openai_chat_extract_reasoning_text(
                            delta.reasoning_content.as_ref(),
                            delta.reasoning_details.as_ref(),
                            delta.reasoning_text.as_ref(),
                        )
                    })
                    .unwrap_or_default();
                if !reasoning_delta.is_empty() {
                    yield CompletionStreamEvent::ThinkingDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta: reasoning_delta,
                    };
                }

                let tool_deltas = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.tool_calls.clone())
                    .unwrap_or_default();

                for raw_tool in tool_deltas {
                    saw_tool_call = true;
                    let tool = utils::parse_json_value::<ChatToolCallWire>(
                        PROVIDER_ID,
                        "chat stream tool_call delta",
                        raw_tool,
                    )?;
                    // Route through the shared accumulator so a call whose
                    // index/id varies across chunks stays on one stream key,
                    // instead of splitting into two tool calls (and losing
                    // arguments). This mirrors the OpenAI chat adapter.
                    let input = crate::provider::openai::chat_tool_stream_input(
                        PROVIDER_ID,
                        tool,
                    )?;
                    for update in tool_stream.ingest(PROVIDER_ID, input)? {
                        yield crate::provider::openai::completion_event_from_tool_stream_update(
                            &provider_id,
                            &model_name,
                            update,
                        );
                    }
                }

                if let Some(raw_usage) = chunk.usage {
                    let usage = utils::parse_json_value::<ChatUsage>(
                        PROVIDER_ID,
                        "chat stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Some(
                        crate::provider::chat_wire::chat_usage_to_completion(usage),
                    );
                }

                let finish_reason = utils::normalize_optional_text(
                    choice
                        .and_then(|item| item.finish_reason.as_deref())
                        .map(ToOwned::to_owned),
                )
                    .filter(|value| !value.eq_ignore_ascii_case("null"));

                if stream_finish_reason.is_none() {
                    stream_finish_reason = finish_reason;
                }
            }

            utils::require_terminal_stream_event(
                PROVIDER_ID,
                "bedrock chat completions",
                stream_finish_reason.is_some(),
            )?;
            utils::require_terminal_stream_event(
                PROVIDER_ID,
                "bedrock chat completions [DONE]",
                done_seen,
            )?;
            yield CompletionStreamEvent::Completed {
                provider_id,
                model: model_name.clone(),
                finish_reason: CompletionFinishReason::normalize_with_tool_calls(
                    CompletionFinishReason::from_provider(stream_finish_reason.as_deref()),
                    saw_tool_call,
                ),
                usage: stream_usage,
                provider_metadata: utils::provider_metadata_with_chat_reasoning_state(
                    None,
                    assistant_reasoning_field_seen,
                    reasoning_details_seen,
                    copilot_reasoning_opaque,
                ),
                end_turn: None,
            };
        };

        Ok(Box::pin(stream))
    }
}
