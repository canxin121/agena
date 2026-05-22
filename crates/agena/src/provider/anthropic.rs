use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use tokio::sync::Mutex;

use crate::{
    error::AppError,
    message::{AttachmentItem, AttachmentKind, Message, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ManagedCredential, ModelRuntime, ProviderModel,
        StreamResumePolicy, ThinkingDisplay, ThinkingRequest, auth::AuthData, prompt_cache, sse,
        utils, wire_message,
    },
    role::Role,
};

const PROVIDER_ID: &str = "anthropic";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const FIRST_PARTY_ANTHROPIC_HOSTS: &[&str] = &["api.anthropic.com", "api-staging.anthropic.com"];
const DEFAULT_ANTHROPIC_BETA_HEADER: &str =
    "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14";
const DEFAULT_COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
const DEFAULT_COPILOT_ANTHROPIC_BETA_HEADER: &str = "interleaved-thinking-2025-05-14";
const ADAPTER_KIND: &str = "anthropic";

#[derive(Clone)]
pub struct AnthropicAdapter {
    id: String,
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    auth_data: Option<Arc<Mutex<AuthData>>>,
    auth_header: String,
    auth_scheme: Option<String>,
    models_url: Option<String>,
    messages_url: Option<String>,
    profile: AnthropicProfile,
    extra_headers: HashMap<String, String>,
    eager_input_streaming_override: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicProfile {
    Standard,
    GithubCopilot,
}

impl AnthropicAdapter {
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
        Self::new_managed_with_id(PROVIDER_ID, client, api_key, base_url, default_model)
    }

    pub fn new_managed_with_id(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        let id = id.into();
        let base_url = utils::normalize_base_url(base_url.into().as_str());
        let mut extra_headers = HashMap::new();
        if Self::is_bundled_base_url(base_url.as_str()) {
            extra_headers.insert(
                "anthropic-beta".to_owned(),
                DEFAULT_ANTHROPIC_BETA_HEADER.to_owned(),
            );
        }
        extra_headers.insert(
            reqwest::header::USER_AGENT.as_str().to_owned(),
            crate::provider::CLAUDE_CODE_API_USER_AGENT.to_owned(),
        );

        Self {
            id,
            client,
            api_key,
            base_url,
            default_model: ModelId::new(default_model),
            auth_data: None,
            auth_header: "x-api-key".to_owned(),
            auth_scheme: None,
            models_url: None,
            messages_url: None,
            profile: AnthropicProfile::Standard,
            extra_headers,
            eager_input_streaming_override: None,
        }
    }

