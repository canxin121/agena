use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::AppError,
    message::Message,
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ModelRuntime, ProviderModel, StreamResumePolicy, sse,
        utils, wire_message,
    },
    role::Role,
};

const ADAPTER_KIND: &str = "ollama";

#[derive(Clone)]
pub struct OllamaAdapter {
    id: String,
    client: reqwest::Client,
    base_url: String,
    default_model: ModelId,
}

impl OllamaAdapter {
    pub fn new(
        id: impl Into<String>,
        client: reqwest::Client,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            client,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
        }
    }

    fn tags_endpoint(&self) -> String {
        format!("{}/api/tags", self.base_url)
    }

    fn chat_endpoint(&self) -> String {
        format!("{}/api/chat", self.base_url)
    }

    fn to_chat_request(&self, request: &CompletionRequest, stream: bool) -> OllamaChatRequest {
        OllamaChatRequest {
            model: request.model.to_string(),
            messages: to_ollama_messages(request),
            tools: (!request.tools.is_empty()).then(|| tools_to_ollama_definitions(&request.tools)),
            stream,
            options: OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_output_tokens,
                stop: request.stop_sequences.clone(),
                top_p: request.top_p,
                top_k: request.top_k,
                seed: request.seed,
            },
        }
    }

    fn parse_models(&self, payload: OllamaTagsResponse) -> Vec<ProviderModel> {
        payload
            .models
            .into_iter()
            .filter_map(|model| {
                let id = model.model.or(model.name)?;
                let model_id = ModelId::new(id.clone());
                let metadata = self.model_metadata(&model_id);
                let capabilities = self.model_capabilities(&model_id);
                let mut entry = ProviderModel::new(self.id.clone(), id)
                    .with_capabilities(capabilities)
                    .with_metadata(metadata);
                if let Some(details) = model.details
                    && let Some(family) = details.family.filter(|value| !value.trim().is_empty())
                {
                    entry.display_name = Some(family);
                }
                Some(entry)
            })
            .collect()
    }

    fn completion_from_response(
        &self,
        fallback_model: &ModelId,
        response: OllamaChatResponse,
    ) -> Result<CompletionResponse, AppError> {
        let usage = usage_from_response(&response);
        let message = response.message.unwrap_or_default();
        let text = message.content.unwrap_or_default();
        let tool_calls = parse_tool_calls(self.id.as_str(), message.tool_calls)?;
        let finish_reason = CompletionFinishReason::from_provider(response.done_reason.as_deref());

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() && !response.done {
            return Err(AppError::Provider(format!(
                "{} returned empty chat payload without finish reason",
                self.id
            )));
        }

        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id.clone()),
            model: ModelId::new(response.model.unwrap_or_else(|| fallback_model.to_string())),
            text,
            reasoning_text: None,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: None,
        })
    }

    #[allow(dead_code)]
    fn completion_response_stream(
        response: CompletionResponse,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>> {
        let provider_id = response.provider_id.clone();
        let model = response.model.clone();
        let mut events = Vec::new();
        if !response.text.is_empty() {
            events.push(Ok(CompletionStreamEvent::TextDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: response.text,
            }));
        }
        if let Some(reasoning) = response.reasoning_text
            && !reasoning.is_empty()
        {
            events.push(Ok(CompletionStreamEvent::ThinkingDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: reasoning,
            }));
        }
        for call in response.tool_calls {
            let CompletionToolCall::Function {
                id,
                name,
                arguments_json,
            } = call;
            events.push(Ok(CompletionStreamEvent::ToolCallSnapshot {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key: id.clone(),
                id: Some(id),
                name: Some(name),
                arguments_json,
            }));
        }
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id,
            model,
            finish_reason: response.finish_reason,
            usage: response.usage,
            provider_metadata: response.provider_metadata,
        }));
        Box::pin(futures_util::stream::iter(events))
    }
}

