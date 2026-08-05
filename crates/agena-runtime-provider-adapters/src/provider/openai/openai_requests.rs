use super::openai_response_types::{OpenAiResponsesResponse, OpenAiUsage};
use crate::provider::{CompletionResponse, ModelRuntime};
use agena_provider::CompletionStreamEvent;
use agena_provider::ProviderNativeToolRoute;
use agena_provider::{
    CompletionFinishReason, CompletionUsage, merge_openai_chat_reasoning_details,
    openai_chat_extract_reasoning_text, openai_chat_extract_text, openai_chat_reasoning_field,
    openai_chat_reasoning_field_from_delta,
};
use futures_util::{SinkExt, StreamExt};

use super::{
    CHAT_COMPLETIONS_ADAPTER_KIND, ChatCompletionRequest, ChatStreamOptions, CompletionRequest,
    ModelId, OpenAiChatCompletionResponse, OpenAiResponsesAdapter, OpenAiResponsesBackend,
    OpenAiResponsesToolPlan, OpenAiTransport, ProviderError, ProviderId, ProviderNativeToolKind,
    REALTIME_ADAPTER_KIND, RequestHeaderContext, Stream, ToolStreamAccumulator,
    chat_tool_stream_input, chat_wire, completion_event_from_tool_stream_update, prompt_cache,
    response_id_metadata, responses_finish_reason_with_tool_calls,
    responses_provider_native_tool_event, responses_tool_stream_input, responses_wire_tool_name,
    sse, utils,
};

impl OpenAiTransport {
    pub(super) fn image_generation_tool_definition(
        config: &agena_provider::ProviderHostedImageGenerationConfig,
    ) -> Result<serde_json::Value, ProviderError> {
        let mut map = serde_json::Map::new();
        map.insert(
            "type".to_owned(),
            serde_json::Value::String("image_generation".to_owned()),
        );
        map.insert(
            "output_format".to_owned(),
            serde_json::Value::String("png".to_owned()),
        );
        if let Some(background) = config.background.as_ref() {
            map.insert(
                "background".to_owned(),
                serde_json::Value::String(background.clone()),
            );
        }
        if let Some(size) = config.size.as_ref() {
            map.insert("size".to_owned(), serde_json::Value::String(size.clone()));
        }
        if let Some(quality) = config.quality.as_ref() {
            map.insert(
                "quality".to_owned(),
                serde_json::Value::String(quality.clone()),
            );
        }
        if let Some(moderation) = config.moderation.as_ref() {
            map.insert(
                "moderation".to_owned(),
                serde_json::Value::String(moderation.clone()),
            );
        }
        Self::merge_tool_provider_options(
            &mut map,
            config.provider_options.as_ref(),
            "image_generation",
        )?;
        Ok(serde_json::Value::Object(map))
    }