    pub fn with_auth_data(mut self, auth_data: Arc<Mutex<AuthData>>) -> Self {
        self.auth_data = Some(auth_data);
        self
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

    pub fn with_models_url(mut self, models_url: Option<String>) -> Self {
        self.models_url = models_url.and_then(|value| utils::normalize_optional_text(Some(value)));
        self
    }

    pub fn with_messages_url(mut self, messages_url: Option<String>) -> Self {
        self.messages_url =
            messages_url.and_then(|value| utils::normalize_optional_text(Some(value)));
        self
    }

    pub fn with_profile(mut self, profile: AnthropicProfile) -> Self {
        self.profile = profile;
        match profile {
            AnthropicProfile::Standard => {}
            AnthropicProfile::GithubCopilot => {
                self.auth_header = "authorization".to_owned();
                self.auth_scheme = Some("Bearer".to_owned());
                self.extra_headers.insert(
                    "anthropic-beta".to_owned(),
                    DEFAULT_COPILOT_ANTHROPIC_BETA_HEADER.to_owned(),
                );
            }
        }
        self
    }

    pub fn with_beta_header(mut self, value: Option<String>) -> Self {
        match value.and_then(|header| utils::normalize_optional_text(Some(header))) {
            Some(value) => {
                self.extra_headers
                    .insert("anthropic-beta".to_owned(), value);
            }
            None => {
                self.extra_headers.remove("anthropic-beta");
            }
        }
        self
    }

    pub fn with_eager_input_streaming_override(mut self, value: Option<bool>) -> Self {
        self.eager_input_streaming_override = value;
        self
    }

    fn configured_public_copilot_base_url(&self) -> bool {
        self.base_url.trim_end_matches('/') == DEFAULT_COPILOT_BASE_URL
    }

    fn resolved_base_url(&self) -> Result<String, AppError> {
        if self.profile != AnthropicProfile::GithubCopilot
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

    fn models_endpoint(&self) -> Result<String, AppError> {
        Ok(self.models_url.clone().unwrap_or_else(|| {
            format!(
                "{}/models",
                self.prompt_cache_base_url().trim_end_matches('/')
            )
        }))
    }

    fn messages_endpoint(&self) -> Result<String, AppError> {
        if let Some(endpoint) = self.messages_url.clone() {
            return Ok(endpoint);
        }

        let base = self.resolved_base_url()?;
        Ok(match self.profile {
            AnthropicProfile::Standard => format!("{}/messages", base.trim_end_matches('/')),
            AnthropicProfile::GithubCopilot => {
                format!("{}/v1/messages", base.trim_end_matches('/'))
            }
        })
    }

    fn map_usage(usage: Option<AnthropicUsage>) -> Option<CompletionUsage> {
        usage.map(map_anthropic_usage)
    }

    async fn complete_by_aggregating_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let fallback_model = request.model.clone();
        let stream = ModelRuntime::complete_stream(self, request).await?;
        utils::aggregate_stream(self.id.as_str(), fallback_model, stream).await
    }

    fn content_to_blocks(message: &Message) -> Vec<AnthropicTextBlock> {
        let projected = wire_message::project(message);
        Self::blocks_from_projected_parts(message, projected.as_slice())
    }

    fn blocks_from_projected_parts(
        message: &Message,
        projected: &[wire_message::WirePart],
    ) -> Vec<AnthropicTextBlock> {
        if projected.is_empty() {
            let text = message.as_text_lossy();
            if text.is_empty() {
                return Vec::new();
            }

            return vec![AnthropicTextBlock::text(text)];
        }

        let mut blocks = Vec::new();
        for part in projected {
            match part {
                wire_message::WirePart::Text { text } => {
                    blocks.push(AnthropicTextBlock::text(text.clone()));
                }
                wire_message::WirePart::Attachment { item } => {
                    blocks.extend(Self::attachment_blocks(item));
                }
                wire_message::WirePart::ToolCall {
                    id,
                    name,
                    arguments_json,
                } => blocks.push(AnthropicTextBlock::tool_use(
                    id.clone(),
                    name.clone(),
                    arguments_json.clone(),
                )),
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    output_json,
                    ..
                } => blocks.push(AnthropicTextBlock::tool_result(
                    tool_call_id.clone(),
                    output_json.clone(),
                )),
            }
        }

        blocks
    }

    fn assistant_messages_from_parts(message: &Message) -> Vec<AnthropicMessage> {
        let projected = wire_message::project(message);
        if !projected
            .iter()
            .any(|part| matches!(part, wire_message::WirePart::ToolResult { .. }))
        {
            return vec![AnthropicMessage {
                role: "assistant".to_owned(),
                content: Self::blocks_from_projected_parts(message, projected.as_slice()),
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
                    Self::flush_assistant_blocks(message, &mut messages, &mut buffered);
                    messages.push(AnthropicMessage {
                        role: "user".to_owned(),
                        content: vec![AnthropicTextBlock::tool_result(
                            tool_call_id.clone(),
                            output_json.clone(),
                        )],
                    });
                }
                wire_message::WirePart::ToolResult { output_json, .. } => {
                    buffered.push(wire_message::WirePart::Text {
                        text: output_json.clone(),
                    });
                }
                other => buffered.push(other.clone()),
            }
        }
        Self::flush_assistant_blocks(message, &mut messages, &mut buffered);

        messages
    }

