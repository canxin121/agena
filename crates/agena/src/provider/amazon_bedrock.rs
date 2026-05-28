use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::{Credentials, provider::ProvideCredentials};
use aws_sigv4::http_request::{SignableBody, SignableRequest, SigningSettings, sign};
use aws_sigv4::sign::v4;
use aws_smithy_eventstream::{
    error::Error as BedrockEventStreamError,
    frame::{UnmarshallMessage, UnmarshalledMessage},
};
use aws_smithy_http::event_stream::Receiver as SmithyEventStreamReceiver;
use aws_smithy_types::{
    body::SdkBody,
    event_stream::{HeaderValue, Message as SmithyEventStreamMessage},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use futures_core::Stream;
use futures_util::{StreamExt, TryStreamExt};
use http_body::Frame;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    error::Error as StdError,
    fmt,
    sync::{Arc, Mutex},
};

use crate::{
    error::AppError,
    message::{AttachmentItem, AttachmentKind, Message, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ModelRuntime, ProviderModel, StreamResumePolicy,
        ThinkingRequest,
        chat_wire::{
            ChatCompletionRequest, ChatCompletionResponse, ChatStreamOptions, ChatToolCallWire,
            ChatUsage,
        },
        prompt_cache, sse, utils, wire_message,
    },
    role::Role,
};

const PROVIDER_ID: &str = "amazon-bedrock";
const BEDROCK_ANTHROPIC_VERSION: &str = "bedrock-2023-05-31";
const JSON_CONTENT_TYPE: &str = "application/json";
const EVENTSTREAM_CONTENT_TYPE: &str = "application/vnd.amazon.eventstream";
const ADAPTER_KIND: &str = "amazon_bedrock";

const CROSS_REGION_PREFIXES: &[&str] = &["global.", "us.", "eu.", "jp.", "apac.", "au."];
const US_MODELS: &[&str] = &[
    "nova-micro",
    "nova-lite",
    "nova-pro",
    "nova-premier",
    "nova-2",
    "claude",
    "deepseek",
];
const EU_REGIONS: &[&str] = &[
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-north-1",
    "eu-central-1",
    "eu-south-1",
    "eu-south-2",
];
const EU_MODELS: &[&str] = &[
    "claude",
    "nova-lite",
    "nova-micro",
    "nova-pro",
    "llama3",
    "pixtral",
];
const AP_MODELS: &[&str] = &["claude", "nova-lite", "nova-micro", "nova-pro"];
const AU_MODELS: &[&str] = &["anthropic.claude-sonnet-4-5", "anthropic.claude-haiku"];

#[derive(Clone)]
enum BedrockAuthMode {
    SigV4 {
        profile: Option<String>,
        static_credentials: Option<Credentials>,
    },
}

struct Sigv4Request<'a> {
    method: reqwest::Method,
    url: String,
    body: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
    body_debug: Option<&'a Value>,
}

