use futures_util::StreamExt;

use super::{
    ADAPTER_KIND, AppError, BTreeMap, CapabilityFamily, CapabilitySupport, CompletionFinishReason,
    CompletionRequest, CompletionResponse, CompletionStreamEvent, CompletionUsage,
    ModelCapabilities, ModelId, ModelRuntime, ModelThinkingMode, OpenAiAdapter, OpenAiBackend,
    OpenAiModelListResponse, OpenAiProfile, OpenAiResponsesCompactRequest,
    OpenAiResponsesCompactResponse, OpenAiResponsesRequest, OpenAiResponsesResponse,
    OpenAiStreamMode, OpenAiUsage, ProviderId, ProviderModel, RequestHeaderContext, Stream,
    StreamResumePolicy, ToolStreamAccumulator, async_trait,
    completion_event_from_tool_stream_update, response_id_metadata,
    responses_finish_reason_with_tool_calls, responses_native_tool_event,
    responses_reasoning_delta, responses_tool_stream_input, sse, utils,
};

#[async_trait]
impl ModelRuntime for OpenAiAdapter {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(self.capability_family)
    }

    fn validate_native_tools_request(
        &self,
        _adapter_id: Option<&crate::model::AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), AppError> {
        self.responses_tool_plan_for_request(request).map(|_| ())
    }

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&crate::model::AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        let mut capabilities = crate::provider::default_capability_registry()
            .capabilities_for_family(self.capability_family, model.as_ref());
        let _ = adapter_id;
        if self.is_dashscope_reasoning_model(model) {
            capabilities.reasoning = CapabilitySupport::Supported;
        }
        capabilities
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&crate::model::AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        let modes = crate::provider::default_model_mode_registry().thinking_modes_for_family(
            self.capability_family,
            adapter_id,
            model.as_ref(),
            &self.model_metadata_for_adapter(adapter_id, model),
        );
        if modes.is_empty() && self.is_dashscope_reasoning_model(model) {
            return Self::dashscope_thinking_modes(model);
        }
        modes
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        let _ = model;
        // OpenAI-compatible backends frequently diverge on `previous_response_id`
        // semantics. Replaying the normalized transcript is slower but reliable.
        false
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        let mut fields = vec![
            ("auth_scope", self.api_key.prompt_cache_scope()),
            ("backend", self.backend_key().to_owned()),
            ("base_url", self.prompt_cache_base_url().to_owned()),
            ("api_mode", self.api_mode_key().to_owned()),
            ("stream_mode", self.stream_mode_key().to_owned()),
            ("auth_header", self.auth_header.clone()),
            (
                "profile",
                match self.profile {
                    OpenAiProfile::Standard => "standard",
                    OpenAiProfile::GithubCopilot => "github_copilot",
                }
                .to_owned(),
            ),
            (
                "capability_family",
                match self.capability_family {
                    CapabilityFamily::OpenAi => "openai",
                    CapabilityFamily::OpenAiCompatible => "openai_compatible",
                    CapabilityFamily::Anthropic => "anthropic",
                    CapabilityFamily::Gemini => "gemini",
                    CapabilityFamily::Bedrock => "bedrock",
                    CapabilityFamily::Gitlab => "gitlab",
                }
                .to_owned(),
            ),
            (
                "uses_responses",
                self.should_use_responses(model.as_ref()).to_string(),
            ),
            (
                "supports_top_level_prompt_cache",
                self.supports_top_level_prompt_cache().to_string(),
            ),
            (
                "extra_headers",
                crate::provider::PromptCacheShape::json_field_value(
                    &utils::prompt_cache_header_entries(&self.extra_headers),
                ),
            ),
        ];
        if let Some(models_url) = self.models_url.as_deref() {
            fields.push(("models_url", models_url.to_owned()));
        }
        if let Some(auth_scheme) = self.auth_scheme.as_deref() {
            fields.push(("auth_scheme", auth_scheme.to_owned()));
        }
        if let Some(auth_account_id) = self.chatgpt_account_id() {
            fields.push(("auth_account_id", auth_account_id));
        }
        if let Some(realtime_ws_url) = self.realtime_ws_url.as_deref() {
            fields.push(("realtime_ws_url", realtime_ws_url.to_owned()));
        }
        Some(crate::provider::PromptCacheShape::from_fields(
            self.id.as_str(),
            fields,
        ))
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let endpoint = self.list_models_endpoint()?;
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let headers = self.auth_headers(RequestHeaderContext::none(), api_key);
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                "list_models",
                "GET",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                None,
            );
            utils::apply_resolved_request_headers(self.client.get(endpoint.as_str()), &headers)
        })
        .await?;

        let payload: OpenAiModelListResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            ADAPTER_KIND,
            "list_models",
            response,
        )
        .await?;
        Ok(payload
            .into_items(self.id.as_str(), self.models_url.as_deref())
            .into_iter()
            .filter_map(|model| self.provider_model_from_listed_model(model))
            .collect())
    }

    #[tracing::instrument(
        skip_all,
        fields(provider = tracing::field::Empty, model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let model = request.model.clone();
        let native_tools_require_responses =
            Self::native_tools_request_requires_responses(&request);

        if !self.should_use_responses(model.as_ref()) {
            if native_tools_require_responses {
                return Err(AppError::Config(format!(
                    "provider `{}` model `{}` configures native hosted tools, but the selected OpenAI API mode resolves to chat; switch this provider/model to Responses mode",
                    self.id, model
                )));
            }
            return self
                .complete_with_chat_api(&request, model.to_string())
                .await;
        }

        let input = self.responses_input_for_request(&request)?;
        let tool_plan = self.responses_tool_plan_for_request(&request)?;
        let reasoning = Self::responses_reasoning_config(&request, model.as_ref());

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            instructions: Self::responses_instructions(&request),
            input,
            tools: tool_plan.tools,
            tool_choice: "auto".to_owned(),
            parallel_tool_calls: Self::responses_parallel_tool_calls(&request),
            include: Self::responses_include(tool_plan.include, reasoning.as_ref()),
            max_output_tokens: self.responses_request_max_output_tokens(&request),
            temperature: request.temperature,
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: request.previous_response_id.clone(),
            store: false,
            stream: false,
            stop: (!request.stop_sequences.is_empty()).then(|| request.stop_sequences.clone()),
            top_p: request.top_p,
            seed: request.seed,
            reasoning,
            service_tier: Self::responses_service_tier(&request),
            text: Self::responses_text_config(&request),
            client_metadata: Self::responses_client_metadata(RequestHeaderContext::from_request(
                &request,
            )),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;

        let response: OpenAiResponsesResponse = match self
            .send_json(
                "complete.responses",
                self.responses_endpoint()?,
                Some(&body_json),
                RequestHeaderContext::from_request(&request),
            )
            .await
        {
            Ok(payload) => payload,
            Err(AppError::HttpStatus { status, .. })
                if !native_tools_require_responses
                    && self.can_fallback_to_chat()
                    && Self::responses_endpoint_unsupported(status) =>
            {
                return self
                    .complete_with_chat_api(&request, model.to_string())
                    .await;
            }
            Err(err) => return Err(err),
        };

        let response_model =
            ModelId::new(response.model.clone().unwrap_or_else(|| model.to_string()));
        let reasoning_text = Self::extract_reasoning_text(&response);
        let finish_reason = CompletionFinishReason::from_provider(response.stop_reason.as_deref());
        let text = Self::extract_text(&response);
        let tool_calls = Self::parse_responses_tool_calls(response.output.as_ref())?;
        let finish_reason =
            responses_finish_reason_with_tool_calls(finish_reason, !tool_calls.is_empty());

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return self.complete_by_aggregating_stream(request).await;
        }

        let usage = Self::map_usage(response.usage);

        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id.as_str()),
            model: response_model,
            text,
            reasoning_text,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: response_id_metadata(response.id),
        })
    }

    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        let model = request.model.clone();
        if self.backend != OpenAiBackend::Api
            || self.profile != OpenAiProfile::Standard
            || self.is_openai_compatible_family()
            || !self.should_use_responses(model.as_ref())
        {
            return Ok(None);
        }

        let mut input_request = request.clone();
        input_request.system = None;
        input_request.previous_response_id = None;
        let input = self.responses_input_for_request(&input_request)?;
        let tool_plan = self.responses_tool_plan_for_request(&request)?;
        let body = OpenAiResponsesCompactRequest {
            model: model.to_string(),
            instructions: Self::responses_instructions(&request),
            input,
            tools: tool_plan.tools,
            include: (!tool_plan.include.is_empty()).then_some(tool_plan.include),
            parallel_tool_calls: Self::responses_parallel_tool_calls(&request),
            prompt_cache_key: request.prompt_cache_key.clone(),
            reasoning: Self::responses_reasoning_config(&request, model.as_ref()),
            service_tier: Self::responses_service_tier(&request),
            text: Self::responses_text_config(&request),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;
        let response: OpenAiResponsesCompactResponse = self
            .send_json(
                "compact.responses",
                self.responses_compact_endpoint()?,
                Some(&body_json),
                RequestHeaderContext::from_request(&request),
            )
            .await?;
        Ok(Self::compact_summary_from_output(
            response.output.as_slice(),
        ))
    }

    #[tracing::instrument(
        skip_all,
        fields(provider = tracing::field::Empty, model = %request.model)
    )]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let model = request.model.clone();
        let native_tools_require_responses =
            Self::native_tools_request_requires_responses(&request);

        if matches!(self.stream_mode, OpenAiStreamMode::RealtimeWebSocket) {
            if native_tools_require_responses {
                return Err(AppError::Config(format!(
                    "provider `{}` model `{}` configures native hosted tools, but OpenAI realtime websocket mode does not support them; use SSE Responses streaming instead",
                    self.id, model
                )));
            }
            return self
                .complete_stream_with_realtime_ws(&request, model.to_string())
                .await;
        }

        if !self.should_use_responses(model.as_ref()) {
            if native_tools_require_responses {
                return Err(AppError::Config(format!(
                    "provider `{}` model `{}` configures native hosted tools, but the selected OpenAI API mode resolves to chat; switch this provider/model to Responses mode",
                    self.id, model
                )));
            }
            return self
                .complete_stream_with_chat_api(&request, model.to_string())
                .await;
        }

        let input = self.responses_input_for_request(&request)?;
        let tool_plan = self.responses_tool_plan_for_request(&request)?;
        let reasoning = Self::responses_reasoning_config(&request, model.as_ref());

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            instructions: Self::responses_instructions(&request),
            input,
            tools: tool_plan.tools,
            tool_choice: "auto".to_owned(),
            parallel_tool_calls: Self::responses_parallel_tool_calls(&request),
            include: Self::responses_include(tool_plan.include, reasoning.as_ref()),
            max_output_tokens: self.responses_request_max_output_tokens(&request),
            temperature: request.temperature,
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: request.previous_response_id.clone(),
            store: false,
            stream: true,
            stop: (!request.stop_sequences.is_empty()).then(|| request.stop_sequences.clone()),
            top_p: request.top_p,
            seed: request.seed,
            reasoning,
            service_tier: Self::responses_service_tier(&request),
            text: Self::responses_text_config(&request),
            client_metadata: Self::responses_client_metadata(RequestHeaderContext::from_request(
                &request,
            )),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let endpoint = self
                .responses_endpoint()
                .expect("responses endpoint should resolve");
            let mut headers =
                self.auth_headers(RequestHeaderContext::from_request(&request), api_key);
            headers.insert(
                reqwest::header::ACCEPT.as_str().to_owned(),
                "text/event-stream".to_owned(),
            );
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.responses",
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
            if self.can_fallback_to_chat()
                && !native_tools_require_responses
                && Self::responses_endpoint_unsupported(response.status())
            {
                return self
                    .complete_stream_with_chat_api(&request, model.to_string())
                    .await;
            }
            return Err(utils::http_status_error_from_response_logged(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.responses",
                response,
            )
            .await);
        }

        if self.should_require_sse_content_type() {
            utils::ensure_response_content_type(self.id.as_str(), &response, "text/event-stream")?;
        }
        utils::adapter_log_http_response_open(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete_stream.responses",
            response.status(),
            response.headers(),
        );
        let provider_name = self.id.clone();
        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(provider_name.as_str());
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut tool_stream = ToolStreamAccumulator::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut stream_tool_call_seen = false;
            let mut completed_emitted = false;
            let mut response_id: Option<String> = None;

            while let Some(event) = events.next().await {
                let event = event?;
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.responses",
                    &event,
                );

                if let Some(err) = utils::responses_stream_error(provider_name.as_str(), &event)? {
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

                if let Some(delta) = responses_reasoning_delta(&event) {
                    stream_has_content = true;
                    yield CompletionStreamEvent::ThinkingDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                if let Some(native_event) =
                    responses_native_tool_event(&provider_id, &model_name, &event)?
                {
                    stream_has_content = true;
                    yield native_event;
                }

                if let Some(tool_event) = utils::responses_tool_event(provider_name.as_str(), &event)? {
                    stream_tool_call_seen = true;
                    let input = responses_tool_stream_input(provider_name.as_str(), tool_event)?;
                    for update in tool_stream.ingest(provider_name.as_str(), input)? {
                        stream_has_content = true;
                        yield completion_event_from_tool_stream_update(
                            &provider_id,
                            &model_name,
                            update,
                        );
                    }
                }

                if let Some(raw_usage) = utils::responses_usage_value(&event) {
                    let usage = utils::parse_json_value::<OpenAiUsage>(
                        provider_name.as_str(),
                        "responses stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Self::map_usage(Some(usage));
                }

                if stream_finish_reason.is_none() {
                    stream_finish_reason = utils::responses_finish_reason(&event);
                }

                if let Some(next_response_id) = utils::responses_response_id(&event) {
                    response_id = Some(next_response_id);
                }

                if utils::responses_is_completed(&event) {
                    let finish_reason = responses_finish_reason_with_tool_calls(
                        CompletionFinishReason::from_provider(stream_finish_reason.as_deref()),
                        stream_tool_call_seen,
                    );
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason,
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
                let finish_reason = responses_finish_reason_with_tool_calls(
                    CompletionFinishReason::from_provider(stream_finish_reason.as_deref()),
                    stream_tool_call_seen,
                );
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason,
                    usage: stream_usage,
                    provider_metadata: response_id_metadata(response_id),
                };
            }
        };

        Ok(Box::pin(stream))
    }
}
