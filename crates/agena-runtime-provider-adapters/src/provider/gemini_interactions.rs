//! Adapter for Google's Gemini Interactions API (`POST <base_url>/interactions`).

use std::collections::{BTreeMap, HashMap};

use agena_domain::{AttachmentItem, AttachmentKind, Model, ModelId, ProviderId, Role};
use agena_provider::{
    CapabilityFamily, CompletionFinishReason, CompletionInputRun, CompletionInputToolResultStatus,
    CompletionRequest, CompletionStreamEvent, CompletionUsage, GEMINI_SKIP_THOUGHT_SIGNATURE,
    GeminiInteractionsContent, GeminiInteractionsFunctionResultPayload,
    GeminiInteractionsGenerationConfig, GeminiInteractionsRequest, GeminiInteractionsStep,
    GeminiInteractionsTool, GeminiInteractionsUsage, GeminiModelListResponse, PromptCacheShape,
    classify_google_interactions_error, gemini_interactions_finish_reason,
    gemini_interactions_thinking_level, gemini_interactions_usage_to_completion,
};
use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    ProviderError,
    provider::{
        CompletionResponse, ManagedCredential, ModelRuntime, should_retry_credential, sse, utils,
        wire_message,
    },
};

const ADAPTER_KIND: &str = "gemini_interactions";
const GEMINI_FINAL_PART_SIGNATURE_KEY: &str = "$final_part";

#[derive(Clone, Default)]
/// Options for [`GeminiInteractionsAdapter`].
pub struct GeminiInteractionsAdapterOptions {
    pub client_identity: crate::ProviderClientIdentity,
    pub auth_header: Option<(String, Option<String>)>,
    pub extra_headers: HashMap<String, String>,
}

#[derive(Clone)]
/// Provider adapter speaking Google's stateless Gemini Interactions API (`store: false`, SSE).
pub struct GeminiInteractionsAdapter {
    provider_id: String,
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    auth_header: (String, Option<String>),
    extra_headers: HashMap<String, String>,
}

impl GeminiInteractionsAdapter {
    pub fn new_managed_with_options(
        provider_id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        options: GeminiInteractionsAdapterOptions,
    ) -> Self {
        let user_agent = options.client_identity.agena_user_agent();
        let mut extra_headers =
            HashMap::from([(reqwest::header::USER_AGENT.as_str().to_owned(), user_agent)]);
        if options
            .extra_headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()))
        {
            extra_headers
                .retain(|key, _| !key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()));
        }
        extra_headers.extend(options.extra_headers);

        let auth_header = options
            .auth_header
            .unwrap_or_else(|| ("x-goog-api-key".to_owned(), None));

        Self {
            provider_id: provider_id.into(),
            client,
            api_key: api_key.with_client_identity(options.client_identity),
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model.into()),
            auth_header,
            extra_headers,
        }
    }

    fn interactions_endpoint(&self) -> String {
        format!("{}/interactions", self.base_url)
    }

    fn list_models_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    async fn send_request<F>(&self, mut build: F) -> Result<reqwest::Response, ProviderError>
    where
        F: FnMut(&str) -> reqwest::RequestBuilder,
    {
        let mut force_refresh = false;

        loop {
            let secret = if force_refresh {
                self.api_key.force_refresh().await?
            } else {
                self.api_key.resolve().await?
            };

            let response =
                utils::send_request_builder(self.provider_id.as_str(), build(secret.as_str()))
                    .await?;
            if !force_refresh && should_retry_credential(response.status()) {
                force_refresh = true;
                continue;
            }

            return Ok(response);
        }
    }

    fn build_interactions_request(
        request: &CompletionRequest,
    ) -> Result<GeminiInteractionsRequest, ProviderError> {
        if !request.provider_native_tools.bindings().is_empty() {
            return Err(ProviderError::Config(
                "gemini_interactions adapter does not currently support provider-hosted native tools; disable provider-hosted tools for this model".to_owned(),
            ));
        }

        let model = request
            .model
            .as_ref()
            .trim_start_matches("models/")
            .to_owned();

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            system_chunks.push(system.clone());
        }

        let mut input = Vec::new();
        for run in &request.turns {
            match run.role {
                Role::System => {
                    let text = wire_message::project_text_lossy(run);
                    if !text.trim().is_empty() {
                        system_chunks.push(text);
                    }
                }
                Role::User => push_user_steps(run, &mut input),
                Role::Assistant => push_assistant_steps(run, &mut input),
                Role::Tool => push_tool_steps(run, &mut input),
            }
        }

        let tools = request
            .tool_api_functions
            .iter()
            .map(|tool| GeminiInteractionsTool {
                tool_type: "function",
                name: tool.name.clone(),
                description: (!tool.description.trim().is_empty())
                    .then(|| tool.description.clone()),
                parameters: Some(tool.input_schema.clone()),
            })
            .collect::<Vec<_>>();

        let thinking_level =
            gemini_interactions_thinking_level(model.as_str(), request.thinking.as_ref())
                .map(str::to_owned);
        let thinking_summaries = thinking_level.as_ref().map(|_| "auto".to_owned());
        let gen_config = GeminiInteractionsGenerationConfig {
            thinking_level,
            thinking_summaries,
            max_output_tokens: request.max_output_tokens,
            tool_choice: None,
        };
        let generation_config = (!gen_config.is_empty()).then_some(gen_config);

        Ok(GeminiInteractionsRequest {
            model,
            store: false,
            stream: true,
            system_instruction: (!system_chunks.is_empty()).then(|| system_chunks.join("\n\n")),
            tools,
            generation_config,
            input,
        })
    }

    async fn http_error_from_response(
        &self,
        response: reqwest::Response,
        model_id: &str,
    ) -> ProviderError {
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let body = utils::response_text_bounded(
            response,
            utils::MAX_PROVIDER_ERROR_RESPONSE_BYTES,
            "Google Interactions error response",
        )
        .await
        .unwrap_or_default();
        let classified = classify_google_interactions_error(
            status.as_u16(),
            retry_after.as_deref(),
            body.as_str(),
            model_id,
        );
        ProviderError::HttpStatus {
            provider: self.provider_id.clone(),
            status,
            body: classified.message,
            kind: classified.kind,
            retryable: classified.retryable,
        }
    }
}