#[derive(Clone)]
pub struct AmazonBedrockAdapter {
    client: reqwest::Client,
    base_url: String,
    default_model: ModelId,
    region: String,
    auth_mode: BedrockAuthMode,
    resolved_sigv4_shape: Arc<Mutex<Option<crate::provider::PromptCacheShape>>>,
}

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

    fn prompt_cache_sigv4_profile(profile: Option<&str>) -> Option<String> {
        profile
            .map(|value| value.to_owned())
            .and_then(|value| utils::normalize_optional_text(Some(value)))
    }

    fn prompt_cache_sigv4_static_credentials_shape(
        credentials: &Credentials,
    ) -> crate::provider::PromptCacheShape {
        let mut shape =
            crate::provider::PromptCacheShape::new(PROVIDER_ID).with_bool("configured", true);

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

    fn prompt_cache_sigv4_runtime_shape_from_credentials(
        credentials: &Credentials,
    ) -> Option<crate::provider::PromptCacheShape> {
        credentials
            .account_id()
            .map(|value| value.as_str().trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|account_id| {
                crate::provider::PromptCacheShape::new(PROVIDER_ID)
                    .with_string("account_id", account_id)
            })
    }

    fn prompt_cache_sigv4_env_shape() -> Option<crate::provider::PromptCacheShape> {
        let mut shape = crate::provider::PromptCacheShape::new(PROVIDER_ID);
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

    fn update_prompt_cache_sigv4_runtime_shape(&self, credentials: &Credentials) {
        if let Ok(mut runtime_shape) = self.resolved_sigv4_shape.lock() {
            *runtime_shape = Self::prompt_cache_sigv4_runtime_shape_from_credentials(credentials);
        }
    }

    fn resolve_model(&self, model: &str) -> String {
        let model = if model.trim().is_empty() {
            self.default_model.to_string()
        } else {
            model.trim().to_owned()
        };
        prefix_bedrock_model(self.region.as_str(), model.as_str())
    }

    fn models_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn completions_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn runtime_endpoint(&self) -> &str {
        self.base_url
            .strip_suffix("/openai/v1")
            .unwrap_or(self.base_url.as_str())
    }

    fn native_anthropic_invoke_endpoint(
        &self,
        model: &str,
        stream: bool,
    ) -> Result<String, AppError> {
        let mut url = url::Url::parse(self.runtime_endpoint()).map_err(|err| {
            AppError::Config(format!(
                "amazon-bedrock invalid runtime base url `{}`: {err}",
                self.runtime_endpoint()
            ))
        })?;
        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                AppError::Config(format!(
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

    async fn resolve_sigv4_credentials(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
    ) -> Result<Credentials, AppError> {
        if let Some(credentials) = static_credentials {
            self.update_prompt_cache_sigv4_runtime_shape(credentials);
            return Ok(credentials.clone());
        }

        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(self.region.clone()));
        if let Some(profile) = profile.filter(|value| !value.trim().is_empty()) {
            loader = loader.profile_name(profile.to_owned());
        }

        let sdk_config = loader.load().await;
        let provider = sdk_config.credentials_provider().ok_or_else(|| {
            AppError::Config("amazon-bedrock could not resolve aws credential provider".to_owned())
        })?;

        let credentials = provider.provide_credentials().await.map_err(|err| {
            AppError::Provider(format!(
                "amazon-bedrock failed to resolve aws credentials from chain: {err}"
            ))
        })?;
        self.update_prompt_cache_sigv4_runtime_shape(&credentials);
        Ok(credentials)
    }

    async fn send_sigv4_request(
        &self,
        operation: &str,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request_input: Sigv4Request<'_>,
    ) -> Result<reqwest::Response, AppError> {
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
        let signing_headers = signed_sigv4_headers(
            &method,
            url.as_str(),
            body.as_deref().unwrap_or(&[]),
            headers.as_slice(),
            &credentials,
            self.region.as_str(),
        )?;

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

        request.send().await.map_err(AppError::from)
    }

    fn parse_models(&self, payload: Value) -> Result<Vec<ProviderModel>, AppError> {
        let parsed: OpenAiCompatibleModelList =
            utils::parse_json_value(PROVIDER_ID, "models list", payload)?;
        let models = match parsed {
            OpenAiCompatibleModelList::Object { data } => data,
            OpenAiCompatibleModelList::Array(data) => data,
        };

        Ok(models
            .into_iter()
            .map(|model| {
                let mut entry = ProviderModel::new(PROVIDER_ID, model.id);
                let capabilities = self.model_capabilities(&entry.id);
                entry = entry.with_capabilities(capabilities);
                entry.display_name = model.display_name.or(model.name);
                entry
            })
            .collect())
    }

    async fn list_models_sigv4(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
    ) -> Result<Vec<ProviderModel>, AppError> {
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

    fn parse_completion(
        &self,
        payload: ChatCompletionResponse,
    ) -> Result<CompletionResponse, AppError> {
        crate::provider::chat_wire::parse_completion_response_with_required_tool_calls(
            PROVIDER_ID,
            self.default_model.as_str(),
            payload,
        )
    }

    fn parse_anthropic_completion(
        &self,
        payload: BedrockAnthropicMessagesResponse,
        fallback_model: &ModelId,
    ) -> Result<CompletionResponse, AppError> {
        let text = payload
            .content
            .iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text.clone())
            .collect::<Vec<_>>()
            .join("");

        let tool_calls = payload
            .content
            .iter()
            .filter(|block| block.kind == "tool_use")
            .map(|block| {
                let id = utils::normalize_optional_text(block.id.clone()).ok_or_else(|| {
                    AppError::Provider(
                        "amazon-bedrock anthropic response returned tool_use without id".to_owned(),
                    )
                })?;

                let name = utils::normalize_optional_text(block.name.clone()).ok_or_else(|| {
                    AppError::Provider(
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
            .collect::<Result<Vec<_>, AppError>>()?;

        let finish_reason = CompletionFinishReason::from_provider(payload.stop_reason.as_deref());

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(
                "amazon-bedrock anthropic completion payload was empty without finish reason"
                    .to_owned(),
            ));
        }

        Ok(CompletionResponse {
            provider_id: ProviderId::new(PROVIDER_ID),
            model: ModelId::new(payload.model.unwrap_or_else(|| fallback_model.to_string())),
            text,
            reasoning_text: None,
            finish_reason,
            tool_calls,
            usage: payload.usage.map(map_bedrock_anthropic_usage),
            provider_metadata: response_id_metadata(payload.id),
        })
    }

    fn is_native_anthropic_model(model: &str) -> bool {
        let normalized = strip_cross_region_prefix(model).to_ascii_lowercase();
        normalized.starts_with("anthropic.") || normalized.contains("claude")
    }

    fn anthropic_tools(
        tools: &[crate::plugin::registry::RegisteredTool],
    ) -> Vec<BedrockAnthropicToolDefinition> {
        tools
            .iter()
            .map(|tool| BedrockAnthropicToolDefinition {
                name: crate::tool::model_safe_tool_name(tool.exposed_name.as_str()),
                description: tool.description_text().to_string(),
                input_schema: crate::tool::model_safe_tool_schema(&tool.sanitized_input_schema()),
                cache_control: None,
                eager_input_streaming: None,
            })
            .collect()
    }

    fn anthropic_content_to_blocks(message: &Message) -> Vec<BedrockAnthropicTextBlock> {
        let projected = wire_message::project(message);
        Self::anthropic_blocks_from_projected_parts(message, projected.as_slice())
    }

    fn anthropic_blocks_from_projected_parts(
        message: &Message,
        projected: &[wire_message::WirePart],
    ) -> Vec<BedrockAnthropicTextBlock> {
        if projected.is_empty() {
            let text = message.as_text_lossy();
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
                wire_message::WirePart::Attachment { item } => {
                    blocks.extend(Self::anthropic_attachment_blocks(item));
                }
                wire_message::WirePart::ToolCall {
                    id,
                    name,
                    arguments_json,
                } => blocks.push(BedrockAnthropicTextBlock::tool_use(
                    id.clone(),
                    crate::tool::model_safe_tool_name(name),
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

    fn anthropic_assistant_messages_from_parts(message: &Message) -> Vec<BedrockAnthropicMessage> {
        let projected = wire_message::project(message);
        if !projected
            .iter()
            .any(|part| matches!(part, wire_message::WirePart::ToolResult { .. }))
        {
            return vec![BedrockAnthropicMessage {
                role: "assistant".to_owned(),
                content: Self::anthropic_blocks_from_projected_parts(message, projected.as_slice()),
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
                    Self::flush_anthropic_assistant_blocks(message, &mut messages, &mut buffered);
                    messages.push(BedrockAnthropicMessage {
                        role: "user".to_owned(),
                        content: vec![BedrockAnthropicTextBlock::tool_result(
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
        Self::flush_anthropic_assistant_blocks(message, &mut messages, &mut buffered);

        messages
    }

    fn flush_anthropic_assistant_blocks(
        message: &Message,
        messages: &mut Vec<BedrockAnthropicMessage>,
        buffered: &mut Vec<wire_message::WirePart>,
    ) {
        if buffered.is_empty() {
            return;
        }
        let content = Self::anthropic_blocks_from_projected_parts(message, buffered.as_slice());
        buffered.clear();
        if content.is_empty() {
            return;
        }
        messages.push(BedrockAnthropicMessage {
            role: "assistant".to_owned(),
            content,
        });
    }

    fn anthropic_attachment_blocks(item: &AttachmentItem) -> Vec<BedrockAnthropicTextBlock> {
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

    fn anthropic_binary_source(item: &AttachmentItem) -> Option<BedrockAnthropicBinarySource> {
        wire_message::base64_with_mime(item)
            .map(|(media_type, data)| BedrockAnthropicBinarySource::base64(media_type, data))
    }

    fn apply_anthropic_prompt_cache_hints(
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

    fn latest_anthropic_user_cache_block(
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

    fn build_anthropic_request(
        request: CompletionRequest,
    ) -> (ModelId, BedrockAnthropicMessagesRequest) {
        let model = request.model.clone();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(BedrockAnthropicTextBlock::text(system.clone()));
        }
        let mut tools =
            (!request.tools.is_empty()).then(|| Self::anthropic_tools(request.tools.as_slice()));

        let mut messages = Vec::new();
        for msg in request.messages {
            match msg.role {
                Role::System => {
                    let text = msg.as_text_lossy();
                    if !text.trim().is_empty() {
                        system_chunks.push(BedrockAnthropicTextBlock::text(text));
                    }
                }
                Role::Assistant => {
                    messages.extend(Self::anthropic_assistant_messages_from_parts(&msg))
                }
                Role::User => messages.push(BedrockAnthropicMessage {
                    role: "user".to_owned(),
                    content: Self::anthropic_content_to_blocks(&msg),
                }),
            }
        }

        Self::apply_anthropic_prompt_cache_hints(
            system_chunks.as_mut_slice(),
            tools.as_deref_mut().unwrap_or(&mut []),
            messages.as_mut_slice(),
        );

        (
            model.clone(),
            BedrockAnthropicMessagesRequest {
                anthropic_version: BEDROCK_ANTHROPIC_VERSION.to_owned(),
                model: model.to_string(),
                max_tokens: request.max_output_tokens.unwrap_or(4096),
                system: (!system_chunks.is_empty()).then_some(system_chunks),
                messages,
                tools,
                thinking: bedrock_anthropic_thinking_body(
                    model.as_str(),
                    request.thinking.as_ref(),
                ),
                temperature: if Self::anthropic_model_supports_sampling_parameters(model.as_str()) {
                    request.temperature
                } else {
                    None
                },
            },
        )
    }

    fn anthropic_invoke_headers(stream: bool) -> Vec<(String, String)> {
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

    fn anthropic_model_uses_adaptive_thinking(model: &str) -> bool {
        let normalized = model.to_ascii_lowercase();
        normalized.contains("claude-opus-4-7")
            || normalized.contains("claude-opus-4.7")
            || normalized.contains("claude-opus-4-6")
            || normalized.contains("claude-opus-4.6")
            || normalized.contains("claude-sonnet-4-6")
            || normalized.contains("claude-sonnet-4.6")
    }

    fn anthropic_model_supports_sampling_parameters(model: &str) -> bool {
        let normalized = model.to_ascii_lowercase();
        !(normalized.contains("claude-opus-4-7") || normalized.contains("claude-opus-4.7"))
    }

    async fn complete_sigv4_anthropic(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
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
                    url: self.native_anthropic_invoke_endpoint(model.as_str(), false)?,
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

    async fn complete_stream_sigv4_anthropic(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let request_override = request.request_override.clone();
        let (model, body) = Self::build_anthropic_request(request);
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
                    url: self.native_anthropic_invoke_endpoint(model.as_str(), true)?,
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
        let response_stream = response.bytes_stream().map_ok(Frame::data);
        let body = SdkBody::from_body_1_x(http_body_util::StreamBody::new(response_stream));
        let receiver = SmithyEventStreamReceiver::<Value, BedrockAnthropicStreamServiceError>::new(
            BedrockAnthropicStreamUnmarshaller,
            body,
        );

        let stream = async_stream::try_stream! {
            let mut receiver = receiver;
            let mut pending_tool_calls: std::collections::HashMap<usize, BedrockAnthropicToolCallState> = std::collections::HashMap::new();
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_usage: Option<BedrockAnthropicUsage> = None;
            let mut stream_has_content = false;
            let mut response_id: Option<String> = None;
            let mut stream_model = initial_model.clone();

            loop {
                let event = match receiver.recv().await {
                    Ok(Some(event)) => event,
                    Ok(None) => break,
                    Err(err) => {
                        if let Some(service) = err.as_service_error() {
                            Err(AppError::ProviderClassified {
                                provider: PROVIDER_ID.to_owned(),
                                message: service.to_string(),
                                kind: crate::error::ProviderErrorKind::ApiError,
                                retryable: service.retryable,
                            })?;
                        }

                        let source = err
                            .into_source()
                            .map(|source| source.to_string())
                            .unwrap_or_else(|err| err.to_string());
                        Err(AppError::Provider(format!(
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
                        if content_block.kind != "tool_use" {
                            continue;
                        }

                        let index = index.ok_or_else(|| {
                            AppError::Provider(
                                "amazon-bedrock anthropic tool_use stream event missing content block index"
                                    .to_owned(),
                            )
                        })?;

                        let id = utils::normalize_optional_text(content_block.id.clone()).ok_or_else(|| {
                            AppError::Provider(
                                "amazon-bedrock anthropic tool_use stream event missing tool id"
                                    .to_owned(),
                            )
                        })?;
                        let name = utils::normalize_optional_text(content_block.name.clone()).ok_or_else(|| {
                            AppError::Provider(
                                "amazon-bedrock anthropic tool_use stream event missing tool name"
                                    .to_owned(),
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
                        if let Some(text) = delta.text.clone().filter(|value| !value.is_empty()) {
                            stream_has_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: stream_model.clone(),
                                delta: text,
                            };
                        }

                        if matches!(delta.kind.as_deref(), Some("input_json_delta")) {
                            let Some(arguments_delta) = utils::optional_non_empty(delta.partial_json.clone()) else {
                                continue;
                            };

                            let index = index.ok_or_else(|| {
                                AppError::Provider(
                                    "amazon-bedrock anthropic tool delta event missing content block index"
                                        .to_owned(),
                                )
                            })?;

                            let state = pending_tool_calls.get_mut(&index).ok_or_else(|| {
                                AppError::Provider(
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
                            stream_finish_reason = delta
                                .stop_reason
                                .or_else(|| message.as_ref().and_then(|item| item.stop_reason.clone()));
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
                            stream_finish_reason = message
                                .as_ref()
                                .and_then(|item| item.stop_reason.clone());
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
                yield CompletionStreamEvent::Completed {
                    provider_id,
                    model: stream_model,
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage.map(map_bedrock_anthropic_usage),
                    provider_metadata: response_id_metadata(response_id),
                };
            }
        };

        Ok(Box::pin(stream))
    }

    async fn complete_sigv4(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        if Self::is_native_anthropic_model(request.model.as_str()) {
            return self
                .complete_sigv4_anthropic(profile, static_credentials, request)
                .await;
        }

        let prompt_cache_key = request.prompt_cache_key.clone();
        let model = self.resolve_model(request.model.as_str());
        let messages =
            crate::provider::chat_wire::request_to_chat_messages_with_assistant_reasoning_field(
                &request, None,
            );
        let body = ChatCompletionRequest {
            model,
            messages,
            tools: None,
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
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
            prompt_cache_key_camel_case: prompt_cache_key.clone(),
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

    async fn complete_stream_sigv4(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        if Self::is_native_anthropic_model(request.model.as_str()) {
            return self
                .complete_stream_sigv4_anthropic(profile, static_credentials, request)
                .await;
        }

        let prompt_cache_key = request.prompt_cache_key.clone();
        let model = self.resolve_model(request.model.as_str());
        let messages =
            crate::provider::chat_wire::request_to_chat_messages_with_assistant_reasoning_field(
                &request, None,
            );

        let body = ChatCompletionRequest {
            model: model.clone(),
            messages,
            tools: None,
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
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
            prompt_cache_key_camel_case: prompt_cache_key.clone(),
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
        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(PROVIDER_ID);
        let model_name = ModelId::new(model);

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ToolCallState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;

            while let Some(event) = events.next().await {
                let event = event?;
                utils::adapter_log_stream_event(
                    PROVIDER_ID,
                    ADAPTER_KIND,
                    "complete_stream.chat",
                    &event,
                );
                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(PROVIDER_ID, "chat stream chunk", event)?;
                let choice = chunk.choices.first();

                let delta = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.content.as_ref())
                    .map(crate::provider::chat_wire::extract_text_from_content)
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
                    let tool = utils::parse_json_value::<ChatToolCallWire>(
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
                                "amazon-bedrock chat stream tool_call delta missing index/id"
                                    .to_owned(),
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
                    if !state.announced && !emitted_any && state.name.is_some() {
                        // Register parameterless tool calls so the shared
                        // aggregator does not silently drop them.
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
                    let usage = utils::parse_json_value::<ChatUsage>(
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
                    provider_id,
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

#[async_trait]
impl ModelRuntime for AmazonBedrockAdapter {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(crate::provider::CapabilityFamily::Bedrock)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        let mut shape = crate::provider::PromptCacheShape::new(PROVIDER_ID)
            .with_string("base_url", self.base_url.clone())
            .with_string("region", self.region.clone())
            .with_bool(
                "native_anthropic_transport",
                matches!(&self.auth_mode, BedrockAuthMode::SigV4 { .. })
                    && Self::is_native_anthropic_model(model.as_str()),
            );

        match &self.auth_mode {
            BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            } => {
                shape.insert_string("auth_mode", "sigv4");
                if let Some(profile) = Self::prompt_cache_sigv4_profile(profile.as_deref()) {
                    shape.insert_string("sigv4.profile", profile);
                }
                if let Some(static_credentials) = static_credentials.as_ref() {
                    shape.extend_prefixed(
                        "sigv4.static",
                        &Self::prompt_cache_sigv4_static_credentials_shape(static_credentials),
                    );
                } else if let Some(env_shape) = Self::prompt_cache_sigv4_env_shape() {
                    shape.extend_prefixed("sigv4.env", &env_shape);
                }
                if let Some(runtime_shape) = self
                    .resolved_sigv4_shape
                    .lock()
                    .ok()
                    .and_then(|shape| shape.clone())
                {
                    shape.extend_prefixed("sigv4.runtime", &runtime_shape);
                }
            }
        }

        Some(shape)
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        match &self.auth_mode {
            BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            } => {
                self.list_models_sigv4(profile.as_deref(), static_credentials.as_ref())
                    .await
            }
        }
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = self.resolve_model(request.model.as_str());
        let request = CompletionRequest {
            model: ModelId::new(model),
            ..request
        };

        match &self.auth_mode {
            BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            } => {
                self.complete_sigv4(profile.as_deref(), static_credentials.as_ref(), request)
                    .await
            }
        }
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = self.resolve_model(request.model.as_str());
        let request = CompletionRequest {
            model: ModelId::new(model),
            ..request
        };

        match &self.auth_mode {
            BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            } => {
                self.complete_stream_sigv4(profile.as_deref(), static_credentials.as_ref(), request)
                    .await
            }
        }
    }
}

fn strip_cross_region_prefix(model: &str) -> &str {
    CROSS_REGION_PREFIXES
        .iter()
        .find_map(|prefix| model.strip_prefix(prefix))
        .unwrap_or(model)
}

fn prefix_bedrock_model(region: &str, model: &str) -> String {
    if model.is_empty() || has_cross_region_prefix(model) || is_bedrock_resource_name(model) {
        return model.to_owned();
    }

    let region = region.to_ascii_lowercase();
    let normalized_model = model.to_ascii_lowercase();

    if region.starts_with("us-")
        && !region.starts_with("us-gov")
        && contains_any(normalized_model.as_str(), US_MODELS)
    {
        return format!("us.{model}");
    }

    if EU_REGIONS.contains(&region.as_str()) && contains_any(normalized_model.as_str(), EU_MODELS) {
        return format!("eu.{model}");
    }

    if region.starts_with("ap-") {
        let is_au_region = region == "ap-southeast-2" || region == "ap-southeast-4";
        if is_au_region && contains_any(normalized_model.as_str(), AU_MODELS) {
            return format!("au.{model}");
        }

        if region == "ap-northeast-1" && contains_any(normalized_model.as_str(), AP_MODELS) {
            return format!("jp.{model}");
        }

        if contains_any(normalized_model.as_str(), AP_MODELS) {
            return format!("apac.{model}");
        }
    }

    model.to_owned()
}

fn has_cross_region_prefix(model: &str) -> bool {
    CROSS_REGION_PREFIXES
        .iter()
        .any(|prefix| model.starts_with(prefix))
}

fn is_bedrock_resource_name(model: &str) -> bool {
    model.starts_with("arn:")
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    response_id.map(|response_id| serde_json::json!({ "response_id": response_id }))
}

fn parse_json_or_string(raw: String) -> Value {
    serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw))
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn map_bedrock_anthropic_usage(usage: BedrockAnthropicUsage) -> CompletionUsage {
    let cache_write_tokens = usage.cache_creation_input_tokens.unwrap_or_else(|| {
        usage
            .cache_creation
            .as_ref()
            .map(BedrockAnthropicCacheCreationUsage::total_input_tokens)
            .unwrap_or_default()
    });

    MessageUsage {
        input_tokens: usage.input_tokens.unwrap_or_default(),
        output_tokens: usage.output_tokens.unwrap_or_default(),
        reasoning_tokens: 0,
        cache_write_tokens,
        cache_read_tokens: usage.cache_read_input_tokens.unwrap_or_default(),
        total_cost: 0.0,
    }
    .into()
}

fn merge_bedrock_anthropic_usage(
    current: Option<BedrockAnthropicUsage>,
    update: BedrockAnthropicUsage,
) -> BedrockAnthropicUsage {
    let Some(current) = current else {
        return update;
    };

    BedrockAnthropicUsage {
        input_tokens: update.input_tokens.or(current.input_tokens),
        output_tokens: update.output_tokens.or(current.output_tokens),
        cache_creation_input_tokens: update
            .cache_creation_input_tokens
            .or(current.cache_creation_input_tokens),
        cache_read_input_tokens: update
            .cache_read_input_tokens
            .or(current.cache_read_input_tokens),
        cache_creation: merge_bedrock_anthropic_cache_creation_usage(
            current.cache_creation,
            update.cache_creation,
        ),
    }
}

fn merge_bedrock_anthropic_cache_creation_usage(
    current: Option<BedrockAnthropicCacheCreationUsage>,
    update: Option<BedrockAnthropicCacheCreationUsage>,
) -> Option<BedrockAnthropicCacheCreationUsage> {
    match (current, update) {
        (Some(current), Some(update)) => Some(BedrockAnthropicCacheCreationUsage {
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

fn signed_sigv4_headers(
    method: &reqwest::Method,
    url: &str,
    body: &[u8],
    headers: &[(String, String)],
    credentials: &Credentials,
    region: &str,
) -> Result<http::HeaderMap, AppError> {
    let identity = credentials.clone().into();
    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name("bedrock")
        .time(std::time::SystemTime::now())
        .settings(SigningSettings::default())
        .build()
        .map_err(|err| AppError::Provider(format!("bedrock signing params error: {err}")))?;

    let signable_request = SignableRequest::new(
        method.as_str(),
        url,
        headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        SignableBody::Bytes(body),
    )
    .map_err(|err| AppError::Provider(format!("bedrock signable request error: {err}")))?;

    let (instructions, _) = sign(signable_request, &signing_params.into())
        .map_err(|err| AppError::Provider(format!("bedrock signing failed: {err}")))?
        .into_parts();

    let mut signing_request = http::Request::builder()
        .method(method.as_str())
        .uri(url)
        .body(())
        .map_err(|err| AppError::Provider(format!("bedrock signing request build error: {err}")))?;

    for (name, value) in headers {
        signing_request.headers_mut().insert(
            http::header::HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
                AppError::Config(format!("bedrock invalid header name `{name}`: {err}"))
            })?,
            http::header::HeaderValue::from_str(value.as_str()).map_err(|err| {
                AppError::Config(format!("bedrock invalid header value for `{name}`: {err}"))
            })?,
        );
    }

    instructions.apply_to_request_http1x(&mut signing_request);
    Ok(signing_request.headers().clone())
}

fn bedrock_anthropic_budget_for_effort(effort: crate::provider::ReasoningEffort) -> u32 {
    match effort {
        crate::provider::ReasoningEffort::Minimal => 1_024,
        crate::provider::ReasoningEffort::Low => 4_000,
        crate::provider::ReasoningEffort::Medium => 10_000,
        crate::provider::ReasoningEffort::High => 16_000,
        crate::provider::ReasoningEffort::Xhigh | crate::provider::ReasoningEffort::Max => 31_999,
    }
}

fn bedrock_anthropic_effort_for_budget(
    model: &str,
    budget_tokens: u32,
) -> Option<crate::provider::ReasoningEffort> {
    if !model.to_ascii_lowercase().contains("claude-opus-4-7")
        && !model.to_ascii_lowercase().contains("claude-opus-4.7")
    {
        return None;
    }

    Some(if budget_tokens <= 4_000 {
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

fn bedrock_anthropic_display(
    model: &str,
    explicit: Option<crate::provider::ThinkingDisplay>,
) -> Option<&'static str> {
    explicit
        .map(crate::provider::ThinkingDisplay::as_str)
        .or_else(|| {
            (model.to_ascii_lowercase().contains("claude-opus-4-7")
                || model.to_ascii_lowercase().contains("claude-opus-4.7"))
            .then_some("summarized")
        })
}

fn bedrock_anthropic_thinking_body(
    model: &str,
    thinking: Option<&ThinkingRequest>,
) -> Option<BedrockAnthropicThinkingConfig> {
    match thinking? {
        ThinkingRequest::Disabled => None,
        ThinkingRequest::Budget { budget_tokens } => {
            if let Some(effort) = bedrock_anthropic_effort_for_budget(model, *budget_tokens) {
                Some(BedrockAnthropicThinkingConfig::Adaptive {
                    effort: Some(effort.as_str()),
                    display: bedrock_anthropic_display(model, None),
                })
            } else {
                Some(BedrockAnthropicThinkingConfig::Enabled {
                    budget_tokens: *budget_tokens,
                })
            }
        }
        ThinkingRequest::Adaptive { effort, display }
            if AmazonBedrockAdapter::anthropic_model_uses_adaptive_thinking(model) =>
        {
            Some(BedrockAnthropicThinkingConfig::Adaptive {
                effort: effort.map(crate::provider::ReasoningEffort::as_str),
                display: bedrock_anthropic_display(model, *display),
            })
        }
        ThinkingRequest::Adaptive { effort, .. } => Some(BedrockAnthropicThinkingConfig::Enabled {
            budget_tokens: bedrock_anthropic_budget_for_effort(
                effort.unwrap_or(crate::provider::ReasoningEffort::High),
            ),
        }),
        ThinkingRequest::Effort { effort }
            if AmazonBedrockAdapter::anthropic_model_uses_adaptive_thinking(model) =>
        {
            Some(BedrockAnthropicThinkingConfig::Adaptive {
                effort: Some(effort.as_str()),
                display: bedrock_anthropic_display(model, None),
            })
        }
        ThinkingRequest::Effort { effort } => Some(BedrockAnthropicThinkingConfig::Enabled {
            budget_tokens: bedrock_anthropic_budget_for_effort(*effort),
        }),
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
struct BedrockAnthropicMessagesRequest {
    anthropic_version: String,
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<BedrockAnthropicTextBlock>>,
    messages: Vec<BedrockAnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<BedrockAnthropicToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<BedrockAnthropicThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BedrockAnthropicThinkingConfig {
    Enabled {
        budget_tokens: u32,
    },
    Adaptive {
        #[serde(skip_serializing_if = "Option::is_none")]
        effort: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<&'static str>,
    },
}

#[derive(Debug, Serialize)]
struct BedrockAnthropicToolDefinition {
    name: String,
    description: String,
    input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_control: Option<prompt_cache::PromptCacheControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    eager_input_streaming: Option<bool>,
}

#[derive(Debug, Serialize)]
struct BedrockAnthropicMessage {
    role: String,
    content: Vec<BedrockAnthropicTextBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BedrockAnthropicTextBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    source: Option<BedrockAnthropicBinarySource>,
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

impl BedrockAnthropicTextBlock {
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

    fn image(source: BedrockAnthropicBinarySource) -> Self {
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

    fn document(source: BedrockAnthropicBinarySource) -> Self {
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
        Self {
            kind: "tool_use".to_owned(),
            text: None,
            source: None,
            id: Some(id.into()),
            name: Some(name.into()),
            input: Some(parse_json_or_string(input_json.into())),
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            kind: "tool_result".to_owned(),
            text: None,
            source: None,
            id: None,
            name: None,
            input: None,
            tool_use_id: Some(tool_use_id.into()),
            content: Some(parse_json_or_string(content.into())),
            cache_control: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BedrockAnthropicBinarySource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

impl BedrockAnthropicBinarySource {
    fn base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            kind: "base64".to_owned(),
            media_type: media_type.into(),
            data: data.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct BedrockAnthropicMessagesResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    content: Vec<BedrockAnthropicTextBlock>,
    #[serde(default)]
    usage: Option<BedrockAnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct BedrockAnthropicUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation: Option<BedrockAnthropicCacheCreationUsage>,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicCacheCreationUsage {
    #[serde(default)]
    ephemeral_1h_input_tokens: Option<u64>,
    #[serde(default)]
    ephemeral_5m_input_tokens: Option<u64>,
}

impl BedrockAnthropicCacheCreationUsage {
    fn total_input_tokens(&self) -> u64 {
        self.ephemeral_1h_input_tokens.unwrap_or_default()
            + self.ephemeral_5m_input_tokens.unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BedrockAnthropicStreamEvent {
    MessageStart {
        #[serde(default)]
        message: BedrockAnthropicStreamMessage,
    },
    ContentBlockStart {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        content_block: BedrockAnthropicStreamContentBlock,
    },
    ContentBlockDelta {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        delta: BedrockAnthropicStreamDelta,
    },
    ContentBlockStop {
        #[serde(default)]
        index: Option<usize>,
    },
    MessageDelta {
        #[serde(default)]
        delta: BedrockAnthropicStreamMessageDelta,
        #[serde(default)]
        usage: Option<BedrockAnthropicUsage>,
        #[serde(default)]
        message: Option<BedrockAnthropicStreamMessage>,
    },
    MessageStop {
        #[serde(default)]
        usage: Option<BedrockAnthropicUsage>,
        #[serde(default)]
        message: Option<BedrockAnthropicStreamMessage>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicStreamContentBlock {
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
struct BedrockAnthropicStreamDelta {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicStreamMessageDelta {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicStreamMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<BedrockAnthropicUsage>,
}

#[derive(Debug, Default)]
struct BedrockAnthropicToolCallState {
    id: String,
    name: String,
}

#[derive(Debug)]
struct BedrockAnthropicStreamServiceError {
    event_type: String,
    message: String,
    retryable: bool,
}

impl BedrockAnthropicStreamServiceError {
    fn from_payload(event_type: &str, payload: Value) -> Self {
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| payload.get("originalMessage").and_then(Value::as_str))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(event_type)
            .to_owned();

        Self {
            event_type: event_type.to_owned(),
            message,
            retryable: matches!(
                event_type,
                "internalServerException"
                    | "modelStreamErrorException"
                    | "throttlingException"
                    | "modelTimeoutException"
                    | "serviceUnavailableException"
            ),
        }
    }
}

impl fmt::Display for BedrockAnthropicStreamServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.event_type, self.message)
    }
}

impl StdError for BedrockAnthropicStreamServiceError {}

#[derive(Debug)]
struct BedrockAnthropicStreamUnmarshaller;

impl UnmarshallMessage for BedrockAnthropicStreamUnmarshaller {
    type Output = Value;
    type Error = BedrockAnthropicStreamServiceError;

    fn unmarshall(
        &self,
        message: &SmithyEventStreamMessage,
    ) -> Result<UnmarshalledMessage<Self::Output, Self::Error>, BedrockEventStreamError> {
        let event_type = message
            .headers()
            .iter()
            .find(|header| header.name().as_str() == ":event-type")
            .and_then(|header| match header.value() {
                HeaderValue::String(value) => Some(value.as_str()),
                _ => None,
            })
            .ok_or_else(|| {
                BedrockEventStreamError::unmarshalling(
                    "amazon-bedrock stream frame missing :event-type header",
                )
            })?;

        let payload = if message.payload().is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_slice::<Value>(message.payload()).map_err(|err| {
                BedrockEventStreamError::unmarshalling(format!(
                    "amazon-bedrock stream frame payload was not valid JSON: {err}"
                ))
            })?
        };

        match event_type {
            "chunk" => {
                let encoded = payload
                    .get("bytes")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        BedrockEventStreamError::unmarshalling(
                            "amazon-bedrock chunk event missing base64 `bytes` field",
                        )
                    })?;
                let decoded = BASE64_STANDARD.decode(encoded).map_err(|err| {
                    BedrockEventStreamError::unmarshalling(format!(
                        "amazon-bedrock chunk event contained invalid base64 payload: {err}"
                    ))
                })?;
                let event = serde_json::from_slice::<Value>(&decoded).map_err(|err| {
                    BedrockEventStreamError::unmarshalling(format!(
                        "amazon-bedrock chunk event contained invalid Anthropic JSON: {err}"
                    ))
                })?;
                Ok(UnmarshalledMessage::Event(event))
            }
            "internalServerException"
            | "modelStreamErrorException"
            | "validationException"
            | "throttlingException"
            | "modelTimeoutException"
            | "serviceUnavailableException" => Ok(UnmarshalledMessage::Error(
                BedrockAnthropicStreamServiceError::from_payload(event_type, payload),
            )),
            other => Err(BedrockEventStreamError::unmarshalling(format!(
                "amazon-bedrock stream returned unknown event type `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Default)]
struct ToolCallState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    announced: bool,
}
