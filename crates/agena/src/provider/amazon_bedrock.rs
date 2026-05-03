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
    error::Error as StdError,
    fmt,
    sync::{Arc, Mutex},
};

use crate::{
    error::AppError,
    message::{AttachmentItem, AttachmentKind, AttachmentSource, Message, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ManagedCredential, ModelProvider,
        OpenAiCompatibleProvider, ProviderModel, StreamResumePolicy, prompt_cache, sse, utils,
        wire_message,
    },
    role::Role,
};

const PROVIDER_ID: &str = "amazon-bedrock";
const BEDROCK_ANTHROPIC_VERSION: &str = "bedrock-2023-05-31";
const JSON_CONTENT_TYPE: &str = "application/json";
const EVENTSTREAM_CONTENT_TYPE: &str = "application/vnd.amazon.eventstream";

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
    Bearer(OpenAiCompatibleProvider),
    SigV4 {
        profile: Option<String>,
        static_credentials: Option<Credentials>,
    },
}

#[derive(Clone)]
pub struct AmazonBedrockProvider {
    client: reqwest::Client,
    base_url: String,
    default_model: ModelId,
    region: String,
    auth_mode: BedrockAuthMode,
    resolved_sigv4_shape: Arc<Mutex<Option<crate::provider::PromptCacheShape>>>,
}

impl AmazonBedrockProvider {
    pub fn new_bearer(
        client: reqwest::Client,
        api_token: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        Self::new_managed_bearer(
            client,
            ManagedCredential::static_value("amazon-bedrock bearer token", api_token.into()),
            base_url,
            default_model,
            region,
        )
    }

