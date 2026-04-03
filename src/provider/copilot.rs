use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    error::AppError,
    message::{AttachmentItem, AttachmentKind, AttachmentSource, MessageUsage},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionUsage, ModelProvider, ProviderModel, StreamResumePolicy, auth::AuthData, sse,
        utils,
    },
    role::Role,
};

const PROVIDER_ID_ENTERPRISE: &str = "github-copilot-enterprise";

#[derive(Clone)]
pub struct CopilotProvider {
    id: String,
    client: reqwest::Client,
    bearer_token: String,
    base_url: String,
    default_model: String,
    models_url: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CopilotProviderOptions {
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub models_url: Option<String>,
}

impl CopilotProvider {
    pub fn from_auth(id: &str, client: reqwest::Client, auth: &AuthData) -> Result<Self, AppError> {
        Self::from_auth_with_options(id, client, auth, CopilotProviderOptions::default())
    }

    pub fn from_auth_with_options(
        id: &str,
        client: reqwest::Client,
        auth: &AuthData,
        options: CopilotProviderOptions,
    ) -> Result<Self, AppError> {
        let AuthData::OAuth {
            refresh,
            access,
            enterprise_url,
            ..
        } = auth
        else {
            return Err(AppError::Config(format!(
                "{id} requires oauth token in auth store"
            )));
        };

        let default_base = if id == PROVIDER_ID_ENTERPRISE {
            let domain = enterprise_url.as_ref().ok_or_else(|| {
                AppError::Config("enterprise_url missing for github-copilot-enterprise".into())
            })?;
            format!("https://copilot-api.{}", normalize_domain(domain))
        } else {
            "https://api.githubcopilot.com".to_owned()
        };

        let base_url = options.base_url.unwrap_or(default_base);
        let default_model = options
            .default_model
            .unwrap_or_else(|| "gpt-4o-mini".to_owned());

        Ok(Self {
            id: id.to_owned(),
            client,
            bearer_token: select_copilot_bearer_token(refresh, access).ok_or_else(|| {
                AppError::Config(format!(
                    "{id} oauth credential is missing usable access/refresh token"
                ))
            })?,
            base_url,
            default_model,
            models_url: options.models_url,
        })
    }

    fn models_endpoint(&self) -> String {
        self.models_url
            .clone()
            .unwrap_or_else(|| format!("{}/models", self.base_url.trim_end_matches('/')))
    }

    fn chat_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn responses_endpoint(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }

    fn should_use_responses(model_id: &str) -> bool {
        let is_gpt5 = model_id
            .strip_prefix("gpt-")
            .and_then(|x| x.split('-').next())
            .and_then(|major| major.parse::<u32>().ok())
            .map(|major| major >= 5)
            .unwrap_or(false);
        is_gpt5 && !model_id.starts_with("gpt-5-mini")
    }

    fn responses_endpoint_unsupported(status: reqwest::StatusCode) -> bool {
        matches!(
            status,
            reqwest::StatusCode::NOT_FOUND
                | reqwest::StatusCode::METHOD_NOT_ALLOWED
                | reqwest::StatusCode::NOT_IMPLEMENTED
        )
    }

    fn parse_responses_tool_calls(
        items: Option<&Vec<OpenAiOutputItem>>,
    ) -> Result<Vec<crate::provider::CompletionToolCall>, AppError> {
        items
            .into_iter()
            .flatten()
            .filter(|item| item.kind.as_deref() == Some("function_call"))
            .map(|item| {
                let id = utils::normalize_optional_text(item.call_id.clone())
                    .or_else(|| utils::normalize_optional_text(item.id.clone()))
                    .ok_or_else(|| {
                        AppError::Provider(
                            "copilot responses payload returned function_call without id/call_id"
                                .to_owned(),
                        )
                    })?;

                let name = utils::normalize_optional_text(item.name.clone()).ok_or_else(|| {
                    AppError::Provider(
                        "copilot responses payload returned function_call without name".to_owned(),
                    )
                })?;

                Ok(crate::provider::CompletionToolCall::Function {
                    id,
                    name,
                    arguments_json: item.arguments.clone().unwrap_or_default(),
                })
            })
            .collect()
    }

