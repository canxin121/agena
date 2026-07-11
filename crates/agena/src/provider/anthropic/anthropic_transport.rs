use serde::Deserialize;

use super::{
    ADAPTER_KIND, ANTHROPIC_VERSION, AnthropicAdapter, AnthropicProfile, AppError, BTreeMap,
    CompletionRequest, CompletionResponse, CompletionStreamEvent, CompletionToolCall, HashMap,
    Stream, utils,
};

impl AnthropicAdapter {
    pub(crate) async fn send_json<R>(
        &self,
        operation: &str,
        endpoint: String,
        body: &serde_json::Value,
        request: Option<&CompletionRequest>,
    ) -> Result<R, AppError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let mut headers = self.auth_headers(api_key, request);
            headers.insert("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned());
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                operation,
                "POST",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                Some(body),
            );
            utils::apply_resolved_request_headers(self.client.post(endpoint.clone()), &headers)
                .json(body)
        })
        .await?;

        utils::parse_json_response_logged(self.id.as_str(), ADAPTER_KIND, operation, response).await
    }

    pub(crate) fn resolved_headers(
        &self,
        request: Option<&CompletionRequest>,
    ) -> HashMap<String, String> {
        let mut headers = request
            .map(|request| {
                utils::merged_request_headers(
                    &self.extra_headers,
                    &request.request_override.headers,
                )
            })
            .unwrap_or_else(|| self.extra_headers.clone());
        if matches!(self.profile, AnthropicProfile::GithubCopilot) {
            utils::ensure_header_case_insensitive(
                &mut headers,
                reqwest::header::USER_AGENT.as_str(),
                crate::provider::claude_code_api_user_agent,
            );
            utils::ensure_header_case_insensitive(&mut headers, "openai-intent", || {
                "conversation-edits".to_owned()
            });
            if let Some(request) = request {
                utils::insert_header_case_insensitive(
                    &mut headers,
                    "x-initiator",
                    Self::initiator(request),
                );
                if Self::is_vision_request(request) {
                    utils::insert_header_case_insensitive(
                        &mut headers,
                        "Copilot-Vision-Request",
                        "true",
                    );
                }
            }
        }
        headers
    }

    pub(crate) fn auth_headers(
        &self,
        api_key: &str,
        request: Option<&CompletionRequest>,
    ) -> BTreeMap<String, String> {
        let mut headers = self.resolved_headers(request);
        headers.insert(
            self.auth_header.clone(),
            utils::auth_header_value(self.auth_scheme.as_deref(), api_key),
        );
        utils::resolved_request_headers(self.id.as_str(), &headers)
    }

    #[allow(dead_code)]
    pub(crate) fn completion_response_stream(
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
