use agena_domain::*;
use agena_provider::{
    CompletionFinishReason, CompletionToolCall, CompletionUsage, GeminiCandidate, GeminiContent,
    GeminiFunctionCall, GeminiFunctionDeclaration, GeminiGenerateRequest, GeminiGenerateResponse,
    GeminiGenerationConfig, GeminiInstruction, GeminiLiveClientContent,
    GeminiLiveConversationRequest, GeminiLiveServerContent, GeminiLiveServerMessage,
    GeminiLiveSetup, GeminiModelListResponse, GeminiPart, GeminiStreamMode, ProviderNativeToolKind,
    ProviderNativeToolRoute, ResponseFormat, gemini_thinking_config, gemini_usage_to_completion,
};

use agena_domain::Role;
use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use std::collections::{HashMap, HashSet};

use crate::{
    ProviderError,
    provider::{
        CompletionResponse, ManagedCredential, ModelRuntime, should_retry_credential, sse, utils,
        wire_message,
    },
};
use agena_provider::CompletionRequest;
use agena_provider::CompletionStreamEvent;
use agena_runtime_contracts::message::AttachmentItem;

mod gemini_adapter;

const PROVIDER_ID: &str = "google";
const ADAPTER_KIND: &str = "gemini";
const GEMINI_FINAL_PART_SIGNATURE_KEY: &str = "$final_part";

#[derive(Clone)]
/// Adapter for Gemini.
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

