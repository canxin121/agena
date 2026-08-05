use agena_domain::Model;
use agena_provider::{CapabilityFamily, ProviderCompactionOutput, StreamResumePolicy};
use futures_util::StreamExt;

use super::{
    CHAT_COMPLETIONS_ADAPTER_KIND, CapabilitySupport, CompletionFinishReason, CompletionRequest,
    CompletionResponse, CompletionStreamEvent, CompletionUsage, ModelCapabilities, ModelId,
    ModelRuntime, ModelThinkingMode, OpenAiChatCompletionsAdapter, OpenAiInputContent,
    OpenAiInputMessage, OpenAiModelListResponse, OpenAiProfile, OpenAiRealtimeAdapter,
    OpenAiResponsesAdapter, OpenAiResponsesBackend, OpenAiResponsesCompactRequest,
    OpenAiResponsesCompactResponse, OpenAiResponsesInputItem, OpenAiResponsesRequest,
    OpenAiResponsesResponse, OpenAiTransport, OpenAiUsage, ProviderError, ProviderId,
    ProviderImageCapabilities, ProviderImageOperation, ProviderImageRequest, ProviderImageResponse,
    ProviderNativeToolArtifact, REALTIME_ADAPTER_KIND, RESPONSES_ADAPTER_KIND,
    RequestHeaderContext, Stream, ToolStreamAccumulator, async_trait,
    completion_event_from_tool_stream_update, openai_reasoning_item_from_event,
    openai_reasoning_items_from_output, openai_responses_metadata,
    responses_finish_reason_with_tool_calls, responses_provider_native_tool_event,
    responses_reasoning_delta, responses_tool_stream_input, sse, utils,
};
impl OpenAiTransport {
    pub(super) fn runtime_model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        let mut capabilities = agena_provider::default_capability_registry()
            .capabilities_for_family(self.capability_family, model.as_ref());
        if self.is_dashscope_reasoning_model(model) {
            capabilities.reasoning = CapabilitySupport::Supported;
        }
        capabilities
    }

    fn prompt_cache_fields(&self, protocol: &'static str) -> Vec<(&'static str, String)> {
        let mut fields = vec![
            ("auth_scope", self.api_key.prompt_cache_scope()),
            ("backend", self.backend_key().to_owned()),
            ("base_url", self.prompt_cache_base_url().to_owned()),
            ("protocol", protocol.to_owned()),
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
                "supports_top_level_prompt_cache",
                self.supports_top_level_prompt_cache().to_string(),
            ),
            (
                "extra_headers",
                agena_provider::PromptCacheShape::json_field_value(
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
        fields
    }

    async fn list_models_for_protocol(
        &self,
        adapter_kind: &'static str,
    ) -> Result<Vec<Model>, ProviderError> {
        let endpoint = self.list_models_endpoint()?;
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let headers = self.auth_headers(RequestHeaderContext::none(), api_key);
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                adapter_kind,
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
            adapter_kind,
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
}

#[async_trait]
impl ModelRuntime for OpenAiResponsesAdapter {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<agena_provider::CapabilityFamily> {
        Some(self.capability_family)
    }

    fn validate_provider_native_tools_request(
        &self,
        _adapter_id: Option<&agena_domain::AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), ProviderError> {
        self.responses_tool_plan_for_request(request).map(|_| ())
    }

    fn image_capabilities(&self, _model: &ModelId) -> Option<ProviderImageCapabilities> {
        if matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex)
            || matches!(self.profile, OpenAiProfile::GithubCopilot)
        {
            return None;
        }
        Some(ProviderImageCapabilities {
            generate: true,
            edit: true,
            accepted_input_mime_types: vec![
                "image/png".to_owned(),
                "image/jpeg".to_owned(),
                "image/webp".to_owned(),
                "image/gif".to_owned(),
            ],
            max_input_bytes: Some(50 * 1024 * 1024),
            max_input_images: Some(16),
        })
    }

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&agena_domain::AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        let _ = adapter_id;
        self.runtime_model_capabilities(model)
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&agena_domain::AdapterId>,
        model: &ModelId,
    ) -> Vec<ModelThinkingMode> {
        let modes = agena_provider::default_model_mode_registry().thinking_modes_for_family(
            self.capability_family,
            adapter_id,
            model.as_ref(),
            &self.model_metadata_for_adapter(adapter_id, model),
        );
        if modes.is_empty() && self.is_dashscope_reasoning_model(model) {
            return OpenAiTransport::dashscope_thinking_modes(model);
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

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        let fields = self.prompt_cache_fields("responses");
        Some(agena_provider::PromptCacheShape::from_fields(
            self.id.as_str(),
            fields,
        ))
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        self.list_models_for_protocol(RESPONSES_ADAPTER_KIND).await
    }

    #[tracing::instrument(
        skip_all,
        fields(provider = tracing::field::Empty, model = %model)
    )]
    async fn execute_image(
        &self,
        model: &ModelId,
        request: ProviderImageRequest,
    ) -> Result<ProviderImageResponse, ProviderError> {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let capabilities = self.image_capabilities(model).ok_or_else(|| {
            ProviderError::Config(format!(
                "provider `{}` model `{model}` does not support direct image requests on this OpenAI Responses route",
                self.id
            ))
        })?;
        let prompt = request.prompt.trim();
        if prompt.is_empty() {
            return Err(ProviderError::Config(
                "direct image request prompt must not be empty".to_owned(),
            ));
        }
        match request.operation {
            ProviderImageOperation::Generate if !request.inputs.is_empty() => {
                return Err(ProviderError::Config(
                    "image generate requests must not contain edit inputs".to_owned(),
                ));
            }
            ProviderImageOperation::Edit if request.inputs.is_empty() => {
                return Err(ProviderError::Config(
                    "image edit requests require at least one input image".to_owned(),
                ));
            }
            _ => {}
        }
        if request.inputs.len() > capabilities.max_input_images.unwrap_or(u32::MAX) as usize {
            return Err(ProviderError::Config(format!(
                "direct image request contains {} inputs, exceeding the route limit of {}",
                request.inputs.len(),
                capabilities.max_input_images.unwrap_or(u32::MAX)
            )));
        }

        let mut content = vec![OpenAiInputContent::InputText {
            text: prompt.to_owned(),
        }];
        for (index, input) in request.inputs.iter().enumerate() {
            if !capabilities
                .accepted_input_mime_types
                .iter()
                .any(|mime| mime == &input.mime)
            {
                return Err(ProviderError::Config(format!(
                    "direct image input {index} uses unsupported MIME type `{}`",
                    input.mime
                )));
            }
            if input.data_base64.trim().is_empty() {
                return Err(ProviderError::Config(format!(
                    "direct image input {index} has an empty base64 payload"
                )));
            }
            if capabilities
                .max_input_bytes
                .is_some_and(|limit| input.size_bytes > limit)
            {
                return Err(ProviderError::Config(format!(
                    "direct image input {index} exceeds the route size limit"
                )));
            }
            content.push(OpenAiInputContent::Image {
                image_url: format!(
                    "data:{};base64,{}",
                    input.mime.trim(),
                    input.data_base64.trim()
                ),
            });
        }

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            instructions: None,
            input: vec![OpenAiResponsesInputItem::Message(OpenAiInputMessage {
                role: "user".to_owned(),
                content,
                copilot_cache_control: None,
            })],
            tools: vec![OpenAiTransport::image_generation_tool_definition(
                &request.options,
            )?],
            // This is the key direct-execution invariant: the adapter forces
            // the only declared hosted tool in this request and returns its
            // terminal artifact from the same API call.
            tool_choice: "required".to_owned(),
            parallel_tool_calls: false,
            include: None,
            max_output_tokens: None,
            temperature: None,
            prompt_cache_key: None,
            previous_response_id: None,
            store: false,
            stream: false,
            top_p: None,
            reasoning: None,
            service_tier: None,
            text: None,
            client_metadata: None,
        };
        let body_json = serde_json::to_value(&body)?;
        let response: OpenAiResponsesResponse = self
            .send_json(
                "image.execute.responses",
                self.responses_endpoint()?,
                Some(&body_json),
                RequestHeaderContext::none(),
            )
            .await?;
        if let Some(event) = response.failure_event() {
            return Err(
                utils::responses_stream_error(self.id.as_str(), &event)?.unwrap_or_else(|| {
                    ProviderError::Provider(format!("{} image response failed", self.id))
                }),
            );
        }
        if let Some(status) = response.unexpected_nonstream_status() {
            return Err(ProviderError::Provider(format!(
                "{} returned non-terminal image response status `{status}`",
                self.id
            )));
        }

        let mut artifacts = Vec::new();
        let mut revised_prompt = None;
        for item in response.output.as_deref().into_iter().flatten() {
            if item.kind.as_deref() != Some("image_generation_call") {
                continue;
            }
            if revised_prompt.is_none() {
                revised_prompt = item
                    .revised_prompt
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned);
            }
            let Some(result) = item
                .result
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let mime = item
                .mime_type
                .as_deref()
                .map(str::trim)
                .filter(|value| value.starts_with("image/"))
                .unwrap_or("image/png")
                .to_owned();
            let extension = mime
                .strip_prefix("image/")
                .filter(|value| !value.is_empty())
                .unwrap_or("png")
                .to_owned();
            artifacts.push(ProviderNativeToolArtifact {
                uri: format!("data:{mime};base64,{result}"),
                mime,
                name: Some(format!("generated-image.{extension}")),
                size_bytes: None,
                sha256: None,
            });
        }
        if artifacts.is_empty() {
            return Err(ProviderError::Provider(format!(
                "{} direct image response contained no completed image_generation_call result",
                self.id
            )));
        }
        let response_model =
            ModelId::new(response.model.clone().unwrap_or_else(|| model.to_string()));
        Ok(ProviderImageResponse {
            provider_id: ProviderId::new(self.id.as_str()),
            model: response_model,
            revised_prompt,
            artifacts,
            usage: OpenAiTransport::map_usage(response.usage),
        })
    }

    #[tracing::instrument(
        skip_all,
        fields(provider = tracing::field::Empty, model = %request.model)
    )]
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let model = request.model.clone();

        let input = self.responses_input_for_request(&request)?;
        let tool_plan = self.responses_tool_plan_for_request(&request)?;
        let reasoning = OpenAiTransport::responses_reasoning_config(&request, model.as_ref());

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            instructions: OpenAiTransport::responses_instructions(&request),
            input,
            tools: tool_plan.tools,
            tool_choice: "auto".to_owned(),
            parallel_tool_calls: OpenAiTransport::responses_parallel_tool_calls(&request),
            include: OpenAiTransport::responses_include(
                tool_plan.include,
                reasoning.as_ref(),
                self.supports_codex_compat_headers() || self.is_official_openai_endpoint(),
            ),
            max_output_tokens: self.responses_request_max_output_tokens(&request),
            temperature: (!matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex))
                .then_some(request.temperature)
                .flatten(),
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: (!matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex))
                .then(|| request.previous_response_id.clone())
                .flatten(),
            store: false,
            stream: false,
            top_p: (!matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex))
                .then_some(request.top_p)
                .flatten(),
            reasoning,
            service_tier: OpenAiTransport::responses_service_tier(&request),
            text: OpenAiTransport::responses_text_config(&request),
            client_metadata: OpenAiTransport::responses_client_metadata(
                RequestHeaderContext::from_request(&request),
            ),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;

        let response: OpenAiResponsesResponse = self
            .send_json(
                "complete.responses",
                self.responses_endpoint()?,
                Some(&body_json),
                RequestHeaderContext::from_request(&request),
            )
            .await?;

        if let Some(event) = response.failure_event() {
            return Err(
                utils::responses_stream_error(self.id.as_str(), &event)?.unwrap_or_else(|| {
                    ProviderError::Provider(format!("{} response failed", self.id))
                }),
            );
        }
        if let Some(status) = response.unexpected_nonstream_status() {
            return Err(ProviderError::Provider(format!(
                "{} returned non-terminal Responses status `{status}` to a non-streaming request",
                self.id
            )));
        }

        let response_model =
            ModelId::new(response.model.clone().unwrap_or_else(|| model.to_string()));
        let reasoning_text = OpenAiTransport::extract_reasoning_text(&response);
        let raw_finish_reason = response.terminal_reason();
        let finish_reason = CompletionFinishReason::from_provider(raw_finish_reason);
        let text = OpenAiTransport::extract_text(&response);
        let tool_calls = OpenAiTransport::parse_responses_tool_calls(response.output.as_ref())?;
        let finish_reason =
            responses_finish_reason_with_tool_calls(finish_reason, !tool_calls.is_empty());

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return self.complete_by_aggregating_stream(request).await;
        }

        let usage = OpenAiTransport::map_usage(response.usage);
        let provider_metadata = openai_responses_metadata(
            response.id,
            openai_reasoning_items_from_output(response.output.as_deref()),
        );

        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id.as_str()),
            model: response_model,
            text,
            reasoning_text,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata,
        })
    }

    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<ProviderCompactionOutput>, ProviderError> {
        let model = request.model.clone();
        if self.backend != OpenAiResponsesBackend::Api
            || self.profile != OpenAiProfile::Standard
            || self.is_openai_compatible_family()
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
            instructions: OpenAiTransport::responses_instructions(&request),
            input,
            tools: tool_plan.tools,
            include: (!tool_plan.include.is_empty()).then_some(tool_plan.include),
            parallel_tool_calls: OpenAiTransport::responses_parallel_tool_calls(&request),
            prompt_cache_key: request.prompt_cache_key.clone(),
            reasoning: OpenAiTransport::responses_reasoning_config(&request, model.as_ref()),
            service_tier: OpenAiTransport::responses_service_tier(&request),
            text: OpenAiTransport::responses_text_config(&request),
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
        Ok(
            (!response.output.is_empty()).then_some(ProviderCompactionOutput::OpenAiResponses {
                items: response.output,
            }),
        )
    }

    #[tracing::instrument(
        skip_all,
        fields(provider = tracing::field::Empty, model = %request.model)
    )]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let model = request.model.clone();

        let input = self.responses_input_for_request(&request)?;
        let tool_plan = self.responses_tool_plan_for_request(&request)?;
        let reasoning = OpenAiTransport::responses_reasoning_config(&request, model.as_ref());

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            instructions: OpenAiTransport::responses_instructions(&request),
            input,
            tools: tool_plan.tools,
            tool_choice: "auto".to_owned(),
            parallel_tool_calls: OpenAiTransport::responses_parallel_tool_calls(&request),
            include: OpenAiTransport::responses_include(
                tool_plan.include,
                reasoning.as_ref(),
                self.supports_codex_compat_headers() || self.is_official_openai_endpoint(),
            ),
            max_output_tokens: self.responses_request_max_output_tokens(&request),
            temperature: (!matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex))
                .then_some(request.temperature)
                .flatten(),
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: (!matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex))
                .then(|| request.previous_response_id.clone())
                .flatten(),
            store: false,
            stream: true,
            top_p: (!matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex))
                .then_some(request.top_p)
                .flatten(),
            reasoning,
            service_tier: OpenAiTransport::responses_service_tier(&request),
            text: OpenAiTransport::responses_text_config(&request),
            client_metadata: OpenAiTransport::responses_client_metadata(
                RequestHeaderContext::from_request(&request),
            ),
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
                RESPONSES_ADAPTER_KIND,
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
            return Err(utils::http_status_error_from_response_logged(
                self.id.as_str(),
                RESPONSES_ADAPTER_KIND,
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
            RESPONSES_ADAPTER_KIND,
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
            let mut stream_tool_call_seen = false;
            let mut completed_emitted = false;
            let mut response_id: Option<String> = None;
            // Keep response-item order stable. A later event for the same item
            // replaces its snapshot in place instead of reordering it by ID.
            let mut reasoning_items = Vec::<(String, serde_json::Value)>::new();

            while let Some(event) = events.next().await {
                let event = event?;
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    RESPONSES_ADAPTER_KIND,
                    "complete_stream.responses",
                    &event,
                );

                if let Some(err) = utils::responses_stream_error(provider_name.as_str(), &event)? {
                    Err(err)?;
                }

                if let Some((item_id, item)) = openai_reasoning_item_from_event(&event) {
                    if let Some((_, current)) = reasoning_items
                        .iter_mut()
                        .find(|(current_id, _)| current_id == &item_id)
                    {
                        *current = item;
                    } else {
                        reasoning_items.push((item_id, item));
                    }
                }

                if let Some(delta) = utils::responses_text_delta(&event) {
                    yield CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                if let Some(delta) = responses_reasoning_delta(&event) {
                    yield CompletionStreamEvent::ThinkingDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                if let Some(provider_native_tool_event) =
                    responses_provider_native_tool_event(&provider_id, &model_name, &event)?
                {
                    yield provider_native_tool_event;
                }

                if let Some(tool_event) = utils::responses_tool_event(provider_name.as_str(), &event)? {
                    stream_tool_call_seen = true;
                    let input = responses_tool_stream_input(provider_name.as_str(), tool_event)?;
                    for update in tool_stream.ingest(provider_name.as_str(), input)? {
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
                    stream_usage = OpenAiTransport::map_usage(Some(usage));
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
                        provider_metadata: openai_responses_metadata(
                            response_id.clone(),
                            reasoning_items.iter().map(|(_, item)| item.clone()),
                        ),
                        end_turn: utils::responses_end_turn(&event),
                    };
                    completed_emitted = true;
                    break;
                }
            }

            utils::require_terminal_stream_event(
                provider_name.as_str(),
                "responses",
                completed_emitted,
            )?;
        };

        Ok(Box::pin(stream))
    }
}