    fn flush_assistant_blocks(
        message: &Message,
        messages: &mut Vec<AnthropicMessage>,
        buffered: &mut Vec<wire_message::WirePart>,
    ) {
        if buffered.is_empty() {
            return;
        }
        let content = Self::blocks_from_projected_parts(message, buffered.as_slice());
        buffered.clear();
        if content.is_empty() {
            return;
        }
        messages.push(AnthropicMessage {
            role: "assistant".to_owned(),
            content,
        });
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

    fn is_bundled_base_url(base_url: &str) -> bool {
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
        if let Some(enabled) = self.eager_input_streaming_override {
            return enabled;
        }

        match self.profile {
            AnthropicProfile::Standard => Self::is_bundled_base_url(self.base_url.as_str()),
            AnthropicProfile::GithubCopilot => false,
        }
    }

    fn tools(
        &self,
        tools: &[crate::plugin::registry::PluginEntry],
    ) -> Vec<AnthropicEntryDefinition> {
        tools
            .iter()
            .map(|tool| AnthropicEntryDefinition {
                name: tool.exposed_name.clone(),
                description: tool.description_text().to_string(),
                input_schema: tool.sanitized_input_schema(),
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
        // Keep cache markers stable across tool-use loops: tool definitions
        // and system text sit above the conversation, while the latest real
        // user message stays fixed as assistant/tool-result messages append.
        if let Some(block) = system.last_mut() {
            block.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }

        if let Some(tool) = tools.last_mut() {
            tool.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }

        if let Some(block) = Self::latest_user_cache_block(messages) {
            block.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }
    }

    fn latest_user_cache_block(
        messages: &mut [AnthropicMessage],
    ) -> Option<&mut AnthropicTextBlock> {
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

    async fn send_json<R>(
        &self,
        operation: &str,
        endpoint: String,
        body: &serde_json::Value,
        request: Option<&CompletionRequest>,
    ) -> Result<R, AppError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let mut headers = self.auth_headers(api_key, request);
            headers.insert("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned());
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                operation,
                "POST",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                Some(body),
            );
            utils::apply_resolved_request_headers(self.client.post(endpoint.clone()), &headers)
                .json(body)
        })
        .await?;

        utils::parse_json_response_logged(self.id.as_str(), ADAPTER_KIND, operation, response).await
    }

    fn resolved_headers(&self, request: Option<&CompletionRequest>) -> HashMap<String, String> {
        let mut headers = request
            .map(|request| {
                utils::merged_request_headers(
                    &self.extra_headers,
                    &request.request_override.headers,
                )
            })
            .unwrap_or_else(|| self.extra_headers.clone());
        if matches!(self.profile, AnthropicProfile::GithubCopilot) {
            headers
                .entry(reqwest::header::USER_AGENT.as_str().to_owned())
                .or_insert_with(|| crate::provider::CLAUDE_CODE_API_USER_AGENT.to_owned());
            headers
                .entry("openai-intent".to_owned())
                .or_insert_with(|| "conversation-edits".to_owned());
            if let Some(request) = request {
                headers.insert(
                    "x-initiator".to_owned(),
                    Self::initiator(request).to_owned(),
                );
                if Self::is_vision_request(request) {
                    headers.insert("Copilot-Vision-Request".to_owned(), "true".to_owned());
                }
            }
        }
        headers
    }

    fn auth_headers(
        &self,
        api_key: &str,
        request: Option<&CompletionRequest>,
    ) -> BTreeMap<String, String> {
        let mut headers = self.resolved_headers(request);
        headers.insert(
            self.auth_header.clone(),
            utils::auth_header_value(self.auth_scheme.as_deref(), api_key),
        );
        utils::resolved_request_headers(self.id.as_str(), &headers)
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

#[async_trait]
impl ModelRuntime for AnthropicAdapter {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(crate::provider::CapabilityFamily::Anthropic)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        Some(
            crate::provider::PromptCacheShape::new(self.id.as_str())
                .with_string("auth_scope", self.api_key.prompt_cache_scope())
                .with_string("base_url", self.prompt_cache_base_url().as_str())
                .with_optional_string("models_url", self.models_url.as_deref())
                .with_optional_string("messages_url", self.messages_url.as_deref())
                .with_string(
                    "profile",
                    match self.profile {
                        AnthropicProfile::Standard => "standard",
                        AnthropicProfile::GithubCopilot => "github_copilot",
                    },
                )
                .with_string("auth_header", self.auth_header.as_str())
                .with_optional_string("auth_scheme", self.auth_scheme.as_deref())
                .with_bool(
                    "bundled_base_url",
                    Self::is_bundled_base_url(self.base_url.as_str()),
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
        let endpoint = self.models_endpoint()?;
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let mut headers = self.auth_headers(api_key, None);
            headers.insert("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned());
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                "list_models",
                "GET",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                None,
            );
            utils::apply_resolved_request_headers(self.client.get(endpoint.as_str()), &headers)
        })
        .await?;

        let payload: AnthropicModelListResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            ADAPTER_KIND,
            "list_models",
            response,
        )
        .await?;
        Ok(payload
            .into_items()
            .into_iter()
            .map(|m| {
                let mut model = ProviderModel::new(PROVIDER_ID, m.id);
                let capabilities = self.model_capabilities(&model.id);
                model = model.with_capabilities(capabilities);
                model.display_name = m.display_name.or(m.name);
                model
            })
            .collect())
    }

    #[tracing::instrument(skip_all, fields(provider = tracing::field::Empty, model = %request.model))]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let model = request.model.clone();
        let stream_fallback_request = request.clone();

        let thinking_parts = anthropic_thinking_parts(model.as_str(), request.thinking.as_ref());
        let include_thinking = thinking_parts.include_thinking();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(AnthropicTextBlock::text(system.clone()));
        }
        let mut tools = (!request.tools.is_empty()).then(|| self.tools(request.tools.as_slice()));

