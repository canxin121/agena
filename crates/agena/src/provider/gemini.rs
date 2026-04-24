use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    error::AppError,
    message::{AttachmentItem, Message, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionUsage, ManagedCredential, ModelProvider, ProviderModel, should_retry_credential,
        sse, utils,
    },
    role::Role,
};

const PROVIDER_ID: &str = "google";

#[derive(Clone)]
pub struct GeminiProvider {
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    auth_mode: GeminiAuthMode,
    extra_headers: HashMap<String, String>,
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

impl GeminiProvider {
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
        Self {
            client,
            api_key,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            auth_mode: GeminiAuthMode::QueryParameter {
                name: "key".to_owned(),
            },
            extra_headers: HashMap::new(),
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

    pub fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.extra_headers = headers;
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

    fn apply_auth(
        &self,
        request: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        match &self.auth_mode {
            GeminiAuthMode::QueryParameter { .. } => request,
            GeminiAuthMode::Header { name, scheme } => request.header(
                name.as_str(),
                utils::auth_header_value(scheme.as_deref(), api_key),
            ),
        }
    }

    fn auth_transport_key(&self) -> &'static str {
        match self.auth_mode {
            GeminiAuthMode::QueryParameter { .. } => "query_parameter",
            GeminiAuthMode::Header { .. } => "header",
        }
    }

    fn message_parts(message: &Message) -> Vec<GeminiPart> {
        let projected_parts = utils::project_session_parts(message);
        if projected_parts.is_empty() {
            let text = message.as_text_lossy();
            return (!text.trim().is_empty())
                .then(|| vec![GeminiPart::text(text)])
                .unwrap_or_default();
        }

        projected_parts
            .iter()
            .map(|part| match part {
                utils::ProjectedSessionPart::Text { text } => GeminiPart::text(text.clone()),
                utils::ProjectedSessionPart::Attachment { item } => Self::attachment_part(item),
                utils::ProjectedSessionPart::ToolCall { name, .. } => {
                    GeminiPart::text(format!("[tool_call:{name}]"))
                }
                utils::ProjectedSessionPart::ToolResult { tool_call_id, .. } => {
                    GeminiPart::text(format!("[tool_result:{tool_call_id}]"))
                }
            })
            .collect()
    }

    fn attachment_part(item: &AttachmentItem) -> GeminiPart {
        utils::attachment_base64_with_mime(item)
            .map(|(mime_type, data)| GeminiPart::inline_data(mime_type, data))
            .unwrap_or_else(|| GeminiPart::text(utils::attachment_hint_text(item)))
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

    async fn complete_by_aggregating_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let fallback_model = request.model.clone();
        let mut stream = ModelProvider::complete_stream(self, request).await?;
        let mut text = String::new();
        let mut completed: Option<(
            ProviderId,
            ModelId,
            Option<CompletionFinishReason>,
            Option<CompletionUsage>,
            Option<serde_json::Value>,
        )> = None;

        while let Some(item) = stream.next().await {
            match item? {
                CompletionStreamEvent::TextDelta { delta, .. } => {
                    text.push_str(delta.as_str());
                }
                CompletionStreamEvent::Completed {
                    provider_id,
                    model,
                    finish_reason,
                    usage,
                    provider_metadata,
                } => {
                    completed = Some((provider_id, model, finish_reason, usage, provider_metadata));
                }
                CompletionStreamEvent::ToolCallDelta { .. } => {}
            }
        }

        let (provider_id, model, finish_reason, usage, provider_metadata) = completed
            .unwrap_or_else(|| {
                (
                    ProviderId::new(PROVIDER_ID),
                    fallback_model,
                    None,
                    None,
                    None,
                )
            });

        if text.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(
                "gemini stream fallback produced empty completion".to_owned(),
            ));
        }

        Ok(CompletionResponse {
            provider_id,
            model,
            text,
            finish_reason,
            tool_calls: Vec::new(),
            usage,
            provider_metadata,
        })
    }
}