#[async_trait]
impl ModelRuntime for GeminiInteractionsAdapter {
    fn id(&self) -> &str {
        self.provider_id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<CapabilityFamily> {
        Some(CapabilityFamily::Gemini)
    }

    fn validate_media_inputs_for_adapter(
        &self,
        _adapter_id: Option<&agena_domain::AdapterId>,
        _model: &ModelId,
        attachments: &[AttachmentItem],
    ) -> Result<(), ProviderError> {
        use agena_runtime_provider::provider::media_input::{self, MediaProtocol};
        media_input::validate(MediaProtocol::Gemini, attachments)
    }

    fn validate_provider_native_tools_request(
        &self,
        _adapter_id: Option<&agena_domain::AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), ProviderError> {
        if !request.provider_native_tools.bindings().is_empty() {
            return Err(ProviderError::Config(
                "gemini_interactions adapter does not currently support provider-hosted native tools; disable provider-hosted tools for this model".to_owned(),
            ));
        }
        Ok(())
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<PromptCacheShape> {
        let mut fields = vec![
            ("auth_scope", self.api_key.prompt_cache_scope()),
            ("base_url", self.base_url.clone()),
            ("default_model", self.default_model.to_string()),
            ("auth_header", self.auth_header.0.clone()),
            (
                "extra_headers",
                PromptCacheShape::json_field_value(&utils::prompt_cache_header_entries(
                    &self.extra_headers,
                )),
            ),
        ];
        if let Some(scheme) = self.auth_header.1.as_deref() {
            fields.push(("auth_scheme", scheme.to_owned()));
        }
        Some(PromptCacheShape::from_fields(
            self.provider_id.as_str(),
            fields,
        ))
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        let mut endpoint = self.list_models_endpoint();
        let mut pagination = crate::provider::model_pagination::ModelPagination::new(&endpoint)?;
        let mut models = Vec::new();
        loop {
            let response = self
                .send_request(|secret| {
                    let mut headers = utils::configured_request_headers(&self.extra_headers);
                    headers.insert(
                        self.auth_header.0.clone(),
                        utils::auth_header_value(self.auth_header.1.as_deref(), secret),
                    );
                    utils::adapter_log_http_request_prepared(
                        self.provider_id.as_str(),
                        ADAPTER_KIND,
                        "list_models",
                        "GET",
                        endpoint.as_str(),
                        headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                        None,
                    );
                    utils::apply_resolved_request_headers(self.client.get(&endpoint), &headers)
                })
                .await?;

            let page: Value = utils::parse_json_response_logged(
                self.provider_id.as_str(),
                ADAPTER_KIND,
                "list_models",
                response,
            )
            .await?;
            let next = pagination.next(&page)?;
            let payload: GeminiModelListResponse = serde_json::from_value(page)
                .map_err(|_| ProviderError::Provider("invalid model list response".into()))?;
            models.extend(payload.models.into_iter().map(|m| {
                let metadata = m.metadata();
                let id = m.name.trim_start_matches("models/").to_owned();
                let model_id = ModelId::new(id);
                let capabilities = self.model_capabilities(&model_id);
                Model {
                    provider_id: ProviderId::new(self.provider_id.clone()),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name: m.display_name,
                    native_compaction: true,
                    capabilities,
                    metadata,
                    thinking_modes: Vec::new(),
                    speed_modes: BTreeMap::new(),
                }
            }));
            let Some(next) = next else { break };
            endpoint = next;
        }
        Ok(models)
    }

    #[tracing::instrument(skip_all, fields(provider = %self.provider_id, model = %request.model))]
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let fallback_model = request.model.clone();
        let stream = self.complete_stream(request).await?;
        utils::aggregate_stream(self.provider_id.as_str(), fallback_model, stream).await
    }

    #[tracing::instrument(skip_all, fields(provider = %self.provider_id, model = %request.model))]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let model = request.model.clone();
        let adapter = self.clone();
        let (body_json, request_headers) = utils::prepare_request_with(move || {
            let body = Self::build_interactions_request(&request)?;
            let body_json =
                utils::prepare_request_body_sync(&body, &request.request_override.body_patch)?;
            let request_headers = utils::merged_request_headers(
                &adapter.extra_headers,
                &request.request_override.headers,
            );
            Ok((body_json, request_headers))
        })
        .await?;

        let endpoint = self.interactions_endpoint();
        let response = self
            .send_request(|secret| {
                let mut headers = utils::configured_request_headers(&request_headers);
                headers.insert(
                    self.auth_header.0.clone(),
                    utils::auth_header_value(self.auth_header.1.as_deref(), secret),
                );
                headers.insert(
                    reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                    "application/json".to_owned(),
                );
                headers.insert(
                    reqwest::header::ACCEPT.as_str().to_owned(),
                    "text/event-stream".to_owned(),
                );
                utils::adapter_log_http_request_prepared(
                    self.provider_id.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.interactions",
                    "POST",
                    endpoint.as_str(),
                    headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                    Some(&body_json),
                );
                utils::apply_resolved_request_headers(self.client.post(&endpoint), &headers)
                    .body(body_json.bytes())
            })
            .await?;

        if !response.status().is_success() {
            return Err(self
                .http_error_from_response(response, model.as_ref())
                .await);
        }

        utils::adapter_log_http_response_open(
            self.provider_id.as_str(),
            ADAPTER_KIND,
            "complete_stream.interactions",
            response.status(),
            response.headers(),
        );

        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(self.provider_id.clone());
        let provider_id_str = self.provider_id.clone();
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut active_steps: HashMap<usize, ActiveStep> = HashMap::new();
            let mut thought_signatures = serde_json::Map::new();
            let mut latest_thought_signature: Option<String> = None;
            let mut response_id: Option<String> = None;
            let mut usage: Option<CompletionUsage> = None;
            let mut final_status: Option<String> = None;
            let mut tool_call_seen = false;
            let mut saw_content = false;

            while let Some(event) = events.next().await {
                let event = event.map_err(|err| utils::json_stream_error(provider_id_str.as_str(), err))?;
                utils::adapter_log_stream_event(
                    provider_id_str.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.interactions",
                    &event,
                );

                let parsed: InteractionsSseEvent =
                    utils::parse_json_value(provider_id_str.as_str(), "interactions sse event", event)?;

                match parsed.event_type.as_str() {
                    "interaction.start" => {
                        if let Some(id) = parsed.interaction.and_then(|item| item.id)
                            && !id.trim().is_empty()
                        {
                            response_id = Some(id);
                        }
                    }
                    "step.start" | "content.start" => {
                        let Some(index) = parsed.index else {
                            continue;
                        };
                        let Some(step) = parsed.step else {
                            continue;
                        };
                        match step.step_type.as_str() {
                            "thought" => {
                                let signature = step.signature.unwrap_or_default();
                                if !signature.trim().is_empty() {
                                    latest_thought_signature = Some(signature.clone());
                                }
                                active_steps.insert(index, ActiveStep::Thought { signature });
                            }
                            "model_output" | "text" => {
                                active_steps.insert(index, ActiveStep::ModelOutput);
                            }
                            "function_call" => {
                                let call_id = utils::normalize_optional_text(step.id)
                                    .unwrap_or_else(|| format!("call-{index}"));
                                let call_name = step.name.unwrap_or_default();
                                if call_name.trim().is_empty() {
                                    Err(ProviderError::Provider(
                                        "Google Interactions function_call step is missing `name`".to_owned(),
                                    ))?;
                                }
                                if let Some(sig) = latest_thought_signature
                                    .as_deref()
                                    .filter(|sig| !sig.trim().is_empty())
                                {
                                    thought_signatures.insert(
                                        call_id.clone(),
                                        Value::String(sig.to_owned()),
                                    );
                                }
                                let initial_arguments = step
                                    .arguments
                                    .and_then(|args| serde_json::to_string(&args).ok());
                                tool_call_seen = true;
                                saw_content = true;
                                active_steps.insert(
                                    index,
                                    ActiveStep::FunctionCall {
                                        id: call_id.clone(),
                                        emitted_any_delta: false,
                                        initial_arguments,
                                    },
                                );
                                yield CompletionStreamEvent::ToolCallDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    stream_key: call_id.clone(),
                                    id: Some(call_id),
                                    name: Some(call_name),
                                    arguments_delta: String::new(),
                                };
                            }
                            _ => {}
                        }
                    }
                    "step.delta" | "content.delta" => {
                        let Some(index) = parsed.index else {
                            continue;
                        };
                        let Some(delta) = parsed.delta else {
                            continue;
                        };
                        let Some(active) = active_steps.get_mut(&index) else {
                            continue;
                        };
                        match (delta.delta_type.as_str(), active) {
                            ("text", ActiveStep::ModelOutput) => {
                                if let Some(text) = delta.text
                                    && !text.is_empty()
                                {
                                    saw_content = true;
                                    yield CompletionStreamEvent::TextDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        delta: text,
                                    };
                                }
                            }
                            ("thought_summary", ActiveStep::Thought { .. }) => {
                                let text = delta
                                    .content
                                    .filter(|c| c.content_type.as_deref().is_none_or(|t| t == "text"))
                                    .and_then(|c| c.text)
                                    .or(delta.text)
                                    .unwrap_or_default();
                                if !text.is_empty() {
                                    saw_content = true;
                                    yield CompletionStreamEvent::ThinkingDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        delta: text,
                                    };
                                }
                            }
                            ("thought_signature", ActiveStep::Thought { signature }) => {
                                if let Some(chunk) = delta.signature
                                    && !chunk.is_empty()
                                {
                                    signature.push_str(chunk.as_str());
                                    if !signature.trim().is_empty() {
                                        latest_thought_signature = Some(signature.clone());
                                    }
                                }
                            }
                            (
                                "arguments_delta",
                                ActiveStep::FunctionCall {
                                    id,
                                    emitted_any_delta,
                                    ..
                                },
                            ) => {
                                let fragment = delta
                                    .arguments
                                    .and_then(|v| match v {
                                        Value::String(s) => Some(s),
                                        other if !other.is_null() => serde_json::to_string(&other).ok(),
                                        _ => None,
                                    })
                                    .or(delta.arguments_delta);
                                if let Some(fragment) = fragment
                                    && !fragment.is_empty()
                                {
                                    *emitted_any_delta = true;
                                    saw_content = true;
                                    yield CompletionStreamEvent::ToolCallDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        stream_key: id.clone(),
                                        id: None,
                                        name: None,
                                        arguments_delta: fragment,
                                    };
                                }
                            }
                            _ => {}
                        }
                    }
                    "step.stop" | "content.stop" => {
                        let Some(index) = parsed.index else {
                            continue;
                        };
                        if let Some(step) = active_steps.remove(&index) {
                            match step {
                                ActiveStep::Thought { signature } => {
                                    if !signature.trim().is_empty() {
                                        latest_thought_signature = Some(signature);
                                    }
                                }
                                ActiveStep::FunctionCall {
                                    id,
                                    emitted_any_delta,
                                    initial_arguments,
                                } => {
                                    if !emitted_any_delta {
                                        let fallback_args = initial_arguments
                                            .filter(|s| !s.trim().is_empty())
                                            .unwrap_or_else(|| "{}".to_owned());
                                        yield CompletionStreamEvent::ToolCallDelta {
                                            provider_id: provider_id.clone(),
                                            model: model_name.clone(),
                                            stream_key: id,
                                            id: None,
                                            name: None,
                                            arguments_delta: fallback_args,
                                        };
                                    }
                                }
                                ActiveStep::ModelOutput => {}
                            }
                        }
                    }
                    "interaction.completed" | "interaction.complete" => {
                        if let Some(interaction) = parsed.interaction {
                            if response_id.is_none()
                                && let Some(id) = interaction.id
                                && !id.trim().is_empty()
                            {
                                response_id = Some(id);
                            }
                            if let Some(wire_usage) = interaction.usage {
                                usage = Some(gemini_interactions_usage_to_completion(wire_usage));
                            }
                            if let Some(status) = interaction.status {
                                final_status = Some(status);
                            }
                        } else {
                            if let Some(wire_usage) = parsed.usage {
                                usage = Some(gemini_interactions_usage_to_completion(wire_usage));
                            }
                            if let Some(status) = parsed.status {
                                final_status = Some(status);
                            }
                        }
                        break;
                    }
                    "error" => {
                        let message = parsed
                            .error
                            .and_then(|err| err.message)
                            .or(parsed.message)
                            .filter(|msg| !msg.trim().is_empty())
                            .unwrap_or_else(|| "Google stream error".to_owned());
                        Err(ProviderError::Provider(message))?;
                    }
                    _ => {}
                }
            }

            let Some(status) = final_status else {
                Err(ProviderError::Provider(format!(
                    "Google stream ended before interaction.completed (received_content={saw_content}); partial output is not a completed reply"
                )))?;
                unreachable!();
            };

            let Some(mapped_finish_reason) = gemini_interactions_finish_reason(status.as_str()) else {
                Err(ProviderError::Provider(format!(
                    "Google interaction ended with status `{status}`"
                )))?;
                unreachable!();
            };

            if let Some(sig) = latest_thought_signature.filter(|s| !s.trim().is_empty()) {
                thought_signatures.insert(
                    GEMINI_FINAL_PART_SIGNATURE_KEY.to_owned(),
                    Value::String(sig),
                );
            }

            let mut metadata = serde_json::Map::new();
            if !thought_signatures.is_empty() {
                metadata.insert(
                    "gemini_thought_signatures".to_owned(),
                    Value::Object(thought_signatures),
                );
            }
            if let Some(id) = response_id {
                metadata.insert("interaction_id".to_owned(), Value::String(id));
            }
            let provider_metadata = (!metadata.is_empty()).then_some(Value::Object(metadata));

            yield CompletionStreamEvent::Completed {
                provider_id,
                model: model_name,
                finish_reason: CompletionFinishReason::normalize_with_tool_calls(
                    Some(mapped_finish_reason),
                    tool_call_seen,
                ),
                usage,
                provider_metadata,
                end_turn: None,
            };
        };

        Ok(Box::pin(stream))
    }
}

