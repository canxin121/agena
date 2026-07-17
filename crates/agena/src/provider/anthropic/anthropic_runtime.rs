use futures_util::StreamExt;

use super::{
    ADAPTER_KIND, ANTHROPIC_VERSION, AnthropicAdapter, AnthropicMessage, AnthropicMessagesRequest,
    AnthropicMessagesResponse, AnthropicModelListResponse, AnthropicProfile, AnthropicSseEvent,
    AnthropicTextBlock, AnthropicThinkingBlockState, AnthropicToolCallState, AnthropicUsage,
    AppError, BTreeMap, CompletionFinishReason, CompletionRequest, CompletionResponse,
    CompletionStreamEvent, CompletionToolCall, HashMap, ModelId, ModelRuntime, PROVIDER_ID,
    ProviderId, ProviderModel, Role, Stream, StreamResumePolicy, anthropic_model_rejects_sampling,
    anthropic_thinking_metadata, anthropic_thinking_parts, async_trait, json_value_to_string,
    map_anthropic_usage, merge_anthropic_usage, sse, utils,
};

impl AnthropicAdapter {
    fn messages_request_body(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> Result<(AnthropicMessagesRequest, bool), AppError> {
        let model = &request.model;
        let max_tokens = request.max_output_tokens.unwrap_or(4096);
        let thinking_parts =
            anthropic_thinking_parts(model.as_ref(), request.thinking.as_ref(), max_tokens);
        let include_thinking = thinking_parts.include_thinking();
        let omit_sampling = include_thinking || anthropic_model_rejects_sampling(model.as_ref());

        let mut system_chunks = Vec::new();
        if let Some(system) = request
            .system
            .as_ref()
            .filter(|system| !system.trim().is_empty())
        {
            system_chunks.push(AnthropicTextBlock::text(system.clone()));
        }
        let mut tools = (!request.tool_api_functions.is_empty()
            || !request.provider_native_tools.bindings().is_empty())
        .then(|| self.tools(request))
        .transpose()?;

        let mut messages = Vec::new();
        for message in &request.messages {
            match message.role {
                Role::System => {
                    let text = message.as_text_lossy();
                    if !text.trim().is_empty() {
                        system_chunks.push(AnthropicTextBlock::text(text));
                    }
                }
                Role::Assistant => Self::extend_request_messages(
                    &mut messages,
                    Self::assistant_messages_from_parts(message),
                ),
                Role::User => Self::push_request_message(
                    &mut messages,
                    AnthropicMessage {
                        role: "user".to_owned(),
                        content: Self::content_to_blocks(message),
                    },
                ),
                Role::Tool => Self::extend_request_messages(
                    &mut messages,
                    Self::tool_messages_from_parts(message),
                ),
            }
        }
        Self::apply_prompt_cache_hints(
            system_chunks.as_mut_slice(),
            tools.as_deref_mut().unwrap_or(&mut []),
            messages.as_mut_slice(),
        );

        Ok((
            AnthropicMessagesRequest {
                model: model.to_string(),
                max_tokens,
                system: (!system_chunks.is_empty()).then_some(system_chunks),
                messages,
                tools,
                temperature: (!omit_sampling).then_some(request.temperature).flatten(),
                stream: stream.then_some(true),
                thinking: thinking_parts.thinking,
                output_config: thinking_parts.output_config,
                stop_sequences: request.stop_sequences.clone(),
                top_p: (!omit_sampling).then_some(request.top_p).flatten(),
                top_k: (!omit_sampling).then_some(request.top_k).flatten(),
            },
            include_thinking,
        ))
    }
}

#[async_trait]
impl ModelRuntime for AnthropicAdapter {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(crate::provider::CapabilityFamily::Anthropic)
    }

