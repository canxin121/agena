use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    config::{ProviderNativeToolKind, ProviderNativeToolRoute},
    error::AppError,
    message::{AttachmentItem, Message, MessageUsage},
    model::{ModelId, ModelMetadata, ModelTokenLimits, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        ManagedCredential, ModelRuntime, ProviderModel, ReasoningEffort, ResponseFormat,
        ThinkingRequest, should_retry_credential, sse, utils, wire_message,
    },
    role::Role,
};

mod gemini_adapter;

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

#[derive(Clone)]
pub struct GeminiAdapterOptions {
    pub auth_header: Option<(String, Option<String>)>,
    pub auth_query_parameter: Option<String>,
    pub extra_headers: HashMap<String, String>,
    pub stream_mode: GeminiStreamMode,
    pub realtime_ws_url: Option<String>,
}

impl Default for GeminiAdapterOptions {
    fn default() -> Self {
        Self {
            auth_header: None,
            auth_query_parameter: None,
            extra_headers: HashMap::new(),
            stream_mode: GeminiStreamMode::Sse,
            realtime_ws_url: None,
        }
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
        let mut fields = vec![
            ("auth_scope", self.api_key.prompt_cache_scope()),
            ("base_url", self.base_url.clone()),
            ("default_model", self.default_model.to_string()),
            ("auth_transport", self.auth_transport_key().to_owned()),
            ("stream_mode", self.stream_mode_key().to_owned()),
            (
                "extra_headers",
                crate::provider::PromptCacheShape::json_field_value(
                    &utils::prompt_cache_header_entries(&self.extra_headers),
                ),
            ),
        ];
        match &self.auth_mode {
            GeminiAuthMode::QueryParameter { name } => {
                fields.push(("auth_query_param", name.clone()));
            }
            GeminiAuthMode::Header { name, scheme } => {
                fields.push(("auth_header", name.clone()));
                if let Some(scheme) = scheme.as_deref() {
                    fields.push(("auth_scheme", scheme.to_owned()));
                }
            }
        }
        if let Some(realtime_ws_url) = self.realtime_ws_url.as_deref() {
            fields.push(("realtime_ws_url", realtime_ws_url.to_owned()));
        }
        Some(crate::provider::PromptCacheShape::from_fields(
            PROVIDER_ID,
            fields,
        ))
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
                let model_id = ModelId::new(id);
                let capabilities = self.model_capabilities(&model_id);
                ProviderModel {
                    provider_id: ProviderId::new(PROVIDER_ID),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name: m.display_name,
                    capabilities,
                    metadata,
                    thinking_modes: std::collections::BTreeMap::new(),
                    speed_modes: std::collections::BTreeMap::new(),
                }
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
                    self.endpoint_with_auth(self.generate_endpoint(model.as_ref()), api_key);
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
                    self.endpoint_with_auth(self.stream_generate_endpoint(model.as_ref()), api_key);
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
                    provider_id.as_ref(),
                    ADAPTER_KIND,
                    "complete_stream.generate_content",
                    &event,
                );

                let chunk: GeminiGenerateResponse =
                    utils::parse_json_value(provider_id.as_ref(), "stream chunk", event)?;
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
    tool_name.trim().to_owned()
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
        // Gemini exposes input/output limits rather than a separate context
        // window field, so use the input ceiling as the best available
        // prompt-window budget.
        let input_limit = self.input_token_limit.map(clamp_u64_to_u32);
        ModelMetadata {
            lifecycle: None,
            limits: ModelTokenLimits {
                context_window_tokens: input_limit,
                max_input_tokens: input_limit,
                max_output_tokens: self.output_token_limit.map(clamp_u64_to_u32),
            },
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
        }
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