#[derive(Clone)]
/// Options for the Gemini adapter.
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

    fn capability_family(&self) -> Option<agena_provider::CapabilityFamily> {
        Some(agena_provider::CapabilityFamily::Gemini)
    }

    fn validate_provider_native_tools_request(
        &self,
        _adapter_id: Option<&agena_domain::AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), ProviderError> {
        build_gemini_tools(request).map(|_| ())
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        let mut fields = vec![
            ("auth_scope", self.api_key.prompt_cache_scope()),
            ("base_url", self.base_url.clone()),
            ("default_model", self.default_model.to_string()),
            ("auth_transport", self.auth_transport_key().to_owned()),
            ("stream_mode", self.stream_mode_key().to_owned()),
            (
                "extra_headers",
                agena_provider::PromptCacheShape::json_field_value(
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
        Some(agena_provider::PromptCacheShape::from_fields(
            PROVIDER_ID,
            fields,
        ))
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
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
                Model {
                    provider_id: ProviderId::new(PROVIDER_ID),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name: m.display_name,
                    native_compaction: true,
                    capabilities,
                    metadata,
                    thinking_modes: Vec::new(),
                    speed_modes: std::collections::BTreeMap::new(),
                }
            })
            .collect())
    }

    #[tracing::instrument(skip_all, fields(provider = "gemini", model = %request.model))]
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let model = request.model.clone();
        let stream_fallback_request = request.clone();
        let body = self.generate_request(&request)?;
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;
        let request_headers = self.completion_request_headers(&request);

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
        let text = candidate.map(gemini_candidate_text).unwrap_or_default();
        let reasoning_text = candidate.and_then(gemini_candidate_reasoning_text);
        let tool_calls = candidate
            .map(gemini_candidate_function_calls)
            .transpose()?
            .unwrap_or_default();
        let finish_reason = CompletionFinishReason::normalize_with_tool_calls(
            CompletionFinishReason::from_provider(
                candidate.and_then(|c| c.finish_reason.as_deref()),
            ),
            !tool_calls.is_empty(),
        );
        let usage = payload.usage_metadata.map(gemini_usage_to_completion);

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
                .and_then(gemini_candidate_provider_metadata),
        })
    }

    #[tracing::instrument(skip_all, fields(provider = "gemini", model = %request.model))]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let model = request.model.clone();

        if matches!(self.stream_mode, GeminiStreamMode::RealtimeWebSocket)
            && !Self::request_contains_tool_results(&request)
        {
            return self
                .complete_stream_with_realtime_ws(&request, model.clone())
                .await;
        }

        let body = self.generate_request(&request)?;
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;
        let request_headers = self.completion_request_headers(&request);

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
            let mut emitted_tool_calls = HashSet::new();
            let mut next_tool_call_index = 0usize;
            let mut saw_content = false;
            let mut fallback_usage: Option<CompletionUsage> = None;
            let mut fallback_provider_metadata: Option<serde_json::Value> = None;
            let mut completed_emitted = false;
            let mut tool_call_seen = false;

            while let Some(event) = events.next().await {
                let event =
                    event.map_err(|err| utils::json_stream_error(PROVIDER_ID, err))?;
                utils::adapter_log_stream_event(
                    provider_id.as_ref(),
                    ADAPTER_KIND,
                    "complete_stream.generate_content",
                    &event,
                );

                let mut chunk: GeminiGenerateResponse =
                    utils::parse_json_value(provider_id.as_ref(), "stream chunk", event)?;
                ensure_gemini_stream_function_call_ids(
                    &mut chunk,
                    &mut next_tool_call_index,
                );
                if let Some(usage) = chunk
                    .usage_metadata
                    .as_ref()
                    .cloned()
                    .map(gemini_usage_to_completion)
                {
                    fallback_usage = Some(usage);
                }
                if let Some(metadata) = chunk
                    .candidates
                    .first()
                    .and_then(gemini_candidate_provider_metadata)
                {
                    fallback_provider_metadata = merge_gemini_provider_metadata(
                        fallback_provider_metadata.take(),
                        Some(metadata),
                    );
                }
                let mut done = false;

                for stream_event in GeminiStreamEvent::from_chunk(
                    chunk,
                    &mut emitted_tool_calls,
                )? {
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
                            let CompletionToolCall::Function {
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
                            let resolved_finish_reason = CompletionFinishReason::normalize_with_tool_calls(
                                CompletionFinishReason::from_provider(Some(finish_reason.as_str())),
                                tool_call_seen,
                            );
                            yield CompletionStreamEvent::Completed {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                finish_reason: resolved_finish_reason,
                                usage: usage.or_else(|| fallback_usage.clone()),
                                provider_metadata: merge_gemini_provider_metadata(
                                    fallback_provider_metadata.clone(),
                                    provider_metadata,
                                ),
                                end_turn: None,
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
                    end_turn: None,
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
    tool_name.to_owned()
}

fn gemini_candidate_text(candidate: &GeminiCandidate) -> String {
    candidate
        .content
        .as_ref()
        .map(gemini_text_from_content)
        .unwrap_or_default()
}

fn gemini_candidate_reasoning_text(candidate: &GeminiCandidate) -> Option<String> {
    candidate
        .content
        .as_ref()
        .and_then(gemini_reasoning_text_from_content)
}

fn gemini_candidate_function_calls(
    candidate: &GeminiCandidate,
) -> Result<Vec<CompletionToolCall>, ProviderError> {
    candidate
        .content
        .as_ref()
        .map(gemini_function_calls_from_content)
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn gemini_candidate_provider_metadata(candidate: &GeminiCandidate) -> Option<serde_json::Value> {
    let mut map = serde_json::Map::new();
    if let Some(s) = candidate.safety_ratings.clone() {
        map.insert("safety_ratings".to_owned(), s);
    }
    if let Some(g) = candidate.grounding_metadata.clone() {
        map.insert("grounding_metadata".to_owned(), g);
    }
    if let Some(content) = candidate.content.as_ref()
        && let Some(signatures) = gemini_thought_signatures_from_content(content)
    {
        map.insert("gemini_thought_signatures".to_owned(), signatures);
    }
    (!map.is_empty()).then_some(serde_json::Value::Object(map))
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
) -> Result<Vec<CompletionToolCall>, ProviderError> {
    let mut calls = Vec::new();
    for (idx, part) in content.parts.iter().enumerate() {
        let Some(call) = part.function_call.as_ref() else {
            continue;
        };
        let arguments_json = gemini_tool_call_arguments_json(call, "generateContent")?;
        calls.push(CompletionToolCall::Function {
            id: call
                .id
                .clone()
                .unwrap_or_else(|| format!("{}-{idx}", call.name)),
            name: call.name.clone(),
            arguments_json,
        });
    }
    Ok(calls)
}

fn gemini_tool_call_arguments_json(
    call: &GeminiFunctionCall,
    protocol: &str,
) -> Result<String, ProviderError> {
    if call.name.trim().is_empty() {
        return Err(ProviderError::Provider(format!(
            "gemini {protocol} response returned functionCall without name"
        )));
    }
    if call.args.is_null() {
        return Ok("{}".to_owned());
    }
    if !call.args.is_object() {
        return Err(ProviderError::Provider(format!(
            "gemini {protocol} response returned non-object functionCall.args for `{}`",
            call.name
        )));
    }
    serde_json::to_string(&call.args).map_err(|error| {
        ProviderError::Provider(format!(
            "gemini {protocol} response returned invalid functionCall.args for `{}`: {error}",
            call.name
        ))
    })
}

fn parse_json_or_object(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Object(Default::default());
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value @ serde_json::Value::Object(_)) => value,
        Ok(_) | Err(_) => serde_json::Value::Object(Default::default()),
    }
}

/// Wrap non-JSON tool output in `{ "output": "<text>" }` because Gemini's
/// `functionResponse.response` field must be a JSON object.
fn parse_json_or_string_object(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Object(Default::default());
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value @ serde_json::Value::Object(_)) => value,
        Ok(other) => serde_json::json!({ "output": other }),
        Err(_) => serde_json::json!({ "output": raw }),
    }
}