    pub(super) async fn complete_with_chat_api(
        &self,
        request: &CompletionRequest,
        model: String,
    ) -> Result<CompletionResponse, ProviderError> {
        if !request.provider_native_tools.bindings().is_empty() {
            return Err(ProviderError::Config(format!(
                "provider `{}` model `{}` configures provider-hosted tools, but the OpenAI Chat Completions adapter does not support them; use the `openai_responses` adapter instead",
                self.id, model
            )));
        }
        let model_id = ModelId::new(model.clone());
        let prompt_cache_key = self
            .supports_chat_prompt_cache_key()
            .then(|| request.prompt_cache_key.clone())
            .flatten();
        let session_affinity = self
            .uses_chat_compatible_request_fields()
            .then_some(request.prompt_cache_key.as_deref())
            .flatten();
        let mut request_override = request.request_override.clone();
        let assistant_reasoning_field = self.assistant_reasoning_field_for_model(&model_id);
        self.apply_dashscope_reasoning_overrides(
            &model_id,
            request.thinking.as_ref(),
            &mut request_override,
        );

        let official_openai_chat = self.is_official_openai_endpoint();
        let body = ChatCompletionRequest {
            model: model.clone(),
            messages: self.chat_messages_for_request(request, assistant_reasoning_field),
            tools: self.chat_tools_for_request(request),
            temperature: request.temperature,
            max_tokens: (!official_openai_chat)
                .then_some(request.max_output_tokens)
                .flatten(),
            max_completion_tokens: official_openai_chat
                .then_some(request.max_output_tokens)
                .flatten(),
            cache_control: self
                .supports_top_level_prompt_cache()
                .then(prompt_cache::PromptCacheControl::ephemeral),
            prompt_cache_key: prompt_cache_key.clone(),
            parallel_tool_calls: request.request_override.parallel_tool_calls(),
            stream: false,
            stream_options: None,
            stop: request.stop_sequences.clone(),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
            verbosity: request.verbosity.clone(),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request_override.body_patch)?;

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let endpoint = self.chat_endpoint().expect("chat endpoint should resolve");
            let mut headers = self.auth_headers(
                RequestHeaderContext::from_chat_request(request, session_affinity),
                api_key,
            );
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                CHAT_COMPLETIONS_ADAPTER_KIND,
                "complete.chat",
                "POST",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                Some(&body_json),
            );
            utils::apply_resolved_request_headers(self.client.post(endpoint), &headers)
                .json(&body_json)
        })
        .await?;

        let payload: OpenAiChatCompletionResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            CHAT_COMPLETIONS_ADAPTER_KIND,
            "complete.chat",
            response,
        )
        .await?;
        let payload = Self::unwrap_chat_completion_response(payload);
        let response_message = payload
            .choices
            .first()
            .and_then(|choice| choice.message.as_ref())
            .or_else(|| {
                payload
                    .choices
                    .first()
                    .and_then(|choice| choice.delta.as_ref())
            });
        let response_reasoning_field = response_message
            .and_then(openai_chat_reasoning_field_from_delta)
            .or(assistant_reasoning_field);
        let reasoning_details =
            response_message.and_then(|message| message.reasoning_details.clone());
        let copilot_reasoning_opaque =
            response_message.and_then(|message| message.reasoning_opaque.clone());
        let mut parsed =
            chat_wire::parse_completion_response(self.id.as_str(), model.as_str(), payload)?;
        parsed.provider_metadata = utils::provider_metadata_with_chat_reasoning_state(
            parsed.provider_metadata.take(),
            response_reasoning_field,
            reasoning_details,
            copilot_reasoning_opaque,
        );
        Ok(parsed)
    }

    pub(super) async fn complete_stream_with_chat_api(
        &self,
        request: &CompletionRequest,
        model: String,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        if !request.provider_native_tools.bindings().is_empty() {
            return Err(ProviderError::Config(format!(
                "provider `{}` model `{}` configures provider-hosted tools, but the OpenAI Chat Completions adapter does not support them; use the `openai_responses` adapter instead",
                self.id, model
            )));
        }
        let model_id = ModelId::new(model.clone());
        let prompt_cache_key = self
            .supports_chat_prompt_cache_key()
            .then(|| request.prompt_cache_key.clone())
            .flatten();
        let session_affinity = self
            .uses_chat_compatible_request_fields()
            .then_some(request.prompt_cache_key.as_deref())
            .flatten();
        let mut request_override = request.request_override.clone();
        let assistant_reasoning_field = self.assistant_reasoning_field_for_model(&model_id);
        self.apply_dashscope_reasoning_overrides(
            &model_id,
            request.thinking.as_ref(),
            &mut request_override,
        );

        let official_openai_chat = self.is_official_openai_endpoint();
        let stream_usage_requested = self.supports_chat_stream_usage();
        let body = ChatCompletionRequest {
            model: model.clone(),
            messages: self.chat_messages_for_request(request, assistant_reasoning_field),
            tools: self.chat_tools_for_request(request),
            temperature: request.temperature,
            max_tokens: (!official_openai_chat)
                .then_some(request.max_output_tokens)
                .flatten(),
            max_completion_tokens: official_openai_chat
                .then_some(request.max_output_tokens)
                .flatten(),
            cache_control: self
                .supports_top_level_prompt_cache()
                .then(prompt_cache::PromptCacheControl::ephemeral),
            prompt_cache_key: prompt_cache_key.clone(),
            parallel_tool_calls: request.request_override.parallel_tool_calls(),
            stream: true,
            stream_options: stream_usage_requested.then_some(ChatStreamOptions {
                include_usage: true,
            }),
            stop: request.stop_sequences.clone(),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
            verbosity: request.verbosity.clone(),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request_override.body_patch)?;

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let endpoint = self.chat_endpoint().expect("chat endpoint should resolve");
            let mut headers = self.auth_headers(
                RequestHeaderContext::from_chat_request(request, session_affinity),
                api_key,
            );
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                CHAT_COMPLETIONS_ADAPTER_KIND,
                "complete_stream.chat",
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
                CHAT_COMPLETIONS_ADAPTER_KIND,
                "complete_stream.chat",
                response,
            )
            .await);
        }

        utils::adapter_log_http_response_open(
            self.id.as_str(),
            CHAT_COMPLETIONS_ADAPTER_KIND,
            "complete_stream.chat",
            response.status(),
            response.headers(),
        );
        let provider_name = self.id.clone();
        let mut events = sse::json_events_with_done(response);
        let provider_id = ProviderId::new(provider_name.as_str());
        let model_name = ModelId::new(model);

        let stream = async_stream::try_stream! {
            // Chat-compatible providers usually identify a tool call by a
            // stable `id`, but some gateways also vary (or replay) the
            // positional `index` while streaming one logical call. Reuse the
            // Responses-path accumulator so id and index are aliases rather
            // than separate model-visible operations.
            let mut tool_stream = ToolStreamAccumulator::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut assistant_reasoning_field_seen: Option<&'static str> = None;
            let mut reasoning_details_seen: Option<serde_json::Value> = None;
            let mut copilot_reasoning_opaque: Option<String> = None;
            let mut done_seen = false;

            while let Some(event) = events.next().await {
                let event = match event? {
                    sse::JsonEventPayload::Event(event) => event,
                    sse::JsonEventPayload::Done => {
                        done_seen = true;
                        break;
                    }
                };
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    CHAT_COMPLETIONS_ADAPTER_KIND,
                    "complete_stream.chat",
                    &event,
                );

                if let Some(err) = utils::chat_stream_error(provider_name.as_str(), &event) {
                    Err(err)?;
                }

                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(provider_name.as_str(), "chat stream chunk", event)?;
                let choice = chunk.choices.first();

                let delta = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.content.as_ref())
                    .map(openai_chat_extract_text)
                    .or_else(|| choice.and_then(|item| item.text.clone()))
                    .unwrap_or_default();

                if !delta.is_empty() {
                    yield CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                let response_delta = choice.and_then(|item| item.delta.as_ref());
                if let Some(delta) = response_delta {
                    if assistant_reasoning_field_seen.is_none() {
                        assistant_reasoning_field_seen =
                            openai_chat_reasoning_field(
                                delta.reasoning_content.as_ref(),
                                delta.reasoning_details.as_ref(),
                            );
                    }
                    if let Some(details) = delta.reasoning_details.as_ref() {
                        merge_openai_chat_reasoning_details(&mut reasoning_details_seen, details);
                    }
                    if let Some(opaque) = delta
                        .reasoning_opaque
                        .as_deref()
                        .filter(|value| !value.is_empty())
                    {
                        if copilot_reasoning_opaque
                            .as_deref()
                            .is_some_and(|current| current != opaque)
                        {
                            Err(ProviderError::Provider(format!(
                                "{provider_name} returned multiple Copilot reasoning_opaque values in one response"
                            )))?;
                        }
                        copilot_reasoning_opaque = Some(opaque.to_owned());
                    }
                }
                let reasoning_delta = response_delta
                    .and_then(|delta| {
                        openai_chat_extract_reasoning_text(
                            delta.reasoning_content.as_ref(),
                            delta.reasoning_details.as_ref(),
                            delta.reasoning_text.as_ref(),
                        )
                    })
                    .unwrap_or_default();

                if !reasoning_delta.is_empty() {
                    yield CompletionStreamEvent::ThinkingDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta: reasoning_delta,
                    };
                }

                let tool_deltas = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.tool_calls.clone())
                    .unwrap_or_default();

                for raw_tool in tool_deltas {
                    let tool = utils::parse_json_value::<chat_wire::ChatToolCallWire>(
                        provider_name.as_str(),
                        "chat stream tool_call delta",
                        raw_tool,
                    )?;
                    let input = chat_tool_stream_input(provider_name.as_str(), tool)?;
                    for update in tool_stream.ingest(provider_name.as_str(), input)? {
                        yield completion_event_from_tool_stream_update(
                            &provider_id,
                            &model_name,
                            update,
                        );
                    }
                }

                if let Some(raw_usage) = chunk.usage {
                    let usage = utils::parse_json_value::<chat_wire::ChatUsage>(
                        provider_name.as_str(),
                        "chat stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Some(chat_wire::chat_usage_to_completion(usage));
                }

                let finish_reason = choice
                    .and_then(|item| item.finish_reason.as_deref())
                    .filter(|value| !value.is_empty() && *value != "null")
                    .map(ToOwned::to_owned);

                if stream_finish_reason.is_none() {
                    stream_finish_reason = finish_reason;
                }
            }

            utils::require_terminal_stream_event(
                provider_name.as_str(),
                "chat completions",
                stream_finish_reason.is_some(),
            )?;
            if stream_usage_requested {
                utils::require_terminal_stream_event(
                    provider_name.as_str(),
                    "chat completions [DONE]",
                    done_seen,
                )?;
            }
            yield CompletionStreamEvent::Completed {
                provider_id: provider_id.clone(),
                model: model_name.clone(),
                finish_reason: CompletionFinishReason::from_provider(
                    stream_finish_reason.as_deref(),
                ),
                usage: stream_usage,
                provider_metadata: utils::provider_metadata_with_chat_reasoning_state(
                    None,
                    assistant_reasoning_field_seen.or(assistant_reasoning_field),
                    reasoning_details_seen,
                    copilot_reasoning_opaque,
                ),
                end_turn: None,
            };
        };

        Ok(Box::pin(stream))
    }

    pub(super) async fn complete_stream_with_realtime_ws(
        &self,
        request: &CompletionRequest,
        model: String,
        realtime_ws_url: Option<&str>,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let ws_endpoint = self.realtime_ws_endpoint(model.as_str(), realtime_ws_url)?;
        let api_key = self.api_key.resolve().await?;
        let handshake = self.realtime_handshake_request(
            &ws_endpoint,
            api_key.as_str(),
            self.uses_chat_compatible_request_fields()
                .then_some(request.prompt_cache_key.as_deref())
                .flatten(),
        )?;
        let handshake_headers = handshake
            .headers()
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (key.as_str().to_owned(), text.to_owned()))
            })
            .collect::<Vec<_>>();
        utils::adapter_log_http_request_json(
            self.id.as_str(),
            REALTIME_ADAPTER_KIND,
            "complete_stream.realtime_ws.handshake",
            "GET",
            ws_endpoint.as_str(),
            handshake_headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
            None,
        );
        let (ws_stream, handshake_response) = tokio_tungstenite::connect_async(handshake)
            .await
            .map_err(|err| {
                ProviderError::Provider(format!("openai realtime websocket connect failed: {err}"))
            })?;
        utils::adapter_log_http_response_open(
            self.id.as_str(),
            REALTIME_ADAPTER_KIND,
            "complete_stream.realtime_ws.handshake",
            handshake_response.status(),
            handshake_response.headers(),
        );

        let provider_name = self.id.clone();
        let provider_id = ProviderId::new(provider_name.as_str());
        let model_name = ModelId::new(model);
        let conversation_items =
            Self::realtime_conversation_items_for_messages(request.messages.as_slice())?;
        let tool_plan = self.responses_tool_plan(request)?;
        let response_tools =
            (!tool_plan.tools.is_empty()).then_some(serde_json::Value::Array(tool_plan.tools));
        let system = request.system.clone();
        let temperature = request.temperature;
        let max_output_tokens = request.max_output_tokens;

        let stream = async_stream::try_stream! {
            let (mut ws_writer, mut ws_reader) = ws_stream.split();

            if let Some(instructions) = system.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                let event = serde_json::json!({
                    "type": "session.update",
                    "session": {
                        "instructions": instructions,
                    }
                });
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    REALTIME_ADAPTER_KIND,
                    "complete_stream.realtime_ws.outbound",
                    &event,
                );
                ws_writer
                    .send(tokio_tungstenite::tungstenite::Message::Text(event.to_string().into()))
                    .await
                    .map_err(|err| {
                        ProviderError::Provider(format!(
                            "openai realtime websocket send session.update failed: {err}"
                        ))
                    })?;
            }

            for item in &conversation_items {
                let event = serde_json::json!({
                    "type": "conversation.item.create",
                    "item": item,
                });
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    REALTIME_ADAPTER_KIND,
                    "complete_stream.realtime_ws.outbound",
                    &event,
                );

                ws_writer
                    .send(tokio_tungstenite::tungstenite::Message::Text(event.to_string().into()))
                    .await
                    .map_err(|err| {
                        ProviderError::Provider(format!(
                            "openai realtime websocket send conversation.item.create failed: {err}"
                        ))
                    })?;
            }

            let mut response = serde_json::json!({
                "output_modalities": ["text"],
            });
            if let Some(temperature) = temperature {
                response["temperature"] = serde_json::json!(temperature);
            }
            if let Some(max_tokens) = max_output_tokens {
                response["max_output_tokens"] = serde_json::json!(max_tokens);
            }
            if let Some(tools) = response_tools.as_ref() {
                response["tools"] = tools.clone();
            }

            let create_event = serde_json::json!({
                "type": "response.create",
                "response": response,
            });
            utils::adapter_log_stream_event(
                provider_name.as_str(),
                REALTIME_ADAPTER_KIND,
                "complete_stream.realtime_ws.outbound",
                &create_event,
            );

            ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Text(create_event.to_string().into()))
                .await
                .map_err(|err| {
                    ProviderError::Provider(format!(
                        "openai realtime websocket send response.create failed: {err}"
                    ))
                })?;

            let mut tool_stream = ToolStreamAccumulator::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_tool_call_seen = false;
            let mut completed_emitted = false;
            let mut response_id: Option<String> = None;

            while let Some(message) = ws_reader.next().await {
                let message = message.map_err(|err| {
                    ProviderError::Provider(format!("openai realtime websocket receive failed: {err}"))
                })?;

                let payload = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        String::from_utf8(bytes.to_vec()).map_err(|err| {
                            ProviderError::Provider(format!(
                                "openai realtime websocket binary frame is not utf-8: {err}"
                            ))
                        })?
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    tokio_tungstenite::tungstenite::Message::Ping(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Frame(_) => continue,
                };

                let event: serde_json::Value = serde_json::from_str(payload.as_str()).map_err(|err| {
                    ProviderError::Provider(format!("openai realtime websocket event decode failed: {err}"))
                })?;
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    REALTIME_ADAPTER_KIND,
                    "complete_stream.realtime_ws.inbound",
                    &event,
                );

                if let Some(err) = utils::responses_stream_error(provider_name.as_str(), &event)? {
                    Err(err)?;
                }

                if let Some(delta) = utils::responses_text_delta(&event) {
                    yield CompletionStreamEvent::TextDelta {
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
                        "realtime stream usage",
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
                        end_turn: utils::responses_end_turn(&event),
                    };
                    completed_emitted = true;
                    break;
                }
            }

            let _ = ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Close(None))
                .await;

            utils::require_terminal_stream_event(
                provider_name.as_str(),
                "realtime",
                completed_emitted,
            )?;
        };

        Ok(Box::pin(stream))
    }

    pub(super) fn extract_text(response: &OpenAiResponsesResponse) -> String {
        if let Some(text) = response.output_text.as_ref() {
            return text.clone();
        }

        response
            .output
            .iter()
            .flatten()
            .filter(|item| item.kind.as_deref() != Some("reasoning"))
            .flat_map(|item| item.content.iter().flatten())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("")
    }

    pub(super) fn extract_reasoning_text(response: &OpenAiResponsesResponse) -> Option<String> {
        let summaries: String = response
            .output
            .iter()
            .flatten()
            .filter(|item| item.kind.as_deref() == Some("reasoning"))
            .flat_map(|item| item.summary.iter().flatten())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("");
        if !summaries.is_empty() {
            return Some(summaries);
        }

        let text: String = response
            .output
            .iter()
            .flatten()
            .filter(|item| item.kind.as_deref() == Some("reasoning"))
            .flat_map(|item| item.content.iter().flatten())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("");
        (!text.is_empty()).then_some(text)
    }

    pub(super) fn map_usage(usage: Option<OpenAiUsage>) -> Option<CompletionUsage> {
        usage.map(|u| {
            let input_tokens_raw = u.input_tokens.unwrap_or_default();
            let cache_read_tokens = u
                .input_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
                .unwrap_or_default();
            let cache_write_tokens = u
                .input_tokens_details
                .as_ref()
                .and_then(|d| d.cache_write_tokens)
                .unwrap_or_default();
            let detailed_reasoning_tokens =
                u.output_tokens_details.and_then(|d| d.reasoning_tokens);
            let raw_output_tokens = u.output_tokens.unwrap_or_default();
            // Copilot-compatible Responses implementations can report
            // reasoning outside `output_tokens`, while OpenAI includes it.
            // `total_tokens` disambiguates the two shapes without guessing.
            let separately_reported_reasoning_tokens = u.total_tokens.and_then(|total| {
                total
                    .checked_sub(input_tokens_raw.saturating_add(raw_output_tokens))
                    .filter(|tokens| *tokens > 0)
            });
            let reasoning_tokens = detailed_reasoning_tokens
                .or(separately_reported_reasoning_tokens)
                .unwrap_or_default();
            // Match Anthropic's convention: `input_tokens` is the uncached
            // portion only. OpenAI's `input_tokens` is inclusive of cache.
            let input_tokens = input_tokens_raw
                .saturating_sub(cache_read_tokens)
                .saturating_sub(cache_write_tokens);
            let total_without_separate_reasoning =
                input_tokens_raw.saturating_add(raw_output_tokens);
            let total_with_separate_reasoning =
                total_without_separate_reasoning.saturating_add(reasoning_tokens);
            let output_includes_reasoning = match u.total_tokens {
                Some(total)
                    if reasoning_tokens > 0
                        && total == total_with_separate_reasoning
                        && total != total_without_separate_reasoning =>
                {
                    false
                }
                Some(total) if total == total_without_separate_reasoning => reasoning_tokens > 0,
                _ => detailed_reasoning_tokens.is_some(),
            };
            let output_tokens = if output_includes_reasoning {
                raw_output_tokens.saturating_sub(reasoning_tokens)
            } else {
                raw_output_tokens
            };
            let recorded_cost = u
                .cost_in_usd_ticks
                .map(|ticks| ticks as f64 / 10_000_000_000.0);
            CompletionUsage {
                requests: 1,
                input_tokens,
                output_tokens,
                reasoning_tokens,
                cache_write_tokens,
                cache_read_tokens,
                total_cost: recorded_cost.unwrap_or_default(),
                recorded_cost: recorded_cost.unwrap_or_default(),
                recorded_cost_available: recorded_cost.is_some(),
                ..CompletionUsage::default()
            }
        })
    }

    pub(super) fn merge_tool_provider_options(
        map: &mut serde_json::Map<String, serde_json::Value>,
        extra: Option<&serde_json::Value>,
        tool_label: &str,
    ) -> Result<(), ProviderError> {
        let Some(extra) = extra else {
            return Ok(());
        };
        let extra = extra.as_object().ok_or_else(|| {
            ProviderError::Config(format!(
                "openai provider-native tool `{tool_label}` provider_options must be a JSON object"
            ))
        })?;
        for (key, value) in extra {
            map.insert(key.clone(), value.clone());
        }
        Ok(())
    }

    pub(super) fn responses_tool_plan(
        &self,
        request: &CompletionRequest,
    ) -> Result<OpenAiResponsesToolPlan, ProviderError> {
        let mut tools = Vec::new();
        let mut include = Vec::new();
        for tool in &request.tool_api_functions {
            let wire_name = responses_wire_tool_name(tool.name.as_str())?;
            let mut map = serde_json::Map::new();
            map.insert(
                "type".to_owned(),
                serde_json::Value::String("function".to_owned()),
            );
            map.insert("name".to_owned(), serde_json::Value::String(wire_name));
            map.insert(
                "description".to_owned(),
                serde_json::Value::String(tool.description.clone()),
            );
            map.insert("parameters".to_owned(), tool.input_schema.clone());
            if tool.strict {
                map.insert("strict".to_owned(), serde_json::Value::Bool(true));
            }
            tools.push(serde_json::Value::Object(map));
        }

        for binding in request.provider_native_tools.bindings() {
            if binding.route != ProviderNativeToolRoute::ProviderHosted {
                return Err(ProviderError::Config(format!(
                    "openai provider-native tool `{}` only supports `provider_hosted` routes in the current runtime",
                    binding.tool.config_key()
                )));
            }
            match binding.tool {
                ProviderNativeToolKind::WebSearch => {
                    let config = &request.provider_native_tools.hosted.web_search;
                    if config.max_results.is_some() {
                        return Err(ProviderError::Config(
                            "openai provider-native tool `web_search` does not support `hosted.web_search.max_results`; use `provider_options` for provider-specific overrides instead".to_owned(),
                        ));
                    }
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "type".to_owned(),
                        serde_json::Value::String("web_search".to_owned()),
                    );
                    if let Some(freshness) = config.freshness {
                        match freshness {
                            agena_provider::ProviderNativeToolFreshness::Auto => {}
                            agena_provider::ProviderNativeToolFreshness::Cached => {
                                map.insert(
                                    "external_web_access".to_owned(),
                                    serde_json::Value::Bool(false),
                                );
                            }
                            agena_provider::ProviderNativeToolFreshness::Live => {
                                map.insert(
                                    "external_web_access".to_owned(),
                                    serde_json::Value::Bool(true),
                                );
                            }
                        }
                    }
                    if let Some(search_context_size) = config.search_context_size.as_ref() {
                        map.insert(
                            "search_context_size".to_owned(),
                            serde_json::Value::String(search_context_size.clone()),
                        );
                    }
                    if !config.user_location.is_empty() {
                        let mut location = serde_json::Map::new();
                        location.insert(
                            "type".to_owned(),
                            serde_json::Value::String("approximate".to_owned()),
                        );
                        if let Some(country) = config.user_location.country.as_ref() {
                            location.insert(
                                "country".to_owned(),
                                serde_json::Value::String(country.clone()),
                            );
                        }
                        if let Some(region) = config.user_location.region.as_ref() {
                            location.insert(
                                "region".to_owned(),
                                serde_json::Value::String(region.clone()),
                            );
                        }
                        if let Some(city) = config.user_location.city.as_ref() {
                            location
                                .insert("city".to_owned(), serde_json::Value::String(city.clone()));
                        }
                        if let Some(timezone) = config.user_location.timezone.as_ref() {
                            location.insert(
                                "timezone".to_owned(),
                                serde_json::Value::String(timezone.clone()),
                            );
                        }
                        map.insert(
                            "user_location".to_owned(),
                            serde_json::Value::Object(location),
                        );
                    }
                    if !config.allowed_domains.is_empty() || !config.blocked_domains.is_empty() {
                        let mut filters = serde_json::Map::new();
                        if !config.allowed_domains.is_empty() {
                            filters.insert(
                                "allowed_domains".to_owned(),
                                serde_json::Value::Array(
                                    config
                                        .allowed_domains
                                        .iter()
                                        .cloned()
                                        .map(serde_json::Value::String)
                                        .collect(),
                                ),
                            );
                        }
                        if !config.blocked_domains.is_empty() {
                            filters.insert(
                                "blocked_domains".to_owned(),
                                serde_json::Value::Array(
                                    config
                                        .blocked_domains
                                        .iter()
                                        .cloned()
                                        .map(serde_json::Value::String)
                                        .collect(),
                                ),
                            );
                        }
                        map.insert("filters".to_owned(), serde_json::Value::Object(filters));
                    }
                    Self::merge_tool_provider_options(
                        &mut map,
                        config.provider_options.as_ref(),
                        "web_search",
                    )?;
                    tools.push(serde_json::Value::Object(map));
                    include.push("web_search_call.action.sources".to_owned());
                }
                ProviderNativeToolKind::FileSearch => {
                    let config = &request.provider_native_tools.hosted.file_search;
                    if matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex)
                        || config.vector_store_ids.is_empty()
                    {
                        continue;
                    }
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "type".to_owned(),
                        serde_json::Value::String("file_search".to_owned()),
                    );
                    map.insert(
                        "vector_store_ids".to_owned(),
                        serde_json::Value::Array(
                            config
                                .vector_store_ids
                                .iter()
                                .cloned()
                                .map(serde_json::Value::String)
                                .collect(),
                        ),
                    );
                    if let Some(max_results) = config.max_results {
                        map.insert(
                            "max_num_results".to_owned(),
                            serde_json::Value::Number(max_results.into()),
                        );
                    }
                    Self::merge_tool_provider_options(
                        &mut map,
                        config.provider_options.as_ref(),
                        "file_search",
                    )?;
                    tools.push(serde_json::Value::Object(map));
                    if config.include_results.unwrap_or(false) {
                        include.push("file_search_call.results".to_owned());
                    }
                }
                ProviderNativeToolKind::CodeExecution => {
                    if matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex) {
                        continue;
                    }
                    let config = &request.provider_native_tools.hosted.code_execution;
                    let container = &config.container;
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "type".to_owned(),
                        serde_json::Value::String("code_interpreter".to_owned()),
                    );
                    if let Some(container_id) = container.id.as_ref() {
                        if container.kind.is_some()
                            || container.memory_limit.is_some()
                            || !container.file_ids.is_empty()
                        {
                            return Err(ProviderError::Config(
                                "openai provider-native tool `code_execution` cannot combine `container.id` with `container.type`, `memory_limit`, or `file_ids`".to_owned(),
                            ));
                        }
                        map.insert(
                            "container".to_owned(),
                            serde_json::Value::String(container_id.clone()),
                        );
                    } else if !container.is_empty() {
                        let kind = container.kind.as_deref().unwrap_or("auto");
                        if kind != "auto" {
                            return Err(ProviderError::Config(format!(
                                "openai provider-native tool `code_execution` only supports container type `auto`, found `{kind}`"
                            )));
                        }
                        let mut container_map = serde_json::Map::new();
                        container_map.insert(
                            "type".to_owned(),
                            serde_json::Value::String("auto".to_owned()),
                        );
                        if let Some(memory_limit) = container.memory_limit.as_ref() {
                            container_map.insert(
                                "memory_limit".to_owned(),
                                serde_json::Value::String(memory_limit.clone()),
                            );
                        }
                        if !container.file_ids.is_empty() {
                            container_map.insert(
                                "file_ids".to_owned(),
                                serde_json::Value::Array(
                                    container
                                        .file_ids
                                        .iter()
                                        .cloned()
                                        .map(serde_json::Value::String)
                                        .collect(),
                                ),
                            );
                        }
                        map.insert(
                            "container".to_owned(),
                            serde_json::Value::Object(container_map),
                        );
                    }
                    Self::merge_tool_provider_options(
                        &mut map,
                        config.provider_options.as_ref(),
                        "code_execution",
                    )?;
                    tools.push(serde_json::Value::Object(map));
                    include.push("code_interpreter_call.outputs".to_owned());
                }
                ProviderNativeToolKind::ImageGeneration => {
                    let config = &request.provider_native_tools.hosted.image_generation;
                    tools.push(Self::image_generation_tool_definition(config)?);
                }
                other => {
                    return Err(ProviderError::Config(format!(
                        "openai provider-native tool `{}` is not supported by the current runtime",
                        other.config_key()
                    )));
                }
            }
        }

        Ok(OpenAiResponsesToolPlan { tools, include })
    }
}

impl OpenAiResponsesAdapter {
    pub(super) async fn complete_by_aggregating_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let fallback_model = request.model.clone();
        let stream = ModelRuntime::complete_stream(self, request).await?;
        utils::aggregate_stream(self.id.as_str(), fallback_model, stream).await
    }
}
