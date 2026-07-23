use agena_domain::Role;
use agena_domain::*;
use agena_provider::{
    CompletionFinishReason, CompletionToolCall, CompletionUsage, OllamaChatMessage,
    OllamaChatRequest, OllamaChatResponse, OllamaFunctionDefinition, OllamaOptions,
    OllamaTagsResponse, OllamaToolCall, OllamaToolDefinition, StreamResumePolicy,
    ollama_usage_to_completion,
};
use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;

use crate::{
    error::AppError,
    provider::{CompletionResponse, ModelRuntime, sse, utils, wire_message},
};
use agena_provider::CompletionRequest;
use agena_provider::CompletionStreamEvent;

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
            tools: (!request.tool_api_functions.is_empty())
                .then(|| tools_to_ollama_definitions(&request.tool_api_functions)),
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

    fn parse_models(&self, payload: OllamaTagsResponse) -> Vec<Model> {
        payload
            .models
            .into_iter()
            .filter_map(|model| {
                let id = model.model.or(model.name)?;
                let model_id = ModelId::new(id.clone());
                let metadata = self.model_metadata(&model_id);
                let capabilities = self.model_capabilities(&model_id);
                let entry = Model {
                    provider_id: ProviderId::new(self.id.clone()),
                    adapter_id: None,
                    id: ModelId::new(id),
                    catalog_model_id: None,
                    display_name: None,
                    native_compaction: true,
                    capabilities,
                    metadata,
                    thinking_modes: Vec::new(),
                    speed_modes: std::collections::BTreeMap::new(),
                };
                if let Some(details) = model.details
                    && let Some(family) = details.family.filter(|value| !value.trim().is_empty())
                {
                    return Some(Model {
                        display_name: Some(family),
                        ..entry
                    });
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
        let usage = ollama_usage_to_completion(&response);
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
}

#[async_trait]
impl ModelRuntime for OllamaAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<agena_provider::CapabilityFamily> {
        Some(agena_provider::CapabilityFamily::OpenAiCompatible)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
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
                    usage = ollama_usage_to_completion(&chunk).or(usage);
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

fn message_text(message: &agena_provider::CompletionInputMessage) -> Option<String> {
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
    tools: &[agena_provider::ToolApiDefinition],
) -> Vec<OllamaToolDefinition> {
    tools
        .iter()
        .cloned()
        .map(|tool| OllamaToolDefinition {
            kind: "function",
            function: OllamaFunctionDefinition {
                name: tool.name,
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
        .enumerate()
        .map(|(index, call)| {
            let name = call
                .function
                .name
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    AppError::Provider(format!(
                        "{provider_id} returned tool call without function.name"
                    ))
                })?;
            let arguments_json =
                serde_json::to_string(&call.function.arguments).map_err(|err| {
                    AppError::Provider(format!(
                        "{provider_id} returned invalid tool call arguments: {err}"
                    ))
                })?;
            Ok(CompletionToolCall::Function {
                id: format!("ollama-call-{index}"),
                name,
                arguments_json,
            })
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
        .map(|(index, call)| {
            let name = call
                .function
                .name
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    AppError::Provider(format!(
                        "{provider_id} returned stream tool call without function.name"
                    ))
                })?;
            let arguments_json =
                serde_json::to_string(&call.function.arguments).map_err(|err| {
                    AppError::Provider(format!(
                        "{provider_id} returned invalid stream tool call arguments: {err}"
                    ))
                })?;
            Ok((
                index,
                ParsedToolCall {
                    id: Some(format!("ollama-call-{index}")),
                    name: Some(name),
                    arguments_json,
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{OllamaToolCall, parse_stream_tool_calls, parse_tool_calls};
    use agena_provider::CompletionToolCall;
    use agena_provider::OllamaFunctionCall;

    fn call(name: Option<&str>) -> OllamaToolCall {
        OllamaToolCall {
            function: OllamaFunctionCall {
                name: name.map(str::to_owned),
                arguments: serde_json::json!({}),
            },
        }
    }

    #[test]
    fn ollama_rejects_tool_calls_without_names() {
        for name in [None, Some(""), Some("   ")] {
            assert!(parse_tool_calls("ollama", vec![call(name)]).is_err());
            assert!(parse_stream_tool_calls("ollama", vec![call(name)]).is_err());
        }
    }

    #[test]
    fn ollama_assigns_distinct_ids_to_parallel_calls_with_the_same_name() {
        let calls = parse_tool_calls(
            "ollama",
            vec![call(Some("tools_help")), call(Some("tools_help"))],
        )
        .expect("valid calls");
        let ids = calls
            .into_iter()
            .map(|call| match call {
                CompletionToolCall::Function { id, .. } => id,
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["ollama-call-0", "ollama-call-1"]);
    }
}