enum ActiveStep {
    Thought {
        signature: String,
    },
    ModelOutput,
    FunctionCall {
        id: String,
        emitted_any_delta: bool,
        initial_arguments: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct InteractionsSseEvent {
    #[serde(default)]
    event_type: String,
    #[serde(default, alias = "step_index")]
    index: Option<usize>,
    #[serde(default, alias = "content")]
    step: Option<InteractionsWireStepStart>,
    #[serde(default)]
    delta: Option<InteractionsWireDelta>,
    #[serde(default)]
    interaction: Option<InteractionsWireInteraction>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    usage: Option<GeminiInteractionsUsage>,
    #[serde(default)]
    error: Option<InteractionsWireError>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InteractionsWireStepStart {
    #[serde(rename = "type", default)]
    step_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<Value>,
    #[serde(default)]
    signature: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InteractionsWireDelta {
    #[serde(rename = "type", default)]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    content: Option<InteractionsWireDeltaContent>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    arguments: Option<Value>,
    #[serde(default)]
    arguments_delta: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InteractionsWireDeltaContent {
    #[serde(rename = "type", default)]
    content_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InteractionsWireInteraction {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    usage: Option<GeminiInteractionsUsage>,
}

#[derive(Debug, Deserialize)]
struct InteractionsWireError {
    #[serde(default)]
    message: Option<String>,
}

fn push_user_steps(run: &CompletionInputRun, steps: &mut Vec<GeminiInteractionsStep>) {
    let projected = wire_message::project(run);
    let mut content = Vec::new();
    for part in &projected {
        match part {
            wire_message::WirePart::Text { text }
            | wire_message::WirePart::SystemMessage { text }
            | wire_message::WirePart::Reasoning { text } => {
                push_text_content(&mut content, text);
            }
            wire_message::WirePart::Attachment { item } => {
                content.push(attachment_to_interactions_content(item));
            }
            wire_message::WirePart::ToolResult {
                tool_call_id,
                function,
                status,
                output_json,
                attachments,
                ..
            } => {
                flush_user_content(steps, &mut content);
                steps.push(build_function_result_step(
                    tool_call_id,
                    function.function_name(),
                    *status,
                    output_json,
                    attachments,
                ));
            }
            wire_message::WirePart::ToolCall { .. } => {}
        }
    }
    if content.is_empty() && projected.is_empty() {
        let fallback = run.as_text_lossy();
        push_text_content(&mut content, fallback.as_str());
    }
    flush_user_content(steps, &mut content);
}

fn push_assistant_steps(run: &CompletionInputRun, steps: &mut Vec<GeminiInteractionsStep>) {
    let projected = wire_message::project(run);
    if projected.is_empty() {
        let fallback = run.as_text_lossy();
        if !fallback.trim().is_empty() {
            push_model_output(steps, fallback);
        }
        return;
    }

    let mut segment = Vec::new();
    for part in &projected {
        match part {
            wire_message::WirePart::ToolResult {
                tool_call_id,
                function,
                status,
                output_json,
                attachments,
                ..
            } => {
                flush_assistant_segment(run, &segment, steps);
                segment.clear();
                steps.push(build_function_result_step(
                    tool_call_id,
                    function.function_name(),
                    *status,
                    output_json,
                    attachments,
                ));
            }
            other => segment.push(other),
        }
    }
    flush_assistant_segment(run, &segment, steps);
}

fn flush_assistant_segment(
    run: &CompletionInputRun,
    parts: &[&wire_message::WirePart],
    steps: &mut Vec<GeminiInteractionsStep>,
) {
    if parts.is_empty() {
        return;
    }
    let signatures = &run.provider_state.gemini_thought_signatures;
    let has_tool_calls = parts
        .iter()
        .any(|part| matches!(part, wire_message::WirePart::ToolCall { .. }));
    let final_sig = signatures
        .get(GEMINI_FINAL_PART_SIGNATURE_KEY)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let mut pending_reasoning = String::new();
    let mut emitted_signed_thought = false;
    let mut last_emitted_signature: Option<String> = None;

    for part in parts {
        match part {
            wire_message::WirePart::Reasoning { text } => {
                if !text.is_empty() {
                    if !pending_reasoning.is_empty() {
                        pending_reasoning.push('\n');
                    }
                    pending_reasoning.push_str(text);
                }
            }
            wire_message::WirePart::Text { text }
            | wire_message::WirePart::SystemMessage { text } => {
                if !has_tool_calls && (!pending_reasoning.is_empty() || final_sig.is_some()) {
                    let sig = if !emitted_signed_thought {
                        final_sig.clone()
                    } else {
                        None
                    };
                    let summary = take_reasoning_summary(&mut pending_reasoning);
                    if sig.is_some() || summary.is_some() {
                        if let Some(s) = sig.as_ref() {
                            emitted_signed_thought = true;
                            last_emitted_signature = Some(s.clone());
                        }
                        steps.push(GeminiInteractionsStep::Thought {
                            signature: sig,
                            summary,
                        });
                    }
                }
                if !text.is_empty() {
                    push_model_output(steps, text.clone());
                }
            }
            wire_message::WirePart::Attachment { item } => {
                push_model_output(steps, wire_message::hint_text(item));
            }
            wire_message::WirePart::ToolCall {
                id,
                function,
                arguments_json,
            } => {
                let call_sig = signatures
                    .get(id)
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .or_else(|| final_sig.clone());

                match call_sig {
                    Some(sig) if last_emitted_signature.as_deref() != Some(sig.as_str()) => {
                        let summary = take_reasoning_summary(&mut pending_reasoning);
                        steps.push(GeminiInteractionsStep::Thought {
                            signature: Some(sig.clone()),
                            summary,
                        });
                        emitted_signed_thought = true;
                        last_emitted_signature = Some(sig);
                    }
                    None if !emitted_signed_thought => {
                        let summary = take_reasoning_summary(&mut pending_reasoning);
                        steps.push(GeminiInteractionsStep::Thought {
                            signature: Some(GEMINI_SKIP_THOUGHT_SIGNATURE.to_owned()),
                            summary,
                        });
                        emitted_signed_thought = true;
                    }
                    _ => {}
                }

                steps.push(GeminiInteractionsStep::FunctionCall {
                    id: id.clone(),
                    name: function.function_name().to_owned(),
                    arguments: parse_json_or_object(arguments_json.as_str()),
                });
            }
            wire_message::WirePart::ToolResult { .. } => {}
        }
    }

    if !pending_reasoning.is_empty() || (!emitted_signed_thought && final_sig.is_some()) {
        let sig = if !emitted_signed_thought {
            final_sig
        } else {
            None
        };
        let summary = take_reasoning_summary(&mut pending_reasoning);
        if sig.is_some() || summary.is_some() {
            steps.push(GeminiInteractionsStep::Thought {
                signature: sig,
                summary,
            });
        }
    }
}

fn push_tool_steps(run: &CompletionInputRun, steps: &mut Vec<GeminiInteractionsStep>) {
    for part in wire_message::project(run) {
        if let wire_message::WirePart::ToolResult {
            tool_call_id,
            function,
            status,
            output_json,
            attachments,
            ..
        } = part
        {
            steps.push(build_function_result_step(
                tool_call_id.as_str(),
                function.function_name(),
                status,
                output_json.as_str(),
                attachments.as_slice(),
            ));
        }
    }
}

fn build_function_result_step(
    call_id: &str,
    function_name: &str,
    status: CompletionInputToolResultStatus,
    output_json: &str,
    attachments: &[AttachmentItem],
) -> GeminiInteractionsStep {
    let name = {
        let trimmed = function_name.trim();
        if trimmed.is_empty() {
            "tool_result".to_owned()
        } else {
            trimmed.to_owned()
        }
    };
    let is_error = status != CompletionInputToolResultStatus::Completed;

    let mut images = Vec::new();
    let mut text_parts = Vec::new();
    if !output_json.is_empty() {
        text_parts.push(output_json.to_owned());
    }
    for item in attachments {
        if let Some(text) = wire_message::attachment_text(item) {
            text_parts.push(text);
        } else if item.kind == AttachmentKind::Image
            && let Some((mime_type, data)) = wire_message::base64_with_mime(item)
        {
            images.push(GeminiInteractionsContent::Image { mime_type, data });
        } else {
            text_parts.push(wire_message::hint_text(item));
        }
    }

    let combined_text = text_parts.join("\n");
    let result = if images.is_empty() {
        GeminiInteractionsFunctionResultPayload::Text(combined_text)
    } else {
        let mut items = Vec::with_capacity(images.len() + 1);
        if !combined_text.is_empty() {
            items.push(GeminiInteractionsContent::Text {
                text: combined_text,
            });
        }
        items.extend(images);
        GeminiInteractionsFunctionResultPayload::Parts(items)
    };

    GeminiInteractionsStep::FunctionResult {
        call_id: call_id.to_owned(),
        name,
        result,
        is_error,
    }
}

fn attachment_to_interactions_content(item: &AttachmentItem) -> GeminiInteractionsContent {
    if let Some(text) = wire_message::attachment_text(item) {
        return GeminiInteractionsContent::Text { text };
    }
    if item.kind == AttachmentKind::Image
        && let Some((mime_type, data)) = wire_message::base64_with_mime(item)
    {
        return GeminiInteractionsContent::Image { mime_type, data };
    }
    GeminiInteractionsContent::Text {
        text: wire_message::hint_text(item),
    }
}

fn push_text_content(content: &mut Vec<GeminiInteractionsContent>, text: &str) {
    if text.is_empty() {
        return;
    }
    content.push(GeminiInteractionsContent::Text {
        text: text.to_owned(),
    });
}

fn flush_user_content(
    steps: &mut Vec<GeminiInteractionsStep>,
    content: &mut Vec<GeminiInteractionsContent>,
) {
    if content.is_empty() {
        return;
    }
    if let Some(GeminiInteractionsStep::UserInput {
        content: previous_content,
    }) = steps.last_mut()
    {
        previous_content.append(content);
        return;
    }
    steps.push(GeminiInteractionsStep::UserInput {
        content: std::mem::take(content),
    });
}

fn push_model_output(steps: &mut Vec<GeminiInteractionsStep>, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(GeminiInteractionsStep::ModelOutput { content }) = steps.last_mut() {
        if let Some(GeminiInteractionsContent::Text { text: prev }) = content.last_mut() {
            prev.push_str(text.as_str());
        } else {
            content.push(GeminiInteractionsContent::Text { text });
        }
        return;
    }
    steps.push(GeminiInteractionsStep::ModelOutput {
        content: vec![GeminiInteractionsContent::Text { text }],
    });
}

fn take_reasoning_summary(pending: &mut String) -> Option<Vec<GeminiInteractionsContent>> {
    let text = std::mem::take(pending);
    (!text.trim().is_empty()).then(|| vec![GeminiInteractionsContent::Text { text }])
}

fn parse_json_or_object(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Object(Default::default());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(value @ Value::Object(_)) => value,
        _ => Value::Object(Default::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{CapabilitySupport, ReasoningEffort, ThinkingRequest, ToolApiFunction};
    use agena_provider::{
        CompletionInputPart, CompletionInputProviderState, CompletionToolCall, ModelToolFunction,
        ProviderErrorKind,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_mock_server(
        status_line: &'static str,
        headers: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let header_end = loop {
                let n = socket.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break buf.len();
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos;
                }
            };
            let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            let body_start = (header_end + 4).min(buf.len());
            while buf.len() < body_start + content_length {
                let n = socket.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let captured = String::from_utf8_lossy(&buf).into_owned();
            let response = format!(
                "{status_line}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            captured
        });
        (format!("http://{addr}"), handle)
    }

    fn base_request(model: &str) -> CompletionRequest {
        CompletionRequest {
            model: ModelId::new(model),
            system: Some("Be concise.".to_owned()),
            turns: vec![CompletionInputRun {
                role: Role::User,
                parts: vec![CompletionInputPart::Text {
                    text: "Hello".to_owned(),
                }],
                provider_state: Default::default(),
            }],
            tool_api_functions: Vec::new(),
            provider_native_tools: Default::default(),
            disable_tools: false,
            temperature: None,
            max_output_tokens: Some(1024),
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,
            provider_compaction: None,
            stop_sequences: Vec::new(),
            top_p: None,
            top_k: None,
            seed: None,
            thinking: Some(ThinkingRequest::Effort {
                effort: ReasoningEffort::High,
            }),
            verbosity: None,
            model_supports_parallel_tool_calls: None,
            response_format: None,
            responses_api_metadata: None,
            request_override: Default::default(),
        }
    }

    #[tokio::test]
    async fn streams_thinking_text_tool_call_thought_signature_and_usage() {
        let sse_body = concat!(
            "data: {\"event_type\":\"interaction.start\",\"interaction\":{\"id\":\"int_1\"}}\n\n",
            "data: {\"event_type\":\"step.start\",\"index\":0,\"step\":{\"type\":\"thought\"}}\n\n",
            "data: {\"event_type\":\"step.delta\",\"index\":0,\"delta\":{\"type\":\"thought_summary\",\"content\":{\"type\":\"text\",\"text\":\"Checking files\"}}}\n\n",
            "data: {\"event_type\":\"step.delta\",\"index\":0,\"delta\":{\"type\":\"thought_signature\",\"signature\":\"sig-part-1\"}}\n\n",
            "data: {\"event_type\":\"step.delta\",\"index\":0,\"delta\":{\"type\":\"thought_signature\",\"signature\":\"-part-2\"}}\n\n",
            "data: {\"event_type\":\"step.stop\",\"index\":0}\n\n",
            "data: {\"event_type\":\"step.start\",\"index\":1,\"step\":{\"type\":\"model_output\"}}\n\n",
            "data: {\"event_type\":\"step.delta\",\"index\":1,\"delta\":{\"type\":\"text\",\"text\":\"Let me inspect.\"}}\n\n",
            "data: {\"event_type\":\"step.stop\",\"index\":1}\n\n",
            "data: {\"event_type\":\"step.start\",\"index\":2,\"step\":{\"type\":\"function_call\",\"id\":\"call_1\",\"name\":\"tools_call\"}}\n\n",
            "data: {\"event_type\":\"step.delta\",\"index\":2,\"delta\":{\"type\":\"arguments_delta\",\"arguments\":\"{\\\"tool\\\":\"}}\n\n",
            "data: {\"event_type\":\"step.delta\",\"index\":2,\"delta\":{\"type\":\"arguments_delta\",\"arguments\":\"\\\"fs.read\\\"}\"}}\n\n",
            "data: {\"event_type\":\"step.stop\",\"index\":2}\n\n",
            "data: {\"event_type\":\"interaction.completed\",\"interaction\":{\"id\":\"int_1\",\"status\":\"requires_action\",\"usage\":{\"total_input_tokens\":100,\"total_cached_tokens\":20,\"total_output_tokens\":30,\"total_thought_tokens\":40,\"total_tokens\":170}}}\n\n",
        );

        let (base_url, server) = spawn_mock_server(
            "HTTP/1.1 200 OK",
            "Content-Type: text/event-stream\r\n",
            sse_body,
        )
        .await;

        let adapter = GeminiInteractionsAdapter::new_managed_with_options(
            "gemini",
            reqwest::Client::new(),
            ManagedCredential::static_value("test", "AIza-test-key"),
            base_url,
            "gemini-4-hydrogen",
            GeminiInteractionsAdapterOptions::default(),
        );

        let response = adapter
            .complete(base_request("gemini-4-hydrogen"))
            .await
            .unwrap();
        let captured = server.await.unwrap();

        assert!(captured.starts_with("POST /interactions HTTP/1.1"));
        assert!(
            captured
                .to_ascii_lowercase()
                .contains("x-goog-api-key: aiza-test-key"),
            "captured request:\n{captured}"
        );
        assert!(captured.contains("\"store\":false"));
        assert!(captured.contains("\"thinking_level\":\"high\""));
        assert!(captured.contains("\"thinking_summaries\":\"auto\""));

        assert_eq!(response.text, "Let me inspect.");
        assert_eq!(response.reasoning_text.as_deref(), Some("Checking files"));
        assert_eq!(
            response.finish_reason,
            Some(CompletionFinishReason::ToolCalls)
        );
        assert_eq!(
            response.tool_calls,
            vec![CompletionToolCall::Function {
                id: "call_1".to_owned(),
                name: "tools_call".to_owned(),
                arguments_json: r#"{"tool":"fs.read"}"#.to_owned(),
            }]
        );
        let usage = response.usage.unwrap();
        assert_eq!(usage.input_tokens, 80);
        assert_eq!(usage.cache_read_tokens, 20);
        assert_eq!(usage.output_tokens, 30);
        assert_eq!(usage.reasoning_tokens, 40);
        assert_eq!(
            response
                .provider_metadata
                .as_ref()
                .and_then(|m| m.pointer("/gemini_thought_signatures/call_1"))
                .and_then(Value::as_str),
            Some("sig-part-1-part-2")
        );
    }

    #[tokio::test]
    async fn lists_models_from_standard_models_endpoint() {
        let body = r#"{"models":[{"name":"models/gemini-4-hydrogen","displayName":"Gemini 4 Hydrogen","inputTokenLimit":1048576,"outputTokenLimit":65536}]}"#;
        let (base_url, server) = spawn_mock_server(
            "HTTP/1.1 200 OK",
            "Content-Type: application/json\r\n",
            body,
        )
        .await;

        let adapter = GeminiInteractionsAdapter::new_managed_with_options(
            "gemini",
            reqwest::Client::new(),
            ManagedCredential::static_value("test", "AIza-test-key"),
            base_url,
            "gemini-4-hydrogen",
            GeminiInteractionsAdapterOptions::default(),
        );

        let models = adapter.list_models().await.unwrap();
        let captured = server.await.unwrap();

        assert!(captured.starts_with("GET /models HTTP/1.1"));
        assert!(
            captured
                .to_ascii_lowercase()
                .contains("x-goog-api-key: aiza-test-key"),
            "captured request:\n{captured}"
        );
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id.as_ref(), "gemini-4-hydrogen");
        assert_eq!(models[0].display_name.as_deref(), Some("Gemini 4 Hydrogen"));
        assert_eq!(
            models[0].capabilities.audio_input,
            CapabilitySupport::Supported
        );
    }

    #[tokio::test]
    async fn respects_custom_auth_header_and_scheme() {
        let sse_body = concat!(
            "data: {\"event_type\":\"step.start\",\"index\":0,\"step\":{\"type\":\"model_output\"}}\n\n",
            "data: {\"event_type\":\"step.delta\",\"index\":0,\"delta\":{\"type\":\"text\",\"text\":\"ok\"}}\n\n",
            "data: {\"event_type\":\"step.stop\",\"index\":0}\n\n",
            "data: {\"event_type\":\"interaction.completed\",\"interaction\":{\"id\":\"int_2\",\"status\":\"completed\"}}\n\n",
        );
        let (base_url, server) = spawn_mock_server(
            "HTTP/1.1 200 OK",
            "Content-Type: text/event-stream\r\n",
            sse_body,
        )
        .await;

        let adapter = GeminiInteractionsAdapter::new_managed_with_options(
            "gemini",
            reqwest::Client::new(),
            ManagedCredential::static_value("test", "proxy-token"),
            base_url,
            "gemini-4-hydrogen",
            GeminiInteractionsAdapterOptions {
                auth_header: Some(("authorization".to_owned(), Some("Bearer".to_owned()))),
                ..Default::default()
            },
        );

        let response = adapter
            .complete(base_request("gemini-4-hydrogen"))
            .await
            .unwrap();
        let captured = server.await.unwrap();

        assert_eq!(response.text, "ok");
        assert!(
            captured
                .to_ascii_lowercase()
                .contains("authorization: bearer proxy-token"),
            "captured request:\n{captured}"
        );
    }

    #[test]
    fn replays_signed_thoughts_and_inserts_skip_sentinel_for_unsigned_tool_calls() {
        let mut request = base_request("gemini-4-hydrogen");
        let signed_turn = CompletionInputRun {
            role: Role::Assistant,
            parts: vec![
                CompletionInputPart::Reasoning {
                    text: "Plan".to_owned(),
                },
                CompletionInputPart::ToolCall {
                    id: "call_signed".to_owned(),
                    function: ModelToolFunction::from(ToolApiFunction::Call),
                    arguments_json: r#"{"tool":"fs.read"}"#.to_owned(),
                },
            ],
            provider_state: CompletionInputProviderState {
                gemini_thought_signatures: BTreeMap::from([(
                    "call_signed".to_owned(),
                    "sig-xyz".to_owned(),
                )]),
                ..Default::default()
            },
        };
        let unsigned_turn = CompletionInputRun {
            role: Role::Assistant,
            parts: vec![CompletionInputPart::ToolCall {
                id: "call_cross_model".to_owned(),
                function: ModelToolFunction::from(ToolApiFunction::Call),
                arguments_json: r#"{"tool":"fs.list"}"#.to_owned(),
            }],
            provider_state: Default::default(),
        };
        request.turns.push(signed_turn);
        request.turns.push(unsigned_turn);

        let wire = GeminiInteractionsAdapter::build_interactions_request(&request).unwrap();
        assert!(matches!(
            &wire.input[1],
            GeminiInteractionsStep::Thought {
                signature: Some(sig),
                summary: Some(_),
            } if sig == "sig-xyz"
        ));
        assert!(matches!(
            &wire.input[2],
            GeminiInteractionsStep::FunctionCall { id, .. } if id == "call_signed"
        ));
        assert!(matches!(
            &wire.input[3],
            GeminiInteractionsStep::Thought {
                signature: Some(sig),
                summary: None,
            } if sig == GEMINI_SKIP_THOUGHT_SIGNATURE
        ));
        assert!(matches!(
            &wire.input[4],
            GeminiInteractionsStep::FunctionCall { id, .. } if id == "call_cross_model"
        ));
    }

    #[tokio::test]
    async fn daily_429_is_non_retryable_and_includes_hint() {
        let error_body = r#"{"error":{"code":429,"message":"You have reached today's Gemini 4 Hydrogen limit. Resets in 4 hours."}}"#;
        let (base_url, _server) = spawn_mock_server(
            "HTTP/1.1 429 Too Many Requests",
            "Content-Type: application/json\r\n",
            error_body,
        )
        .await;

        let adapter = GeminiInteractionsAdapter::new_managed_with_options(
            "gemini",
            reqwest::Client::new(),
            ManagedCredential::static_value("test", "AIza-test-key"),
            base_url,
            "gemini-4-hydrogen",
            GeminiInteractionsAdapterOptions::default(),
        );

        let err = adapter
            .complete(base_request("gemini-4-hydrogen"))
            .await
            .unwrap_err();
        assert!(!err.retryable());
        assert_eq!(
            err.provider_error_kind(),
            Some(ProviderErrorKind::QuotaExceeded)
        );
        assert!(err.to_string().contains("Daily limit reached"));
    }

    #[tokio::test]
    async fn premature_stream_eof_returns_error() {
        let sse_body = concat!(
            "data: {\"event_type\":\"step.start\",\"index\":0,\"step\":{\"type\":\"model_output\"}}\n\n",
            "data: {\"event_type\":\"step.delta\",\"index\":0,\"delta\":{\"type\":\"text\",\"text\":\"partial\"}}\n\n",
        );
        let (base_url, _server) = spawn_mock_server(
            "HTTP/1.1 200 OK",
            "Content-Type: text/event-stream\r\n",
            sse_body,
        )
        .await;

        let adapter = GeminiInteractionsAdapter::new_managed_with_options(
            "gemini",
            reqwest::Client::new(),
            ManagedCredential::static_value("test", "AIza-test-key"),
            base_url,
            "gemini-4-hydrogen",
            GeminiInteractionsAdapterOptions::default(),
        );

        let err = adapter
            .complete(base_request("gemini-4-hydrogen"))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("ended before interaction.completed")
        );
    }
}
