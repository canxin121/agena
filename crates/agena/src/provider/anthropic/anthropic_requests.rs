use super::{
    AnthropicAdapter, AnthropicBinarySource, AnthropicMessage, AnthropicProfile,
    AnthropicTextBlock, AnthropicUsage, AppError, AttachmentItem, AttachmentKind,
    CompletionRequest, CompletionResponse, CompletionUsage, FIRST_PARTY_ANTHROPIC_HOSTS, Message,
    ModelRuntime, ProviderToolKind, ProviderToolRoute, Role, Value, anthropic_wire_tool_name,
    map_anthropic_usage, prompt_cache, utils, wire_message,
};

impl AnthropicAdapter {
    pub(crate) fn thinking_blocks_from_message(message: &Message) -> Vec<AnthropicTextBlock> {
        message
            .provider_state
            .as_ref()
            .into_iter()
            .flat_map(|state| state.anthropic_thinking_blocks.iter())
            .filter_map(|block| {
                let block = serde_json::from_value::<AnthropicTextBlock>(block.clone()).ok()?;
                matches!(block.kind.as_str(), "thinking" | "redacted_thinking").then_some(block)
            })
            .collect()
    }

    pub(crate) fn map_usage(usage: Option<AnthropicUsage>) -> Option<CompletionUsage> {
        usage.map(map_anthropic_usage)
    }

