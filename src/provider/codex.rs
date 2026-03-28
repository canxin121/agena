use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::{
    auth::{AuthData, AuthStore, refresh_openai_token},
    error::AppError,
    message::MessageUsage,
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionUsage, ModelProvider, ProviderModel, StreamResumePolicy, sse, utils,
    },
    role::Role,
};

const PROVIDER_ID: &str = "openai";
const CODEX_API_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
const MAX_AUTH_RETRY_ATTEMPTS: usize = 2;
const DEFAULT_MODEL: &str = "gpt-5.3-codex";

pub struct CodexProvider {
    client: reqwest::Client,
    auth_store: Arc<dyn AuthStore>,
    state: Mutex<CodexAuthState>,
    default_model: String,
}

impl CodexProvider {
    pub fn from_auth(
        client: reqwest::Client,
        auth_store: Arc<dyn AuthStore>,
        auth: &AuthData,
    ) -> Result<Self, AppError> {
        Self::from_auth_with_default_model(client, auth_store, auth, DEFAULT_MODEL)
    }

    pub fn from_auth_with_default_model(
        client: reqwest::Client,
        auth_store: Arc<dyn AuthStore>,
        auth: &AuthData,
        default_model: impl Into<String>,
    ) -> Result<Self, AppError> {
        let AuthData::OAuth {
            refresh,
            access,
            expires_at_ms,
            account_id,
            ..
        } = auth
        else {
            return Err(AppError::Config(
                "openai codex provider requires oauth credential".to_owned(),
            ));
        };

        Ok(Self {
            client,
            auth_store,
            state: Mutex::new(CodexAuthState {
                refresh: refresh.clone(),
                access: access.clone(),
                expires_at_ms: *expires_at_ms,
                account_id: account_id.clone(),
            }),
            default_model: default_model.into(),
        })
    }

    fn auth_data_from_state(state: &CodexAuthState) -> AuthData {
        AuthData::OAuth {
            refresh: state.refresh.clone(),
            access: state.access.clone(),
            expires_at_ms: state.expires_at_ms,
            account_id: state.account_id.clone(),
            enterprise_url: None,
        }
    }

    async fn refresh_and_persist(
        &self,
        snapshot: CodexAuthState,
    ) -> Result<CodexAuthState, AppError> {
        let refreshed = refresh_openai_token(snapshot.refresh.as_str()).await?;
        let updated = CodexAuthState {
            refresh: refreshed.refresh,
            access: refreshed.access,
            expires_at_ms: refreshed.expires_at_ms,
            account_id: refreshed.account_id.or(snapshot.account_id),
        };

        {
            let mut guard = self
                .state
                .lock()
                .map_err(|_| AppError::Internal("codex state lock poisoned".to_owned()))?;
            *guard = updated.clone();
        }

        self.auth_store
            .set(PROVIDER_ID, Self::auth_data_from_state(&updated))?;

        Ok(updated)
    }

    async fn force_refresh_access_token(&self) -> Result<CodexAuthState, AppError> {
        let snapshot = self
            .state
            .lock()
            .map_err(|_| AppError::Internal("codex state lock poisoned".to_owned()))?
            .clone();
        self.refresh_and_persist(snapshot).await
    }

    async fn ensure_access_token(&self) -> Result<CodexAuthState, AppError> {
        let snapshot = self
            .state
            .lock()
            .map_err(|_| AppError::Internal("codex state lock poisoned".to_owned()))?
            .clone();

        let now_ms = chrono::Utc::now().timestamp_millis();
        let needs_refresh = snapshot.access.trim().is_empty()
            || (snapshot.expires_at_ms > 0 && snapshot.expires_at_ms <= now_ms);
        if !needs_refresh {
            return Ok(snapshot);
        }

        self.refresh_and_persist(snapshot).await
    }

    async fn send_request(
        &self,
        auth: &CodexAuthState,
        body: &OpenAiResponsesRequest,
    ) -> Result<reqwest::Response, AppError> {
        let mut req = self
            .client
            .post(CODEX_API_ENDPOINT)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("originator", "agena")
            .header(reqwest::header::USER_AGENT, "agena/0.1.0")
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", auth.access),
            )
            .json(body);

        if let Some(account_id) = auth.account_id.as_ref().filter(|s| !s.trim().is_empty()) {
            req = req.header("ChatGPT-Account-Id", account_id);
        }