#[async_trait]
impl ModelProvider for GeminiProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn model_capabilities(&self, model: &ModelId) -> crate::provider::ModelCapabilities {
        crate::provider::default_capability_registry()
            .capabilities_for_family(crate::provider::CapabilityFamily::Gemini, model.as_str())
    }

    fn model_metadata(&self, model: &ModelId) -> crate::provider::ModelMetadata {
        crate::provider::default_model_metadata_registry()
            .metadata_for_family(crate::provider::CapabilityFamily::Gemini, model.as_str())
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        Some(
            crate::provider::PromptCacheShape::new(PROVIDER_ID)
                .with_string("auth_scope", self.api_key.prompt_cache_scope())
                .with_string("base_url", self.base_url.as_str())
                .with_string("default_model", self.default_model.as_str())
                .with_string("auth_transport", self.auth_transport_key())
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
                .with_json(
                    "extra_headers",
                    &utils::prompt_cache_header_entries(&self.extra_headers),
                ),
        )
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let response = self
            .send_request(|api_key| {
                let endpoint = self.endpoint_with_auth(self.list_models_endpoint(), api_key);
                utils::apply_extra_headers(
                    self.apply_auth(self.client.get(endpoint), api_key),
                    &self.extra_headers,
                )
            })
            .await?;

        let payload: GeminiModelListResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;
        Ok(payload
            .models
            .into_iter()
            .map(|m| {
                let id = m.name.trim_start_matches("models/").to_owned();
                let mut model = ProviderModel::new(PROVIDER_ID, id);
                let capabilities = self.model_capabilities(&model.id);
                model = model.with_capabilities(capabilities);
                model.display_name = m.display_name;
                model
            })
            .collect())
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = request.model.clone();
        let stream_fallback_request = request.clone();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(system.clone());
        }

        let mut contents = Vec::new();
        for msg in request.messages {
            match msg.role {
                Role::System => system_chunks.push(msg.as_text_lossy()),
                Role::Assistant => contents.push(GeminiContent {
                    role: Some("model".to_owned()),
                    parts: Self::message_parts(&msg),
                }),
                Role::User | Role::Tool => contents.push(GeminiContent {
                    role: Some("user".to_owned()),
                    parts: Self::message_parts(&msg),
                }),
            }
        }

        let body = GeminiGenerateRequest {
            system_instruction: (!system_chunks.is_empty()).then(|| GeminiInstruction {
                parts: vec![GeminiPart::text(system_chunks.join("\n\n"))],
            }),
            contents,
            generation_config: GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_output_tokens,
            },
            stream: None,
        };

        let response = self
            .send_request(|api_key| {
                let endpoint =
                    self.endpoint_with_auth(self.generate_endpoint(model.as_str()), api_key);
                utils::apply_extra_headers(
                    self.apply_auth(
                        self.client
                            .post(endpoint)
                            .header(reqwest::header::CONTENT_TYPE, "application/json")
                            .json(&body),
                        api_key,
                    ),
                    &self.extra_headers,
                )
            })
            .await?;

        let payload: GeminiGenerateResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;
        let text = payload
            .candidates
            .first()
            .map(GeminiCandidate::text)
            .unwrap_or_default();

        let finish_reason = payload
            .candidates
            .first()
            .and_then(|c| c.finish_reason.clone());
        let usage = payload.usage_metadata.map(map_gemini_usage);

        if text.is_empty() {
            return self
                .complete_by_aggregating_stream(stream_fallback_request)
                .await;
        }

        Ok(CompletionResponse {
            provider_id: ProviderId::new(PROVIDER_ID),
            model,
            text,
            finish_reason: CompletionFinishReason::from_provider(finish_reason.as_deref()),
            tool_calls: Vec::new(),
            usage,
            provider_metadata: payload
                .candidates
                .first()
                .and_then(GeminiCandidate::provider_metadata),
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

        let mut contents = Vec::new();
        for msg in request.messages {
            match msg.role {
                Role::System => system_chunks.push(msg.as_text_lossy()),
                Role::Assistant => contents.push(GeminiContent {
                    role: Some("model".to_owned()),
                    parts: Self::message_parts(&msg),
                }),
                Role::User | Role::Tool => contents.push(GeminiContent {
                    role: Some("user".to_owned()),
                    parts: Self::message_parts(&msg),
                }),
            }
        }

        let body = GeminiGenerateRequest {
            system_instruction: (!system_chunks.is_empty()).then(|| GeminiInstruction {
                parts: vec![GeminiPart::text(system_chunks.join("\n\n"))],
            }),
            contents,
            generation_config: GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_output_tokens,
            },
            stream: Some(true),
        };

        let response = self
            .send_request(|api_key| {
                let endpoint =
                    self.endpoint_with_auth(self.stream_generate_endpoint(model.as_str()), api_key);
                utils::apply_extra_headers(
                    self.apply_auth(
                        self.client
                            .post(endpoint)
                            .header(reqwest::header::CONTENT_TYPE, "application/json")
                            .json(&body),
                        api_key,
                    ),
                    &self.extra_headers,
                )
            })
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(PROVIDER_ID);
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut emitted = String::new();
            let mut saw_content = false;
            let mut fallback_usage: Option<crate::provider::CompletionUsage> = None;
            let mut fallback_provider_metadata: Option<serde_json::Value> = None;
            let mut completed_emitted = false;

            while let Some(event) = events.next().await {
                let event = event?;

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

                for stream_event in GeminiStreamEvent::from_chunk(chunk, &mut emitted) {
                    match stream_event {
                        GeminiStreamEvent::TextDelta(delta) => {
                            saw_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta,
                            };
                        }
                        GeminiStreamEvent::Completed {
                            finish_reason,
                            usage,
                            provider_metadata,
                        } => {
                            completed_emitted = true;
                            yield CompletionStreamEvent::Completed {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                finish_reason: CompletionFinishReason::from_provider(
                                    Some(finish_reason.as_str()),
                                ),
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
                    finish_reason: None,
                    usage: fallback_usage,
                    provider_metadata: fallback_provider_metadata,
                };
            }
        };

        Ok(Box::pin(stream))
    }
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

#[derive(Debug, Serialize, Deserialize)]
struct GeminiPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(
        default,
        rename = "inlineData",
        skip_serializing_if = "Option::is_none"
    )]
    inline_data: Option<GeminiInlineData>,
}