    pub fn new_managed_bearer(
        client: reqwest::Client,
        api_token: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        let base_url = utils::normalize_base_url(base_url.into().as_str());
        let default_model = ModelId::new(default_model);
        let region = region.into();
        Self {
            client: client.clone(),
            base_url: base_url.clone(),
            default_model: default_model.clone(),
            region,
            auth_mode: BedrockAuthMode::Bearer(OpenAiCompatibleProvider::new_managed(
                PROVIDER_ID,
                client,
                api_token,
                base_url,
                default_model,
            )),
            resolved_sigv4_shape: Arc::new(Mutex::new(None)),
        }
    }

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
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        method: reqwest::Method,
        url: String,
        body: Option<Vec<u8>>,
        headers: Vec<(String, String)>,
    ) -> Result<reqwest::Response, AppError> {
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

        let mut request = self.client.request(method, url);
        for (name, value) in signing_headers.iter() {
            request = request.header(name, value);
        }
        // Apply chat.headers plugin hook on top of signed headers.
        request = utils::apply_request_headers(PROVIDER_ID, request, &Default::default());
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
                profile,
                static_credentials,
                reqwest::Method::GET,
                self.models_endpoint(),
                None,
                Vec::new(),
            )
            .await?;

        let payload: Value = utils::parse_json_response(PROVIDER_ID, response).await?;
        self.parse_models(payload)
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
            PROVIDER_ID,
            payload
                .choices
                .first()
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.tool_calls.as_ref()),
        )?;

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(
                "amazon-bedrock returned empty completion payload without finish reason".to_owned(),
            ));
        }

        let usage = payload.usage.map(|u| {
            MessageUsage {
                input_tokens: u.prompt_tokens.unwrap_or_default(),
                output_tokens: u.completion_tokens.unwrap_or_default(),
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 0.0,
            }
            .into()
        });

        Ok(CompletionResponse {
            provider_id: ProviderId::new(PROVIDER_ID),
            model: ModelId::new(
                payload
                    .model
                    .unwrap_or_else(|| self.default_model.to_string()),
            ),
            text,
            reasoning_text: None,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: None,
        })
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
        tools: &[crate::tool::EntryDefinition],
    ) -> Vec<BedrockAnthropicEntryDefinition> {
        tools
            .iter()
            .map(|tool| BedrockAnthropicEntryDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
                cache_control: None,
                eager_input_streaming: None,
            })
            .collect()
    }

    fn anthropic_content_to_blocks(message: &Message) -> Vec<BedrockAnthropicTextBlock> {
        let projected = wire_message::project(message);
        if projected.is_empty() {
            let text = message.as_text_lossy();
            if text.is_empty() {
                return Vec::new();
            }

            if message.role == Role::Tool {
                return vec![BedrockAnthropicTextBlock::tool_result("tool", text)];
            }

            return vec![BedrockAnthropicTextBlock::text(text)];
        }

        let mut blocks = Vec::new();
        for part in projected {
            match part {
                wire_message::WirePart::Text { text } => {
                    blocks.push(BedrockAnthropicTextBlock::text(text));
                }
                wire_message::WirePart::Attachment { item } => {
                    blocks.extend(Self::anthropic_attachment_blocks(&item));
                }
                wire_message::WirePart::ToolCall {
                    id,
                    name,
                    arguments_json,
                } => blocks.push(BedrockAnthropicTextBlock::tool_use(
                    id,
                    name,
                    arguments_json,
                )),
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    output_json,
                } => blocks.push(BedrockAnthropicTextBlock::tool_result(
                    tool_call_id,
                    output_json,
                )),
            }
        }

        blocks
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
        tools: &mut [BedrockAnthropicEntryDefinition],
        messages: &mut [BedrockAnthropicMessage],
    ) {
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
                Role::Assistant => messages.push(BedrockAnthropicMessage {
                    role: "assistant".to_owned(),
                    content: Self::anthropic_content_to_blocks(&msg),
                }),
                Role::User | Role::Tool => messages.push(BedrockAnthropicMessage {
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
                temperature: request.temperature,
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

    async fn complete_sigv4_anthropic(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let (model, body) = Self::build_anthropic_request(request);

        let response = self
            .send_sigv4_request(
                profile,
                static_credentials,
                reqwest::Method::POST,
                self.native_anthropic_invoke_endpoint(model.as_str(), false)?,
                Some(serde_json::to_vec(&body)?),
                Self::anthropic_invoke_headers(false),
            )
            .await?;

        let payload: BedrockAnthropicMessagesResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;
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
        let (model, body) = Self::build_anthropic_request(request);
        let response = self
            .send_sigv4_request(
                profile,
                static_credentials,
                reqwest::Method::POST,
                self.native_anthropic_invoke_endpoint(model.as_str(), true)?,
                Some(serde_json::to_vec(&body)?),
                Self::anthropic_invoke_headers(true),
            )
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

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

                let parsed: BedrockAnthropicStreamEvent =
                    utils::parse_json_value(PROVIDER_ID, "bedrock anthropic stream event", event)?;

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
                                model: stream_model.clone(),
                                stream_key: format!("idx:{index}"),
                                id: Some(state.id.clone()),
                                name: Some(state.name.clone()),
                                arguments_delta,
                            };
                        }
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
        let messages = convert_messages(request.system, request.messages);
        let body = ChatCompletionRequest {
            model,
            messages,
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            stream: false,
            stream_options: None,
            prompt_cache_key: prompt_cache_key.clone(),
            prompt_cache_key_camel_case: prompt_cache_key.clone(),
        };
        let mut headers = vec![(
            reqwest::header::CONTENT_TYPE.as_str().to_owned(),
            JSON_CONTENT_TYPE.to_owned(),
        )];
        if let Some(session_affinity) = prompt_cache_key {
            headers.push(("x-session-affinity".to_owned(), session_affinity));
        }

        let response = self
            .send_sigv4_request(
                profile,
                static_credentials,
                reqwest::Method::POST,
                self.completions_endpoint(),
                Some(serde_json::to_vec(&body)?),
                headers,
            )
            .await?;

        let payload: ChatCompletionResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;
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
        let messages = convert_messages(request.system, request.messages);

        let body = ChatCompletionRequest {
            model: model.clone(),
            messages,
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            stream: true,
            stream_options: Some(ChatStreamOptions {
                include_usage: true,
            }),
            prompt_cache_key: prompt_cache_key.clone(),
            prompt_cache_key_camel_case: prompt_cache_key.clone(),
        };
        let mut headers = vec![(
            reqwest::header::CONTENT_TYPE.as_str().to_owned(),
            JSON_CONTENT_TYPE.to_owned(),
        )];
        if let Some(session_affinity) = prompt_cache_key {
            headers.push(("x-session-affinity".to_owned(), session_affinity));
        }

        let response = self
            .send_sigv4_request(
                profile,
                static_credentials,
                reqwest::Method::POST,
                self.completions_endpoint(),
                Some(serde_json::to_vec(&body)?),
                headers,
            )
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

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
                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(PROVIDER_ID, "chat stream chunk", event)?;
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
                    if let Some(function) = tool.function {
                        if let Some(name) = utils::normalize_optional_text(function.name) {
                            state.name = Some(name);
                        }
                        if let Some(args) = function.arguments
                            && !args.is_empty() {
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
impl ModelProvider for AmazonBedrockProvider {
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
            BedrockAuthMode::Bearer(provider) => {
                shape.insert_string("auth_mode", "bearer");
                if let Some(bearer_shape) = provider.prompt_cache_shape(model) {
                    shape.extend_prefixed("bearer", &bearer_shape);
                }
            }
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
            BedrockAuthMode::Bearer(inner) => inner.list_models().await,
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
            BedrockAuthMode::Bearer(inner) => inner.complete(request).await,
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
            BedrockAuthMode::Bearer(inner) => inner.complete_stream(request).await,
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
        let projected_parts = wire_message::project(&message);
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

fn provider_message_to_openai_value(message: &Message, parts: &[wire_message::WirePart]) -> Value {
    if parts.is_empty() {
        return Value::String(message.as_text_lossy());
    }

    projected_parts_to_openai_value(parts)
}

fn attachment_upload_name(item: &AttachmentItem) -> String {
    wire_message::filename(item)
        .map(str::to_owned)
        .unwrap_or_else(|| item.summary_label())
}

fn attachment_file_content_value(item: &AttachmentItem) -> Option<Value> {
    let filename = attachment_upload_name(item);
    match &item.source {
        AttachmentSource::Base64 { .. } | AttachmentSource::DataUrl { .. } => {
            wire_message::data_url(item).map(|file_data| {
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
        AttachmentKind::Image => wire_message::media_url(item)
            .map(|url| {
                serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": url }
                })
            })
            .unwrap_or_else(|| {
                serde_json::json!({
                    "type": "text",
                    "text": wire_message::hint_text(item),
                })
            }),
        AttachmentKind::Audio
        | AttachmentKind::Video
        | AttachmentKind::Pdf
        | AttachmentKind::File => attachment_file_content_value(item).unwrap_or_else(|| {
            serde_json::json!({
                "type": "text",
                "text": wire_message::hint_text(item),
            })
        }),
    }
}

fn projected_parts_to_openai_value(parts: &[wire_message::WirePart]) -> Value {
    let items = parts
        .iter()
        .map(|part| match part {
            wire_message::WirePart::Text { text } => {
                serde_json::json!({ "type": "text", "text": text })
            }
            wire_message::WirePart::Attachment { item } => attachment_content_value(item),
            wire_message::WirePart::ToolCall { name, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_call:{name}]") })
            }
            wire_message::WirePart::ToolResult { tool_call_id, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_result:{tool_call_id}]") })
            }
        })
        .collect::<Vec<_>>();
    Value::Array(items)
}

fn assistant_content_and_tool_calls(
    message: &Message,
    parts: &[wire_message::WirePart],
) -> (Option<Value>, Vec<ChatToolCallRequest>) {
    if parts.is_empty() {
        return (Some(Value::String(message.as_text_lossy())), Vec::new());
    }

    let mut text_chunks = Vec::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        match part {
            wire_message::WirePart::Text { text } => text_chunks.push(text.clone()),
            wire_message::WirePart::ToolCall {
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
            wire_message::WirePart::Attachment { item } => {
                text_chunks.push(wire_message::hint_text(item));
            }
            wire_message::WirePart::ToolResult { tool_call_id, .. } => {
                text_chunks.push(format!("[tool_result:{tool_call_id}]"));
            }
        }
    }
    let content = (!text_chunks.is_empty()).then(|| Value::String(text_chunks.join("")));
    (content, tool_calls)
}

fn ordered_tool_and_user_messages_from_parts(parts: &[wire_message::WirePart]) -> Vec<ChatMessage> {
    let has_tool_message = parts.iter().any(|part| {
        matches!(
            part,
            wire_message::WirePart::ToolResult { tool_call_id, .. }
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
            wire_message::WirePart::ToolResult {
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
            wire_message::WirePart::ToolResult { output_json, .. } => {
                buffered_parts.push(wire_message::WirePart::Text {
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

fn session_text_lossy(message: &Message, projected_parts: &[wire_message::WirePart]) -> String {
    if projected_parts.is_empty() {
        message.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(projected_parts)
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
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<ChatStreamOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    #[serde(
        default,
        rename = "promptCacheKey",
        skip_serializing_if = "Option::is_none"
    )]
    prompt_cache_key_camel_case: Option<String>,
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

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
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
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
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
    tools: Option<Vec<BedrockAnthropicEntryDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
struct BedrockAnthropicEntryDefinition {
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
}

#[cfg(test)]
mod tests {
    use aws_credential_types::Credentials;
    use futures_util::StreamExt;
    use mockito::Matcher;

    use super::*;
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
    fn keeps_existing_cross_region_prefix() {
        assert_eq!(
            prefix_bedrock_model("us-east-1", "us.anthropic.claude-3-7-sonnet"),
            "us.anthropic.claude-3-7-sonnet"
        );
    }

    #[test]
    fn applies_us_prefix_for_supported_models() {
        assert_eq!(
            prefix_bedrock_model("us-east-1", "anthropic.claude-3-7-sonnet"),
            "us.anthropic.claude-3-7-sonnet"
        );
    }

    #[test]
    fn applies_eu_prefix_for_regional_models() {
        assert_eq!(
            prefix_bedrock_model("eu-west-1", "meta.llama3-70b-instruct"),
            "eu.meta.llama3-70b-instruct"
        );
    }

    #[test]
    fn applies_jp_prefix_for_tokyo_cross_region_models() {
        assert_eq!(
            prefix_bedrock_model("ap-northeast-1", "amazon.nova-pro-v1:0"),
            "jp.amazon.nova-pro-v1:0"
        );
    }

    #[test]
    fn applies_au_prefix_for_supported_australia_models() {
        assert_eq!(
            prefix_bedrock_model("ap-southeast-2", "anthropic.claude-sonnet-4-5"),
            "au.anthropic.claude-sonnet-4-5"
        );
    }

    #[test]
    fn keeps_bedrock_arn_model_without_cross_region_prefix() {
        assert_eq!(
            prefix_bedrock_model(
                "us-east-1",
                "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/claude-sonnet"
            ),
            "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/claude-sonnet"
        );
    }

    #[test]
    fn prompt_cache_shape_changes_when_sigv4_profile_changes() {
        let provider_a = AmazonBedrockProvider::new_sigv4(
            reqwest::Client::new(),
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1",
            "amazon.nova-pro-v1:0",
            "us-east-1",
            Some("profile-a".to_owned()),
            None,
        );
        let provider_b = AmazonBedrockProvider::new_sigv4(
            reqwest::Client::new(),
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1",
            "amazon.nova-pro-v1:0",
            "us-east-1",
            Some("profile-b".to_owned()),
            None,
        );

        let shape_a = provider_a
            .prompt_cache_shape(&crate::model::ModelId::new("amazon.nova-pro-v1:0"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&crate::model::ModelId::new("amazon.nova-pro-v1:0"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn prompt_cache_shape_changes_when_sigv4_static_access_key_changes() {
        let provider_a = AmazonBedrockProvider::new_sigv4(
            reqwest::Client::new(),
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1",
            "amazon.nova-pro-v1:0",
            "us-east-1",
            None,
            Some(Credentials::new(
                "AKIDEXAMPLEA",
                "secret-a",
                Some("session-a".to_owned()),
                None,
                "test",
            )),
        );
        let provider_b = AmazonBedrockProvider::new_sigv4(
            reqwest::Client::new(),
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1",
            "amazon.nova-pro-v1:0",
            "us-east-1",
            None,
            Some(Credentials::new(
                "AKIDEXAMPLEB",
                "secret-b",
                Some("session-b".to_owned()),
                None,
                "test",
            )),
        );

        let shape_a = provider_a
            .prompt_cache_shape(&crate::model::ModelId::new("amazon.nova-pro-v1:0"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&crate::model::ModelId::new("amazon.nova-pro-v1:0"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn prompt_cache_shape_ignores_sigv4_session_token_changes() {
        let provider_a = AmazonBedrockProvider::new_sigv4(
            reqwest::Client::new(),
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1",
            "amazon.nova-pro-v1:0",
            "us-east-1",
            None,
            Some(Credentials::new(
                "AKIDEXAMPLE",
                "secret",
                Some("session-a".to_owned()),
                None,
                "test",
            )),
        );
        let provider_b = AmazonBedrockProvider::new_sigv4(
            reqwest::Client::new(),
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1",
            "amazon.nova-pro-v1:0",
            "us-east-1",
            None,
            Some(Credentials::new(
                "AKIDEXAMPLE",
                "secret",
                Some("session-b".to_owned()),
                None,
                "test",
            )),
        );

        let shape_a = provider_a
            .prompt_cache_shape(&crate::model::ModelId::new("amazon.nova-pro-v1:0"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&crate::model::ModelId::new("amazon.nova-pro-v1:0"))
            .expect("shape should exist");

        assert_eq!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn prompt_cache_shape_changes_when_sigv4_runtime_account_id_changes() {
        let provider = test_sigv4_provider(
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1".to_owned(),
            "amazon.nova-pro-v1:0",
        );
        {
            let mut runtime_shape = provider
                .resolved_sigv4_shape
                .lock()
                .expect("runtime shape lock should succeed");
            *runtime_shape = Some(
                crate::provider::PromptCacheShape::new(PROVIDER_ID)
                    .with_string("account_id", "111111111111"),
            );
        }
        let shape_a = provider
            .prompt_cache_shape(&crate::model::ModelId::new("amazon.nova-pro-v1:0"))
            .expect("shape should exist");

        {
            let mut runtime_shape = provider
                .resolved_sigv4_shape
                .lock()
                .expect("runtime shape lock should succeed");
            *runtime_shape = Some(
                crate::provider::PromptCacheShape::new(PROVIDER_ID)
                    .with_string("account_id", "222222222222"),
            );
        }
        let shape_b = provider
            .prompt_cache_shape(&crate::model::ModelId::new("amazon.nova-pro-v1:0"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn anthropic_tools_leave_eager_input_streaming_disabled() {
        let tools = AmazonBedrockProvider::anthropic_tools(&[sample_tool_definition()]);

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].eager_input_streaming, None);
    }

    #[test]
    fn convert_messages_preserves_interleaved_tool_result_and_follow_up_order() {
        let mut message = crate::message::Message::prompt_parts(
            Role::Tool,
            vec![
                crate::message::PartContent::text("Before"),
                crate::message::PartContent::ToolExecution(
                    crate::message::ToolExecutionPart::Completed {
                        call_id: 1,
                        invocation: crate::message::ToolInvocation::Custom {
                            name: "tool_one".to_owned(),
                            input: crate::message::StructuredObject::default(),
                        },
                        output_text: "{\"result\":1}".to_owned(),
                        blocks: Vec::new(),
                        attachments: Vec::new(),
                        details: crate::message::ToolOutput::default(),
                        lifecycle: crate::message::TimeRange::default(),
                    },
                ),
                crate::message::PartContent::text("Middle"),
                crate::message::PartContent::ToolExecution(
                    crate::message::ToolExecutionPart::Completed {
                        call_id: 2,
                        invocation: crate::message::ToolInvocation::Custom {
                            name: "tool_two".to_owned(),
                            input: crate::message::StructuredObject::default(),
                        },
                        output_text: "{\"result\":2}".to_owned(),
                        blocks: Vec::new(),
                        attachments: Vec::new(),
                        details: crate::message::ToolOutput::default(),
                        lifecycle: crate::message::TimeRange::default(),
                    },
                ),
                crate::message::PartContent::text("After"),
            ],
        );
        message.parts[1].operation_id = Some("call_1".to_owned());
        message.parts[3].operation_id = Some("call_2".to_owned());

        let messages = convert_messages(None, vec![message]);

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

    #[test]
    fn sigv4_signing_includes_auth_and_date_headers() {
        let credentials = Credentials::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            None,
            None,
            "test",
        );

        let headers = signed_sigv4_headers(
            &reqwest::Method::POST,
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions",
            br#"{"model":"anthropic.claude-3-7-sonnet"}"#,
            &[(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            )],
            &credentials,
            "us-east-1",
        )
        .expect("signing should succeed");

        let authorization = headers
            .get(reqwest::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .expect("authorization header should be present");
        assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"));
        assert!(authorization.contains("/us-east-1/bedrock/aws4_request"));
        assert!(headers.get("x-amz-date").is_some());
        assert_eq!(
            headers
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
    }

    #[test]
    fn sigv4_signing_keeps_session_security_token() {
        let credentials = Credentials::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            Some("session-token-value".to_owned()),
            None,
            "test",
        );

        let headers = signed_sigv4_headers(
            &reqwest::Method::GET,
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/models",
            &[],
            &[],
            &credentials,
            "us-east-1",
        )
        .expect("signing should succeed");

        assert_eq!(
            headers
                .get("x-amz-security-token")
                .and_then(|value| value.to_str().ok()),
            Some("session-token-value")
        );
    }

    #[tokio::test]
    async fn complete_sigv4_sends_signed_headers_over_http() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .match_header(
                "authorization",
                Matcher::Regex("^AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/.*".to_owned()),
            )
            .match_header("x-amz-date", Matcher::Regex("^[0-9]{8}T[0-9]{6}Z$".to_owned()))
            .match_header("x-amz-security-token", "session-token-value")
            .match_header("x-session-affinity", "session-42")
            .match_header("content-type", "application/json")
            .match_body(Matcher::Regex(
                "\\\"prompt_cache_key\\\":\\\"session-42\\\"".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"model":"us.amazon.nova-pro-v1:0","choices":[{"message":{"content":"hello from bedrock"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":3}}"#,
            )
            .create();

        let provider = test_sigv4_provider(server.url(), "amazon.nova-pro-v1:0");
        let response = provider
            .complete(CompletionRequest {
                prompt_cache_key: Some("session-42".to_owned()),
                ..test_request("amazon.nova-pro-v1:0")
            })
            .await
            .expect("completion request should succeed");

        mock.assert();
        assert_eq!(response.text, "hello from bedrock");
    }

    #[tokio::test]
    async fn complete_sigv4_claude_models_use_native_invoke_with_anthropic_cache_hints() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock(
                "POST",
                Matcher::Regex(
                    "^/model/us\\.anthropic\\.claude-3-7-sonnet-20250219-v1(?::|%3A)0/invoke$"
                        .to_owned(),
                ),
            )
            .match_header(
                "authorization",
                Matcher::Regex("^AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/.*".to_owned()),
            )
            .match_header(
                "x-amz-date",
                Matcher::Regex("^[0-9]{8}T[0-9]{6}Z$".to_owned()),
            )
            .match_header("x-amz-security-token", "session-token-value")
            .match_header("content-type", "application/json")
            .match_body(Matcher::Regex(
                "\\\"anthropic_version\\\":\\\"bedrock-2023-05-31\\\"".to_owned(),
            ))
            .match_body(Matcher::Regex(
                "\\\"cache_control\\\":\\{\\\"type\\\":\\\"ephemeral\\\"\\}".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "id": "msg_bedrock_1",
                    "model": "us.anthropic.claude-3-7-sonnet-20250219-v1:0",
                    "stop_reason": "end_turn",
                    "content": [{
                        "type": "text",
                        "text": "hello native bedrock"
                    }],
                    "usage": {
                        "input_tokens": 2,
                        "output_tokens": 3,
                        "cache_creation": {
                            "ephemeral_5m_input_tokens": 7
                        },
                        "cache_read_input_tokens": 5
                    }
                })
                .to_string(),
            )
            .create();

        let provider =
            test_sigv4_provider(server.url(), "anthropic.claude-3-7-sonnet-20250219-v1:0");
        let response = provider
            .complete(CompletionRequest {
                system: Some("system".to_owned()),
                tools: vec![sample_tool_definition()],
                ..test_request("anthropic.claude-3-7-sonnet-20250219-v1:0")
            })
            .await
            .expect("native anthropic invoke should succeed");

        mock.assert();
        assert_eq!(response.text, "hello native bedrock");
        assert_eq!(
            response
                .usage
                .as_ref()
                .map(|usage| usage.cache_write_tokens),
            Some(7)
        );
        assert_eq!(
            response.usage.as_ref().map(|usage| usage.cache_read_tokens),
            Some(5)
        );
        assert_eq!(
            response
                .provider_metadata
                .as_ref()
                .and_then(|value| value.get("response_id"))
                .and_then(|value| value.as_str()),
            Some("msg_bedrock_1")
        );
    }

    #[tokio::test]
    async fn complete_stream_sigv4_sends_signed_headers_over_http() {
        let mut server = mockito::Server::new_async().await;
        let stream_body = "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n\
data: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n\
data: [DONE]\n\n";

        let mock = server
            .mock("POST", "/chat/completions")
            .match_header(
                "authorization",
                Matcher::Regex("^AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/.*".to_owned()),
            )
            .match_header(
                "x-amz-date",
                Matcher::Regex("^[0-9]{8}T[0-9]{6}Z$".to_owned()),
            )
            .match_header("x-amz-security-token", "session-token-value")
            .match_header("x-session-affinity", "session-42")
            .match_header("content-type", "application/json")
            .match_body(Matcher::Regex(
                "\\\"prompt_cache_key\\\":\\\"session-42\\\"".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(stream_body)
            .create();

        let provider = test_sigv4_provider(server.url(), "amazon.nova-pro-v1:0");
        let mut stream = provider
            .complete_stream(CompletionRequest {
                prompt_cache_key: Some("session-42".to_owned()),
                ..test_request("amazon.nova-pro-v1:0")
            })
            .await
            .expect("stream request should succeed");

        let mut saw_delta = false;
        let mut saw_completed = false;

        while let Some(event) = stream.next().await {
            match event.expect("stream event should be valid") {
                CompletionStreamEvent::TextDelta { delta, .. } => {
                    if delta == "hello" {
                        saw_delta = true;
                    }
                }
                CompletionStreamEvent::Completed { .. } => {
                    saw_completed = true;
                }
                _ => {}
            }
        }

        mock.assert();
        assert!(saw_delta, "expected text delta from stream");
        assert!(saw_completed, "expected completion event from stream");
    }

    #[tokio::test]
    async fn complete_stream_sigv4_claude_models_use_native_invoke_with_response_stream() {
        let mut server = mockito::Server::new_async().await;
        let stream_body = [
            encode_bedrock_chunk(serde_json::json!({
                "type": "message_start",
                "message": {
                    "id": "msg_bedrock_stream_1",
                    "model": "us.anthropic.claude-3-7-sonnet-20250219-v1:0",
                    "usage": {
                        "input_tokens": 11
                    }
                }
            })),
            encode_bedrock_chunk(serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {
                    "type": "text_delta",
                    "text": "hello native stream"
                }
            })),
            encode_bedrock_chunk(serde_json::json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": "end_turn"
                },
                "usage": {
                    "output_tokens": 5,
                    "cache_creation": {
                        "ephemeral_5m_input_tokens": 7
                    }
                }
            })),
            encode_bedrock_chunk(serde_json::json!({
                "type": "message_stop"
            })),
        ]
        .concat();

        let mock = server
            .mock(
                "POST",
                Matcher::Regex(
                    "^/model/us\\.anthropic\\.claude-3-7-sonnet-20250219-v1(?::|%3A)0/invoke-with-response-stream$"
                        .to_owned(),
                ),
            )
            .match_header(
                "authorization",
                Matcher::Regex("^AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/.*".to_owned()),
            )
            .match_header(
                "x-amz-date",
                Matcher::Regex("^[0-9]{8}T[0-9]{6}Z$".to_owned()),
            )
            .match_header("x-amz-security-token", "session-token-value")
            .match_header("content-type", "application/json")
            .match_header("accept", "application/vnd.amazon.eventstream")
            .match_header("x-amzn-bedrock-accept", "application/json")
            .match_body(Matcher::Regex(
                "\\\"anthropic_version\\\":\\\"bedrock-2023-05-31\\\"".to_owned(),
            ))
            .match_body(Matcher::Regex(
                "\\\"cache_control\\\":\\{\\\"type\\\":\\\"ephemeral\\\"\\}".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/vnd.amazon.eventstream")
            .with_body(stream_body)
            .create();

        let provider =
            test_sigv4_provider(server.url(), "anthropic.claude-3-7-sonnet-20250219-v1:0");
        let mut stream = provider
            .complete_stream(CompletionRequest {
                system: Some("system".to_owned()),
                tools: vec![sample_tool_definition()],
                ..test_request("anthropic.claude-3-7-sonnet-20250219-v1:0")
            })
            .await
            .expect("native anthropic stream should succeed");

        let mut saw_delta = false;
        let mut completed_usage: Option<CompletionUsage> = None;
        let mut completed_metadata: Option<serde_json::Value> = None;

        while let Some(event) = stream.next().await {
            match event.expect("stream event should be valid") {
                CompletionStreamEvent::TextDelta { delta, .. } => {
                    if delta == "hello native stream" {
                        saw_delta = true;
                    }
                }
                CompletionStreamEvent::Completed {
                    usage,
                    provider_metadata,
                    ..
                } => {
                    completed_usage = usage;
                    completed_metadata = provider_metadata;
                }
                _ => {}
            }
        }

        mock.assert();
        assert!(saw_delta, "expected native bedrock text delta");
        assert_eq!(
            completed_usage.as_ref().map(|usage| usage.input_tokens),
            Some(11)
        );
        assert_eq!(
            completed_usage.as_ref().map(|usage| usage.output_tokens),
            Some(5)
        );
        assert_eq!(
            completed_usage
                .as_ref()
                .map(|usage| usage.cache_write_tokens),
            Some(7)
        );
        assert_eq!(
            completed_metadata
                .as_ref()
                .and_then(|value| value.get("response_id"))
                .and_then(|value| value.as_str()),
            Some("msg_bedrock_stream_1")
        );
    }

    fn encode_bedrock_chunk(event: Value) -> Vec<u8> {
        encode_bedrock_stream_message(
            "chunk",
            serde_json::json!({
                "bytes": BASE64_STANDARD.encode(
                    serde_json::to_vec(&event).expect("event json should serialize")
                )
            }),
        )
    }

    fn encode_bedrock_stream_message(event_type: &str, payload: Value) -> Vec<u8> {
        let mut buffer = Vec::new();
        let message = aws_smithy_types::event_stream::Message::new(
            serde_json::to_vec(&payload).expect("payload json should serialize"),
        )
        .add_header(aws_smithy_types::event_stream::Header::new(
            ":message-type",
            aws_smithy_types::event_stream::HeaderValue::String("event".into()),
        ))
        .add_header(aws_smithy_types::event_stream::Header::new(
            ":event-type",
            aws_smithy_types::event_stream::HeaderValue::String(event_type.to_owned().into()),
        ));
        aws_smithy_eventstream::frame::write_message_to(&message, &mut buffer)
            .expect("event stream frame should encode");
        buffer
    }

    fn test_sigv4_provider(base_url: String, default_model: &str) -> AmazonBedrockProvider {
        AmazonBedrockProvider {
            client: reqwest::Client::new(),
            base_url,
            default_model: crate::model::ModelId::new(default_model),
            region: "us-east-1".to_owned(),
            auth_mode: BedrockAuthMode::SigV4 {
                profile: None,
                static_credentials: Some(Credentials::new(
                    "AKIDEXAMPLE",
                    "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
                    Some("session-token-value".to_owned()),
                    None,
                    "test",
                )),
            },
            resolved_sigv4_shape: Arc::new(Mutex::new(None)),
        }
    }

    fn test_request(model: &str) -> CompletionRequest {
        CompletionRequest {
            model: crate::model::ModelId::new(model),
            system: None,
            messages: vec![crate::message::Message::prompt_text(Role::User, "hello")],
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
        }
    }
}