        req.send().await.map_err(AppError::from)
    }

    async fn send_with_retry_on_unauthorized(
        &self,
        body: &OpenAiResponsesRequest,
    ) -> Result<reqwest::Response, AppError> {
        let mut auth = self.ensure_access_token().await?;

        for attempt in 0..MAX_AUTH_RETRY_ATTEMPTS {
            let response = self.send_request(&auth, body).await?;
            if !matches!(
                response.status(),
                reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
            ) {
                return Ok(response);
            }

            if attempt + 1 >= MAX_AUTH_RETRY_ATTEMPTS {
                return Ok(response);
            }

            auth = self.force_refresh_access_token().await?;
        }

        Err(AppError::Internal(
            "codex auth retry loop exited unexpectedly".to_owned(),
        ))
    }

    fn to_responses_input(request: &CompletionRequest) -> Vec<OpenAiResponsesInputItem> {
        let mut input = Vec::new();

        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            Self::push_responses_text_message(&mut input, "system", system.clone());
        }

        for message in &request.messages {
            Self::append_responses_items_for_message(&mut input, message);
        }

        input
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
            content: vec![OpenAiInputContent {
                kind: "input_text".to_owned(),
                text,
            }],
        }));
    }

    fn append_responses_items_for_message(
        input: &mut Vec<OpenAiResponsesInputItem>,
        message: &crate::message::Message,
    ) {
        let projected_parts = utils::project_session_parts(message);

        match message.role {
            Role::System => Self::push_responses_text_message(
                input,
                "system",
                session_text_lossy(message, projected_parts.as_slice()),
            ),
            Role::User => Self::push_responses_text_message(
                input,
                "user",
                session_text_lossy(message, projected_parts.as_slice()),
            ),
            Role::Assistant => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "assistant", message.as_text_lossy());
                } else {
                    let mut text_chunks = Vec::new();
                    for part in projected_parts {
                        match part {
                            utils::ProjectedSessionPart::Text { text } => text_chunks.push(text),
                            utils::ProjectedSessionPart::ImageUrl { url } => {
                                text_chunks.push(format!("[image:{url}]"));
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
                                            output: output_json,
                                        },
                                    ));
                                }
                            }
                        }
                    }

                    if !text_chunks.is_empty() {
                        Self::push_responses_text_message(input, "assistant", text_chunks.join(""));
                    }
                }
            }
            Role::Tool => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "user", message.as_text_lossy());
                } else {
                    let mut fallback_text = Vec::new();
                    let mut emitted_output = false;
                    for part in projected_parts {
                        match part {
                            utils::ProjectedSessionPart::ToolResult {
                                tool_call_id,
                                output_json,
                            } => {
                                if tool_call_id.trim().is_empty() {
                                    fallback_text.push(output_json);
                                    continue;
                                }
                                emitted_output = true;
                                input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                                    OpenAiFunctionCallOutputItem {
                                        kind: "function_call_output",
                                        call_id: tool_call_id,
                                        output: output_json,
                                    },
                                ));
                            }
                            utils::ProjectedSessionPart::Text { text } => fallback_text.push(text),
                            utils::ProjectedSessionPart::ImageUrl { url } => {
                                fallback_text.push(format!("[image:{url}]"));
                            }
                            utils::ProjectedSessionPart::ToolCall { name, .. } => {
                                fallback_text.push(format!("[tool_call:{name}]"));
                            }
                        }
                    }

                    if !emitted_output || !fallback_text.is_empty() {
                        Self::push_responses_text_message(input, "user", fallback_text.join(""));
                    }
                }
            }
        }
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
                            "codex responses payload returned function_call without id/call_id"
                                .to_owned(),
                        )
                    })?;

                let name = utils::normalize_optional_text(item.name.clone()).ok_or_else(|| {
                    AppError::Provider(
                        "codex responses payload returned function_call without name".to_owned(),
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

#[async_trait]
impl ModelProvider for CodexProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![ProviderModel {
            provider_id: PROVIDER_ID.to_owned(),
            id: self.default_model.clone(),
            display_name: Some("Codex OAuth model".to_owned()),
        }])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = if request.model.trim().is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        let input = Self::to_responses_input(&request);

        let body = OpenAiResponsesRequest::new(
            model.clone(),
            input,
            request.max_output_tokens,
            request.temperature,
            false,
        );

        let response = self.send_with_retry_on_unauthorized(&body).await?;
        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

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

        let finish_reason = CompletionFinishReason::from_provider(payload.stop_reason.as_deref());
        let tool_calls = Self::parse_responses_tool_calls(payload.output.as_ref())?;

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(
                "codex responses payload was empty without finish reason".to_owned(),
            ));
        }

        Ok(CompletionResponse {
            provider_id: PROVIDER_ID.to_owned(),
            model: payload.model.unwrap_or(model),
            text,
            finish_reason,
            tool_calls,
            usage: payload.usage.map(map_openai_usage),
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

        let input = Self::to_responses_input(&request);

        let body = OpenAiResponsesRequest::new(
            model.clone(),
            input,
            request.max_output_tokens,
            request.temperature,
            true,
        );

        let response = self.send_with_retry_on_unauthorized(&body).await?;
        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

        let mut event_stream = sse::json_events(response);
        let provider_id = PROVIDER_ID.to_owned();
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ResponsesToolState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut completed_emitted = false;

            while let Some(event) = event_stream.next().await {
                let event = event?;

                if let Some(err) = utils::responses_stream_error(PROVIDER_ID, &event)? {
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

                if let Some(tool_event) = utils::responses_tool_event(PROVIDER_ID, &event)? {
                    let key = tool_event.stream_key(PROVIDER_ID)?;

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
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
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
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
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
                        PROVIDER_ID,
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

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Clone)]
struct CodexAuthState {
    refresh: String,
    access: String,
    expires_at_ms: i64,
    account_id: Option<String>,
}

#[derive(Debug, Default)]
struct ResponsesToolState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesRequest {
    model: String,
    input: Vec<OpenAiResponsesInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

impl OpenAiResponsesRequest {
    fn new(
        model: String,
        input: Vec<OpenAiResponsesInputItem>,
        max_output_tokens: Option<u32>,
        temperature: Option<f32>,
        stream: bool,
    ) -> Self {
        Self {
            model,
            input,
            max_output_tokens,
            temperature,
            stream,
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenAiInputMessage {
    role: String,
    content: Vec<OpenAiInputContent>,
}

#[derive(Debug, Serialize)]
struct OpenAiInputContent {
    #[serde(rename = "type")]
    kind: String,
    text: String,
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
    output: String,
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