    fn is_vision_request(request: &CompletionRequest) -> bool {
        request.messages.iter().any(|message| {
            utils::project_session_parts(message).iter().any(|part| {
                matches!(
                    part,
                    utils::ProjectedSessionPart::Attachment { item }
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

    fn base_headers(
        &self,
        request: &CompletionRequest,
    ) -> Result<reqwest::header::HeaderMap, AppError> {
        let mut headers = reqwest::header::HeaderMap::new();
        let authorization = reqwest::header::HeaderValue::from_str(
            format!("Bearer {}", self.bearer_token).as_str(),
        )
        .map_err(|e| AppError::Config(format!("invalid copilot bearer token header: {e}")))?;

        headers.insert(reqwest::header::AUTHORIZATION, authorization);
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static("agena/0.1.0"),
        );
        headers.insert(
            "openai-intent",
            reqwest::header::HeaderValue::from_static("conversation-edits"),
        );
        headers.insert(
            "x-initiator",
            reqwest::header::HeaderValue::from_static(Self::initiator(request)),
        );
        if Self::is_vision_request(request) {
            headers.insert(
                "Copilot-Vision-Request",
                reqwest::header::HeaderValue::from_static("true"),
            );
        }
        Ok(headers)
    }
}

#[async_trait]
impl ModelProvider for CopilotProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn model_capabilities(&self, model: &str) -> crate::provider::ModelCapabilities {
        crate::provider::default_capability_registry()
            .capabilities_for_family(crate::provider::CapabilityFamily::OpenAi, model)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let authorization = reqwest::header::HeaderValue::from_str(
            format!("Bearer {}", self.bearer_token).as_str(),
        )
        .map_err(|e| AppError::Config(format!("invalid copilot bearer token header: {e}")))?;

        let response = self
            .client
            .get(self.models_endpoint())
            .header(reqwest::header::AUTHORIZATION, authorization)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(self.id.as_str(), response).await);
        }

        let payload: ModelListResponse = response.json().await?;

        Ok(payload
            .into_items()
            .into_iter()
            .map(|m| {
                let capabilities = self.model_capabilities(m.id.as_str());
                let mut model =
                    ProviderModel::new(self.id.clone(), m.id).with_capabilities(capabilities);
                model.display_name = m.name;
                model
            })
            .collect())
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = if request.model.trim().is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        if Self::should_use_responses(model.as_str()) {
            let body = OpenAiResponsesRequest::from_request(model.clone(), &request);
            let response = self
                .client
                .post(self.responses_endpoint())
                .headers(self.base_headers(&request)?)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&body)
                .send()
                .await?;

            if response.status().is_success() {
                let payload: OpenAiResponsesResponse = response.json().await?;
                let text = payload
                    .output_text
                    .or_else(|| {
                        payload.output.as_ref().map(|items| {
                            items
                                .iter()
                                .flat_map(|item| item.content.iter().flatten())
                                .filter_map(|part| part.text.clone())
                                .collect::<Vec<_>>()
                                .join("")
                        })
                    })
                    .unwrap_or_default();

                let finish_reason =
                    CompletionFinishReason::from_provider(payload.stop_reason.as_deref());
                let tool_calls = Self::parse_responses_tool_calls(payload.output.as_ref())?;

                if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
                    return Err(AppError::Provider(
                        "copilot responses payload was empty without finish reason".to_owned(),
                    ));
                }

                return Ok(CompletionResponse {
                    provider_id: self.id.clone(),
                    model: payload.model.unwrap_or(model.clone()),
                    text,
                    finish_reason,
                    tool_calls,
                    usage: payload.usage.map(map_openai_usage),
                    provider_metadata: None,
                });
            }

            if !Self::responses_endpoint_unsupported(response.status()) {
                return Err(
                    utils::http_status_error_from_response(self.id.as_str(), response).await,
                );
            }
        }

        let body = ChatCompletionRequest::from_request(model.clone(), &request);
        let response = self
            .client
            .post(self.chat_endpoint())
            .headers(self.base_headers(&request)?)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(self.id.as_str(), response).await);
        }

