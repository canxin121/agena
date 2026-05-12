use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::{
    error::AppError,
    message::{AttachmentItem, AttachmentKind, Message, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ManagedCredential, ModelProvider, ProviderModel,
        ThinkingRequest, prompt_cache, sse, utils, wire_message,
    },
    role::Role,
};

const PROVIDER_ID: &str = "anthropic";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const FIRST_PARTY_ANTHROPIC_HOSTS: &[&str] = &["api.anthropic.com", "api-staging.anthropic.com"];
const DEFAULT_ANTHROPIC_BETA_HEADER: &str =
    "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14";

#[derive(Clone)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    auth_header: String,
    auth_scheme: Option<String>,
    extra_headers: HashMap<String, String>,
}

impl AnthropicProvider {
    pub fn new(
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_managed(
            client,
            ManagedCredential::static_value("anthropic api key", api_key.into()),
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
        let base_url = utils::normalize_base_url(base_url.into().as_str());
        let mut extra_headers = HashMap::new();
        if Self::is_first_party_base_url(base_url.as_str()) {
            extra_headers.insert(
                "anthropic-beta".to_owned(),
                DEFAULT_ANTHROPIC_BETA_HEADER.to_owned(),
            );
        }

        Self {
            client,
            api_key,
            base_url,
            default_model: ModelId::new(default_model),
            auth_header: "x-api-key".to_owned(),
            auth_scheme: None,
            extra_headers,
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
        self.extra_headers.extend(headers);
        self
    }

    fn models_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn messages_endpoint(&self) -> String {
        format!("{}/messages", self.base_url)
    }

    fn map_usage(usage: Option<AnthropicUsage>) -> Option<CompletionUsage> {
        usage.map(map_anthropic_usage)
    }

    async fn complete_by_aggregating_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let fallback_model = request.model.clone();
        let stream = ModelProvider::complete_stream(self, request).await?;
        utils::aggregate_stream(PROVIDER_ID, fallback_model, stream).await
    }

    fn content_to_blocks(message: &Message) -> Vec<AnthropicTextBlock> {
        let projected = wire_message::project(message);
        if projected.is_empty() {
            let text = message.as_text_lossy();
            if text.is_empty() {
                return Vec::new();
            }

            if message.role == Role::Tool {
                return vec![AnthropicTextBlock::tool_result("tool", text)];
            }

            return vec![AnthropicTextBlock::text(text)];
        }

        let mut blocks = Vec::new();
        for part in projected {
            match part {
                wire_message::WirePart::Text { text } => {
                    blocks.push(AnthropicTextBlock::text(text));
                }
                wire_message::WirePart::Attachment { item } => {
                    blocks.extend(Self::attachment_blocks(&item));
                }
                wire_message::WirePart::ToolCall {
                    id,
                    name,
                    arguments_json,
                } => blocks.push(AnthropicTextBlock::tool_use(id, name, arguments_json)),
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    output_json,
                    ..
                } => blocks.push(AnthropicTextBlock::tool_result(tool_call_id, output_json)),
            }
        }

        blocks
    }

    fn attachment_blocks(item: &AttachmentItem) -> Vec<AnthropicTextBlock> {
        match item.kind {
            AttachmentKind::Image => Self::binary_source(item)
                .map(AnthropicTextBlock::image)
                .into_iter()
                .collect(),
            AttachmentKind::Pdf => Self::binary_source(item)
                .map(AnthropicTextBlock::document)
                .into_iter()
                .collect(),
            AttachmentKind::File => wire_message::attachment_text(item)
                .map(AnthropicTextBlock::text)
                .into_iter()
                .collect(),
            AttachmentKind::Audio | AttachmentKind::Video => Vec::new(),
        }
        .into_iter()
        .chain(match item.kind {
            AttachmentKind::Audio | AttachmentKind::Video => {
                Some(AnthropicTextBlock::text(wire_message::hint_text(item)))
            }
            AttachmentKind::Image | AttachmentKind::Pdf if Self::binary_source(item).is_none() => {
                Some(AnthropicTextBlock::text(wire_message::hint_text(item)))
            }
            AttachmentKind::File if wire_message::attachment_text(item).is_none() => {
                Some(AnthropicTextBlock::text(wire_message::hint_text(item)))
            }
            _ => None,
        })
        .collect()
    }

    fn binary_source(item: &AttachmentItem) -> Option<AnthropicBinarySource> {
        wire_message::base64_with_mime(item)
            .map(|(media_type, data)| AnthropicBinarySource::base64(media_type, data))
    }