#[async_trait]
impl ModelRuntime for OllamaAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(crate::provider::CapabilityFamily::OpenAiCompatible)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let endpoint = self.tags_endpoint();
        utils::adapter_log_http_request_json(
            self.id.as_str(),
            ADAPTER_KIND,
            "list_models",
            "GET",
            endpoint.as_str(),
            std::iter::empty::<(&str, &str)>(),
            None,
        );
        let response = self.client.get(endpoint.as_str()).send().await?;
        let payload: OllamaTagsResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            ADAPTER_KIND,
            "list_models",
            response,
        )
        .await?;
        Ok(self.parse_models(payload))
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let fallback_model = request.model.clone();
        let body = self.to_chat_request(&request, false);
        let endpoint = self.chat_endpoint();
        let body_json = serde_json::to_value(&body).map_err(AppError::from)?;
        utils::adapter_log_http_request_json(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete.chat",
            "POST",
            endpoint.as_str(),
            [(reqwest::header::CONTENT_TYPE.as_str(), "application/json")],
            Some(&body_json),
        );
        let response = self
            .client
            .post(endpoint.as_str())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;
        let payload: OllamaChatResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete.chat",
            response,
        )
        .await?;
        self.completion_from_response(&fallback_model, payload)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let body = self.to_chat_request(&request, true);
        let endpoint = self.chat_endpoint();
        let body_json = serde_json::to_value(&body).map_err(AppError::from)?;
        utils::adapter_log_http_request_json(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete_stream.chat",
            "POST",
            endpoint.as_str(),
            [(reqwest::header::CONTENT_TYPE.as_str(), "application/json")],
            Some(&body_json),
        );
        let response = self
            .client
            .post(endpoint.as_str())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response_logged(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.chat",
                response,
            )
            .await);
        }

        utils::adapter_log_http_response_open(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete_stream.chat",
            response.status(),
            response.headers(),
        );
        let mut events = sse::json_lines(response);
        let provider_id = ProviderId::new(self.id.clone());
        let model_name = request.model.clone();
        let provider_label = self.id.clone();

        let stream = async_stream::try_stream! {
            let mut finish_reason: Option<String> = None;
            let mut usage: Option<CompletionUsage> = None;
            let mut emitted_content = false;
            let mut completed = false;

            while let Some(event) = events.next().await {
                let event = event?;
                utils::adapter_log_stream_event(
                    provider_label.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.chat",
                    &event,
                );
                let chunk: OllamaChatResponse = utils::parse_json_value(
                    provider_label.as_str(),
                    "chat stream chunk",
                    event,
                )?;

                if finish_reason.is_none() {
                    finish_reason = chunk.done_reason.clone();
                }
                usage = usage_from_response(&chunk).or(usage);
                let chunk_model = chunk.model.clone();

                if let Some(message) = chunk.message {
                    if let Some(delta) = message.content.filter(|value| !value.is_empty()) {
                        emitted_content = true;
                        yield CompletionStreamEvent::TextDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            delta,
                        };
                    }

                    for (index, tool_call) in parse_stream_tool_calls(provider_label.as_str(), message.tool_calls)? {
                        emitted_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: format!("idx:{index}"),
                            id: tool_call.id,
                            name: tool_call.name,
                            arguments_delta: tool_call.arguments_json,
                        };
                    }
                }

                if chunk.done {
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: ModelId::new(chunk_model.unwrap_or_else(|| model_name.to_string())),
                        finish_reason: CompletionFinishReason::from_provider(finish_reason.as_deref()),
                        usage: usage.clone(),
                        provider_metadata: None,
                    };
                    completed = true;
                    break;
                }
            }

            if !completed && (emitted_content || finish_reason.is_some() || usage.is_some()) {
                yield CompletionStreamEvent::Completed {
                    provider_id,
                    model: model_name,
                    finish_reason: CompletionFinishReason::from_provider(finish_reason.as_deref()),
                    usage,
                    provider_metadata: None,
                };
            }
        };

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaTagModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagModel {
    name: Option<String>,
    model: Option<String>,
    #[serde(default)]
    details: Option<OllamaModelDetails>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelDetails {
    #[serde(default)]
    family: Option<String>,
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OllamaToolDefinition>>,
    stream: bool,
    #[serde(skip_serializing_if = "OllamaOptions::is_empty")]
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize, Default)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
}

impl OllamaOptions {
    fn is_empty(&self) -> bool {
        self.temperature.is_none()
            && self.num_predict.is_none()
            && self.stop.is_empty()
            && self.top_p.is_none()
            && self.top_k.is_none()
            && self.seed.is_none()
    }
}

#[derive(Debug, Serialize)]
struct OllamaToolDefinition {
    #[serde(rename = "type")]
    kind: &'static str,
    function: OllamaFunctionDefinition,
}

#[derive(Debug, Serialize)]
struct OllamaFunctionDefinition {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Deserialize, Default)]
struct OllamaChatMessageResponse {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OllamaToolCall>,
}

#[derive(Debug, Deserialize)]
struct OllamaToolCall {
    function: OllamaFunctionCall,
}

#[derive(Debug, Deserialize)]
struct OllamaFunctionCall {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    message: Option<OllamaChatMessageResponse>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}

struct ParsedToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments_json: String,
}

fn to_ollama_messages(request: &CompletionRequest) -> Vec<OllamaChatMessage> {
    let mut messages = Vec::new();
    if let Some(system) = request
        .system
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        messages.push(OllamaChatMessage {
            role: "system".to_owned(),
            content: system.clone(),
        });
    }

    for message in &request.messages {
        if let Some(content) = message_text(message) {
            messages.push(OllamaChatMessage {
                role: role_name(message.role).to_owned(),
                content,
            });
        }
    }
    messages
}

fn message_text(message: &Message) -> Option<String> {
    let text = wire_message::project_text_lossy(message);
    let trimmed = text.trim();
    (!trimmed.is_empty()).then_some(text)
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    }
}