#[async_trait]
impl ModelRuntime for OpenAiChatCompletionsAdapter {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<CapabilityFamily> {
        Some(self.capability_family)
    }

    fn validate_provider_native_tools_request(
        &self,
        _adapter_id: Option<&agena_domain::AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), ProviderError> {
        if request.provider_native_tools.bindings().is_empty() {
            Ok(())
        } else {
            Err(ProviderError::Config(format!(
                "provider `{}` model `{}` configures OpenAI-hosted tools, but the Chat Completions protocol does not support them; select the `openai_responses` adapter",
                self.id, request.model
            )))
        }
    }

    fn model_capabilities_for_adapter(
        &self,
        _adapter_id: Option<&agena_domain::AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        self.runtime_model_capabilities(model)
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&agena_domain::AdapterId>,
        model: &ModelId,
    ) -> Vec<ModelThinkingMode> {
        let modes = agena_provider::default_model_mode_registry().thinking_modes_for_family(
            self.capability_family,
            adapter_id,
            model.as_ref(),
            &self.model_metadata_for_adapter(adapter_id, model),
        );
        if modes.is_empty() && self.is_dashscope_reasoning_model(model) {
            return OpenAiTransport::dashscope_thinking_modes(model);
        }
        modes
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn supports_prompt_continuation(&self, _model: &ModelId) -> bool {
        false
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        Some(agena_provider::PromptCacheShape::from_fields(
            self.id.as_str(),
            self.prompt_cache_fields("chat_completions"),
        ))
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        self.list_models_for_protocol(CHAT_COMPLETIONS_ADAPTER_KIND)
            .await
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        self.complete_with_chat_api(&request, request.model.to_string())
            .await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        self.complete_stream_with_chat_api(&request, request.model.to_string())
            .await
    }
}

#[async_trait]
impl ModelRuntime for OpenAiRealtimeAdapter {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<CapabilityFamily> {
        Some(self.capability_family)
    }