    fn is_first_party_base_url(base_url: &str) -> bool {
        url::Url::parse(base_url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.to_owned()))
            .map(|host| {
                FIRST_PARTY_ANTHROPIC_HOSTS
                    .iter()
                    .any(|candidate| host.eq_ignore_ascii_case(candidate))
            })
            .unwrap_or(false)
    }

    fn supports_eager_input_streaming(&self) -> bool {
        Self::is_first_party_base_url(self.base_url.as_str())
    }

    fn tools(&self, tools: &[crate::tool::EntryDefinition]) -> Vec<AnthropicEntryDefinition> {
        tools
            .iter()
            .map(|tool| AnthropicEntryDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
                cache_control: None,
                eager_input_streaming: self.supports_eager_input_streaming().then_some(true),
            })
            .collect()
    }

    fn apply_prompt_cache_hints(
        system: &mut [AnthropicTextBlock],
        tools: &mut [AnthropicEntryDefinition],
        messages: &mut [AnthropicMessage],
    ) {
        // Keep Anthropic cache markers within the documented four-breakpoint
        // envelope: up to two system blocks, the final tool schema, and the
        // last message block. Claude Code also deliberately uses a single
        // message-level marker to avoid unstable intermediate breakpoints.
        for block in system.iter_mut().take(2) {
            block.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }

        if let Some(tool) = tools.last_mut() {
            tool.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }

        if let Some(block) = messages
            .iter_mut()
            .rev()
            .find_map(|message| message.content.last_mut())
        {
            block.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }
    }

    async fn send_json<R>(&self, endpoint: String, body: &impl Serialize) -> Result<R, AppError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            self.apply_headers(
                self.client
                    .post(endpoint.clone())
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
                api_key,
            )
            .json(body)
        })
        .await?;

        utils::parse_json_response(PROVIDER_ID, response).await
    }

    fn apply_headers(
        &self,
        req: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        let auth_value = utils::auth_header_value(self.auth_scheme.as_deref(), api_key);
        let req = req.header(self.auth_header.as_str(), auth_value);
        utils::apply_request_headers(PROVIDER_ID, req, &self.extra_headers)
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(crate::provider::CapabilityFamily::Anthropic)
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        Some(
            crate::provider::PromptCacheShape::new(PROVIDER_ID)
                .with_string("auth_scope", self.api_key.prompt_cache_scope())
                .with_string("base_url", self.base_url.as_str())
                .with_string("auth_header", self.auth_header.as_str())
                .with_optional_string("auth_scheme", self.auth_scheme.as_deref())
                .with_bool(
                    "first_party_base_url",
                    Self::is_first_party_base_url(self.base_url.as_str()),
                )
                .with_bool(
                    "eager_input_streaming",
                    self.supports_eager_input_streaming(),
                )
                .with_json(
                    "extra_headers",
                    &utils::prompt_cache_header_entries(&self.extra_headers),
                ),
        )
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            self.apply_headers(
                self.client
                    .get(self.models_endpoint())
                    .header("anthropic-version", ANTHROPIC_VERSION),
                api_key,
            )
        })
        .await?;

        let payload: AnthropicModelListResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;
        Ok(payload
            .data
            .into_iter()
            .map(|m| {
                let mut model = ProviderModel::new(PROVIDER_ID, m.id);
                let capabilities = self.model_capabilities(&model.id);
                model = model.with_capabilities(capabilities);
                model.display_name = m.display_name;
                model
            })
            .collect())
    }

    #[tracing::instrument(skip_all, fields(provider = "anthropic", model = %request.model))]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = request.model.clone();
        let stream_fallback_request = request.clone();

        let thinking_body = anthropic_thinking_body(request.thinking.as_ref());
        let include_thinking = thinking_body.is_some();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(AnthropicTextBlock::text(system.clone()));
        }
        let mut tools = (!request.tools.is_empty()).then(|| self.tools(request.tools.as_slice()));

        let mut messages = Vec::new();
        for msg in request.messages {
            match msg.role {
                Role::System => {
                    let text = msg.as_text_lossy();
                    if !text.trim().is_empty() {
                        system_chunks.push(AnthropicTextBlock::text(text));
                    }
                }
                Role::Assistant => messages.push(AnthropicMessage {
                    role: "assistant".to_owned(),
                    content: Self::content_to_blocks(&msg),
                }),
                Role::User | Role::Tool => messages.push(AnthropicMessage {
                    role: "user".to_owned(),
                    content: Self::content_to_blocks(&msg),
                }),
            }
        }
        Self::apply_prompt_cache_hints(
            system_chunks.as_mut_slice(),
            tools.as_deref_mut().unwrap_or(&mut []),
            messages.as_mut_slice(),
        );

        let body = AnthropicMessagesRequest {
            model: model.to_string(),
            max_tokens: request.max_output_tokens.unwrap_or(4096),
            system: (!system_chunks.is_empty()).then_some(system_chunks),
            messages,
            tools,
            temperature: request.temperature,
            stream: None,
            thinking: thinking_body,
            stop_sequences: request.stop_sequences,
            top_p: request.top_p,
            top_k: request.top_k,
        };

        let response: AnthropicMessagesResponse =
            self.send_json(self.messages_endpoint(), &body).await?;

        let text = response
            .content
            .iter()
            .filter(|c| c.kind == "text")
            .filter_map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("");

        let reasoning_text = if include_thinking {
            let thinking = response
                .content
                .iter()
                .filter(|c| c.kind == "thinking")
                .filter_map(|c| c.text.clone())
                .collect::<Vec<_>>()
                .join("");
            if thinking.is_empty() {
                None
            } else {
                Some(thinking)
            }
        } else {
            None
        };

        let tool_calls = response
            .content
            .iter()
            .filter(|c| c.kind == "tool_use")
            .map(|c| {
                let id = utils::normalize_optional_text(c.id.clone()).ok_or_else(|| {
                    AppError::Provider("anthropic returned tool_use block without id".to_owned())
                })?;

                let name = utils::normalize_optional_text(c.name.clone()).ok_or_else(|| {
                    AppError::Provider("anthropic returned tool_use block without name".to_owned())
                })?;

                Ok(CompletionToolCall::Function {
                    id,
                    name,
                    arguments_json: c
                        .input
                        .as_ref()
                        .map(json_value_to_string)
                        .unwrap_or_else(|| "{}".to_owned()),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;

        let finish_reason = CompletionFinishReason::from_provider(response.stop_reason.as_deref());

        if text.is_empty() && tool_calls.is_empty() {
            return self
                .complete_by_aggregating_stream(stream_fallback_request)
                .await;
        }

        Ok(CompletionResponse {
            provider_id: ProviderId::new(PROVIDER_ID),
            model: ModelId::new(response.model),
            text,
            reasoning_text,
            finish_reason,
            tool_calls,
            usage: Self::map_usage(response.usage),
            provider_metadata: None,
        })
    }

    #[tracing::instrument(skip_all, fields(provider = "anthropic", model = %request.model))]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = request.model.clone();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(AnthropicTextBlock::text(system.clone()));
        }
        let mut tools = (!request.tools.is_empty()).then(|| self.tools(request.tools.as_slice()));

        let mut messages = Vec::new();
        for msg in request.messages {
            match msg.role {
                Role::System => {
                    let text = msg.as_text_lossy();
                    if !text.trim().is_empty() {
                        system_chunks.push(AnthropicTextBlock::text(text));
                    }
                }
                Role::Assistant => messages.push(AnthropicMessage {
                    role: "assistant".to_owned(),
                    content: Self::content_to_blocks(&msg),
                }),
                Role::User | Role::Tool => messages.push(AnthropicMessage {
                    role: "user".to_owned(),
                    content: Self::content_to_blocks(&msg),
                }),
            }
        }
        Self::apply_prompt_cache_hints(
            system_chunks.as_mut_slice(),
            tools.as_deref_mut().unwrap_or(&mut []),
            messages.as_mut_slice(),
        );

        let body = AnthropicMessagesRequest {
            model: model.to_string(),
            max_tokens: request.max_output_tokens.unwrap_or(4096),
            system: (!system_chunks.is_empty()).then_some(system_chunks),
            messages,
            tools,
            temperature: request.temperature,
            stream: Some(true),
            thinking: anthropic_thinking_body(request.thinking.as_ref()),
            stop_sequences: request.stop_sequences,
            top_p: request.top_p,
            top_k: request.top_k,
        };

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            self.apply_headers(
                self.client
                    .post(self.messages_endpoint())
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
                api_key,
            )
            .json(&body)
        })
        .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(PROVIDER_ID);
        let model_name = model;
        let include_thinking = anthropic_thinking_body(request.thinking.as_ref()).is_some();

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: HashMap<usize, AnthropicToolCallState> = HashMap::new();
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_usage: Option<AnthropicUsage> = None;
            let mut stream_has_content = false;

            while let Some(event) = events.next().await {
                let event = event?;
                let parsed: AnthropicSseEvent =
                    utils::parse_json_value(provider_id.as_str(), "stream event", event)?;

                match parsed {
                    AnthropicSseEvent::MessageStart { message } => {
                        if let Some(usage) = message.usage {
                            stream_usage =
                                Some(merge_anthropic_usage(stream_usage.take(), usage));
                        }
                    }
                    AnthropicSseEvent::ContentBlockStart {
                        index,
                        content_block,
                    } => {
                        if content_block.kind != "tool_use" {
                            continue;
                        }

                        let index = index.ok_or_else(|| {
                            AppError::Provider(
                                "anthropic tool_use stream event missing content block index"
                                    .to_owned(),
                            )
                        })?;

                        let id = utils::normalize_optional_text(content_block.id.clone()).ok_or_else(|| {
                            AppError::Provider(
                                "anthropic tool_use stream event missing tool id".to_owned(),
                            )
                        })?;
                        let name = utils::normalize_optional_text(content_block.name.clone()).ok_or_else(|| {
                            AppError::Provider(
                                "anthropic tool_use stream event missing tool name".to_owned(),
                            )
                        })?;

                        let state = pending_tool_calls.entry(index).or_default();
                        state.id = id;
                        state.name = name;

                        let arguments_delta = content_block
                            .input
                            .as_ref()
                            .map(json_value_to_string)
                            .and_then(|value| {
                                if value.is_empty() || value == "{}" {
                                    None
                                } else {
                                    Some(value)
                                }
                            })
                            .unwrap_or_default();

                        // Always emit at least one ToolCallDelta so the
                        // shared aggregator records the tool call. Without
                        // this, a tool_use block whose input arrives only
                        // via the start event (or with no input at all)
                        // would be dropped because the aggregator only
                        // tracks calls it has seen a delta for.
                        stream_has_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: format!("idx:{index}"),
                            id: Some(state.id.clone()),
                            name: Some(state.name.clone()),
                            arguments_delta,
                        };
                    }
                    AnthropicSseEvent::ContentBlockDelta { index, delta } => {
                        // Text content
                        if let Some(text_delta) = delta.text.clone().filter(|v| !v.is_empty()) {
                            stream_has_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta: text_delta,
                            };
                        }

                        // Thinking/reasoning content — only yield when thinking was requested
                        if include_thinking
                            && let Some(thinking_delta) = delta.thinking.clone().filter(|v| !v.is_empty()) {
                                stream_has_content = true;
                                yield CompletionStreamEvent::ThinkingDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    delta: thinking_delta,
                                };
                            }

                        let is_tool_delta = matches!(delta.kind.as_deref(), Some("input_json_delta"));
                        if is_tool_delta {
                            let Some(arguments_delta) = utils::optional_non_empty(delta.partial_json.clone())
                            else {
                                continue;
                            };

                            let index = index.ok_or_else(|| {
                                AppError::Provider(
                                    "anthropic tool delta event missing content block index"
                                        .to_owned(),
                                )
                            })?;

                            let state = pending_tool_calls.get_mut(&index).ok_or_else(|| {
                                AppError::Provider(
                                    "anthropic tool delta received before tool_use start"
                                        .to_owned(),
                                )
                            })?;

                            stream_has_content = true;
                            yield CompletionStreamEvent::ToolCallDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                stream_key: format!("idx:{index}"),
                                id: Some(state.id.clone()),
                                name: Some(state.name.clone()),
                                arguments_delta,
                            };
                        }
                    }
                    AnthropicSseEvent::ContentBlockStop { index } => {
                        if let Some(index) = index {
                            pending_tool_calls.remove(&index);
                        }
                    }
                    AnthropicSseEvent::MessageDelta {
                        delta,
                        usage,
                        message,
                    } => {
                        if stream_finish_reason.is_none() {
                            stream_finish_reason = delta
                                .stop_reason
                                .or_else(|| message.as_ref().and_then(|item| item.stop_reason.clone()));
                        }

                        if let Some(usage) = usage.or_else(|| message.and_then(|item| item.usage)) {
                            stream_usage =
                                Some(merge_anthropic_usage(stream_usage.take(), usage));
                        }
                    }
                    AnthropicSseEvent::MessageStop { usage, message } => {
                        if stream_finish_reason.is_none() {
                            stream_finish_reason = message
                                .as_ref()
                                .and_then(|item| item.stop_reason.clone());
                        }

                        if let Some(usage) = usage.or_else(|| message.and_then(|item| item.usage)) {
                            stream_usage =
                                Some(merge_anthropic_usage(stream_usage.take(), usage));
                        }

                        break;
                    }
                    AnthropicSseEvent::Other => {}
                }
            }

            if stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some() {
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage.map(map_anthropic_usage),
                    provider_metadata: None,
                };
            }
        };

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Serialize)]
struct AnthropicMessagesRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<AnthropicTextBlock>>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicEntryDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    stop_sequences: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
}

