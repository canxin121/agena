use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::{
    error::AppError,
    message::{AttachmentItem, AttachmentKind, AttachmentSource, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionUsage, ModelProvider, ProviderModel, StreamResumePolicy,
        auth::{AuthData, AuthStore, refresh_openai_token},
        sse, utils, wire_message,
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
    auth_provider_id: String,
    state: Mutex<CodexAuthState>,
    default_model: ModelId,
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
        Self::from_auth_with_options(client, auth_store, auth, default_model, PROVIDER_ID)
    }

    pub fn from_auth_with_options(
        client: reqwest::Client,
        auth_store: Arc<dyn AuthStore>,
        auth: &AuthData,
        default_model: impl Into<String>,
        auth_provider_id: impl Into<String>,
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
            auth_provider_id: auth_provider_id.into(),
            state: Mutex::new(CodexAuthState {
                refresh: refresh.clone(),
                access: access.clone(),
                expires_at_ms: *expires_at_ms,
                account_id: account_id.clone(),
            }),
            default_model: ModelId::new(default_model),
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

        self.auth_store.set(
            self.auth_provider_id.as_str(),
            Self::auth_data_from_state(&updated),
        )?;

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

        if let Some(window_id) = body.window_id_header() {
            req = req.header("x-codex-window-id", window_id);
        }

        // chat.headers plugin chain: lets plugins add/remove headers per
        // request without touching codex internals.
        req = super::utils::apply_request_headers(PROVIDER_ID, req, &Default::default());

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

    fn responses_tools(tools: &[crate::tool::EntryDefinition]) -> Vec<OpenAiResponsesTool> {
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
        wire_message::filename(item)
            .map(str::to_owned)
            .unwrap_or_else(|| item.summary_label())
    }

    fn responses_file_content(item: &AttachmentItem) -> Option<OpenAiInputContent> {
        let filename = Some(Self::attachment_upload_name(item));
        match &item.source {
            AttachmentSource::Base64 { .. } | AttachmentSource::DataUrl { .. } => {
                wire_message::data_url(item).map(|file_data| OpenAiInputContent::File {
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
            AttachmentKind::Image => wire_message::media_url(item)
                .map(|image_url| OpenAiInputContent::Image { image_url })
                .unwrap_or_else(|| OpenAiInputContent::Text {
                    text: wire_message::hint_text(item),
                }),
            AttachmentKind::Audio
            | AttachmentKind::Video
            | AttachmentKind::Pdf
            | AttachmentKind::File => {
                Self::responses_file_content(item).unwrap_or_else(|| OpenAiInputContent::Text {
                    text: wire_message::hint_text(item),
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

    fn flush_assistant_responses_text(
        input: &mut Vec<OpenAiResponsesInputItem>,
        text_chunks: &mut Vec<String>,
    ) {
        if text_chunks.is_empty() {
            return;
        }

        let text = text_chunks.join("");
        text_chunks.clear();
        Self::push_responses_text_message(input, "assistant", text);
    }

    fn push_responses_message_from_parts(
        input: &mut Vec<OpenAiResponsesInputItem>,
        role: &str,
        parts: &[wire_message::WirePart],
    ) {
        let content = Self::responses_input_contents_from_parts(parts);
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
        let projected_parts = wire_message::project(message);

        match message.role {
            Role::System => Self::push_responses_text_message(
                input,
                "system",
                session_text_lossy(message, projected_parts.as_slice()),
            ),
            Role::User => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "user", message.as_text_lossy());
                } else {
                    Self::push_responses_message_from_parts(
                        input,
                        "user",
                        projected_parts.as_slice(),
                    );
                }
            }
            Role::Assistant => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "assistant", message.as_text_lossy());
                } else {
                    let mut text_chunks = Vec::new();
                    for part in projected_parts {
                        match part {
                            wire_message::WirePart::Text { text } => text_chunks.push(text),
                            wire_message::WirePart::Attachment { item } => {
                                text_chunks.push(wire_message::hint_text(&item));
                            }
                            wire_message::WirePart::ToolCall {
                                id,
                                name,
                                arguments_json,
                            } => {
                                Self::flush_assistant_responses_text(input, &mut text_chunks);
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
                            wire_message::WirePart::ToolResult {
                                tool_call_id,
                                output_json,
                                ..
                            } => {
                                Self::flush_assistant_responses_text(input, &mut text_chunks);
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

                    Self::flush_assistant_responses_text(input, &mut text_chunks);
                }
            }
            Role::Tool => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "user", message.as_text_lossy());
                } else {
                    let tool_results = wire_message::tool_results(projected_parts.as_slice());
                    let extra_parts =
                        wire_message::non_tool_result_parts(projected_parts.as_slice());

                    if tool_results.len() > 1 {
                        let mut buffered_parts = Vec::new();
                        for part in projected_parts {
                            match part {
                                wire_message::WirePart::ToolResult {
                                    tool_call_id,
                                    output_json,
                                    ..
                                } => {
                                    if !buffered_parts.is_empty() {
                                        Self::push_responses_message_from_parts(
                                            input,
                                            "user",
                                            buffered_parts.as_slice(),
                                        );
                                        buffered_parts.clear();
                                    }

                                    if tool_call_id.trim().is_empty() {
                                        buffered_parts.push(wire_message::WirePart::Text {
                                            text: output_json,
                                        });
                                    } else {
                                        input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                                            OpenAiFunctionCallOutputItem {
                                                kind: "function_call_output",
                                                call_id: tool_call_id,
                                                output: serde_json::Value::String(output_json),
                                            },
                                        ));
                                    }
                                }
                                other => buffered_parts.push(other),
                            }
                        }

                        if !buffered_parts.is_empty() {
                            Self::push_responses_message_from_parts(
                                input,
                                "user",
                                buffered_parts.as_slice(),
                            );
                        }
                    } else if let Some((tool_call_id, output_json)) =
                        tool_results.into_iter().next()
                    {
                        if tool_call_id.trim().is_empty() {
                            let mut fallback_parts =
                                vec![wire_message::WirePart::Text { text: output_json }];
                            fallback_parts.extend(extra_parts);
                            Self::push_responses_message_from_parts(
                                input,
                                "user",
                                fallback_parts.as_slice(),
                            );
                        } else {
                            input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                                OpenAiFunctionCallOutputItem {
                                    kind: "function_call_output",
                                    call_id: tool_call_id,
                                    output: Self::multimodal_function_output_value(
                                        output_json.as_str(),
                                        extra_parts.as_slice(),
                                    ),
                                },
                            ));
                        }
                    } else {
                        Self::push_responses_message_from_parts(
                            input,
                            "user",
                            projected_parts.as_slice(),
                        );
                    }
                }
            }
        }
    }

    fn responses_input_contents_from_parts(
        parts: &[wire_message::WirePart],
    ) -> Vec<OpenAiInputContent> {
        parts
            .iter()
            .map(|part| match part {
                wire_message::WirePart::Text { text } => {
                    OpenAiInputContent::Text { text: text.clone() }
                }
                wire_message::WirePart::Attachment { item } => {
                    Self::responses_content_from_attachment(item)
                }
                wire_message::WirePart::ToolCall { name, .. } => OpenAiInputContent::Text {
                    text: format!("[tool_call:{name}]"),
                },
                wire_message::WirePart::ToolResult { tool_call_id, .. } => {
                    OpenAiInputContent::Text {
                        text: format!("[tool_result:{tool_call_id}]"),
                    }
                }
            })
            .collect()
    }

    fn multimodal_function_output_value(
        output_json: &str,
        extra_parts: &[wire_message::WirePart],
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
        content.extend(Self::responses_input_contents_from_parts(extra_parts));
        serde_json::to_value(content).expect("codex function_call_output content should serialize")
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
    projected_parts: &[wire_message::WirePart],
) -> String {
    if projected_parts.is_empty() {
        message.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(projected_parts)
    }
}

fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    response_id.map(|response_id| serde_json::json!({ "response_id": response_id }))
}