    fn validate_provider_native_tools_request(
        &self,
        _adapter_id: Option<&agena_domain::AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), ProviderError> {
        if request.provider_native_tools.bindings().is_empty() {
            Ok(())
        } else {
            Err(ProviderError::Config(format!(
                "provider `{}` model `{}` configures OpenAI-hosted tools, but the Realtime protocol does not support those hosted tool definitions",
                self.id, request.model
            )))
        }
    }

    fn model_capabilities_for_adapter(
        &self,
        _adapter_id: Option<&agena_domain::AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        self.runtime_model_capabilities(model)
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&agena_domain::AdapterId>,
        model: &ModelId,
    ) -> Vec<ModelThinkingMode> {
        agena_provider::default_model_mode_registry().thinking_modes_for_family(
            self.capability_family,
            adapter_id,
            model.as_ref(),
            &self.model_metadata_for_adapter(adapter_id, model),
        )
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn supports_prompt_continuation(&self, _model: &ModelId) -> bool {
        false
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        let mut fields = self.prompt_cache_fields("realtime");
        if let Some(realtime_ws_url) = self.realtime_ws_url.as_deref() {
            fields.push(("realtime_ws_url", realtime_ws_url.to_owned()));
        }
        Some(agena_provider::PromptCacheShape::from_fields(
            self.id.as_str(),
            fields,
        ))
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        self.list_models_for_protocol(REALTIME_ADAPTER_KIND).await
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let fallback_model = request.model.clone();
        let stream = self.complete_stream(request).await?;
        utils::aggregate_stream(self.id.as_str(), fallback_model, stream).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        self.complete_stream_with_realtime_ws(
            &request,
            request.model.to_string(),
            self.realtime_ws_url.as_deref(),
        )
        .await
    }
}