fn tools_to_ollama_definitions(
    tools: &[crate::plugin::registry::RegisteredTool],
) -> Vec<OllamaToolDefinition> {
    tools
        .iter()
        .map(crate::tool::ModelToolSpec::from_registered_tool)
        .map(|tool| OllamaToolDefinition {
            kind: "function",
            function: OllamaFunctionDefinition {
                name: tool.provider_safe_name,
                description: tool.description,
                parameters: tool.input_schema,
            },
        })
        .collect()
}

fn parse_tool_calls(
    provider_id: &str,
    calls: Vec<OllamaToolCall>,
) -> Result<Vec<CompletionToolCall>, AppError> {
    calls
        .into_iter()
        .filter_map(|call| {
            let name = call
                .function
                .name
                .filter(|value| !value.trim().is_empty())?;
            let arguments_json = serde_json::to_string(&call.function.arguments).map_err(|err| {
                AppError::Provider(format!(
                    "{provider_id} returned invalid tool call arguments: {err}"
                ))
            });
            Some(
                arguments_json.map(|arguments_json| CompletionToolCall::Function {
                    id: name.clone(),
                    name,
                    arguments_json,
                }),
            )
        })
        .collect()
}

fn parse_stream_tool_calls(
    provider_id: &str,
    calls: Vec<OllamaToolCall>,
) -> Result<Vec<(usize, ParsedToolCall)>, AppError> {
    calls
        .into_iter()
        .enumerate()
        .filter_map(|(index, call)| {
            let name = call
                .function
                .name
                .filter(|value| !value.trim().is_empty())?;
            let arguments_json = serde_json::to_string(&call.function.arguments).map_err(|err| {
                AppError::Provider(format!(
                    "{provider_id} returned invalid stream tool call arguments: {err}"
                ))
            });
            Some(arguments_json.map(|arguments_json| {
                (
                    index,
                    ParsedToolCall {
                        id: Some(name.clone()),
                        name: Some(name),
                        arguments_json,
                    },
                )
            }))
        })
        .collect()
}

fn usage_from_response(response: &OllamaChatResponse) -> Option<CompletionUsage> {
    let input = response.prompt_eval_count.unwrap_or_default();
    let output = response.eval_count.unwrap_or_default();
    (input > 0 || output > 0).then_some(CompletionUsage {
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: 0,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        total_cost: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_completion_request(messages: Vec<Message>) -> CompletionRequest {
        CompletionRequest {
            model: ModelId::new("llama3.1"),
            system: None,
            messages,
            tools: Vec::new(),
            native_tools: crate::config::ProviderNativeToolsConfig::default(),
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
            verbosity: None,
            response_format: None,
            responses_api_metadata: None,
            request_override: crate::model::ModelSpeedModeRequestOverride::default(),
        }
    }

    #[test]
    fn ollama_chat_request_uses_structured_tools() {
        let adapter = OllamaAdapter::new(
            "ollama",
            reqwest::Client::new(),
            "http://localhost:11434",
            "llama3.1",
        );
        let mut request = test_completion_request(vec![Message::prompt_text(Role::User, "hi")]);
        request.tools = vec![crate::plugin::registry::RegisteredTool::new(
            "streaming-fixture",
            crate::plugin::sdk::ToolDefinition::new(
                "stream_fixture.count",
                serde_json::json!({ "type": "object" }),
            )
            .description("Count rows."),
        )];

        let body = adapter.to_chat_request(&request, false);
        assert_eq!(body.messages.len(), 1);
        assert_eq!(
            body.tools.expect("tools should be present")[0]
                .function
                .name,
            crate::tool::model_safe_tool_name("streaming-fixture.stream_fixture.count")
        );
    }

    #[test]
    fn ollama_completion_parses_native_tool_calls() {
        let adapter = OllamaAdapter::new(
            "ollama",
            reqwest::Client::new(),
            "http://localhost:11434",
            "llama3.1",
        );
        let response = OllamaChatResponse {
            model: Some("llama3.1".to_owned()),
            message: Some(OllamaChatMessageResponse {
                content: Some(String::new()),
                tool_calls: vec![OllamaToolCall {
                    function: OllamaFunctionCall {
                        name: Some(crate::tool::model_safe_tool_name("a.b.c")),
                        arguments: serde_json::json!({ "x": 1 }),
                    },
                }],
            }),
            done: true,
            done_reason: Some("stop".to_owned()),
            prompt_eval_count: None,
            eval_count: None,
        };

        let parsed = adapter
            .completion_from_response(&ModelId::new("llama3.1"), response)
            .expect("structured response should parse");

        assert_eq!(parsed.text, "");
        assert_eq!(
            parsed.tool_calls,
            vec![CompletionToolCall::Function {
                id: crate::tool::model_safe_tool_name("a.b.c"),
                name: crate::tool::model_safe_tool_name("a.b.c"),
                arguments_json: "{\"x\":1}".to_owned(),
            }]
        );
        assert_eq!(parsed.finish_reason, Some(CompletionFinishReason::Stop));
    }
}
