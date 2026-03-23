use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::{Credentials, provider::ProvideCredentials};
use aws_sigv4::http_request::{SignableBody, SignableRequest, SigningSettings, sign};
use aws_sigv4::sign::v4;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    auth::AuthData,
    error::AppError,
    message::MessageUsage,
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ModelProvider, OpenAiCompatibleProvider,
        ProviderContent, ProviderContentPart, ProviderModel, sse, utils,
    },
    role::Role,
};

const PROVIDER_ID: &str = "amazon-bedrock";
const DEFAULT_MODEL: &str = "anthropic.claude-3-7-sonnet-20250219-v1:0";

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
    default_model: String,
    region: String,
    auth_mode: BedrockAuthMode,
}

impl AmazonBedrockProvider {
    pub fn from_env_and_auth(
        client: reqwest::Client,
        auth: Option<&AuthData>,
    ) -> Result<Option<Self>, AppError> {
        let api_token = env_non_empty("AWS_BEARER_TOKEN_BEDROCK")
            .or_else(|| scoped_env_non_empty("API_KEY"))
            .or_else(|| auth.and_then(AuthData::api_key).map(ToOwned::to_owned));

        let profile = scoped_env_non_empty("PROFILE").or_else(|| env_non_empty("AWS_PROFILE"));
        let static_credentials = resolve_static_credentials();

        if api_token.is_none()
            && static_credentials.is_none()
            && !should_enable_credential_chain(profile.as_deref())
        {
            return Ok(None);
        }

        let region = scoped_env_non_empty("REGION")
            .or_else(|| env_non_empty("AWS_REGION"))
            .unwrap_or_else(|| "us-east-1".to_owned());

        let base_url = scoped_env_non_empty("ENDPOINT")
            .or_else(|| env_non_empty("AWS_BEDROCK_ENDPOINT"))
            .or_else(|| scoped_env_non_empty("BASE_URL"))
            .or_else(|| env_non_empty("AWS_BEDROCK_BASE_URL"))
            .unwrap_or_else(|| format!("https://bedrock-runtime.{region}.amazonaws.com/openai/v1"));

        let default_model = scoped_env_non_empty("MODEL")
            .or_else(|| env_non_empty("AWS_BEDROCK_MODEL"))
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());