    pub(crate) async fn complete_by_aggregating_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let fallback_model = request.model.clone();
        let stream = ModelRuntime::complete_stream(self, request).await?;
        utils::aggregate_stream(self.id.as_ref(), fallback_model, stream).await
    }

    pub(crate) fn content_to_blocks(message: &Message) -> Vec<AnthropicTextBlock> {
        let projected = wire_message::project(message);
        Self::blocks_from_projected_parts(message, projected.as_slice())
    }

    pub(crate) fn blocks_from_projected_parts(
        message: &Message,
        projected: &[wire_message::WirePart],
    ) -> Vec<AnthropicTextBlock> {
        if projected.is_empty() {
            let text = message.as_text_lossy();
            if text.is_empty() {
                return Vec::new();
            }

            return vec![AnthropicTextBlock::text(text)];
        }

        let mut blocks = Vec::new();
        for part in projected {
            match part {
                wire_message::WirePart::Text { text } => {
                    blocks.push(AnthropicTextBlock::text(text.clone()));
                }
                wire_message::WirePart::Attachment { item } => {
                    blocks.extend(Self::attachment_blocks(item));
                }
                wire_message::WirePart::ToolCall {
                    id,
                    function,
                    arguments_json,
                } => blocks.push(AnthropicTextBlock::tool_use(
                    id.clone(),
                    anthropic_wire_tool_name(function.function_name()),
                    arguments_json.clone(),
                )),
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    output_json,
                    ..
                } => blocks.push(AnthropicTextBlock::tool_result(
                    tool_call_id.clone(),
                    output_json.clone(),
                )),
            }
        }

        blocks
    }

    pub(crate) fn assistant_messages_from_parts(message: &Message) -> Vec<AnthropicMessage> {
        let projected = wire_message::project(message);
        if !projected
            .iter()
            .any(|part| matches!(part, wire_message::WirePart::ToolResult { .. }))
        {
            let mut content = Self::thinking_blocks_from_message(message);
            content.extend(Self::blocks_from_projected_parts(
                message,
                projected.as_slice(),
            ));
            return vec![AnthropicMessage {
                role: "assistant".to_owned(),
                content,
            }];
        }

        let mut messages = Vec::new();
        let mut buffered = Vec::<wire_message::WirePart>::new();
        for part in &projected {
            match part {
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    output_json,
                    ..
                } if !tool_call_id.trim().is_empty() => {
                    Self::flush_assistant_blocks(message, &mut messages, &mut buffered);
                    Self::push_request_message(
                        &mut messages,
                        AnthropicMessage {
                            role: "user".to_owned(),
                            content: vec![AnthropicTextBlock::tool_result(
                                tool_call_id.clone(),
                                output_json.clone(),
                            )],
                        },
                    );
                }
                wire_message::WirePart::ToolResult { output_json, .. } => {
                    buffered.push(wire_message::WirePart::Text {
                        text: output_json.clone(),
                    });
                }
                other => buffered.push(other.clone()),
            }
        }
        Self::flush_assistant_blocks(message, &mut messages, &mut buffered);

        messages
    }

    pub(crate) fn tool_messages_from_parts(message: &Message) -> Vec<AnthropicMessage> {
        let content = wire_message::project(message)
            .into_iter()
            .filter_map(|part| match part {
                wire_message::WirePart::ToolResult {
                    tool_call_id,
                    output_json,
                    ..
                } if !tool_call_id.trim().is_empty() => {
                    Some(AnthropicTextBlock::tool_result(tool_call_id, output_json))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        (!content.is_empty())
            .then(|| AnthropicMessage {
                role: "user".to_owned(),
                content,
            })
            .into_iter()
            .collect()
    }

    pub(crate) fn push_request_message(
        messages: &mut Vec<AnthropicMessage>,
        mut message: AnthropicMessage,
    ) {
        if message.content.is_empty() {
            return;
        }
        if message.role == "user"
            && let Some(previous) = messages.last_mut()
            && previous.role == "user"
        {
            previous.content.append(&mut message.content);
            return;
        }
        messages.push(message);
    }

    pub(crate) fn extend_request_messages(
        messages: &mut Vec<AnthropicMessage>,
        extension: impl IntoIterator<Item = AnthropicMessage>,
    ) {
        for message in extension {
            Self::push_request_message(messages, message);
        }
    }

    pub(crate) fn flush_assistant_blocks(
        message: &Message,
        messages: &mut Vec<AnthropicMessage>,
        buffered: &mut Vec<wire_message::WirePart>,
    ) {
        if buffered.is_empty() {
            return;
        }
        let content = Self::blocks_from_projected_parts(message, buffered.as_slice());
        buffered.clear();
        if content.is_empty() {
            return;
        }
        let mut content = content;
        if !messages.iter().any(|message| message.role == "assistant") {
            let mut thinking = Self::thinking_blocks_from_message(message);
            thinking.append(&mut content);
            content = thinking;
        }
        Self::push_request_message(
            messages,
            AnthropicMessage {
                role: "assistant".to_owned(),
                content,
            },
        );
    }

    pub(crate) fn attachment_blocks(item: &AttachmentItem) -> Vec<AnthropicTextBlock> {
        match item.kind {
            AttachmentKind::Image => Self::binary_source(item)
                .map(AnthropicTextBlock::image)
                .into_iter()
                .collect(),
            AttachmentKind::Pdf => Self::binary_source(item)
                .map(AnthropicTextBlock::document)
                .into_iter()
                .collect(),
            AttachmentKind::File => wire_message::attachment_text(item)
                .map(AnthropicTextBlock::text)
                .into_iter()
                .collect(),
            AttachmentKind::Audio | AttachmentKind::Video => Vec::new(),
        }
        .into_iter()
        .chain(match item.kind {
            AttachmentKind::Audio | AttachmentKind::Video => {
                Some(AnthropicTextBlock::text(wire_message::hint_text(item)))
            }
            AttachmentKind::Image | AttachmentKind::Pdf if Self::binary_source(item).is_none() => {
                Some(AnthropicTextBlock::text(wire_message::hint_text(item)))
            }
            AttachmentKind::File if wire_message::attachment_text(item).is_none() => {
                Some(AnthropicTextBlock::text(wire_message::hint_text(item)))
            }
            _ => None,
        })
        .collect()
    }

    pub(crate) fn binary_source(item: &AttachmentItem) -> Option<AnthropicBinarySource> {
        wire_message::base64_with_mime(item)
            .map(|(media_type, data)| AnthropicBinarySource::base64(media_type, data))
    }

    pub(crate) fn is_vision_request(request: &CompletionRequest) -> bool {
        request.messages.iter().any(|message| {
            wire_message::project(message).iter().any(|part| {
                matches!(
                    part,
                    wire_message::WirePart::Attachment { item }
                        if item.kind == AttachmentKind::Image
                )
            })
        })
    }

    pub(crate) fn initiator(request: &CompletionRequest) -> &'static str {
        match request.messages.last().map(|m| m.role) {
            Some(Role::User) => "user",
            _ => "agent",
        }
    }

    pub(crate) fn is_bundled_base_url(base_url: &str) -> bool {
        url::Url::parse(base_url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.to_owned()))
            .map(|host| {
                FIRST_PARTY_ANTHROPIC_HOSTS
                    .iter()
                    .any(|candidate| host.eq_ignore_ascii_case(candidate))
            })
            .unwrap_or(false)
    }

    pub(crate) fn supports_eager_input_streaming(&self) -> bool {
        if let Some(enabled) = self.eager_input_streaming_override {
            return enabled;
        }

        match self.profile {
            AnthropicProfile::Standard => Self::is_bundled_base_url(self.base_url.as_str()),
            AnthropicProfile::GithubCopilot => false,
        }
    }

    pub(crate) fn merge_tool_provider_options(
        map: &mut serde_json::Map<String, Value>,
        extra: Option<&Value>,
        tool_label: &str,
    ) -> Result<(), AppError> {
        let Some(extra) = extra else {
            return Ok(());
        };
        let extra = extra.as_object().ok_or_else(|| {
            AppError::Config(format!(
                "anthropic provider tool `{tool_label}` provider_options must be a JSON object"
            ))
        })?;
        for (key, value) in extra {
            map.insert(key.clone(), value.clone());
        }
        Ok(())
    }

    pub(crate) fn tools(&self, request: &CompletionRequest) -> Result<Vec<Value>, AppError> {
        let mut tools = self.tool_api_function_tools(request);
        tools.extend(self.provider_tools(request)?);
        Ok(tools)
    }

    pub(crate) fn tool_api_function_tools(&self, request: &CompletionRequest) -> Vec<Value> {
        let mut tools = Vec::new();
        for tool in crate::tool::tool_api_definitions(request.tool_api_functions.as_slice()) {
            let mut map = serde_json::Map::new();
            map.insert(
                "name".to_owned(),
                Value::String(anthropic_wire_tool_name(tool.name.as_str())),
            );
            map.insert("description".to_owned(), Value::String(tool.description));
            map.insert("input_schema".to_owned(), tool.input_schema);
            if self.supports_eager_input_streaming() {
                map.insert("eager_input_streaming".to_owned(), Value::Bool(true));
            }
            tools.push(Value::Object(map));
        }
        tools
    }

    pub(crate) fn provider_tools(
        &self,
        request: &CompletionRequest,
    ) -> Result<Vec<Value>, AppError> {
        let mut tools = Vec::new();
        for binding in request.provider_tools.bindings() {
            if binding.route != ProviderToolRoute::ProviderHosted {
                return Err(AppError::Config(format!(
                    "anthropic provider tool `{}` only supports `provider_hosted` routes in the current runtime",
                    binding.tool.config_key()
                )));
            }
            match binding.tool {
                ProviderToolKind::WebSearch => {
                    let config = &request.provider_tools.hosted.web_search;
                    if config.freshness.is_some()
                        || config.max_results.is_some()
                        || config.search_context_size.is_some()
                    {
                        return Err(AppError::Config(
                            "anthropic provider tool `web_search` only supports domain filters, user_location, and provider_options in the current runtime".to_owned(),
                        ));
                    }
                    if !config.allowed_domains.is_empty() && !config.blocked_domains.is_empty() {
                        return Err(AppError::Config(
                            "anthropic provider tool `web_search` cannot set both `allowed_domains` and `blocked_domains` in the same request".to_owned(),
                        ));
                    }
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "type".to_owned(),
                        Value::String("web_search_20250305".to_owned()),
                    );
                    map.insert("name".to_owned(), Value::String("web_search".to_owned()));
                    if !config.allowed_domains.is_empty() {
                        map.insert(
                            "allowed_domains".to_owned(),
                            Value::Array(
                                config
                                    .allowed_domains
                                    .iter()
                                    .cloned()
                                    .map(Value::String)
                                    .collect(),
                            ),
                        );
                    }
                    if !config.blocked_domains.is_empty() {
                        map.insert(
                            "blocked_domains".to_owned(),
                            Value::Array(
                                config
                                    .blocked_domains
                                    .iter()
                                    .cloned()
                                    .map(Value::String)
                                    .collect(),
                            ),
                        );
                    }
                    if !config.user_location.is_empty() {
                        let mut location = serde_json::Map::new();
                        location.insert("type".to_owned(), Value::String("approximate".to_owned()));
                        if let Some(country) = config.user_location.country.as_ref() {
                            location.insert("country".to_owned(), Value::String(country.clone()));
                        }
                        if let Some(region) = config.user_location.region.as_ref() {
                            location.insert("region".to_owned(), Value::String(region.clone()));
                        }
                        if let Some(city) = config.user_location.city.as_ref() {
                            location.insert("city".to_owned(), Value::String(city.clone()));
                        }
                        if let Some(timezone) = config.user_location.timezone.as_ref() {
                            location.insert("timezone".to_owned(), Value::String(timezone.clone()));
                        }
                        map.insert("user_location".to_owned(), Value::Object(location));
                    }
                    Self::merge_tool_provider_options(
                        &mut map,
                        config.provider_options.as_ref(),
                        "web_search",
                    )?;
                    tools.push(Value::Object(map));
                }
                other => {
                    return Err(AppError::Config(format!(
                        "anthropic provider tool `{}` is not supported by the current runtime",
                        other.config_key()
                    )));
                }
            }
        }

        Ok(tools)
    }

    pub(crate) fn apply_prompt_cache_hints(
        system: &mut [AnthropicTextBlock],
        tools: &mut [Value],
        messages: &mut [AnthropicMessage],
    ) {
        // Keep cache markers stable across tool-use loops: tool definitions
        // and system text sit above the conversation, while the latest real
        // user message stays fixed as assistant/tool-result messages append.
        if let Some(block) = system.last_mut() {
            block.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }

        if let Some(tool) = tools.last_mut().and_then(Value::as_object_mut) {
            tool.insert(
                "cache_control".to_owned(),
                serde_json::to_value(prompt_cache::PromptCacheControl::ephemeral())
                    .expect("anthropic prompt cache control should serialize"),
            );
        }

        if let Some(block) = Self::latest_user_cache_block(messages) {
            block.cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }
    }

    pub(crate) fn latest_user_cache_block(
        messages: &mut [AnthropicMessage],
    ) -> Option<&mut AnthropicTextBlock> {
        messages.iter_mut().rev().find_map(|message| {
            if message.role != "user" || message.content.is_empty() {
                return None;
            }
            let index = message
                .content
                .iter()
                .rposition(|block| block.kind != "tool_result")?;
            message.content.get_mut(index)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_user_messages_are_combined_into_one_anthropic_turn() {
        let mut messages = vec![AnthropicMessage {
            role: "assistant".to_owned(),
            content: vec![AnthropicTextBlock::text("calling tools")],
        }];
        AnthropicAdapter::push_request_message(
            &mut messages,
            AnthropicMessage {
                role: "user".to_owned(),
                content: vec![AnthropicTextBlock::tool_result("toolu_1", "first")],
            },
        );
        AnthropicAdapter::push_request_message(
            &mut messages,
            AnthropicMessage {
                role: "user".to_owned(),
                content: vec![AnthropicTextBlock::tool_result("toolu_2", "second")],
            },
        );

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].content.len(), 2);
        assert_eq!(
            messages[1].content[0].tool_use_id.as_deref(),
            Some("toolu_1")
        );
        assert_eq!(
            messages[1].content[1].tool_use_id.as_deref(),
            Some("toolu_2")
        );
    }
}