    fn validate_provider_native_tools_request(
        &self,
        _adapter_id: Option<&crate::model::AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), AppError> {
        self.tools(request).map(|_| ())
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        let mut fields = vec![
            ("auth_scope", self.api_key.prompt_cache_scope()),
            ("base_url", self.prompt_cache_base_url().to_owned()),
            (
                "profile",
                match self.profile {
                    AnthropicProfile::Standard => "standard",
                    AnthropicProfile::GithubCopilot => "github_copilot",
                }
                .to_owned(),
            ),
            ("auth_header", self.auth_header.clone()),
            (
                "bundled_base_url",
                Self::is_bundled_base_url(self.base_url.as_str()).to_string(),
            ),
            (
                "eager_input_streaming",
                self.supports_eager_input_streaming().to_string(),
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
        if let Some(messages_url) = self.messages_url.as_deref() {
            fields.push(("messages_url", messages_url.to_owned()));
        }
        if let Some(auth_scheme) = self.auth_scheme.as_deref() {
            fields.push(("auth_scheme", auth_scheme.to_owned()));
        }
        Some(crate::provider::PromptCacheShape::from_fields(
            self.id.as_str(),
            fields,
        ))
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let endpoint = self.models_endpoint()?;
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let mut headers = self.auth_headers(api_key, None);
            headers.insert("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned());
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

        let payload: AnthropicModelListResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            ADAPTER_KIND,
            "list_models",
            response,
        )
        .await?;
        Ok(payload
            .into_items()
            .into_iter()
            .filter(|m| {
                self.profile != AnthropicProfile::GithubCopilot
                    || (m.copilot.visible() && m.copilot.uses_messages_endpoint())
            })
            .map(|m| {
                let metadata = m.copilot.metadata(m.id.as_str());
                let model_id = ModelId::new(m.id);
                let mut capabilities = self.model_capabilities(&model_id);
                if self.profile == AnthropicProfile::GithubCopilot {
                    capabilities = m
                        .copilot
                        .capabilities()
                        .merged_with_fallbacks_from(&capabilities);
                }
                ProviderModel {
                    provider_id: ProviderId::new(PROVIDER_ID),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name: m.display_name.or(m.name),
                    capabilities,
                    metadata,
                    thinking_modes: BTreeMap::new(),
                    speed_modes: BTreeMap::new(),
                }
            })
            .collect())
    }

    #[tracing::instrument(skip_all, fields(provider = tracing::field::Empty, model = %request.model))]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let stream_fallback_request = request.clone();
        let (body, include_thinking) = self.messages_request_body(&request, false)?;
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;

        let response: AnthropicMessagesResponse = self
            .send_json(
                "complete.messages",
                self.messages_endpoint()?,
                &body_json,
                Some(&request),
            )
            .await?;

        let text = response
            .content
            .iter()
            .filter(|c| c.kind == "text")
            .filter_map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("");

        let reasoning_text = if include_thinking {
            let thinking = response
                .content
                .iter()
                .filter(|c| c.kind == "thinking")
                .filter_map(|c| c.thinking.clone())
                .collect::<Vec<_>>()
                .join("");
            if thinking.is_empty() {
                None
            } else {
                Some(thinking)
            }
        } else {
            None
        };

        let tool_calls = response
            .content
            .iter()
            .filter(|c| c.kind == "tool_use")
            .map(|c| {
                let id = utils::normalize_optional_text(c.id.clone()).ok_or_else(|| {
                    AppError::Provider("anthropic returned tool_use block without id".to_owned())
                })?;

                let name = utils::optional_non_empty(c.name.clone()).ok_or_else(|| {
                    AppError::Provider("anthropic returned tool_use block without name".to_owned())
                })?;

                Ok(CompletionToolCall::Function {
                    id,
                    name,
                    arguments_json: c
                        .input
                        .as_ref()
                        .map(json_value_to_string)
                        .unwrap_or_else(|| "{}".to_owned()),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let finish_reason = CompletionFinishReason::from_provider(response.stop_reason.as_deref());
        let provider_metadata = anthropic_thinking_metadata(response.content.as_slice());

        if text.is_empty() && tool_calls.is_empty() {
            return self
                .complete_by_aggregating_stream(stream_fallback_request)
                .await;
        }

        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id.as_str()),
            model: ModelId::new(response.model),
            text,
            reasoning_text,
            finish_reason,
            tool_calls,
            usage: Self::map_usage(response.usage),
            provider_metadata,
        })
    }

    #[tracing::instrument(skip_all, fields(provider = tracing::field::Empty, model = %request.model))]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let model = request.model.clone();
        let (body, include_thinking) = self.messages_request_body(&request, true)?;
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let endpoint = self
                .messages_endpoint()
                .expect("messages endpoint should resolve");
            let mut headers = self.auth_headers(api_key, Some(&request));
            headers.insert("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned());
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::logged_json_post(
                &self.client,
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.messages",
                endpoint.as_str(),
                &headers,
                &body_json,
            )
        })
        .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response_logged(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.messages",
                response,
            )
            .await);
        }

        utils::adapter_log_http_response_open(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete_stream.messages",
            response.status(),
            response.headers(),
        );
        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(self.id.as_str());
        let model_name = model;
        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: HashMap<usize, AnthropicToolCallState> = HashMap::new();
            let mut thinking_blocks = BTreeMap::<usize, AnthropicThinkingBlockState>::new();
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_usage: Option<AnthropicUsage> = None;
            let mut stream_has_content = false;

            while let Some(event) = events.next().await {
                let event = event?;
                utils::adapter_log_stream_event(
                    provider_id.as_ref(),
                    ADAPTER_KIND,
                    "complete_stream.messages",
                    &event,
                );
                let parsed: AnthropicSseEvent =
                    utils::parse_json_value(provider_id.as_ref(), "stream event", event)?;

                match parsed {
                    AnthropicSseEvent::MessageStart { message } => {
                        if let Some(usage) = message.usage {
                            stream_usage =
                                Some(merge_anthropic_usage(stream_usage.take(), usage));
                        }
                    }
                    AnthropicSseEvent::ContentBlockStart {
                        index,
                        content_block,
                    } => {
                        if matches!(
                            content_block.kind.as_str(),
                            "thinking" | "redacted_thinking"
                        ) {
                            let index = index.ok_or_else(|| {
                                AppError::Provider(
                                    "anthropic thinking stream event missing content block index"
                                        .to_owned(),
                                )
                            })?;
                            thinking_blocks.insert(
                                index,
                                AnthropicThinkingBlockState {
                                    kind: content_block.kind,
                                    thinking: content_block.thinking.unwrap_or_default(),
                                    signature: content_block.signature,
                                    data: content_block.data,
                                },
                            );
                            continue;
                        }
                        if content_block.kind != "tool_use" {
                            continue;
                        }

                        let index = index.ok_or_else(|| {
                            AppError::Provider(
                                "anthropic tool_use stream event missing content block index"
                                    .to_owned(),
                            )
                        })?;

                        let id = utils::normalize_optional_text(content_block.id.clone()).ok_or_else(|| {
                            AppError::Provider(
                                "anthropic tool_use stream event missing tool id".to_owned(),
                            )
                        })?;
                        let name = utils::optional_non_empty(content_block.name.clone()).ok_or_else(|| {
                            AppError::Provider(
                                "anthropic tool_use stream event missing tool name".to_owned(),
                            )
                        })?;

                        let state = pending_tool_calls.entry(index).or_default();
                        state.id = id;
                        state.name = name;

                        let arguments_delta = content_block
                            .input
                            .as_ref()
                            .map(json_value_to_string)
                            .filter(|value| !value.is_empty() && value != "{}")
                            .unwrap_or_default();

                        // Always emit at least one ToolCallDelta so the
                        // shared aggregator records the tool call. Without
                        // this, a tool_use block whose input arrives only
                        // via the start event (or with no input at all)
                        // would be dropped because the aggregator only
                        // tracks calls it has seen a delta for.
                        stream_has_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: format!("idx:{index}"),
                            id: Some(state.id.clone()),
                            name: Some(state.name.clone()),
                            arguments_delta,
                        };
                    }
                    AnthropicSseEvent::ContentBlockDelta { index, delta } => {
                        if let Some(index) = index
                            && let Some(block) = thinking_blocks.get_mut(&index)
                        {
                            if let Some(thinking) = delta.thinking.as_deref() {
                                block.thinking.push_str(thinking);
                            }
                            if let Some(signature) = delta.signature.filter(|value| !value.is_empty()) {
                                block.signature = Some(signature);
                            }
                        }
                        // Text content
                        if let Some(text_delta) = delta.text.clone().filter(|v| !v.is_empty()) {
                            stream_has_content = true;
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta: text_delta,
                            };
                        }

                        // Thinking/reasoning content — only yield when thinking was requested
                        if include_thinking
                            && let Some(thinking_delta) = delta.thinking.clone().filter(|v| !v.is_empty()) {
                                stream_has_content = true;
                                yield CompletionStreamEvent::ThinkingDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    delta: thinking_delta,
                                };
                            }

                        let is_tool_delta = matches!(delta.kind.as_deref(), Some("input_json_delta"));
                        if is_tool_delta {
                            let Some(arguments_delta) = utils::optional_non_empty(delta.partial_json.clone())
                            else {
                                continue;
                            };

                            let index = index.ok_or_else(|| {
                                AppError::Provider(
                                    "anthropic tool delta event missing content block index"
                                        .to_owned(),
                                )
                            })?;

                            let state = pending_tool_calls.get_mut(&index).ok_or_else(|| {
                                AppError::Provider(
                                    "anthropic tool delta received before tool_use start"
                                        .to_owned(),
                                )
                            })?;

                            stream_has_content = true;
                            yield CompletionStreamEvent::ToolCallDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                stream_key: format!("idx:{index}"),
                                id: Some(state.id.clone()),
                                name: Some(state.name.clone()),
                                arguments_delta,
                            };
                        }
                    }
                    AnthropicSseEvent::ContentBlockStop { index } => {
                        if let Some(index) = index {
                            pending_tool_calls.remove(&index);
                        }
                    }
                    AnthropicSseEvent::MessageDelta {
                        delta,
                        usage,
                        message,
                    } => {
                        if stream_finish_reason.is_none() {
                            stream_finish_reason = delta
                                .stop_reason
                                .or_else(|| message.as_ref().and_then(|item| item.stop_reason.clone()));
                        }

                        if let Some(usage) = usage.or_else(|| message.and_then(|item| item.usage)) {
                            stream_usage =
                                Some(merge_anthropic_usage(stream_usage.take(), usage));
                        }
                    }
                    AnthropicSseEvent::MessageStop { usage, message } => {
                        if stream_finish_reason.is_none() {
                            stream_finish_reason = message
                                .as_ref()
                                .and_then(|item| item.stop_reason.clone());
                        }

                        if let Some(usage) = usage.or_else(|| message.and_then(|item| item.usage)) {
                            stream_usage =
                                Some(merge_anthropic_usage(stream_usage.take(), usage));
                        }

                        break;
                    }
                    AnthropicSseEvent::Other => {}
                }
            }

            if stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some() {
                let thinking_blocks = thinking_blocks
                    .into_values()
                    .filter_map(AnthropicThinkingBlockState::into_value)
                    .collect::<Vec<_>>();
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage.map(map_anthropic_usage),
                    provider_metadata: (!thinking_blocks.is_empty()).then(|| {
                        serde_json::json!({ "anthropic_thinking_blocks": thinking_blocks })
                    }),
                };
            }
        };

        Ok(Box::pin(stream))
    }
}