        let auth_mode = if let Some(token) = api_token {
            BedrockAuthMode::Bearer(OpenAiCompatibleProvider::new(
                PROVIDER_ID,
                client.clone(),
                token,
                base_url.clone(),
                default_model.clone(),
            ))
        } else {
            BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            }
        };

        Ok(Some(Self {
            client,
            base_url: utils::normalize_base_url(base_url.as_str()),
            default_model,
            region,
            auth_mode,
        }))
    }

    fn resolve_model(&self, model: &str) -> String {
        let model = if model.trim().is_empty() {
            self.default_model.clone()
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

    async fn resolve_sigv4_credentials(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
    ) -> Result<Credentials, AppError> {
        if let Some(credentials) = static_credentials {
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

        provider.provide_credentials().await.map_err(|err| {
            AppError::Provider(format!(
                "amazon-bedrock failed to resolve aws credentials from chain: {err}"
            ))
        })
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
            .map(|model| ProviderModel {
                provider_id: PROVIDER_ID.to_owned(),
                id: model.id,
                display_name: model.display_name.or(model.name),
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
            provider_id: PROVIDER_ID.to_owned(),
            model: payload.model.unwrap_or_else(|| self.default_model.clone()),
            text,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: None,
        })
    }

    async fn complete_sigv4(
        &self,
        profile: Option<&str>,
        static_credentials: Option<&Credentials>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let model = self.resolve_model(request.model.as_str());
        let messages = convert_messages(request.system, request.messages);
        let body = ChatCompletionRequest {
            model,
            messages,
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            stream: false,
            stream_options: None,
        };

        let response = self
            .send_sigv4_request(
                profile,
                static_credentials,
                reqwest::Method::POST,
                self.completions_endpoint(),
                Some(serde_json::to_vec(&body)?),
                vec![(
                    reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                    "application/json".to_owned(),
                )],
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
        };

        let response = self
            .send_sigv4_request(
                profile,
                static_credentials,
                reqwest::Method::POST,
                self.completions_endpoint(),
                Some(serde_json::to_vec(&body)?),
                vec![(
                    reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                    "application/json".to_owned(),
                )],
            )
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

        let mut events = sse::json_events(response);
        let model_name = model;

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
                        provider_id: PROVIDER_ID.to_owned(),
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
                        if let Some(args) = function.arguments {
                            if !args.is_empty() {
                                state.arguments.push_str(args.as_str());
                                stream_has_content = true;
                                yield CompletionStreamEvent::ToolCallDelta {
                                    provider_id: PROVIDER_ID.to_owned(),
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
                    provider_id: PROVIDER_ID.to_owned(),
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

    fn default_model(&self) -> &str {
        self.default_model.as_str()
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
        let request = CompletionRequest { model, ..request };

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
        let request = CompletionRequest { model, ..request };

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

fn convert_messages(
    system: Option<String>,
    messages: Vec<crate::provider::ProviderMessage>,
) -> Vec<ChatMessage> {
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
        match message.role {
            Role::System => {
                result.push(ChatMessage {
                    role: "system".to_owned(),
                    content: Some(Value::String(message.as_text_lossy())),
                    tool_call_id: None,
                    tool_calls: None,
                });
            }
            Role::User => {
                result.push(ChatMessage {
                    role: "user".to_owned(),
                    content: Some(provider_content_to_openai_value(&message.content)),
                    tool_call_id: None,
                    tool_calls: None,
                });
            }
            Role::Assistant => {
                let (content, tool_calls) = assistant_content_and_tool_calls(&message.content);
                result.push(ChatMessage {
                    role: "assistant".to_owned(),
                    content,
                    tool_call_id: None,
                    tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
                });
            }
            Role::Tool => {
                let tool_messages = tool_messages_from_content(&message.content);
                if tool_messages.is_empty() {
                    result.push(ChatMessage {
                        role: "tool".to_owned(),
                        content: Some(Value::String(message.as_text_lossy())),
                        tool_call_id: Some("tool".to_owned()),
                        tool_calls: None,
                    });
                } else {
                    result.extend(tool_messages);
                }
            }
        }
    }

    result
}

fn provider_content_to_openai_value(content: &ProviderContent) -> Value {
    match content {
        ProviderContent::Text(text) => Value::String(text.clone()),
        ProviderContent::Parts(parts) => {
            let items = parts
                .iter()
                .map(|part| match part {
                    ProviderContentPart::Text { text } => {
                        serde_json::json!({ "type": "text", "text": text })
                    }
                    ProviderContentPart::ImageUrl { url } => serde_json::json!({
                        "type": "image_url",
                        "image_url": { "url": url }
                    }),
                    ProviderContentPart::ToolCall { name, .. } => {
                        serde_json::json!({ "type": "text", "text": format!("[tool_call:{name}]") })
                    }
                    ProviderContentPart::ToolResult { tool_call_id, .. } => {
                        serde_json::json!({ "type": "text", "text": format!("[tool_result:{tool_call_id}]") })
                    }
                })
                .collect::<Vec<_>>();
            Value::Array(items)
        }
    }
}

fn assistant_content_and_tool_calls(
    content: &ProviderContent,
) -> (Option<Value>, Vec<ChatToolCallRequest>) {
    match content {
        ProviderContent::Text(text) => (Some(Value::String(text.clone())), Vec::new()),
        ProviderContent::Parts(parts) => {
            let mut text_chunks = Vec::new();
            let mut tool_calls = Vec::new();
            for part in parts {
                match part {
                    ProviderContentPart::Text { text } => text_chunks.push(text.clone()),
                    ProviderContentPart::ToolCall {
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
                    ProviderContentPart::ImageUrl { url } => {
                        text_chunks.push(format!("[image:{url}]"));
                    }
                    ProviderContentPart::ToolResult { tool_call_id, .. } => {
                        text_chunks.push(format!("[tool_result:{tool_call_id}]"));
                    }
                }
            }
            let content = (!text_chunks.is_empty()).then(|| Value::String(text_chunks.join("")));
            (content, tool_calls)
        }
    }
}

fn tool_messages_from_content(content: &ProviderContent) -> Vec<ChatMessage> {
    let ProviderContent::Parts(parts) = content else {
        return Vec::new();
    };

    parts
        .iter()
        .filter_map(|part| match part {
            ProviderContentPart::ToolResult {
                tool_call_id,
                output_json,
            } => Some(ChatMessage {
                role: "tool".to_owned(),
                content: Some(Value::String(output_json.clone())),
                tool_call_id: Some(tool_call_id.clone()),
                tool_calls: None,
            }),
            _ => None,
        })
        .collect()
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

fn prefix_bedrock_model(region: &str, model: &str) -> String {
    if model.is_empty() || has_cross_region_prefix(model) {
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

fn resolve_static_credentials() -> Option<Credentials> {
    let access_key_id =
        scoped_env_non_empty("ACCESS_KEY_ID").or_else(|| env_non_empty("AWS_ACCESS_KEY_ID"))?;
    let secret_access_key = scoped_env_non_empty("SECRET_ACCESS_KEY")
        .or_else(|| env_non_empty("AWS_SECRET_ACCESS_KEY"))?;
    let session_token =
        scoped_env_non_empty("SESSION_TOKEN").or_else(|| env_non_empty("AWS_SESSION_TOKEN"));

    Some(Credentials::new(
        access_key_id,
        secret_access_key,
        session_token,
        None,
        "agena-bedrock-static",
    ))
}

fn should_enable_credential_chain(profile: Option<&str>) -> bool {
    profile.is_some_and(|value| !value.trim().is_empty())
        || env_non_empty("AWS_ACCESS_KEY_ID").is_some()
        || env_non_empty("AWS_WEB_IDENTITY_TOKEN_FILE").is_some()
        || env_non_empty("AWS_CONTAINER_CREDENTIALS_FULL_URI").is_some()
        || env_non_empty("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_some()
}

fn has_cross_region_prefix(model: &str) -> bool {
    CROSS_REGION_PREFIXES
        .iter()
        .any(|prefix| model.starts_with(prefix))
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
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

fn scoped_env_non_empty(suffix: &str) -> Option<String> {
    env_non_empty(format!("AGENA_PROVIDER_AMAZON_BEDROCK_{suffix}").as_str())
        .or_else(|| env_non_empty(format!("AMAZON_BEDROCK_{suffix}").as_str()))
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
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
    fn credential_chain_can_be_enabled_by_profile_hint() {
        assert!(should_enable_credential_chain(Some("default")));
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
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"model":"anthropic.claude-3-7-sonnet","choices":[{"message":{"content":"hello from bedrock"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":3}}"#,
            )
            .create();

        let provider = test_sigv4_provider(server.url());
        let response = provider
            .complete(test_request())
            .await
            .expect("completion request should succeed");

        mock.assert();
        assert_eq!(response.text, "hello from bedrock");
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
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(stream_body)
            .create();

        let provider = test_sigv4_provider(server.url());
        let mut stream = provider
            .complete_stream(test_request())
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

    fn test_sigv4_provider(base_url: String) -> AmazonBedrockProvider {
        AmazonBedrockProvider {
            client: reqwest::Client::new(),
            base_url,
            default_model: DEFAULT_MODEL.to_owned(),
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
        }
    }

    fn test_request() -> CompletionRequest {
        CompletionRequest {
            model: DEFAULT_MODEL.to_owned(),
            system: None,
            messages: vec![crate::provider::ProviderMessage::new(Role::User, "hello")],
            temperature: None,
            max_output_tokens: None,
        }
    }
}
