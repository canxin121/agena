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
        should_retry_credential, sse, utils,
    },
    role::Role,
};

const PROVIDER_ID: &str = "anthropic";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Clone)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    include_thinking: bool,
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
        Self {
            client,
            api_key,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            include_thinking: false,
            auth_header: "x-api-key".to_owned(),
            auth_scheme: None,
            extra_headers: HashMap::from([(
                "anthropic-beta".to_owned(),
                "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14"
                    .to_owned(),
            )]),
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

    pub fn with_include_thinking(mut self, include_thinking: bool) -> Self {
        self.include_thinking = include_thinking;
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

    fn content_to_blocks(message: &Message) -> Vec<AnthropicTextBlock> {
        let projected = utils::project_session_parts(message);
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
                utils::ProjectedSessionPart::Text { text } => {
                    blocks.push(AnthropicTextBlock::text(text));
                }
                utils::ProjectedSessionPart::Attachment { item } => {
                    blocks.extend(Self::attachment_blocks(&item));
                }
                utils::ProjectedSessionPart::ToolCall {
                    id,
                    name,
                    arguments_json,
                } => blocks.push(AnthropicTextBlock::tool_use(id, name, arguments_json)),
                utils::ProjectedSessionPart::ToolResult {
                    tool_call_id,
                    output_json,
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
            AttachmentKind::File => utils::attachment_text(item)
                .map(AnthropicTextBlock::text)
                .into_iter()
                .collect(),
            AttachmentKind::Audio | AttachmentKind::Video => Vec::new(),
        }
        .into_iter()
        .chain(
            match item.kind {
                AttachmentKind::Audio | AttachmentKind::Video => {
                    Some(AnthropicTextBlock::text(utils::attachment_hint_text(item)))
                }
                AttachmentKind::Image | AttachmentKind::Pdf
                    if Self::binary_source(item).is_none() =>
                {
                    Some(AnthropicTextBlock::text(utils::attachment_hint_text(item)))
                }
                AttachmentKind::File if utils::attachment_text(item).is_none() => {
                    Some(AnthropicTextBlock::text(utils::attachment_hint_text(item)))
                }
                _ => None,
            }
            .into_iter(),
        )
        .collect()
    }

    fn binary_source(item: &AttachmentItem) -> Option<AnthropicBinarySource> {
        utils::attachment_base64_with_mime(item)
            .map(|(media_type, data)| AnthropicBinarySource::base64(media_type, data))
    }

    fn tools(tools: &[crate::tool::ToolDefinition]) -> Vec<AnthropicToolDefinition> {
        tools
            .iter()
            .map(|tool| AnthropicToolDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
            .collect()
    }

    async fn send_json<R>(&self, endpoint: String, body: &impl Serialize) -> Result<R, AppError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = self
            .send_request(|api_key| {
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
        utils::apply_extra_headers(req, &self.extra_headers)
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

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn model_capabilities(&self, model: &ModelId) -> crate::provider::ModelCapabilities {
        crate::provider::default_capability_registry()
            .capabilities_for_family(crate::provider::CapabilityFamily::Anthropic, model.as_str())
    }

    fn model_metadata(&self, model: &ModelId) -> crate::provider::ModelMetadata {
        crate::provider::default_model_metadata_registry()
            .metadata_for_family(crate::provider::CapabilityFamily::Anthropic, model.as_str())
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let response = self
            .send_request(|api_key| {
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

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = request.model.clone();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(system.clone());
        }
        let tools = (!request.tools.is_empty()).then(|| Self::tools(request.tools.as_slice()));

        let mut messages = Vec::new();
        for msg in request.messages {
            match msg.role {
                Role::System => system_chunks.push(msg.as_text_lossy()),
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

        let body = AnthropicMessagesRequest {
            model: model.to_string(),
            max_tokens: request.max_output_tokens.unwrap_or(4096),
            system: (!system_chunks.is_empty()).then(|| system_chunks.join("\n\n")),
            messages,
            tools,
            temperature: request.temperature,
            stream: None,
        };

        let response: AnthropicMessagesResponse =
            self.send_json(self.messages_endpoint(), &body).await?;
        let text = response
            .content
            .iter()
            .filter(|c| c.kind == "text" || (self.include_thinking && c.kind == "thinking"))
            .filter_map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("");

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

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(
                "anthropic completion payload was empty without finish reason".to_owned(),
            ));
        }

        Ok(CompletionResponse {
            provider_id: ProviderId::new(PROVIDER_ID),
            model: ModelId::new(response.model),
            text,
            finish_reason,
            tool_calls,
            usage: Self::map_usage(response.usage),
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
        let model = request.model.clone();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(system.clone());
        }
        let tools = (!request.tools.is_empty()).then(|| Self::tools(request.tools.as_slice()));

        let mut messages = Vec::new();
        for msg in request.messages {
            match msg.role {
                Role::System => system_chunks.push(msg.as_text_lossy()),
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

        let body = AnthropicMessagesRequest {
            model: model.to_string(),
            max_tokens: request.max_output_tokens.unwrap_or(4096),
            system: (!system_chunks.is_empty()).then(|| system_chunks.join("\n\n")),
            messages,
            tools,
            temperature: request.temperature,
            stream: Some(true),
        };

        let response = self
            .send_request(|api_key| {
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
        let include_thinking = self.include_thinking;

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

                        if let Some(arguments_delta) = content_block
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
                        {
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
                    AnthropicSseEvent::ContentBlockDelta { index, delta } => {
                        let text = delta
                            .text
                            .clone()
                            .or_else(|| if include_thinking { delta.thinking.clone() } else { None })
                            .filter(|value| !value.is_empty());

                        if let Some(delta) = text {
                            stream_has_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta,
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
                            stream_usage = Some(usage);
                        }
                    }
                    AnthropicSseEvent::MessageStop { usage, message } => {
                        if stream_finish_reason.is_none() {
                            stream_finish_reason = message
                                .as_ref()
                                .and_then(|item| item.stop_reason.clone());
                        }

                        if let Some(usage) = usage.or_else(|| message.and_then(|item| item.usage)) {
                            stream_usage = Some(usage);
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
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AnthropicToolDefinition {
    name: String,
    description: String,
    input_schema: Value,
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
    MessageUsage {
        input_tokens: u.input_tokens.unwrap_or_default(),
        output_tokens: u.output_tokens.unwrap_or_default(),
        reasoning_tokens: 0,
        cache_write_tokens: u.cache_creation_input_tokens.unwrap_or_default(),
        cache_read_tokens: u.cache_read_input_tokens.unwrap_or_default(),
        total_cost: 0.0,
    }
    .into()
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
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicSseEvent {
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
    async fn complete_parses_tool_use_object_input() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/messages")
            .match_header(
                "anthropic-beta",
                "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14",
            )
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
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test]
    async fn complete_stream_parses_typed_anthropic_events() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":3,\"output_tokens\":2,\"cache_creation_input_tokens\":1,\"cache_read_input_tokens\":1}}\n\n",
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
}