        let mut messages = Vec::new();
        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    let text = msg.as_text_lossy();
                    if !text.trim().is_empty() {
                        system_chunks.push(AnthropicTextBlock::text(text));
                    }
                }
                Role::Assistant => messages.extend(Self::assistant_messages_from_parts(msg)),
                Role::User => messages.push(AnthropicMessage {
                    role: "user".to_owned(),
                    content: Self::content_to_blocks(msg),
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
            thinking: thinking_parts.thinking,
            output_config: thinking_parts.output_config,
            stop_sequences: request.stop_sequences.clone(),
            top_p: request.top_p,
            top_k: request.top_k,
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;

        let response: AnthropicMessagesResponse = self
            .send_json(
                "complete.messages",
                self.messages_endpoint()?,
                &body_json,
                Some(&request),
            )
            .await?;

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
            provider_id: ProviderId::new(self.id.as_str()),
            model: ModelId::new(response.model),
            text,
            reasoning_text,
            finish_reason,
            tool_calls,
            usage: Self::map_usage(response.usage),
            provider_metadata: None,
        })
    }

    #[tracing::instrument(skip_all, fields(provider = tracing::field::Empty, model = %request.model))]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let model = request.model.clone();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(AnthropicTextBlock::text(system.clone()));
        }
        let mut tools = (!request.tools.is_empty()).then(|| self.tools(request.tools.as_slice()));

        let mut messages = Vec::new();
        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    let text = msg.as_text_lossy();
                    if !text.trim().is_empty() {
                        system_chunks.push(AnthropicTextBlock::text(text));
                    }
                }
                Role::Assistant => messages.extend(Self::assistant_messages_from_parts(msg)),
                Role::User => messages.push(AnthropicMessage {
                    role: "user".to_owned(),
                    content: Self::content_to_blocks(msg),
                }),
            }
        }
        Self::apply_prompt_cache_hints(
            system_chunks.as_mut_slice(),
            tools.as_deref_mut().unwrap_or(&mut []),
            messages.as_mut_slice(),
        );

        let thinking_parts = anthropic_thinking_parts(model.as_str(), request.thinking.as_ref());
        let body = AnthropicMessagesRequest {
            model: model.to_string(),
            max_tokens: request.max_output_tokens.unwrap_or(4096),
            system: (!system_chunks.is_empty()).then_some(system_chunks),
            messages,
            tools,
            temperature: request.temperature,
            stream: Some(true),
            thinking: thinking_parts.thinking,
            output_config: thinking_parts.output_config,
            stop_sequences: request.stop_sequences.clone(),
            top_p: request.top_p,
            top_k: request.top_k,
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let endpoint = self
                .messages_endpoint()
                .expect("messages endpoint should resolve");
            let mut headers = self.auth_headers(api_key, Some(&request));
            headers.insert("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned());
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.messages",
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
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.messages",
                response,
            )
            .await);
        }

        utils::adapter_log_http_response_open(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete_stream.messages",
            response.status(),
            response.headers(),
        );
        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(self.id.as_str());
        let model_name = model;
        let include_thinking =
            anthropic_thinking_parts(model_name.as_str(), request.thinking.as_ref())
                .include_thinking();

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: HashMap<usize, AnthropicToolCallState> = HashMap::new();
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_usage: Option<AnthropicUsage> = None;
            let mut stream_has_content = false;

            while let Some(event) = events.next().await {
                let event = event?;
                utils::adapter_log_stream_event(
                    provider_id.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.messages",
                    &event,
                );
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
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<AnthropicOutputConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    stop_sequences: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
}

#[derive(Debug, Serialize)]
struct AnthropicOutputConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<&'static str>,
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

#[derive(Debug, Default)]
struct AnthropicThinkingParts {
    thinking: Option<serde_json::Value>,
    output_config: Option<AnthropicOutputConfig>,
}

impl AnthropicThinkingParts {
    fn include_thinking(&self) -> bool {
        self.thinking.is_some()
    }
}

fn anthropic_model_requires_adaptive_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-opus-4-7")
        || normalized.contains("claude-opus-4.7")
        || normalized.contains("claude-mythos-preview")
}