impl GeminiPart {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            inline_data: None,
        }
    }

    fn inline_data(mime_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            text: None,
            inline_data: Some(GeminiInlineData {
                mime_type: mime_type.into(),
                data: data.into(),
            }),
        }
    }
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
            .map(|content| {
                content
                    .parts
                    .iter()
                    .filter_map(|part| part.text.clone())
                    .collect::<Vec<_>>()
                    .join("")
            })
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

fn map_gemini_usage(u: GeminiUsageMetadata) -> crate::provider::CompletionUsage {
    MessageUsage {
        input_tokens: u.prompt_token_count.unwrap_or_default(),
        output_tokens: u.candidates_token_count.unwrap_or_default(),
        reasoning_tokens: u.thoughts_token_count.unwrap_or_default(),
        cache_write_tokens: 0,
        cache_read_tokens: u.cached_content_token_count.unwrap_or_default(),
        total_cost: 0.0,
    }
    .into()
}

#[derive(Debug)]
enum GeminiStreamEvent {
    TextDelta(String),
    Completed {
        finish_reason: String,
        usage: Option<crate::provider::CompletionUsage>,
        provider_metadata: Option<serde_json::Value>,
    },
}

impl GeminiStreamEvent {
    fn from_chunk(chunk: GeminiGenerateResponse, emitted: &mut String) -> Vec<Self> {
        let mut events = Vec::new();
        let candidate = chunk.candidates.first();

        if let Some(candidate) = candidate {
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

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;

    use crate::message::Message;
    use crate::provider::CompletionRequest;

    #[test]
    fn prompt_cache_shape_changes_when_auth_scope_changes() {
        let provider_a = GeminiProvider::new_managed(
            reqwest::Client::new(),
            ManagedCredential::environment("gemini env", "google", "api_key", "GEMINI_API_KEY_A"),
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-flash",
        );
        let provider_b = GeminiProvider::new_managed(
            reqwest::Client::new(),
            ManagedCredential::environment("gemini env", "google", "api_key", "GEMINI_API_KEY_B"),
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-flash",
        );

        let shape_a = provider_a
            .prompt_cache_shape(&crate::model::ModelId::new("gemini-2.5-flash"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&crate::model::ModelId::new("gemini-2.5-flash"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn prompt_cache_shape_ignores_volatile_or_secret_extra_headers() {
        let provider_a = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-flash",
        )
        .with_extra_headers(HashMap::from([
            ("x-gemini-route".to_owned(), "backend-a".to_owned()),
            ("x-request-id".to_owned(), "req-a".to_owned()),
            ("traceparent".to_owned(), "trace-a".to_owned()),
            ("authorization".to_owned(), "Bearer secret-a".to_owned()),
        ]));
        let provider_b = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-flash",
        )
        .with_extra_headers(HashMap::from([
            ("x-gemini-route".to_owned(), "backend-a".to_owned()),
            ("x-request-id".to_owned(), "req-b".to_owned()),
            ("traceparent".to_owned(), "trace-b".to_owned()),
            ("authorization".to_owned(), "Bearer secret-b".to_owned()),
        ]));

        let shape_a = provider_a
            .prompt_cache_shape(&crate::model::ModelId::new("gemini-2.5-flash"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&crate::model::ModelId::new("gemini-2.5-flash"))
            .expect("shape should exist");

        assert_eq!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn prompt_cache_shape_changes_when_auth_transport_changes() {
        let query_provider = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-flash",
        );
        let header_provider = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-flash",
        )
        .with_auth_header("x-goog-api-key", None::<String>);

        let query_shape = query_provider
            .prompt_cache_shape(&crate::model::ModelId::new("gemini-2.5-flash"))
            .expect("shape should exist");
        let header_shape = header_provider
            .prompt_cache_shape(&crate::model::ModelId::new("gemini-2.5-flash"))
            .expect("shape should exist");

        assert_ne!(query_shape.fingerprint(), header_shape.fingerprint());
    }

    #[tokio::test]
    async fn complete_applies_extra_headers() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/models/gemini-2.5-flash:generateContent")
            .match_query(mockito::Matcher::UrlEncoded(
                "key".to_owned(),
                "test-key".to_owned(),
            ))
            .match_header("x-gemini-route", "backend-a")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "candidates": [{
                        "content": { "parts": [{ "text": "Hello" }] },
                        "finishReason": "STOP"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            server.url(),
            "gemini-2.5-flash",
        )
        .with_extra_headers(HashMap::from([(
            "x-gemini-route".to_owned(),
            "backend-a".to_owned(),
        )]));

        let response = provider
            .complete(CompletionRequest {
                model: crate::model::ModelId::new("gemini-2.5-flash"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "Hello");
    }

    #[tokio::test]
    async fn complete_uses_header_auth_when_configured() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/models/gemini-2.5-flash:generateContent")
            .match_header("x-goog-api-key", "test-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "candidates": [{
                        "content": { "parts": [{ "text": "Hello" }] },
                        "finishReason": "STOP"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            server.url(),
            "gemini-2.5-flash",
        )
        .with_auth_header("x-goog-api-key", None::<String>);

        let response = provider
            .complete(CompletionRequest {
                model: crate::model::ModelId::new("gemini-2.5-flash"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "Hello");
    }

    #[tokio::test]
    async fn complete_falls_back_to_stream_when_candidate_text_is_empty() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/models/gemini-2.5-flash:generateContent")
            .match_query(mockito::Matcher::UrlEncoded(
                "key".to_owned(),
                "test-key".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "candidates": [{
                        "content": { "parts": [] },
                        "finishReason": "STOP"
                    }],
                    "usageMetadata": {
                        "promptTokenCount": 4,
                        "candidatesTokenCount": 2
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;
        let _stream = server
            .mock("POST", "/models/gemini-2.5-flash:streamGenerateContent")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("alt".to_owned(), "sse".to_owned()),
                mockito::Matcher::UrlEncoded("key".to_owned(), "test-key".to_owned()),
            ]))
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(concat!(
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"fallback \"}]}}]}\n\n",
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"fallback text\"}]}}]}\n\n",
                "data: {\"candidates\":[{\"finishReason\":\"STOP\",\"content\":{\"parts\":[{\"text\":\"fallback text\"}]},\"safetyRatings\":[{\"category\":\"SAFE\"}]}],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2}}\n\n",
                "data: [DONE]\n\n"
            ))
            .create_async()
            .await;

        let provider = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            server.url(),
            "gemini-2.5-flash",
        );

        let response = provider
            .complete(CompletionRequest {
                model: crate::model::ModelId::new("gemini-2.5-flash"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("empty candidate text should fall back to stream aggregation");

        assert_eq!(response.text, "fallback text");
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::Stop)
        ));
        let usage = response.usage.expect("usage should be present");
        assert_eq!(usage.input_tokens, 4);
        assert_eq!(usage.output_tokens, 2);
        let metadata = response
            .provider_metadata
            .expect("provider metadata should be present");
        assert!(metadata.get("safety_ratings").is_some());
    }

    #[tokio::test]
    async fn complete_stream_parses_typed_gemini_chunks() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hel\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"finishReason\":\"STOP\",\"content\":{\"parts\":[{\"text\":\"Hello\"}]},\"safetyRatings\":[{\"category\":\"SAFE\"}]}],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2,\"thoughtsTokenCount\":1,\"cachedContentTokenCount\":1}}\n\n",
            "data: [DONE]\n\n"
        );

        let _mock = server
            .mock("POST", "/models/gemini-2.5-flash:streamGenerateContent")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("alt".to_owned(), "sse".to_owned()),
                mockito::Matcher::UrlEncoded("key".to_owned(), "test-key".to_owned()),
            ]))
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            server.url(),
            "gemini-2.5-flash",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: crate::model::ModelId::new("gemini-2.5-flash"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
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
                    provider_metadata,
                    ..
                } => {
                    assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                    let usage = usage.expect("usage should be present");
                    assert_eq!(usage.input_tokens, 4);
                    assert_eq!(usage.output_tokens, 2);
                    assert_eq!(usage.reasoning_tokens, 1);
                    assert_eq!(usage.cache_read_tokens, 1);
                    let metadata = provider_metadata.expect("provider metadata should be present");
                    assert!(metadata.get("safety_ratings").is_some());
                    done = true;
                }
                _ => {}
            }
        }

        assert_eq!(text, "Hello");
        assert!(done);
    }

    #[tokio::test]
    async fn complete_stream_emits_completed_when_stream_ends_without_finish_reason() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Ack\"}]}}],",
            "\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":1,\"totalTokenCount\":5,\"cachedContentTokenCount\":2}}\n\n"
        );
        let _mock = server
            .mock("POST", "/models/gemini-2.5-flash:streamGenerateContent")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("alt".to_owned(), "sse".to_owned()),
                mockito::Matcher::UrlEncoded("key".to_owned(), "test-key".to_owned()),
            ]))
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            server.url(),
            "gemini-2.5-flash",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: crate::model::ModelId::new("gemini-2.5-flash"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
            })
            .await
            .expect("stream should start");

        let mut text = String::new();
        let mut completed = None;

        while let Some(item) = stream.next().await {
            match item.expect("stream item should parse") {
                CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(delta.as_str()),
                CompletionStreamEvent::Completed {
                    finish_reason,
                    usage,
                    ..
                } => {
                    completed = Some((finish_reason, usage));
                }
                CompletionStreamEvent::ToolCallDelta { .. } => {}
            }
        }

        assert_eq!(text, "Ack");
        let (finish_reason, usage) = completed.expect("completed event should be emitted");
        assert!(finish_reason.is_none());
        let usage = usage.expect("usage should be present");
        assert_eq!(usage.input_tokens, 4);
        assert_eq!(usage.output_tokens, 1);
        assert_eq!(usage.cache_read_tokens, 2);
    }
}