fn merge_gemini_tool_provider_options(
    map: &mut serde_json::Map<String, serde_json::Value>,
    extra: Option<&serde_json::Value>,
    tool_label: &str,
) -> Result<(), ProviderError> {
    let Some(extra) = extra else {
        return Ok(());
    };
    let extra = extra.as_object().ok_or_else(|| {
        ProviderError::Config(format!(
            "gemini provider-native tool `{tool_label}` provider_options must be a JSON object"
        ))
    })?;
    for (key, value) in extra {
        map.insert(key.clone(), value.clone());
    }
    Ok(())
}

fn build_gemini_tools(
    request: &CompletionRequest,
) -> Result<Option<Vec<serde_json::Value>>, ProviderError> {
    let provider_native_tool_bindings = request.provider_native_tools.bindings();
    if provider_native_tool_bindings.is_empty() && request.tool_api_functions.is_empty() {
        return Ok(None);
    }
    if !provider_native_tool_bindings.is_empty() && !request.tool_api_functions.is_empty() {
        return Err(ProviderError::Config(
            "Gemini provider-hosted tools cannot be combined with function tools in the current API; remove plugin tools or disable provider-hosted tools for this model".to_owned(),
        ));
    }

    let mut tools = Vec::new();
    if !request.tool_api_functions.is_empty() {
        let function_declarations = request
            .tool_api_functions
            .iter()
            .cloned()
            .map(|tool| {
                Ok(GeminiFunctionDeclaration {
                    name: gemini_wire_tool_name(tool.name.as_str()),
                    description: tool.description,
                    parameters_json_schema: Some(tool.input_schema),
                })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        let mut map = serde_json::Map::new();
        map.insert(
            "functionDeclarations".to_owned(),
            serde_json::to_value(function_declarations)
                .expect("gemini function declarations should serialize"),
        );
        tools.push(serde_json::Value::Object(map));
    }

    for binding in provider_native_tool_bindings {
        if binding.route != ProviderNativeToolRoute::ProviderHosted {
            return Err(ProviderError::Config(format!(
                "gemini provider-native tool `{}` only supports `provider_hosted` routes in the current runtime",
                binding.tool.config_key()
            )));
        }
        match binding.tool {
            ProviderNativeToolKind::WebSearch => {
                let config = &request.provider_native_tools.hosted.web_search;
                if !config.allowed_domains.is_empty()
                    || !config.blocked_domains.is_empty()
                    || config.freshness.is_some()
                    || !config.user_location.is_empty()
                    || config.max_results.is_some()
                    || config.search_context_size.is_some()
                {
                    return Err(ProviderError::Config(
                        "gemini provider-native tool `web_search` currently only supports `provider_options`; other hosted web_search fields are not implemented for Gemini".to_owned(),
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
                let config = &request.provider_native_tools.hosted.url_context;
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
                let config = &request.provider_native_tools.hosted.code_execution;
                if !config.container.is_empty() {
                    return Err(ProviderError::Config(
                        "gemini provider-native tool `code_execution` does not support `hosted.code_execution.container`; use `provider_options` for Gemini-specific overrides instead".to_owned(),
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
                return Err(ProviderError::Config(format!(
                    "gemini provider-native tool `{}` is not supported by the current runtime",
                    other.config_key()
                )));
            }
        }
    }

    Ok(Some(tools))
}

#[derive(Debug)]
enum GeminiStreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCall(CompletionToolCall),
    Completed {
        finish_reason: String,
        usage: Option<CompletionUsage>,
        provider_metadata: Option<serde_json::Value>,
    },
}

impl GeminiStreamEvent {
    fn from_chunk(
        chunk: GeminiGenerateResponse,
        emitted_tool_calls: &mut HashSet<String>,
    ) -> Result<Vec<Self>, ProviderError> {
        let mut events = Vec::new();
        let candidate = chunk.candidates.first();

        if let Some(candidate) = candidate {
            let reasoning_delta = gemini_candidate_reasoning_text(candidate).unwrap_or_default();
            if !reasoning_delta.is_empty() {
                events.push(Self::ThinkingDelta(reasoning_delta));
            }

            let text_delta = gemini_candidate_text(candidate);
            if !text_delta.is_empty() {
                events.push(Self::TextDelta(text_delta));
            }

            // Gemini chunks contain complete functionCall parts, but separate
            // calls can arrive in separate chunks. Deduplicate only by the
            // stable call ID instead of treating each chunk as a cumulative
            // snapshot whose length can only grow.
            for call in gemini_candidate_function_calls(candidate)? {
                let CompletionToolCall::Function { id, .. } = &call;
                if emitted_tool_calls.insert(id.clone()) {
                    events.push(Self::ToolCall(call));
                }
            }

            if let Some(finish_reason) = candidate.finish_reason.clone() {
                events.push(Self::Completed {
                    finish_reason,
                    usage: chunk.usage_metadata.map(gemini_usage_to_completion),
                    provider_metadata: gemini_candidate_provider_metadata(candidate),
                });
            }
        }

        Ok(events)
    }
}

fn ensure_gemini_stream_function_call_ids(
    chunk: &mut GeminiGenerateResponse,
    next_tool_call_index: &mut usize,
) {
    for candidate in &mut chunk.candidates {
        let Some(content) = candidate.content.as_mut() else {
            continue;
        };
        for part in &mut content.parts {
            let Some(call) = part.function_call.as_mut() else {
                continue;
            };
            let call_index = *next_tool_call_index;
            *next_tool_call_index += 1;
            if call.id.as_deref().is_none_or(str::is_empty) {
                let name = call.name.trim();
                call.id = Some(if name.is_empty() {
                    format!("function-{call_index}")
                } else {
                    format!("{name}-{call_index}")
                });
            }
        }
    }
}

fn gemini_live_server_content_provider_metadata(
    content: &GeminiLiveServerContent,
) -> Option<serde_json::Value> {
    let grounding = content
        .grounding_metadata
        .clone()
        .map(|metadata| serde_json::json!({ "grounding_metadata": metadata }));
    let signatures = content.model_turn.as_ref().and_then(|content| {
        gemini_thought_signatures_from_content(content)
            .map(|signatures| serde_json::json!({ "gemini_thought_signatures": signatures }))
    });
    merge_gemini_provider_metadata(grounding, signatures)
}

fn gemini_thought_signatures_from_content(content: &GeminiContent) -> Option<serde_json::Value> {
    let mut signatures = serde_json::Map::new();
    let mut final_part_signature: Option<String> = None;
    for (index, part) in content.parts.iter().enumerate() {
        let signature = part
            .thought_signature
            .as_deref()
            .filter(|signature| !signature.is_empty());
        let Some(call) = part.function_call.as_ref() else {
            if let Some(signature) = signature {
                final_part_signature = Some(signature.to_owned());
            }
            continue;
        };
        let Some(signature) = signature else {
            continue;
        };
        let call_id = call
            .id
            .clone()
            .unwrap_or_else(|| format!("{}-{index}", call.name));
        signatures.insert(call_id, serde_json::Value::String(signature.to_owned()));
    }
    if let Some(signature) = final_part_signature {
        signatures.insert(
            GEMINI_FINAL_PART_SIGNATURE_KEY.to_owned(),
            serde_json::Value::String(signature),
        );
    }
    (!signatures.is_empty()).then_some(serde_json::Value::Object(signatures))
}

fn merge_gemini_provider_metadata(
    current: Option<serde_json::Value>,
    update: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut merged = current
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let update = update.and_then(|value| value.as_object().cloned());
    for (key, value) in update.unwrap_or_default() {
        if key == "gemini_thought_signatures" {
            let mut signatures = merged
                .remove(key.as_str())
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            if let Some(next) = value.as_object() {
                signatures.extend(next.clone());
            }
            if !signatures.is_empty() {
                merged.insert(key, serde_json::Value::Object(signatures));
            }
        } else {
            merged.insert(key, value);
        }
    }
    (!merged.is_empty()).then_some(serde_json::Value::Object(merged))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_provider::GeminiUsageMetadata;

    fn request_with_tool_api_function() -> CompletionRequest {
        use agena_plugin_host::registry::RegisteredTool;
        use agena_plugin_host::sdk::{Plugin, PluginKey};

        let manifest = agena_bundled_plugins::tool::new_tool_api_plugin().manifest();
        let plugin_key =
            PluginKey::new(manifest.namespace, manifest.name).expect("tools plugin key");
        let handler = manifest
            .tools
            .into_iter()
            .find(|tool| tool.name == "help")
            .expect("help Tool API handler");
        let mut request: CompletionRequest = serde_json::from_value(serde_json::json!({
            "model": "gemini-test",
            "messages": []
        }))
        .expect("minimal completion request");
        request.tool_api_functions.push(
            agena_runtime_tools::tool::ToolApiBinding::from_registered_tool(
                RegisteredTool::new(plugin_key, handler).expect("registered Tool API handler"),
            )
            .expect("Tool API binding")
            .definition(),
        );
        request
    }

    #[test]
    fn custom_functions_cannot_be_combined_with_google_search() {
        let mut request = request_with_tool_api_function();
        request.provider_native_tools.routes.web_search =
            Some(ProviderNativeToolRoute::ProviderHosted);

        let error = build_gemini_tools(&request).expect_err("mixed tool request must fail closed");
        assert!(error.to_string().contains("cannot be combined"));
    }

    #[test]
    fn generate_content_body_does_not_contain_a_nonstandard_stream_flag() {
        let request = GeminiGenerateRequest {
            system_instruction: None,
            contents: vec![],
            generation_config: GeminiGenerationConfig {
                temperature: None,
                max_output_tokens: None,
                top_p: None,
                top_k: None,
                stop_sequences: vec![],
                response_mime_type: None,
                response_json_schema: None,
                thinking_config: None,
                response_modalities: None,
            },
            tools: None,
            tool_config: None,
        };
        let value = serde_json::to_value(request).expect("serialize GenerateContent request");

        assert!(value.get("stream").is_none());
    }

    #[test]
    fn function_parts_preserve_ids_signatures_and_object_shapes() {
        let call = serde_json::to_value(GeminiPart::function_call(
            Some("call_123".to_owned()),
            "lookup",
            serde_json::json!({ "query": "rust" }),
            Some("signed-thought".to_owned()),
        ))
        .expect("serialize function call");
        assert_eq!(call["functionCall"]["id"], "call_123");
        assert_eq!(call["thoughtSignature"], "signed-thought");
        assert_eq!(call["functionCall"]["args"]["query"], "rust");

        let response = serde_json::to_value(GeminiPart::function_response(
            Some("call_123".to_owned()),
            "lookup",
            parse_json_or_string_object("plain output"),
        ))
        .expect("serialize function response");
        assert_eq!(response["functionResponse"]["id"], "call_123");
        assert_eq!(
            response["functionResponse"]["response"]["output"],
            "plain output"
        );
    }

    #[test]
    fn adjacent_same_role_contents_are_merged_without_crossing_role_boundaries() {
        let mut contents = Vec::new();
        GeminiAdapter::push_content(
            &mut contents,
            GeminiContent {
                role: Some("user".to_owned()),
                parts: vec![GeminiPart::function_response(
                    Some("call_1".to_owned()),
                    "first",
                    serde_json::json!({ "result": 1 }),
                )],
            },
        );
        GeminiAdapter::push_content(
            &mut contents,
            GeminiContent {
                role: Some("user".to_owned()),
                parts: vec![GeminiPart::function_response(
                    Some("call_2".to_owned()),
                    "second",
                    serde_json::json!({ "result": 2 }),
                )],
            },
        );
        GeminiAdapter::push_content(
            &mut contents,
            GeminiContent {
                role: Some("model".to_owned()),
                parts: vec![GeminiPart::text("done")],
            },
        );

        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0].role.as_deref(), Some("user"));
        assert_eq!(contents[0].parts.len(), 2);
        assert_eq!(contents[1].role.as_deref(), Some("model"));
    }

    #[test]
    fn function_call_arguments_reject_non_object_json() {
        assert_eq!(
            parse_json_or_object(r#"{"valid":true}"#),
            serde_json::json!({ "valid": true })
        );
        assert_eq!(parse_json_or_object(r#"[1,2,3]"#), serde_json::json!({}));
        assert_eq!(parse_json_or_object("invalid"), serde_json::json!({}));
    }

    #[test]
    fn provider_function_calls_require_names_and_object_arguments() {
        let unnamed = GeminiFunctionCall {
            id: Some("call-1".to_owned()),
            name: "".to_owned(),
            args: serde_json::json!({}),
        };
        assert!(gemini_tool_call_arguments_json(&unnamed, "test").is_err());

        let non_object = GeminiFunctionCall {
            id: Some("call-2".to_owned()),
            name: "tools_help".to_owned(),
            args: serde_json::json!(["session.get"]),
        };
        let error = gemini_tool_call_arguments_json(&non_object, "test")
            .expect_err("non-object args must fail");
        assert!(error.to_string().contains("non-object functionCall.args"));
    }

    #[test]
    fn json_schema_fields_use_the_official_names_without_rewriting_schema() {
        let schema = serde_json::json!({
            "$defs": { "item": { "type": "string" } },
            "type": "object",
            "properties": { "value": { "$ref": "#/$defs/item" } }
        });
        let declaration = serde_json::to_value(GeminiFunctionDeclaration {
            name: "lookup".to_owned(),
            description: "Lookup".to_owned(),
            parameters_json_schema: Some(schema.clone()),
        })
        .expect("serialize function declaration");
        assert_eq!(declaration["parametersJsonSchema"], schema);
        assert!(declaration.get("parameters").is_none());

        let config = GeminiGenerationConfig {
            temperature: None,
            max_output_tokens: None,
            top_p: None,
            top_k: None,
            stop_sequences: vec![],
            response_mime_type: Some("application/json".to_owned()),
            response_json_schema: Some(serde_json::json!({ "type": "object" })),
            thinking_config: None,
            response_modalities: None,
        };
        let config = serde_json::to_value(config).expect("serialize generation config");
        assert_eq!(config["responseJsonSchema"]["type"], "object");
        assert!(config.get("responseSchema").is_none());
    }

    #[test]
    fn current_thinking_controls_use_model_specific_wire_values() {
        let dynamic = gemini_thinking_config(
            "gemini-2.5-flash",
            Some(&ThinkingRequest::Adaptive {
                effort: None,
                display: None,
            }),
        )
        .expect("2.5 thinking config");
        assert_eq!(dynamic.thinking_budget, Some(-1));
        assert_eq!(dynamic.include_thoughts, Some(true));

        let clamped_pro = gemini_thinking_config(
            "gemini-2.5-pro",
            Some(&ThinkingRequest::Budget { budget_tokens: 0 }),
        )
        .expect("2.5 Pro thinking config");
        assert_eq!(clamped_pro.thinking_budget, Some(128));

        let clamped_lite = gemini_thinking_config(
            "gemini-2.5-flash-lite",
            Some(&ThinkingRequest::Budget { budget_tokens: 1 }),
        )
        .expect("2.5 Flash Lite thinking config");
        assert_eq!(clamped_lite.thinking_budget, Some(512));

        let clamped_max = gemini_thinking_config(
            "gemini-2.5-flash",
            Some(&ThinkingRequest::Budget {
                budget_tokens: u32::MAX,
            }),
        )
        .expect("2.5 Flash thinking config");
        assert_eq!(clamped_max.thinking_budget, Some(24_576));

        let medium = gemini_thinking_config(
            "gemini-3.1-pro-preview",
            Some(&ThinkingRequest::Effort {
                effort: ReasoningEffort::Medium,
            }),
        )
        .expect("Gemini 3 thinking config");
        assert_eq!(medium.thinking_level, Some("MEDIUM"));

        let minimal = gemini_thinking_config(
            "gemini-3.1-pro-preview",
            Some(&ThinkingRequest::Effort {
                effort: ReasoningEffort::Minimal,
            }),
        )
        .expect("minimal Gemini 3.1 Pro thinking config");
        assert_eq!(minimal.thinking_level, Some("LOW"));

        let disabled =
            gemini_thinking_config("gemini-3.1-pro-preview", Some(&ThinkingRequest::Disabled))
                .expect("disabled Gemini 3 config");
        assert_eq!(disabled.thinking_level, Some("LOW"));
        assert_eq!(disabled.include_thoughts, Some(false));
    }

    #[test]
    fn usage_keeps_visible_candidates_separate_from_thoughts() {
        let usage = gemini_usage_to_completion(GeminiUsageMetadata {
            prompt_token_count: Some(120),
            candidates_token_count: Some(40),
            thoughts_token_count: Some(30),
            cached_content_token_count: Some(20),
            tool_use_prompt_token_count: None,
            total_token_count: None,
        });

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 40);
        assert_eq!(usage.reasoning_tokens, 30);
        assert_eq!(usage.cache_read_tokens, 20);
    }

    #[test]
    fn thought_signatures_are_keyed_by_exact_function_call_id() {
        let content = GeminiContent {
            role: Some("model".to_owned()),
            parts: vec![GeminiPart::function_call(
                Some("call_exact".to_owned()),
                "lookup",
                serde_json::json!({}),
                Some("signed-thought".to_owned()),
            )],
        };
        let signatures =
            gemini_thought_signatures_from_content(&content).expect("thought signature metadata");

        assert_eq!(signatures["call_exact"], "signed-thought");
    }

    #[test]
    fn thought_signatures_remain_attached_to_the_exact_returned_part() {
        let content = GeminiContent {
            role: Some("model".to_owned()),
            parts: vec![
                GeminiPart::function_call(
                    Some("first".to_owned()),
                    "lookup",
                    serde_json::json!({}),
                    Some("first-signature".to_owned()),
                ),
                GeminiPart::function_call(
                    Some("second".to_owned()),
                    "lookup",
                    serde_json::json!({}),
                    None,
                ),
                GeminiPart {
                    text: Some("done".to_owned()),
                    thought_signature: Some("final-signature".to_owned()),
                    ..GeminiPart::default()
                },
            ],
        };
        let signatures =
            gemini_thought_signatures_from_content(&content).expect("thought signatures");

        assert_eq!(signatures["first"], "first-signature");
        assert!(signatures.get("second").is_none());
        assert_eq!(
            signatures[GEMINI_FINAL_PART_SIGNATURE_KEY],
            "final-signature"
        );
    }

    #[test]
    fn streaming_function_calls_in_separate_chunks_are_all_emitted() {
        fn chunk(name: &str, signature: &str) -> GeminiGenerateResponse {
            GeminiGenerateResponse {
                candidates: vec![GeminiCandidate {
                    content: Some(GeminiContent {
                        role: Some("model".to_owned()),
                        parts: vec![GeminiPart::function_call(
                            None,
                            name,
                            serde_json::json!({}),
                            Some(signature.to_owned()),
                        )],
                    }),
                    finish_reason: None,
                    safety_ratings: None,
                    grounding_metadata: None,
                }],
                usage_metadata: None,
            }
        }

        let mut first = chunk("tool_a", "signature-a");
        let mut second = chunk("tool_b", "signature-b");
        let mut next_tool_call_index = 0;
        ensure_gemini_stream_function_call_ids(&mut first, &mut next_tool_call_index);
        ensure_gemini_stream_function_call_ids(&mut second, &mut next_tool_call_index);

        assert_eq!(
            first.candidates[0].content.as_ref().unwrap().parts[0]
                .function_call
                .as_ref()
                .unwrap()
                .id
                .as_deref(),
            Some("tool_a-0")
        );
        assert_eq!(
            second.candidates[0].content.as_ref().unwrap().parts[0]
                .function_call
                .as_ref()
                .unwrap()
                .id
                .as_deref(),
            Some("tool_b-1")
        );
        assert_eq!(
            gemini_candidate_provider_metadata(&first.candidates[0]).unwrap()["gemini_thought_signatures"]
                ["tool_a-0"],
            "signature-a"
        );

        let mut emitted_tool_calls = HashSet::new();
        let first_events = GeminiStreamEvent::from_chunk(first, &mut emitted_tool_calls)
            .expect("valid first chunk");
        let second_events = GeminiStreamEvent::from_chunk(second, &mut emitted_tool_calls)
            .expect("valid second chunk");

        assert!(matches!(
            first_events.as_slice(),
            [GeminiStreamEvent::ToolCall(_)]
        ));
        assert!(matches!(
            second_events.as_slice(),
            [GeminiStreamEvent::ToolCall(_)]
        ));
    }

    #[test]
    fn streaming_function_calls_with_the_same_provider_id_are_not_reemitted() {
        let make_chunk = || GeminiGenerateResponse {
            candidates: vec![GeminiCandidate {
                content: Some(GeminiContent {
                    role: Some("model".to_owned()),
                    parts: vec![GeminiPart::function_call(
                        Some("provider-call-id".to_owned()),
                        "lookup",
                        serde_json::json!({}),
                        None,
                    )],
                }),
                finish_reason: None,
                safety_ratings: None,
                grounding_metadata: None,
            }],
            usage_metadata: None,
        };
        let mut emitted_tool_calls = HashSet::new();

        let first = GeminiStreamEvent::from_chunk(make_chunk(), &mut emitted_tool_calls)
            .expect("valid first chunk");
        let duplicate = GeminiStreamEvent::from_chunk(make_chunk(), &mut emitted_tool_calls)
            .expect("valid duplicate chunk");

        assert!(matches!(first.as_slice(), [GeminiStreamEvent::ToolCall(_)]));
        assert!(duplicate.is_empty());
    }

    #[test]
    fn streaming_text_chunks_are_treated_as_deltas_even_when_they_repeat_or_share_prefixes() {
        fn chunk(text: &str, thought: bool) -> GeminiGenerateResponse {
            GeminiGenerateResponse {
                candidates: vec![GeminiCandidate {
                    content: Some(GeminiContent {
                        role: Some("model".to_owned()),
                        parts: vec![GeminiPart {
                            text: Some(text.to_owned()),
                            thought: Some(thought),
                            ..GeminiPart::default()
                        }],
                    }),
                    finish_reason: None,
                    safety_ratings: None,
                    grounding_metadata: None,
                }],
                usage_metadata: None,
            }
        }

        let mut emitted_tool_calls = HashSet::new();
        let first = GeminiStreamEvent::from_chunk(chunk("Echo", false), &mut emitted_tool_calls)
            .expect("valid text chunk");
        let repeated = GeminiStreamEvent::from_chunk(chunk("Echo", false), &mut emitted_tool_calls)
            .expect("valid repeated chunk");
        let prefixed =
            GeminiStreamEvent::from_chunk(chunk("Echo again", false), &mut emitted_tool_calls)
                .expect("valid prefixed chunk");
        let thought = GeminiStreamEvent::from_chunk(chunk("Think", true), &mut emitted_tool_calls)
            .expect("valid thought chunk");

        assert!(
            matches!(first.as_slice(), [GeminiStreamEvent::TextDelta(value)] if value == "Echo")
        );
        assert!(
            matches!(repeated.as_slice(), [GeminiStreamEvent::TextDelta(value)] if value == "Echo")
        );
        assert!(
            matches!(prefixed.as_slice(), [GeminiStreamEvent::TextDelta(value)] if value == "Echo again")
        );
        assert!(
            matches!(thought.as_slice(), [GeminiStreamEvent::ThinkingDelta(value)] if value == "Think")
        );
    }
}