        let payload: ChatCompletionResponse = response.json().await?;
        let text = payload
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.content.as_ref())
            .map(extract_chat_content_text)
            .or_else(|| payload.choices.first().and_then(|c| c.text.clone()))
            .unwrap_or_default();

        let finish_reason = CompletionFinishReason::from_provider(
            payload
                .choices
                .first()
                .and_then(|c| c.finish_reason.as_deref()),
        );

        let tool_calls = parse_chat_tool_calls(
            payload
                .choices
                .first()
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.tool_calls.as_ref()),
        )?;

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(
                "copilot chat completion payload was empty without finish reason".to_owned(),
            ));
        }

        Ok(CompletionResponse {
            provider_id: self.id.clone(),
            model: payload.model.unwrap_or(model),
            text,
            finish_reason,
            tool_calls,
            usage: payload.usage.map(|u| {
                MessageUsage {
                    input_tokens: u.prompt_tokens.unwrap_or_default(),
                    output_tokens: u.completion_tokens.unwrap_or_default(),
                    reasoning_tokens: 0,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                    total_cost: 0.0,
                }
                .into()
            }),
            provider_metadata: None,
        })
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = if request.model.trim().is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        if Self::should_use_responses(model.as_str()) {
            let body =
                OpenAiResponsesRequest::from_request(model.clone(), &request).with_stream(true);
            let response = self
                .client
                .post(self.responses_endpoint())
                .headers(self.base_headers(&request)?)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&body)
                .send()
                .await?;

            if response.status().is_success() {
                let mut events = sse::json_events(response);
                let provider_id = self.id.clone();
                let model_name = model.clone();

                let stream = async_stream::try_stream! {
                    let mut pending_tool_calls: std::collections::BTreeMap<String, ResponsesToolState> = std::collections::BTreeMap::new();
                    let mut stream_usage: Option<CompletionUsage> = None;
                    let mut stream_finish_reason: Option<String> = None;
                    let mut stream_has_content = false;
                    let mut completed_emitted = false;

                    while let Some(event) = events.next().await {
                        let event = event?;

                        if let Some(err) = utils::responses_stream_error(provider_id.as_str(), &event)?
                        {
                            Err(err)?;
                        }

                        if let Some(delta) = utils::responses_text_delta(&event) {
                            stream_has_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta,
                            };
                        }

                        if let Some(tool_event) =
                            utils::responses_tool_event(provider_id.as_str(), &event)?
                        {
                            let key = tool_event.stream_key(provider_id.as_str())?;

                            let state = pending_tool_calls.entry(key.clone()).or_default();
                            if let Some(id) = tool_event.id.clone() {
                                state.id = Some(id);
                            }
                            if let Some(name) = tool_event.name.clone() {
                                state.name = Some(name);
                            }

                            match tool_event.kind {
                                utils::ResponsesToolEventKind::Delta => {
                                    if let Some(arguments_delta) =
                                        tool_event.arguments.filter(|s| !s.is_empty())
                                    {
                                        state.arguments.push_str(arguments_delta.as_str());
                                        stream_has_content = true;
                                        yield CompletionStreamEvent::ToolCallDelta {
                                            provider_id: provider_id.clone(),
                                            model: model_name.clone(),
                                            stream_key: key.clone(),
                                            id: state.id.clone(),
                                            name: state.name.clone(),
                                            arguments_delta,
                                        };
                                    }
                                }
                                utils::ResponsesToolEventKind::Added => {
                                    if let Some(arguments_snapshot) =
                                        tool_event.arguments.filter(|s| !s.is_empty())
                                    {
                                        let arguments_delta = if arguments_snapshot
                                            .starts_with(&state.arguments)
                                        {
                                            arguments_snapshot[state.arguments.len()..].to_owned()
                                        } else {
                                            arguments_snapshot.clone()
                                        };

                                        if arguments_snapshot.starts_with(&state.arguments) {
                                            state.arguments.push_str(arguments_delta.as_str());
                                        } else {
                                            state.arguments = arguments_snapshot;
                                        }

                                        if !arguments_delta.is_empty() {
                                            stream_has_content = true;
                                            yield CompletionStreamEvent::ToolCallDelta {
                                                provider_id: provider_id.clone(),
                                                model: model_name.clone(),
                                                stream_key: key.clone(),
                                                id: state.id.clone(),
                                                name: state.name.clone(),
                                                arguments_delta,
                                            };
                                        }
                                    }
                                }
                                utils::ResponsesToolEventKind::Done => {
                                    if let Some(arguments_snapshot) =
                                        tool_event.arguments.filter(|s| !s.is_empty())
                                    {
                                        let arguments_delta = if arguments_snapshot
                                            .starts_with(&state.arguments)
                                        {
                                            arguments_snapshot[state.arguments.len()..].to_owned()
                                        } else {
                                            arguments_snapshot.clone()
                                        };

                                        if arguments_snapshot.starts_with(&state.arguments) {
                                            state.arguments.push_str(arguments_delta.as_str());
                                        } else {
                                            state.arguments = arguments_snapshot;
                                        }

                                        if !arguments_delta.is_empty() {
                                            stream_has_content = true;
                                            yield CompletionStreamEvent::ToolCallDelta {
                                                provider_id: provider_id.clone(),
                                                model: model_name.clone(),
                                                stream_key: key.clone(),
                                                id: state.id.clone(),
                                                name: state.name.clone(),
                                                arguments_delta,
                                            };
                                        }
                                    }

                                    pending_tool_calls.remove(key.as_str());
                                }
                            }
                        }

                        if let Some(raw_usage) = utils::responses_usage_value(&event) {
                            let usage = utils::parse_json_value::<OpenAiUsage>(
                                provider_id.as_str(),
                                "responses stream usage",
                                raw_usage,
                            )?;
                            stream_usage = Some(map_openai_usage(usage));
                        }

                        if stream_finish_reason.is_none() {
                            stream_finish_reason = utils::responses_finish_reason(&event);
                        }

                        if utils::responses_is_completed(&event) {
                            yield CompletionStreamEvent::Completed {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                finish_reason: CompletionFinishReason::from_provider(
                                    stream_finish_reason.as_deref(),
                                ),
                                usage: stream_usage.clone(),
                                provider_metadata: None,
                            };
                            completed_emitted = true;
                            break;
                        }
                    }

                    if !completed_emitted
                        && (stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some())
                    {
                        yield CompletionStreamEvent::Completed {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            finish_reason: CompletionFinishReason::from_provider(
                                stream_finish_reason.as_deref(),
                            ),
                            usage: stream_usage,
                            provider_metadata: None,
                        };
                    }
                };

                return Ok(Box::pin(stream));
            }

            if !Self::responses_endpoint_unsupported(response.status()) {
                return Err(
                    utils::http_status_error_from_response(self.id.as_str(), response).await,
                );
            }
        }

        let body = ChatCompletionRequest::from_request(model.clone(), &request).with_stream(true);
        let response = self
            .client
            .post(self.chat_endpoint())
            .headers(self.base_headers(&request)?)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(self.id.as_str(), response).await);
        }

        let mut events = sse::json_events(response);
        let provider_id = self.id.clone();
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ChatToolCallState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;

            while let Some(event) = events.next().await {
                let event = event?;

                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(provider_id.as_str(), "chat stream chunk", event)?;
                let choice = chunk.choices.first();

                if let Some(delta) = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.content.as_ref())
                    .map(extract_chat_content_text)
                    .or_else(|| choice.and_then(|item| item.text.clone()))
                {
                    if !delta.is_empty() {
                        stream_has_content = true;
                        yield CompletionStreamEvent::TextDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            delta,
                        };
                    }
                }

                let tool_deltas = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.tool_calls.clone())
                    .unwrap_or_default();

                for raw_tool in tool_deltas {
                    let tool = utils::parse_json_value::<ChatToolCall>(
                        provider_id.as_str(),
                        "chat stream tool_call delta",
                        raw_tool,
                    )?;
                    let id = utils::normalize_optional_text(tool.id.clone());
                    let key = tool
                        .index
                        .map(|idx| format!("idx:{idx}"))
                        .or_else(|| id.as_ref().map(|value| format!("id:{value}")))
                        .ok_or_else(|| {
                            AppError::Provider(format!(
                                "{} chat stream tool_call delta missing index/id",
                                provider_id
                            ))
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
                }

                if let Some(raw_usage) = chunk.usage {
                    let usage = utils::parse_json_value::<ChatUsage>(
                        provider_id.as_str(),
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
                    provider_id: provider_id.clone(),
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

#[derive(Debug, Deserialize)]
struct ModelItem {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ModelListResponse {
    Array(Vec<ModelItem>),
    Object {
        #[serde(default)]
        data: Vec<ModelItem>,
    },
}

impl ModelListResponse {
    fn into_items(self) -> Vec<ModelItem> {
        match self {
            Self::Array(items) => items,
            Self::Object { data } => data,
        }
    }
}

fn build_chat_messages(request: &CompletionRequest) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
        messages.push(ChatMessage {
            role: "system".to_owned(),
            content: Some(system.clone()),
            tool_call_id: None,
            tool_calls: None,
        });
    }

    for message in &request.messages {
        let projected_parts = utils::project_session_parts(message);
        match message.role {
            Role::System => messages.push(ChatMessage {
                role: "system".to_owned(),
                content: Some(session_text_lossy(message, projected_parts.as_slice())),
                tool_call_id: None,
                tool_calls: None,
            }),
            Role::User => messages.push(ChatMessage {
                role: "user".to_owned(),
                content: Some(session_text_lossy(message, projected_parts.as_slice())),
                tool_call_id: None,
                tool_calls: None,
            }),
            Role::Assistant => {
                let (content, tool_calls) =
                    assistant_chat_content_and_tool_calls(message, projected_parts.as_slice());
                messages.push(ChatMessage {
                    role: "assistant".to_owned(),
                    content,
                    tool_call_id: None,
                    tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
                });
            }
            Role::Tool => {
                let tool_messages = tool_chat_messages(projected_parts.as_slice());
                let extra_parts = utils::non_tool_result_parts(projected_parts.as_slice());
                if tool_messages.is_empty() {
                    messages.push(ChatMessage {
                        role: "user".to_owned(),
                        content: Some(session_text_lossy(message, projected_parts.as_slice())),
                        tool_call_id: None,
                        tool_calls: None,
                    });
                } else {
                    messages.extend(tool_messages);
                    if !extra_parts.is_empty() {
                        messages.push(ChatMessage {
                            role: "user".to_owned(),
                            content: Some(utils::projected_parts_text_lossy(
                                extra_parts.as_slice(),
                            )),
                            tool_call_id: None,
                            tool_calls: None,
                        });
                    }
                }
            }
        }
    }

    messages
}

fn chat_tools(tools: &[crate::tool::ToolDefinition]) -> Vec<ChatToolDefinition> {
    tools
        .iter()
        .map(|tool| ChatToolDefinition {
            kind: "function".to_owned(),
            function: ChatFunctionDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            },
        })
        .collect()
}

fn assistant_chat_content_and_tool_calls(
    message: &crate::message::Message,
    projected_parts: &[utils::ProjectedSessionPart],
) -> (Option<String>, Vec<ChatToolCallRequest>) {
    if projected_parts.is_empty() {
        return (Some(message.as_text_lossy()), Vec::new());
    }

    let mut text_chunks = Vec::new();
    let mut tool_calls = Vec::new();

    for part in projected_parts {
        match part {
            utils::ProjectedSessionPart::Text { text } => text_chunks.push(text.clone()),
            utils::ProjectedSessionPart::Attachment { item } => {
                text_chunks.push(utils::attachment_hint_text(item));
            }
            utils::ProjectedSessionPart::ToolCall {
                id,
                name,
                arguments_json,
            } => tool_calls.push(ChatToolCallRequest {
                kind: "function".to_owned(),
                id: id.clone(),
                function: ChatFunctionCallRequest {
                    name: name.clone(),
                    arguments: arguments_json.clone(),
                },
            }),
            utils::ProjectedSessionPart::ToolResult { tool_call_id, .. } => {
                text_chunks.push(format!("[tool_result:{tool_call_id}]"));
            }
        }
    }

    let content = (!text_chunks.is_empty()).then(|| text_chunks.join(""));
    (content, tool_calls)
}

fn tool_chat_messages(parts: &[utils::ProjectedSessionPart]) -> Vec<ChatMessage> {
    parts
        .iter()
        .filter_map(|part| match part {
            utils::ProjectedSessionPart::ToolResult {
                tool_call_id,
                output_json,
            } => {
                if tool_call_id.trim().is_empty() {
                    None
                } else {
                    Some(ChatMessage {
                        role: "tool".to_owned(),
                        content: Some(output_json.clone()),
                        tool_call_id: Some(tool_call_id.clone()),
                        tool_calls: None,
                    })
                }
            }
            _ => None,
        })
        .collect()
}

fn build_responses_input(request: &CompletionRequest) -> Vec<OpenAiResponsesInputItem> {
    let mut input = Vec::new();

    if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
        push_responses_text_message(&mut input, "system", system.clone());
    }

    for message in &request.messages {
        append_responses_items_for_message(&mut input, message);
    }

    input
}

fn responses_tools(tools: &[crate::tool::ToolDefinition]) -> Vec<OpenAiResponsesTool> {
    tools
        .iter()
        .map(|tool| OpenAiResponsesTool {
            kind: "function",
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        })
        .collect()
}

fn attachment_upload_name(item: &AttachmentItem) -> String {
    utils::attachment_filename(item)
        .map(str::to_owned)
        .unwrap_or_else(|| item.summary_label())
}

fn responses_file_content(item: &AttachmentItem) -> Option<OpenAiInputContent> {
    let filename = Some(attachment_upload_name(item));
    match &item.source {
        AttachmentSource::Base64 { .. } | AttachmentSource::DataUrl { .. } => {
            utils::attachment_data_url(item).map(|file_data| OpenAiInputContent::File {
                file_data: Some(file_data),
                file_id: None,
                file_url: None,
                filename,
            })
        }
        AttachmentSource::FileId { file_id } => {
            let file_id = file_id.trim();
            (!file_id.is_empty()).then(|| OpenAiInputContent::File {
                file_data: None,
                file_id: Some(file_id.to_owned()),
                file_url: None,
                filename,
            })
        }
        AttachmentSource::Url { url } => {
            let file_url = url.trim();
            (!file_url.is_empty()).then(|| OpenAiInputContent::File {
                file_data: None,
                file_id: None,
                file_url: Some(file_url.to_owned()),
                filename,
            })
        }
        AttachmentSource::LocalPath { .. } => None,
    }
}

fn responses_content_from_attachment(item: &AttachmentItem) -> OpenAiInputContent {
    match item.kind {
        AttachmentKind::Image => utils::attachment_media_url(item)
            .map(|image_url| OpenAiInputContent::Image { image_url })
            .unwrap_or_else(|| OpenAiInputContent::Text {
                text: utils::attachment_hint_text(item),
            }),
        AttachmentKind::Audio
        | AttachmentKind::Video
        | AttachmentKind::Pdf
        | AttachmentKind::File => {
            responses_file_content(item).unwrap_or_else(|| OpenAiInputContent::Text {
                text: utils::attachment_hint_text(item),
            })
        }
    }
}

fn push_responses_text_message(
    input: &mut Vec<OpenAiResponsesInputItem>,
    role: &str,
    text: String,
) {
    if text.trim().is_empty() {
        return;
    }

    input.push(OpenAiResponsesInputItem::Message(OpenAiInputMessage {
        role: role.to_owned(),
        content: vec![OpenAiInputContent::Text { text }],
    }));
}

fn push_responses_message_from_parts(
    input: &mut Vec<OpenAiResponsesInputItem>,
    role: &str,
    parts: &[utils::ProjectedSessionPart],
) {
    let content = responses_input_contents_from_parts(parts);
    if content.is_empty() {
        return;
    }

    input.push(OpenAiResponsesInputItem::Message(OpenAiInputMessage {
        role: role.to_owned(),
        content,
    }));
}

fn append_responses_items_for_message(
    input: &mut Vec<OpenAiResponsesInputItem>,
    message: &crate::message::Message,
) {
    let projected_parts = utils::project_session_parts(message);
    match message.role {
        Role::System => push_responses_text_message(
            input,
            "system",
            session_text_lossy(message, &projected_parts),
        ),
        Role::User => {
            if projected_parts.is_empty() {
                push_responses_text_message(input, "user", message.as_text_lossy());
            } else {
                push_responses_message_from_parts(input, "user", projected_parts.as_slice());
            }
        }
        Role::Assistant => {
            if projected_parts.is_empty() {
                push_responses_text_message(input, "assistant", message.as_text_lossy());
            } else {
                let mut text_chunks = Vec::new();
                for part in projected_parts {
                    match part {
                        utils::ProjectedSessionPart::Text { text } => text_chunks.push(text),
                        utils::ProjectedSessionPart::Attachment { item } => {
                            text_chunks.push(utils::attachment_hint_text(&item));
                        }
                        utils::ProjectedSessionPart::ToolCall {
                            id,
                            name,
                            arguments_json,
                        } => {
                            if !id.trim().is_empty() && !name.trim().is_empty() {
                                input.push(OpenAiResponsesInputItem::FunctionCall(
                                    OpenAiFunctionCallItem {
                                        kind: "function_call",
                                        call_id: id,
                                        name,
                                        arguments: arguments_json,
                                    },
                                ));
                            }
                        }
                        utils::ProjectedSessionPart::ToolResult {
                            tool_call_id,
                            output_json,
                        } => {
                            if !tool_call_id.trim().is_empty() {
                                input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                                    OpenAiFunctionCallOutputItem {
                                        kind: "function_call_output",
                                        call_id: tool_call_id,
                                        output: serde_json::Value::String(output_json),
                                    },
                                ));
                            }
                        }
                    }
                }

                if !text_chunks.is_empty() {
                    push_responses_text_message(input, "assistant", text_chunks.join(""));
                }
            }
        }
        Role::Tool => {
            if projected_parts.is_empty() {
                push_responses_text_message(input, "user", message.as_text_lossy());
            } else {
                let tool_result = utils::first_tool_result(projected_parts.as_slice());
                let extra_parts = utils::non_tool_result_parts(projected_parts.as_slice());

                if let Some((tool_call_id, output_json)) = tool_result {
                    if tool_call_id.trim().is_empty() {
                        let mut fallback_parts =
                            vec![utils::ProjectedSessionPart::Text { text: output_json }];
                        fallback_parts.extend(extra_parts);
                        push_responses_message_from_parts(input, "user", fallback_parts.as_slice());
                    } else {
                        input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                            OpenAiFunctionCallOutputItem {
                                kind: "function_call_output",
                                call_id: tool_call_id,
                                output: multimodal_function_output_value(
                                    output_json.as_str(),
                                    extra_parts.as_slice(),
                                ),
                            },
                        ));
                    }
                } else {
                    push_responses_message_from_parts(input, "user", projected_parts.as_slice());
                }
            }
        }
    }
}