#[derive(Debug, Serialize)]
struct AnthropicEntryDefinition {
    name: String,
    description: String,
    input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_control: Option<prompt_cache::PromptCacheControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    eager_input_streaming: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicTextBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicTextBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    source: Option<AnthropicBinarySource>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<Value>,
    #[serde(default)]
    tool_use_id: Option<String>,
    #[serde(default)]
    content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_control: Option<prompt_cache::PromptCacheControl>,
}

impl AnthropicTextBlock {
    fn text(text: impl Into<String>) -> Self {
        Self {
            kind: "text".to_owned(),
            text: Some(text.into()),
            source: None,
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    fn image(source: AnthropicBinarySource) -> Self {
        Self {
            kind: "image".to_owned(),
            text: None,
            source: Some(source),
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    fn document(source: AnthropicBinarySource) -> Self {
        Self {
            kind: "document".to_owned(),
            text: None,
            source: Some(source),
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input_json: impl Into<String>,
    ) -> Self {
        let input = parse_json_or_string(input_json.into());
        Self {
            kind: "tool_use".to_owned(),
            text: None,
            source: None,
            id: Some(id.into()),
            name: Some(name.into()),
            input: Some(input),
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        let content = parse_json_or_string(content.into());
        Self {
            kind: "tool_result".to_owned(),
            text: None,
            source: None,
            id: None,
            name: None,
            input: None,
            tool_use_id: Some(tool_use_id.into()),
            content: Some(content),
            cache_control: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicBinarySource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

impl AnthropicBinarySource {
    fn base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            kind: "base64".to_owned(),
            media_type: media_type.into(),
            data: data.into(),
        }
    }
}

fn parse_json_or_string(raw: String) -> Value {
    serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw))
}

fn map_anthropic_usage(u: AnthropicUsage) -> CompletionUsage {
    let cache_write_tokens = u.cache_creation_input_tokens.unwrap_or_else(|| {
        u.cache_creation
            .as_ref()
            .map(AnthropicCacheCreationUsage::total_input_tokens)
            .unwrap_or_default()
    });

    MessageUsage {
        input_tokens: u.input_tokens.unwrap_or_default(),
        output_tokens: u.output_tokens.unwrap_or_default(),
        reasoning_tokens: 0,
        cache_write_tokens,
        cache_read_tokens: u.cache_read_input_tokens.unwrap_or_default(),
        total_cost: 0.0,
    }
    .into()
}

fn anthropic_thinking_body(thinking: Option<&ThinkingRequest>) -> Option<serde_json::Value> {
    let budget_tokens = match thinking? {
        ThinkingRequest::Enabled { budget_tokens } => *budget_tokens,
        ThinkingRequest::Effort { effort } => match effort {
            crate::provider::ReasoningEffort::Minimal => 1_024,
            crate::provider::ReasoningEffort::Low => 4_000,
            crate::provider::ReasoningEffort::Medium => 10_000,
            crate::provider::ReasoningEffort::High => 16_000,
            crate::provider::ReasoningEffort::Xhigh | crate::provider::ReasoningEffort::Max => {
                31_999
            }
        },
        ThinkingRequest::Disabled => return None,
    };
    Some(serde_json::json!({
        "type": "enabled",
        "budget_tokens": budget_tokens
    }))
}

fn merge_anthropic_usage(
    current: Option<AnthropicUsage>,
    update: AnthropicUsage,
) -> AnthropicUsage {
    let Some(current) = current else {
        return update;
    };

    AnthropicUsage {
        input_tokens: update.input_tokens.or(current.input_tokens),
        output_tokens: update.output_tokens.or(current.output_tokens),
        cache_creation_input_tokens: update
            .cache_creation_input_tokens
            .or(current.cache_creation_input_tokens),
        cache_read_input_tokens: update
            .cache_read_input_tokens
            .or(current.cache_read_input_tokens),
        cache_creation: merge_anthropic_cache_creation_usage(
            current.cache_creation,
            update.cache_creation,
        ),
    }
}

fn merge_anthropic_cache_creation_usage(
    current: Option<AnthropicCacheCreationUsage>,
    update: Option<AnthropicCacheCreationUsage>,
) -> Option<AnthropicCacheCreationUsage> {
    match (current, update) {
        (Some(current), Some(update)) => Some(AnthropicCacheCreationUsage {
            ephemeral_1h_input_tokens: update
                .ephemeral_1h_input_tokens
                .or(current.ephemeral_1h_input_tokens),
            ephemeral_5m_input_tokens: update
                .ephemeral_5m_input_tokens
                .or(current.ephemeral_5m_input_tokens),
        }),
        (None, Some(update)) => Some(update),
        (Some(current), None) => Some(current),
        (None, None) => None,
    }
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicModelListResponse {
    data: Vec<AnthropicModel>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessagesResponse {
    model: String,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    content: Vec<AnthropicTextBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation: Option<AnthropicCacheCreationUsage>,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicCacheCreationUsage {
    #[serde(default)]
    ephemeral_1h_input_tokens: Option<u64>,
    #[serde(default)]
    ephemeral_5m_input_tokens: Option<u64>,
}

impl AnthropicCacheCreationUsage {
    fn total_input_tokens(&self) -> u64 {
        self.ephemeral_1h_input_tokens.unwrap_or_default()
            + self.ephemeral_5m_input_tokens.unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicSseEvent {
    MessageStart {
        #[serde(default)]
        message: AnthropicSseMessage,
    },
    ContentBlockStart {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        content_block: AnthropicSseContentBlock,
    },
    ContentBlockDelta {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        delta: AnthropicSseDelta,
    },
    ContentBlockStop {
        #[serde(default)]
        index: Option<usize>,
    },
    MessageDelta {
        #[serde(default)]
        delta: AnthropicSseMessageDelta,
        #[serde(default)]
        usage: Option<AnthropicUsage>,
        #[serde(default)]
        message: Option<AnthropicSseMessage>,
    },
    MessageStop {
        #[serde(default)]
        usage: Option<AnthropicUsage>,
        #[serde(default)]
        message: Option<AnthropicSseMessage>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicSseContentBlock {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicSseDelta {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicSseMessageDelta {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicSseMessage {
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Default)]
struct AnthropicToolCallState {
    id: String,
    name: String,
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;
    use crate::message::Message;
    use crate::provider::CompletionRequest;
    use crate::tool::{EntryBehavior, EntryDefinition};

    fn sample_tool_definition() -> EntryDefinition {
        EntryDefinition::plugin(
            "project_search",
            "Search project files.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
            EntryBehavior::ReadOnly,
            "fixture",
        )
    }

    #[test]
    fn apply_prompt_cache_hints_limits_breakpoints_and_marks_last_tool() {
        let mut system = vec![
            AnthropicTextBlock::text("system-1"),
            AnthropicTextBlock::text("system-2"),
            AnthropicTextBlock::text("system-3"),
        ];
        let mut tools = vec![
            AnthropicEntryDefinition {
                name: "tool_a".to_owned(),
                description: "first".to_owned(),
                input_schema: serde_json::json!({ "type": "object" }),
                cache_control: None,
                eager_input_streaming: None,
            },
            AnthropicEntryDefinition {
                name: "tool_b".to_owned(),
                description: "second".to_owned(),
                input_schema: serde_json::json!({ "type": "object" }),
                cache_control: None,
                eager_input_streaming: None,
            },
        ];
        let mut messages = vec![
            AnthropicMessage {
                role: "user".to_owned(),
                content: vec![AnthropicTextBlock::text("older-user")],
            },
            AnthropicMessage {
                role: "assistant".to_owned(),
                content: vec![AnthropicTextBlock::text("assistant")],
            },
            AnthropicMessage {
                role: "user".to_owned(),
                content: vec![AnthropicTextBlock::text("latest-user")],
            },
        ];

        AnthropicProvider::apply_prompt_cache_hints(
            system.as_mut_slice(),
            tools.as_mut_slice(),
            messages.as_mut_slice(),
        );

        assert!(system[0].cache_control.is_some());
        assert!(system[1].cache_control.is_some());
        assert!(system[2].cache_control.is_none());

        assert!(tools[0].cache_control.is_none());
        assert!(tools[1].cache_control.is_some());

        assert!(messages[0].content[0].cache_control.is_none());
        assert!(messages[1].content[0].cache_control.is_none());
        assert!(messages[2].content[0].cache_control.is_some());
    }

    #[test]
    fn prompt_cache_shape_changes_when_auth_scope_changes() {
        let provider_a = AnthropicProvider::new_managed(
            reqwest::Client::new(),
            ManagedCredential::environment(
                "anthropic env",
                "anthropic",
                "api_key",
                "ANTHROPIC_API_KEY_A",
            ),
            "https://api.anthropic.com",
            "claude-3-7-sonnet-latest",
        );
        let provider_b = AnthropicProvider::new_managed(
            reqwest::Client::new(),
            ManagedCredential::environment(
                "anthropic env",
                "anthropic",
                "api_key",
                "ANTHROPIC_API_KEY_B",
            ),
            "https://api.anthropic.com",
            "claude-3-7-sonnet-latest",
        );

        let shape_a = provider_a
            .prompt_cache_shape(&crate::model::ModelId::new("claude-3-7-sonnet-latest"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&crate::model::ModelId::new("claude-3-7-sonnet-latest"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn first_party_base_url_enables_default_beta_headers_and_eager_input_streaming() {
        let provider = AnthropicProvider::new(
            reqwest::Client::new(),
            "ak-test",
            "https://api.anthropic.com/v1",
            "claude-3-7-sonnet-latest",
        );
        let tools = provider.tools(&[sample_tool_definition()]);

        assert_eq!(
            provider
                .extra_headers
                .get("anthropic-beta")
                .map(String::as_str),
            Some(DEFAULT_ANTHROPIC_BETA_HEADER)
        );
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].eager_input_streaming, Some(true));
    }

    #[test]
    fn proxy_base_url_disables_default_beta_headers_and_eager_input_streaming() {
        let provider = AnthropicProvider::new(
            reqwest::Client::new(),
            "ak-test",
            "https://gateway.example.com/anthropic/v1",
            "claude-3-7-sonnet-latest",
        );
        let tools = provider.tools(&[sample_tool_definition()]);

        assert!(!provider.extra_headers.contains_key("anthropic-beta"));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].eager_input_streaming, None);
    }

    #[test]
    fn map_anthropic_usage_falls_back_to_nested_cache_creation_details() {
        let usage = map_anthropic_usage(AnthropicUsage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            cache_creation_input_tokens: None,
            cache_read_input_tokens: Some(3),
            cache_creation: Some(AnthropicCacheCreationUsage {
                ephemeral_1h_input_tokens: Some(7),
                ephemeral_5m_input_tokens: Some(11),
            }),
        });

        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.cache_write_tokens, 18);
        assert_eq!(usage.cache_read_tokens, 3);
    }

    #[tokio::test]
    async fn complete_parses_tool_use_object_input() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/messages")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "claude-3-7-sonnet-latest",
                    "stop_reason": "tool_use",
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "call_1",
                            "name": "search",
                            "input": { "q": "rust" }
                        }
                    ],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = AnthropicProvider::new(
            reqwest::Client::new(),
            "ak-test",
            server.url(),
            "claude-3-7-sonnet-latest",
        );

        let response = provider
            .complete(CompletionRequest {
                model: crate::model::ModelId::new("claude-3-7-sonnet-latest"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(128),
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
            .expect("tool_use response should parse");

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
    async fn complete_includes_tools_in_messages_request() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/messages")
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
                    "model": "claude-3-7-sonnet-latest",
                    "stop_reason": "end_turn",
                    "content": [{
                        "type": "text",
                        "text": "ok"
                    }],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = AnthropicProvider::new(
            reqwest::Client::new(),
            "ak-test",
            server.url(),
            "claude-3-7-sonnet-latest",
        );

        let response = provider
            .complete(CompletionRequest {
                model: crate::model::ModelId::new("claude-3-7-sonnet-latest"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: vec![sample_tool_definition()],
                temperature: None,
                max_output_tokens: Some(128),
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
            .expect("completion should succeed");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test]
    async fn complete_serializes_prompt_cache_hints_on_system_and_recent_messages() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/messages")
            .match_body(mockito::Matcher::Regex(
                "\\\"cache_control\\\":\\{\\\"type\\\":\\\"ephemeral\\\"\\}".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "claude-3-7-sonnet-latest",
                    "stop_reason": "end_turn",
                    "content": [{
                        "type": "text",
                        "text": "ok"
                    }],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = AnthropicProvider::new(
            reqwest::Client::new(),
            "ak-test",
            server.url(),
            "claude-3-7-sonnet-latest",
        );

        let response = provider
            .complete(CompletionRequest {
                model: crate::model::ModelId::new("claude-3-7-sonnet-latest"),
                system: Some("system".to_string()),
                messages: vec![
                    Message::prompt_text(crate::role::Role::Assistant, "earlier"),
                    Message::prompt_text(crate::role::Role::User, "hello"),
                ],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(128),
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
            .expect("completion should succeed");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test]
    async fn complete_falls_back_to_stream_when_message_payload_is_empty() {
        let mut server = mockito::Server::new_async().await;
        let _message = server
            .mock("POST", "/messages")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "claude-3-7-sonnet-latest",
                    "stop_reason": "end_turn",
                    "content": [],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;
        let _stream = server
            .mock("POST", "/messages")
            .expect(1)
            .match_body(mockito::Matcher::Regex("\\\"stream\\\":true".to_owned()))
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(concat!(
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10}}}\n\n",
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"fallback \"}}\n\n",
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"stream text\"}}\n\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
                "data: {\"type\":\"message_stop\"}\n\n",
                "data: [DONE]\n\n"
            ))
            .create_async()
            .await;

        let provider = AnthropicProvider::new(
            reqwest::Client::new(),
            "ak-test",
            server.url(),
            "claude-3-7-sonnet-latest",
        );

        let response = provider
            .complete(CompletionRequest {
                model: crate::model::ModelId::new("claude-3-7-sonnet-latest"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(128),
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
            .expect("empty message payload should fall back to stream aggregation");

        assert_eq!(response.text, "fallback stream text");
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::Stop)
        ));
        let usage = response.usage.expect("usage should be present");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
    }

    #[tokio::test]
    async fn complete_stream_parses_typed_anthropic_events() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2,\"cache_creation\":{\"ephemeral_5m_input_tokens\":1},\"cache_read_input_tokens\":1}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
            "data: [DONE]\n\n"
        );

        let _mock = server
            .mock("POST", "/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider = AnthropicProvider::new(
            reqwest::Client::new(),
            "ak-test",
            server.url(),
            "claude-3-7-sonnet-latest",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: crate::model::ModelId::new("claude-3-7-sonnet-latest"),
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
            .expect("stream should start");

        let mut text = String::new();
        let mut done = false;

        while let Some(item) = stream.next().await {
            match item.expect("stream item should parse") {
                CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(delta.as_str()),
                CompletionStreamEvent::Completed {
                    finish_reason,
                    usage,
                    ..
                } => {
                    assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                    let usage = usage.expect("usage should be present");
                    assert_eq!(usage.input_tokens, 3);
                    assert_eq!(usage.output_tokens, 2);
                    assert_eq!(usage.cache_write_tokens, 1);
                    assert_eq!(usage.cache_read_tokens, 1);
                    done = true;
                }
                _ => {}
            }
        }

        assert_eq!(text, "Hello");
        assert!(done);
    }

    #[tokio::test]
    async fn complete_stream_emits_tool_call_delta_for_tool_use_events() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"search\",\"input\":{}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"q\\\":\\\"ru\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"st\\\"}\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );

        let _mock = server
            .mock("POST", "/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider = AnthropicProvider::new(
            reqwest::Client::new(),
            "ak-test",
            server.url(),
            "claude-3-7-sonnet-latest",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: crate::model::ModelId::new("claude-3-7-sonnet-latest"),
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
            .expect("stream should start");

        let mut args = String::new();
        let mut done = false;

        while let Some(item) = stream.next().await {
            match item.expect("stream item should parse") {
                CompletionStreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                    ..
                } => {
                    assert_eq!(id.as_deref(), Some("toolu_1"));
                    assert_eq!(name.as_deref(), Some("search"));
                    args.push_str(arguments_delta.as_str());
                }
                CompletionStreamEvent::Completed { finish_reason, .. } => {
                    assert!(matches!(
                        finish_reason,
                        Some(CompletionFinishReason::ToolCalls)
                    ));
                    done = true;
                }
                _ => {}
            }
        }

        assert_eq!(args, "{\"q\":\"rust\"}");
        assert!(done);
    }

    #[tokio::test]
    async fn complete_stream_records_empty_input_tool_use_via_aggregator() {
        let mut server = mockito::Server::new_async().await;
        // Tool with no parameters: ContentBlockStart carries empty input,
        // there are no input_json_delta events, then ContentBlockStop.
        // The aggregator must still surface this as a tool call.
        let body = concat!(
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"now\",\"input\":{}}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}\n\n",
            "data: [DONE]\n\n"
        );

        let _mock = server
            .mock("POST", "/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider = AnthropicProvider::new(
            reqwest::Client::new(),
            "ak-test",
            server.url(),
            "claude-3-7-sonnet-latest",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: crate::model::ModelId::new("claude-3-7-sonnet-latest"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "what time")],
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
            .expect("stream should start");

        let mut tool_call_seen: Option<(String, String, String)> = None;
        let mut completed = false;
        while let Some(item) = stream.next().await {
            match item.expect("stream item should parse") {
                CompletionStreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                    ..
                } => {
                    tool_call_seen = Some((
                        id.unwrap_or_default(),
                        name.unwrap_or_default(),
                        arguments_delta,
                    ));
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

        let (id, name, args) = tool_call_seen.expect("tool call delta should be emitted");
        assert_eq!(id, "toolu_2");
        assert_eq!(name, "now");
        assert!(args.is_empty() || args == "{}");
        assert!(completed);
    }
}
