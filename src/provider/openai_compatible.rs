use std::collections::HashMap;

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::AppError,
    message::MessageUsage,
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ModelProvider, ProviderContent, ProviderContentPart,
        ProviderModel, sse, utils,
    },
    role::Role,
};

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    id: String,
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    default_model: String,
    auth_header: String,
    auth_scheme: Option<String>,
    extra_headers: HashMap<String, String>,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            client,
            api_key: api_key.into(),
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: default_model.into(),
            auth_header: "authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            extra_headers: HashMap::new(),
        }
    }

    pub fn with_auth_header(
        mut self,
        header: impl Into<String>,
        scheme: Option<impl Into<String>>,
    ) -> Self {
        self.auth_header = header.into();
        self.auth_scheme = scheme.map(|v| v.into());
        self
    }

    pub fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.extra_headers = headers;
        self
    }

    fn models_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn completions_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn apply_auth_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let auth_value =
            utils::auth_header_value(self.auth_scheme.as_deref(), self.api_key.as_str());

        req = req.header(self.auth_header.as_str(), auth_value);
        utils::apply_extra_headers(req, &self.extra_headers)
    }

    fn parse_models(&self, payload: Value) -> Result<Vec<ProviderModel>, AppError> {
        let parsed: OpenAiCompatibleModelList =
            utils::parse_json_value(self.id.as_str(), "models list", payload)?;
        let models = match parsed {
            OpenAiCompatibleModelList::Object { data } => data,
            OpenAiCompatibleModelList::Array(data) => data,
        };

        Ok(models
            .into_iter()
            .map(|model| ProviderModel {
                provider_id: self.id.clone(),
                id: model.id,
                display_name: model.display_name.or(model.name),
            })
            .collect())
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
            self.id.as_str(),
            payload
                .choices
                .first()
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.tool_calls.as_ref()),
        )?;

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(AppError::Provider(format!(
                "{} returned empty completion payload without finish reason",
                self.id
            )));
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
            provider_id: self.id.clone(),
            model: payload.model.unwrap_or_else(|| self.default_model.clone()),
            text,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: None,
        })
    }
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

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let request = self.apply_auth_headers(self.client.get(self.models_endpoint()));
        let response = request.send().await?;
        let payload: Value = utils::parse_json_response(self.id.as_str(), response).await?;
        self.parse_models(payload)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = if request.model.trim().is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        let messages = Self::convert_messages(request.system, request.messages);

        let body = ChatCompletionRequest {
            model,
            messages,
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            stream: false,
            stream_options: None,
        };

        let req = self
            .apply_auth_headers(self.client.post(self.completions_endpoint()))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body);

        let response = req.send().await?;
        let payload: ChatCompletionResponse =
            utils::parse_json_response(self.id.as_str(), response).await?;
        self.parse_completion(payload)
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

        let messages = Self::convert_messages(request.system, request.messages);

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

        let req = self
            .apply_auth_headers(self.client.post(self.completions_endpoint()))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body);

        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(self.id.as_str(), response).await);
        }

        let mut events = sse::json_events(response);
        let provider_id = self.id.clone();
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ToolCallState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;

            while let Some(event) = events.next().await {
                let event = event?;
                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(provider_id.as_str(), "chat stream chunk", event)?;
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
                        provider_id: provider_id.clone(),
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

#[derive(Debug, Default)]
struct ToolCallState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderMessage;

    #[tokio::test]
    async fn complete_parses_text_tool_calls_usage() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": "Need tool",
                            "tool_calls": [{
                                "id": "call_1",
                                "function": {
                                    "name": "search",
                                    "arguments": "{\"q\":\"rust\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 11,
                        "completion_tokens": 7
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let response = provider
            .complete(CompletionRequest {
                model: "gpt-4o-mini".to_owned(),
                system: None,
                messages: vec![ProviderMessage::new(crate::role::Role::User, "hello")],
                temperature: None,
                max_output_tokens: Some(128),
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "Need tool");
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::ToolCalls)
        ));
        assert_eq!(response.tool_calls.len(), 1);
        match &response.tool_calls[0] {
            CompletionToolCall::Function {
                id,
                name,
                arguments_json,
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "search");
                assert_eq!(arguments_json, "{\"q\":\"rust\"}");
            }
        }
        let usage = response.usage.expect("usage should be present");
        assert_eq!(usage.input_tokens, 11);
        assert_eq!(usage.output_tokens, 7);
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
                    "data": [{ "name": "missing id" }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let err = provider
            .list_models()
            .await
            .expect_err("invalid model payload should fail");

        assert!(matches!(err, AppError::Provider(_)));
    }

    #[tokio::test]
    async fn complete_stream_emits_text_tool_delta_and_completed() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"search\",\"arguments\":\"{\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );

        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: "gpt-4o-mini".to_owned(),
                system: None,
                messages: vec![ProviderMessage::new(crate::role::Role::User, "hello")],
                temperature: None,
                max_output_tokens: Some(64),
            })
            .await
            .expect("stream should start");

        let mut saw_text = false;
        let mut saw_tool_delta = false;
        let mut saw_completed = false;

        while let Some(item) = stream.next().await {
            match item.expect("stream event should parse") {
                CompletionStreamEvent::TextDelta { delta, .. } => {
                    if delta == "Hel" || delta == "lo" {
                        saw_text = true;
                    }
                }
                CompletionStreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                    ..
                } => {
                    if id.as_deref() == Some("call_1")
                        && name.as_deref() == Some("search")
                        && (arguments_delta == "{" || arguments_delta == "}")
                    {
                        saw_tool_delta = true;
                    }
                }
                CompletionStreamEvent::Completed {
                    finish_reason,
                    usage,
                    ..
                } => {
                    assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                    let usage = usage.expect("usage should be present");
                    assert_eq!(usage.input_tokens, 3);
                    assert_eq!(usage.output_tokens, 2);
                    saw_completed = true;
                }
            }
        }

        assert!(saw_text);
        assert!(saw_tool_delta);
        assert!(saw_completed);
    }

    #[tokio::test]
    async fn complete_stream_returns_structured_error_details() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": {
                        "message": "invalid request",
                        "type": "invalid_request_error",
                        "code": "bad_request"
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let err = match provider
            .complete_stream(CompletionRequest {
                model: "gpt-4o-mini".to_owned(),
                system: None,
                messages: vec![ProviderMessage::new(crate::role::Role::User, "hello")],
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
        {
            Ok(_) => panic!("stream should fail with provider error"),
            Err(err) => err,
        };

        match err {
            AppError::HttpStatus { body, status, .. } => {
                assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
                assert!(body.contains("invalid request"));
                assert!(body.contains("type=invalid_request_error"));
                assert!(body.contains("code=\"bad_request\""));
            }
            other => panic!("unexpected error type: {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_supports_legacy_choice_text_payload() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "text-davinci-003",
                    "choices": [{
                        "text": "legacy completion text",
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "text-davinci-003",
        );

        let response = provider
            .complete(CompletionRequest {
                model: "text-davinci-003".to_owned(),
                system: None,
                messages: vec![ProviderMessage::new(crate::role::Role::User, "hello")],
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("legacy completion payload should parse");

        assert_eq!(response.text, "legacy completion text");
        assert!(matches!(
            response.finish_reason,
            Some(CompletionFinishReason::Stop)
        ));
    }

    #[tokio::test]
    async fn complete_stream_sends_include_usage_option() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex("\"include_usage\":true".to_owned()))
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(
                concat!(
                    "data: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
                    "data: [DONE]\n\n"
                ),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: "gpt-4o-mini".to_owned(),
                system: None,
                messages: vec![ProviderMessage::new(crate::role::Role::User, "hello")],
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("stream should start");

        let mut saw_completed = false;
        while let Some(item) = stream.next().await {
            if let CompletionStreamEvent::Completed { usage, .. } =
                item.expect("event should parse")
            {
                let usage = usage.expect("usage should be present");
                assert_eq!(usage.input_tokens, 1);
                assert_eq!(usage.output_tokens, 1);
                saw_completed = true;
            }
        }

        assert!(saw_completed);
    }

    #[tokio::test]
    async fn complete_stream_preserves_usage_emitted_after_finish_reason() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/chat/completions")
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

        let provider = OpenAiCompatibleProvider::new(
            "mock-provider",
            reqwest::Client::new(),
            "sk-test",
            server.url(),
            "gpt-4o-mini",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: "gpt-4o-mini".to_owned(),
                system: None,
                messages: vec![ProviderMessage::new(crate::role::Role::User, "hello")],
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("stream should start");

        let mut saw_completed = false;
        while let Some(item) = stream.next().await {
            if let CompletionStreamEvent::Completed {
                finish_reason,
                usage,
                ..
            } = item.expect("event should parse")
            {
                assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                let usage = usage.expect("usage should be present");
                assert_eq!(usage.input_tokens, 4);
                assert_eq!(usage.output_tokens, 2);
                saw_completed = true;
            }
        }

        assert!(saw_completed);
    }
}