fn responses_input_contents_from_parts(
    parts: &[utils::ProjectedSessionPart],
) -> Vec<OpenAiInputContent> {
    parts
        .iter()
        .map(|part| match part {
            utils::ProjectedSessionPart::Text { text } => {
                OpenAiInputContent::Text { text: text.clone() }
            }
            utils::ProjectedSessionPart::Attachment { item } => {
                responses_content_from_attachment(item)
            }
            utils::ProjectedSessionPart::ToolCall { name, .. } => OpenAiInputContent::Text {
                text: format!("[tool_call:{name}]"),
            },
            utils::ProjectedSessionPart::ToolResult { tool_call_id, .. } => {
                OpenAiInputContent::Text {
                    text: format!("[tool_result:{tool_call_id}]"),
                }
            }
        })
        .collect()
}

fn multimodal_function_output_value(
    output_json: &str,
    extra_parts: &[utils::ProjectedSessionPart],
) -> serde_json::Value {
    if extra_parts.is_empty() {
        return serde_json::Value::String(output_json.to_owned());
    }

    let mut content = Vec::new();
    if !output_json.trim().is_empty() {
        content.push(OpenAiInputContent::Text {
            text: output_json.to_owned(),
        });
    }
    content.extend(responses_input_contents_from_parts(extra_parts));
    serde_json::to_value(content).expect("copilot function_call_output content should serialize")
}