fn anthropic_model_supports_adaptive_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    anthropic_model_requires_adaptive_thinking(model)
        || normalized.contains("claude-opus-4-6")
        || normalized.contains("claude-opus-4.6")
        || normalized.contains("claude-sonnet-4-6")
        || normalized.contains("claude-sonnet-4.6")
}

fn anthropic_budget_for_effort(effort: crate::provider::ReasoningEffort) -> u32 {
    match effort {
        crate::provider::ReasoningEffort::Minimal => 1_024,
        crate::provider::ReasoningEffort::Low => 4_000,
        crate::provider::ReasoningEffort::Medium => 10_000,
        crate::provider::ReasoningEffort::High => 16_000,
        crate::provider::ReasoningEffort::Xhigh | crate::provider::ReasoningEffort::Max => 31_999,
    }
}

fn anthropic_effort_for_budget(
    model: &str,
    budget_tokens: u32,
) -> Option<crate::provider::ReasoningEffort> {
    anthropic_model_requires_adaptive_thinking(model).then_some(if budget_tokens <= 4_000 {
        crate::provider::ReasoningEffort::Low
    } else if budget_tokens <= 10_000 {
        crate::provider::ReasoningEffort::Medium
    } else if budget_tokens <= 16_000 {
        crate::provider::ReasoningEffort::High
    } else if budget_tokens < 31_999 {
        crate::provider::ReasoningEffort::Xhigh
    } else {
        crate::provider::ReasoningEffort::Max
    })
}

fn anthropic_default_display(
    model: &str,
    explicit: Option<ThinkingDisplay>,
) -> Option<ThinkingDisplay> {
    explicit.or_else(|| {
        anthropic_model_requires_adaptive_thinking(model).then_some(ThinkingDisplay::Summarized)
    })
}

fn anthropic_adaptive_parts(
    model: &str,
    effort: Option<crate::provider::ReasoningEffort>,
    display: Option<ThinkingDisplay>,
) -> AnthropicThinkingParts {
    let display = anthropic_default_display(model, display);
    let mut thinking = serde_json::Map::new();
    thinking.insert(
        "type".to_owned(),
        serde_json::Value::String("adaptive".to_owned()),
    );
    if let Some(display) = display {
        thinking.insert(
            "display".to_owned(),
            serde_json::Value::String(display.as_str().to_owned()),
        );
    }

    AnthropicThinkingParts {
        thinking: Some(serde_json::Value::Object(thinking)),
        output_config: effort.map(|effort| AnthropicOutputConfig {
            effort: Some(effort.as_str()),
        }),
    }
}

fn anthropic_enabled_parts(budget_tokens: u32) -> AnthropicThinkingParts {
    AnthropicThinkingParts {
        thinking: Some(serde_json::json!({
            "type": "enabled",
            "budget_tokens": budget_tokens
        })),
        output_config: None,
    }
}

fn anthropic_thinking_parts(
    model: &str,
    thinking: Option<&ThinkingRequest>,
) -> AnthropicThinkingParts {
    match thinking {
        None | Some(ThinkingRequest::Disabled) => AnthropicThinkingParts::default(),
        Some(ThinkingRequest::Budget { budget_tokens }) => {
            if let Some(effort) = anthropic_effort_for_budget(model, *budget_tokens) {
                anthropic_adaptive_parts(model, Some(effort), None)
            } else {
                anthropic_enabled_parts(*budget_tokens)
            }
        }
        Some(ThinkingRequest::Adaptive { effort, display })
            if anthropic_model_supports_adaptive_thinking(model) =>
        {
            anthropic_adaptive_parts(model, *effort, *display)
        }
        Some(ThinkingRequest::Adaptive { effort, .. }) => anthropic_enabled_parts(
            anthropic_budget_for_effort(effort.unwrap_or(crate::provider::ReasoningEffort::High)),
        ),
        Some(ThinkingRequest::Effort { effort })
            if anthropic_model_supports_adaptive_thinking(model) =>
        {
            anthropic_adaptive_parts(model, Some(*effort), None)
        }
        Some(ThinkingRequest::Effort { effort }) => {
            anthropic_enabled_parts(anthropic_budget_for_effort(*effort))
        }
    }
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
#[serde(untagged)]
enum AnthropicModelListResponse {
    Wrapped { data: Vec<AnthropicModel> },
    Bare(Vec<AnthropicModel>),
}

impl AnthropicModelListResponse {
    fn into_items(self) -> Vec<AnthropicModel> {
        match self {
            Self::Wrapped { data } => data,
            Self::Bare(data) => data,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
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