#[async_trait]
impl ModelProvider for CodexProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(crate::provider::CapabilityFamily::OpenAi)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn supports_prompt_continuation(&self, _model: &ModelId) -> bool {
        true
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        let auth_account_id = self
            .state
            .lock()
            .ok()
            .and_then(|state| utils::normalize_optional_text(state.account_id.clone()));
        Some(
            crate::provider::PromptCacheShape::new(PROVIDER_ID)
                .with_string("auth_provider_id", self.auth_provider_id.as_str())
                .with_optional_string("auth_account_id", auth_account_id)
                .with_string("default_model", self.default_model.as_str())
                .with_string("endpoint", CODEX_API_ENDPOINT),
        )
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new(PROVIDER_ID, self.default_model.clone())
                .with_display_name("Codex OAuth model")
                .with_capabilities(self.model_capabilities(&self.default_model)),
        ])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = request.model.clone();

        let input = Self::to_responses_input(&request);

        let body = OpenAiResponsesRequest::new(
            model.to_string(),
            input,
            request.tools.as_slice(),
            request.max_output_tokens,
            request.temperature,
            request.prompt_cache_key.clone(),
            request.previous_response_id.clone(),
            request.prompt_window_generation,
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
            provider_id: ProviderId::new(PROVIDER_ID),
            model: ModelId::new(payload.model.unwrap_or_else(|| model.to_string())),
            text,
            reasoning_text: None,
            finish_reason,
            tool_calls,
            usage: payload.usage.map(map_openai_usage),
            provider_metadata: response_id_metadata(payload.id),
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

        let input = Self::to_responses_input(&request);

        let body = OpenAiResponsesRequest::new(
            model.to_string(),
            input,
            request.tools.as_slice(),
            request.max_output_tokens,
            request.temperature,
            request.prompt_cache_key.clone(),
            request.previous_response_id.clone(),
            request.prompt_window_generation,
            true,
        );

        let response = self.send_with_retry_on_unauthorized(&body).await?;
        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

        let mut event_stream = sse::json_events(response);
        let provider_id = ProviderId::new(PROVIDER_ID);
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ResponsesToolState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut completed_emitted = false;
            let mut response_id: Option<String> = None;

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

                if let Some(next_response_id) = utils::responses_response_id(&event) {
                    response_id = Some(next_response_id);
                }

                if utils::responses_is_completed(&event) {
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason: CompletionFinishReason::from_provider(
                            stream_finish_reason.as_deref(),
                        ),
                        usage: stream_usage.clone(),
                        provider_metadata: response_id_metadata(response_id.clone()),
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
                    provider_metadata: response_id_metadata(response_id),
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiResponsesTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    #[serde(skip)]
    prompt_window_generation: Option<u64>,
    stream: bool,
}

impl OpenAiResponsesRequest {
    #[allow(clippy::too_many_arguments)]
    fn new(
        model: String,
        input: Vec<OpenAiResponsesInputItem>,
        tools: &[crate::tool::EntryDefinition],
        max_output_tokens: Option<u32>,
        temperature: Option<f32>,
        prompt_cache_key: Option<String>,
        previous_response_id: Option<String>,
        prompt_window_generation: Option<u64>,
        stream: bool,
    ) -> Self {
        Self {
            model,
            input,
            tools: CodexProvider::responses_tools(tools),
            max_output_tokens,
            temperature,
            prompt_cache_key,
            previous_response_id,
            prompt_window_generation,
            stream,
        }
    }

    fn window_id_header(&self) -> Option<String> {
        self.prompt_cache_key.as_ref().map(|prompt_cache_key| {
            format!(
                "{}:{}",
                prompt_cache_key,
                self.prompt_window_generation.unwrap_or_default()
            )
        })
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
    id: Option<String>,
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use super::*;
    use crate::message::{
        AttachmentItem, AttachmentKind, AttachmentSource, Message, PartContent, StructuredObject,
        TimeRange, ToolExecutionPart, ToolInvocation, ToolOutput,
    };
    use crate::tool::{EntryBehavior, EntryDefinition};

    #[derive(Default)]
    struct MemoryAuthStore {
        values: Mutex<HashMap<String, crate::provider::auth::AuthData>>,
    }

    impl crate::provider::auth::AuthStore for MemoryAuthStore {
        fn all(&self) -> Result<HashMap<String, crate::provider::auth::AuthData>, AppError> {
            Ok(self
                .values
                .lock()
                .expect("auth store lock should succeed")
                .clone())
        }

        fn get(
            &self,
            provider_id: &str,
        ) -> Result<Option<crate::provider::auth::AuthData>, AppError> {
            Ok(self
                .values
                .lock()
                .expect("auth store lock should succeed")
                .get(provider_id)
                .cloned())
        }

        fn set(
            &self,
            provider_id: &str,
            auth: crate::provider::auth::AuthData,
        ) -> Result<(), AppError> {
            self.values
                .lock()
                .expect("auth store lock should succeed")
                .insert(provider_id.to_owned(), auth);
            Ok(())
        }

        fn remove(&self, provider_id: &str) -> Result<(), AppError> {
            self.values
                .lock()
                .expect("auth store lock should succeed")
                .remove(provider_id);
            Ok(())
        }
    }

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

    fn sample_png_data_url() -> &'static str {
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO9W7tYAAAAASUVORK5CYII="
    }

    fn tool_result_message_with_image(tool_call_id: &str) -> Message {
        let mut message = Message::prompt_parts(
            crate::role::Role::Tool,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 0,
                    invocation: ToolInvocation {
                        name: "tool".to_owned(),
                        input: StructuredObject::default(),
                    },
                    output_text: "{\"ok\":true}".to_owned(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: ToolOutput::default(),
                    lifecycle: TimeRange::default(),
                }),
                PartContent::attachments(vec![AttachmentItem {
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
                }]),
            ],
        );
        if let Some(part) = message.parts.first_mut() {
            part.operation_id = Some(tool_call_id.to_owned());
        }
        message
    }

    fn multi_tool_result_message(tool_call_ids: &[&str]) -> Message {
        let mut parts = Vec::new();
        for (index, _) in tool_call_ids.iter().enumerate() {
            parts.push(PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: index as i64,
                invocation: ToolInvocation {
                    name: format!("tool_{index}"),
                    input: StructuredObject::default(),
                },
                output_text: format!("{{\"result\":{index}}}"),
                blocks: Vec::new(),
                attachments: Vec::new(),
                details: ToolOutput::default(),
                lifecycle: TimeRange::default(),
            }));
        }

        let mut message = Message::prompt_parts(crate::role::Role::Tool, parts);
        for (index, tool_call_id) in tool_call_ids.iter().enumerate() {
            if let Some(part) = message.parts.get_mut(index) {
                part.operation_id = Some((*tool_call_id).to_owned());
            }
        }
        message
    }

    #[test]
    fn responses_request_serializes_tools() {
        let request = OpenAiResponsesRequest::new(
            "gpt-5.3-codex".to_string(),
            Vec::new(),
            &[sample_tool_definition()],
            Some(128),
            None,
            None,
            None,
            None,
            false,
        );

        let json = serde_json::to_value(request).expect("request should serialize");
        assert_eq!(json["tools"][0]["name"], "project_search");
        assert_eq!(
            json["tools"][0]["parameters"]["properties"]["query"]["type"],
            "string"
        );
    }

    #[test]
    fn responses_request_serializes_cache_fields_and_window_header() {
        let request = OpenAiResponsesRequest::new(
            "gpt-5.3-codex".to_string(),
            Vec::new(),
            &[sample_tool_definition()],
            Some(128),
            Some(0.2),
            Some("session-42".to_string()),
            Some("resp_prev".to_string()),
            Some(4),
            true,
        );

        let json = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(json["prompt_cache_key"], "session-42");
        assert_eq!(json["previous_response_id"], "resp_prev");
        assert_eq!(request.window_id_header().as_deref(), Some("session-42:4"));
    }

    #[test]
    fn prompt_cache_shape_changes_when_account_id_changes() {
        let auth_store = Arc::new(MemoryAuthStore::default());
        let provider_a = CodexProvider::from_auth_with_options(
            reqwest::Client::new(),
            auth_store.clone(),
            &crate::provider::auth::AuthData::OAuth {
                refresh: "refresh-a".to_owned(),
                access: "access-a".to_owned(),
                expires_at_ms: 0,
                account_id: Some("acct-a".to_owned()),
                enterprise_url: None,
            },
            "gpt-5.3-codex",
            "openai",
        )
        .expect("codex provider should construct");
        let provider_b = CodexProvider::from_auth_with_options(
            reqwest::Client::new(),
            auth_store,
            &crate::provider::auth::AuthData::OAuth {
                refresh: "refresh-b".to_owned(),
                access: "access-b".to_owned(),
                expires_at_ms: 0,
                account_id: Some("acct-b".to_owned()),
                enterprise_url: None,
            },
            "gpt-5.3-codex",
            "openai",
        )
        .expect("codex provider should construct");

        let shape_a = provider_a
            .prompt_cache_shape(&crate::model::ModelId::new("gpt-5.3-codex"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&crate::model::ModelId::new("gpt-5.3-codex"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }

    #[test]
    fn responses_input_encodes_tool_result_images_as_multimodal_function_output() {
        let request = CompletionRequest {
            model: crate::model::ModelId::new("gpt-5.3-codex"),
            system: None,
            messages: vec![tool_result_message_with_image("call_1")],
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
        };

        let input = CodexProvider::to_responses_input(&request);
        let json = serde_json::to_value(&input).expect("responses input should serialize");
        let items = json.as_array().expect("responses input should be an array");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "call_1");
        assert!(items[0]["output"].is_array());
        assert_eq!(items[0]["output"][0]["type"], "input_text");
        assert_eq!(items[0]["output"][0]["text"], "{\"ok\":true}");
        assert_eq!(items[0]["output"][1]["type"], "input_image");
        assert_eq!(items[0]["output"][1]["image_url"], sample_png_data_url());
    }

    #[test]
    fn responses_input_preserves_assistant_part_order_around_tool_calls() {
        let mut assistant = Message::prompt_text(crate::role::Role::Assistant, "Before ");
        assistant.push_part(crate::message::MessagePart::with_content(
            2,
            assistant.id,
            assistant.created_at,
            crate::message::ExecutionStatus::Completed,
            crate::message::PartContent::ToolExecution(
                crate::message::ToolExecutionPart::Completed {
                    call_id: 1,
                    invocation: crate::message::ToolInvocation {
                        name: "search".to_owned(),
                        input: crate::message::StructuredObject::default(),
                    },
                    output_text: String::new(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: crate::message::ToolOutput::default(),
                    lifecycle: crate::message::TimeRange::default(),
                },
            ),
        ));
        if let Some(part) = assistant.parts.last_mut() {
            part.operation_id = Some("call_1".to_owned());
        }
        assistant.push_part(crate::message::MessagePart::with_content(
            3,
            assistant.id,
            assistant.created_at,
            crate::message::ExecutionStatus::Completed,
            crate::message::PartContent::text("After"),
        ));

        let request = CompletionRequest {
            model: crate::model::ModelId::new("gpt-5.3-codex"),
            system: None,
            messages: vec![assistant],
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
        };

        let input = CodexProvider::to_responses_input(&request);
        let json = serde_json::to_value(&input).expect("responses input should serialize");
        let items = json.as_array().expect("responses input should be an array");

        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["role"], "assistant");
        assert_eq!(items[0]["content"][0]["text"], "Before ");
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["call_id"], "call_1");
        assert_eq!(items[1]["name"], "search");
        assert_eq!(items[2]["role"], "assistant");
        assert_eq!(items[2]["content"][0]["text"], "After");
    }

    #[test]
    fn responses_input_emits_all_tool_results_from_single_tool_message() {
        let request = CompletionRequest {
            model: crate::model::ModelId::new("gpt-5.3-codex"),
            system: None,
            messages: vec![multi_tool_result_message(&["call_1", "call_2"])],
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
        };

        let input = CodexProvider::to_responses_input(&request);
        let json = serde_json::to_value(&input).expect("responses input should serialize");
        let items = json.as_array().expect("responses input should be an array");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "call_1");
        assert_eq!(items[0]["output"], "{\"result\":0}");
        assert_eq!(items[1]["type"], "function_call_output");
        assert_eq!(items[1]["call_id"], "call_2");
        assert_eq!(items[1]["output"], "{\"result\":1}");
    }
}