fn session_text_lossy(
    message: &crate::message::Message,
    projected_parts: &[utils::ProjectedSessionPart],
) -> String {
    if projected_parts.is_empty() {
        message.as_text_lossy()
    } else {
        utils::projected_parts_text_lossy(projected_parts)
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatToolDefinition>>,
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

impl ChatCompletionRequest {
    fn from_request(model: String, request: &CompletionRequest) -> Self {
        let messages = build_chat_messages(request);

        Self {
            model,
            messages,
            tools: (!request.tools.is_empty()).then(|| chat_tools(request.tools.as_slice())),
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            stream: false,
            stream_options: None,
        }
    }

    fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self.stream_options = stream.then_some(ChatStreamOptions {
            include_usage: true,
        });
        self
    }
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
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

#[derive(Debug, Serialize)]
struct ChatToolDefinition {
    #[serde(rename = "type")]
    kind: String,
    function: ChatFunctionDefinition,
}

#[derive(Debug, Serialize)]
struct ChatFunctionDefinition {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<ChatMessageOut>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatMessageOut {
    #[serde(default)]
    content: Option<serde_json::Value>,
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
struct OpenAiResponsesRequest {
    model: String,
    input: Vec<OpenAiResponsesInputItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiResponsesTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

impl OpenAiResponsesRequest {
    fn from_request(model: String, request: &CompletionRequest) -> Self {
        let input = build_responses_input(request);

        Self {
            model,
            input,
            tools: responses_tools(request.tools.as_slice()),
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            stream: false,
        }
    }

    fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesTool {
    #[serde(rename = "type")]
    kind: &'static str,
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct OpenAiInputMessage {
    role: String,
    content: Vec<OpenAiInputContent>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum OpenAiInputContent {
    #[serde(rename = "input_text")]
    Text { text: String },
    #[serde(rename = "input_image")]
    Image { image_url: String },
    #[serde(rename = "input_file")]
    File {
        #[serde(skip_serializing_if = "Option::is_none")]
        file_data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAiResponsesInputItem {
    Message(OpenAiInputMessage),
    FunctionCall(OpenAiFunctionCallItem),
    FunctionCallOutput(OpenAiFunctionCallOutputItem),
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionCallItem {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionCallOutputItem {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: String,
    output: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Option<Vec<OpenAiOutputItem>>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputItem {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    content: Option<Vec<OpenAiOutputContent>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputContent {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    output_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    input_tokens_details: Option<OpenAiInputTokenDetails>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputTokenDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OpenAiInputTokenDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

#[derive(Debug, Default)]
struct ChatToolCallState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Default)]
struct ResponsesToolState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

fn parse_chat_tool_calls(
    calls: Option<&Vec<ChatToolCall>>,
) -> Result<Vec<crate::provider::CompletionToolCall>, AppError> {
    calls
        .into_iter()
        .flatten()
        .map(|call| {
            let id = utils::normalize_optional_text(call.id.clone()).ok_or_else(|| {
                AppError::Provider(
                    "copilot chat completion returned tool_call without id".to_owned(),
                )
            })?;

            let function = call.function.as_ref().ok_or_else(|| {
                AppError::Provider(
                    "copilot chat completion returned tool_call without function".to_owned(),
                )
            })?;

            let name = utils::normalize_optional_text(function.name.clone()).ok_or_else(|| {
                AppError::Provider(
                    "copilot chat completion returned tool_call without function.name".to_owned(),
                )
            })?;

            Ok(crate::provider::CompletionToolCall::Function {
                id,
                name,
                arguments_json: function.arguments.clone().unwrap_or_default(),
            })
        })
        .collect()
}

fn extract_chat_content_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn map_openai_usage(u: OpenAiUsage) -> CompletionUsage {
    MessageUsage {
        input_tokens: u.input_tokens.unwrap_or_default(),
        output_tokens: u.output_tokens.unwrap_or_default(),
        reasoning_tokens: u
            .output_tokens_details
            .and_then(|d| d.reasoning_tokens)
            .unwrap_or_default(),
        cache_write_tokens: 0,
        cache_read_tokens: u
            .input_tokens_details
            .and_then(|d| d.cached_tokens)
            .unwrap_or_default(),
        total_cost: 0.0,
    }
    .into()
}

fn select_copilot_bearer_token(refresh: &str, access: &str) -> Option<String> {
    let refresh = refresh.trim();
    if !refresh.is_empty() {
        return Some(refresh.to_owned());
    }

    let access = access.trim();
    if access.is_empty() {
        None
    } else {
        Some(access.to_owned())
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

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;
    use crate::message::{AttachmentItem, AttachmentKind, AttachmentSource, Message, PartContent};

    use crate::provider::CompletionRequest;

    fn mock_provider(base_url: String) -> CopilotProvider {
        CopilotProvider {
            id: "github-copilot".to_owned(),
            client: reqwest::Client::new(),
            bearer_token: "test-token".to_owned(),
            base_url,
            default_model: "gpt-4o-mini".to_owned(),
            models_url: None,
        }
    }

    fn sample_png_data_url() -> &'static str {
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO9W7tYAAAAASUVORK5CYII="
    }

    #[tokio::test]
    async fn list_models_parses_data_envelope() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/models")
            .match_header("authorization", "Bearer test-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "data": [
                        { "id": "gpt-4o-mini", "name": "GPT-4o mini" }
                    ]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let models = mock_provider(server.url())
            .list_models()
            .await
            .expect("list_models should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-4o-mini");
        assert_eq!(models[0].display_name.as_deref(), Some("GPT-4o mini"));
    }

    #[test]
    fn is_vision_request_detects_projected_image_parts() {
        let request = CompletionRequest {
            model: "gpt-5".to_owned(),
            system: None,
            messages: vec![Message::prompt_parts(
                crate::role::Role::User,
                vec![PartContent::attachments(vec![AttachmentItem {
                    kind: AttachmentKind::Image,
                    mime: "image/png".to_owned(),
                    source: AttachmentSource::DataUrl {
                        url: sample_png_data_url().to_owned(),
                    },
                    filename: Some("image.png".to_owned()),
                    title: None,
                    size_bytes: Some(68),
                    sha256: None,
                    width: Some(1),
                    height: Some(1),
                    duration_ms: None,
                    page_count: None,
                }])],
            )],
            tools: Vec::new(),
            temperature: None,
            max_output_tokens: None,
        };

        assert!(CopilotProvider::is_vision_request(&request));
    }

    #[test]
    fn from_auth_prefers_refresh_token_for_bearer() {
        let provider = CopilotProvider::from_auth(
            "github-copilot",
            reqwest::Client::new(),
            &AuthData::OAuth {
                refresh: "refresh-token".to_owned(),
                access: "access-token".to_owned(),
                expires_at_ms: 0,
                account_id: None,
                enterprise_url: None,
            },
        )
        .expect("copilot provider should build from oauth auth data");

        assert_eq!(provider.bearer_token, "refresh-token");
    }

    #[test]
    fn from_auth_requires_non_empty_oauth_token() {
        let err = match CopilotProvider::from_auth(
            "github-copilot",
            reqwest::Client::new(),
            &AuthData::OAuth {
                refresh: "   ".to_owned(),
                access: "".to_owned(),
                expires_at_ms: 0,
                account_id: None,
                enterprise_url: None,
            },
        ) {
            Ok(_) => panic!("provider should reject empty oauth token fields"),
            Err(err) => err,
        };

        assert!(matches!(err, AppError::Config(_)));
    }

    #[tokio::test]
    async fn list_models_rejects_invalid_shape() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "data": [
                        { "name": "missing id" }
                    ]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let err = mock_provider(server.url())
            .list_models()
            .await
            .expect_err("invalid model payload should fail");

        assert!(matches!(err, AppError::SerdeJson(_) | AppError::Http(_)));
    }

    #[tokio::test]
    async fn complete_falls_back_to_chat_when_responses_unsupported() {
        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": { "message": "responses not supported" }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let _chat = server
            .mock("POST", "/chat/completions")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-5",
                    "choices": [{
                        "text": "copilot fallback chat text",
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let mut provider = mock_provider(server.url());
        provider.default_model = "gpt-5".to_owned();

        let response = provider
            .complete(CompletionRequest {
                model: "gpt-5".to_owned(),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("responses 404 should fall back to chat");

        assert_eq!(response.text, "copilot fallback chat text");
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::Stop)
        ));
    }

    #[tokio::test]
    async fn complete_responses_parses_function_tool_calls() {
        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-5",
                    "output_text": "",
                    "stop_reason": "tool_calls",
                    "output": [{
                        "type": "function_call",
                        "call_id": "call_1",
                        "name": "search",
                        "arguments": "{\"q\":\"rust\"}"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let mut provider = mock_provider(server.url());
        provider.default_model = "gpt-5".to_owned();

        let response = provider
            .complete(CompletionRequest {
                model: "gpt-5".to_owned(),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("responses payload should parse tool calls");

        assert_eq!(response.tool_calls.len(), 1);
        match &response.tool_calls[0] {
            crate::provider::CompletionToolCall::Function {
                id,
                name,
                arguments_json,
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "search");
                assert_eq!(arguments_json, "{\"q\":\"rust\"}");
            }
        }
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::ToolCalls)
        ));
    }

    #[tokio::test]
    async fn complete_stream_chat_preserves_usage_emitted_after_finish_reason() {
        let mut server = mockito::Server::new_async().await;
        let _chat = server
            .mock("POST", "/chat/completions")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
                "data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2}}\n\n",
                "data: [DONE]\n\n"
            ))
            .create_async()
            .await;

        let mut provider = mock_provider(server.url());
        provider.default_model = "gpt-4o".to_owned();

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: "gpt-4o".to_owned(),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("chat stream should start");

        let mut done = false;
        while let Some(item) = stream.next().await {
            if let CompletionStreamEvent::Completed {
                finish_reason,
                usage,
                ..
            } = item.expect("stream event should parse")
            {
                assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                let usage = usage.expect("usage should be present");
                assert_eq!(usage.input_tokens, 4);
                assert_eq!(usage.output_tokens, 2);
                done = true;
            }
        }

        assert!(done);
    }

    #[tokio::test]
    async fn complete_stream_responses_emits_tool_call_delta() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"search\",\"arguments\":\"\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"part1\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"part2\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Done\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"stop_reason\":\"tool_calls\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n",
            "data: [DONE]\n\n"
        );

        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let mut provider = mock_provider(server.url());
        provider.default_model = "gpt-5".to_owned();

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: "gpt-5".to_owned(),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(64),
            })
            .await
            .expect("responses stream should start");

        let mut text = String::new();
        let mut tool = String::new();
        let mut done = false;
        while let Some(item) = stream.next().await {
            match item.expect("stream item should parse") {
                CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(delta.as_str()),
                CompletionStreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                    ..
                } => {
                    assert_eq!(id.as_deref(), Some("call_1"));
                    assert_eq!(name.as_deref(), Some("search"));
                    tool.push_str(arguments_delta.as_str());
                }
                CompletionStreamEvent::Completed {
                    finish_reason,
                    usage,
                    ..
                } => {
                    assert!(matches!(
                        finish_reason,
                        Some(CompletionFinishReason::ToolCalls)
                    ));
                    let usage = usage.expect("usage should be present");
                    assert_eq!(usage.input_tokens, 3);
                    assert_eq!(usage.output_tokens, 2);
                    done = true;
                }
            }
        }

        assert_eq!(text, "Done");
        assert_eq!(tool, "part1part2");
        assert!(done);
    }
}
