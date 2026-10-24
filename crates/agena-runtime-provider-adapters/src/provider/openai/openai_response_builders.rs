use super::RESPONSES_ADAPTER_KIND;
use super::openai_response_types::apply_chat_prompt_cache_hints;
use super::openai_wire::openai_chat_tool_name;
use super::{
    AttachmentItem, AttachmentKind, AttachmentSource, BTreeMap, CHATGPT_CODEX_ORIGINATOR,
    CompletionRequest, CompletionToolCall, Deserialize, HashMap, ModelId, OpenAiFunctionCallItem,
    OpenAiFunctionCallOutputItem, OpenAiInputContent, OpenAiInputMessage, OpenAiListedModel,
    OpenAiOutputItem, OpenAiProfile, OpenAiResponsesBackend, OpenAiResponsesInputItem,
    OpenAiResponsesReasoningConfig, OpenAiResponsesTextConfig, OpenAiResponsesTextFormat,
    OpenAiResponsesToolPlan, OpenAiTransport, ProviderError, ProviderId, RequestHeaderContext,
    Role, chat_wire, clear_responses_prompt_cache_hints, responses_input_call_id,
    responses_model_tool_name, responses_output_call_id, session_text_lossy, utils,
    validate_responses_input, wire_message,
};
use agena_domain::Model;
use agena_provider::{CompletionInputRun, ProviderCompactionContext, ResponsesApiRequestMetadata};

impl OpenAiTransport {
    pub(super) fn is_vision_request(request: &CompletionRequest) -> bool {
        request.turns.iter().any(|message| {
            wire_message::project(message).iter().any(|part| {
                matches!(
                    part,
                    wire_message::WirePart::Attachment { item }
                        if item.kind == AttachmentKind::Image
                )
            })
        })
    }

    pub(super) fn initiator(request: &CompletionRequest) -> &'static str {
        match request.turns.last().map(|m| m.role) {
            Some(Role::User) => "user",
            _ => "agent",
        }
    }

    pub(super) fn chat_messages_for_request(
        &self,
        request: &CompletionRequest,
        assistant_reasoning_field: Option<&str>,
    ) -> Vec<chat_wire::ChatMessage> {
        let mut messages = chat_wire::request_to_chat_messages_with_assistant_reasoning_field(
            request,
            assistant_reasoning_field,
        );
        for message in &mut messages {
            if let Some(tool_calls) = message.tool_calls.as_mut() {
                for tool_call in tool_calls {
                    tool_call.function.name =
                        openai_chat_tool_name(tool_call.function.name.as_str());
                }
            }
        }
        if matches!(self.profile, OpenAiProfile::GithubCopilot) {
            apply_chat_prompt_cache_hints(messages.as_mut_slice());
        }
        messages
    }

    pub(super) fn chat_tools_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Option<Vec<chat_wire::ChatToolDefinition>> {
        (!request.tool_api_functions.is_empty()).then(|| {
            request
                .tool_api_functions
                .iter()
                .cloned()
                .map(|tool| chat_wire::ChatToolDefinition {
                    kind: "function".to_owned(),
                    function: chat_wire::ChatFunctionDefinition {
                        name: openai_chat_tool_name(tool.name.as_str()),
                        description: tool.description,
                        parameters: tool.input_schema,
                    },
                })
                .collect()
        })
    }

    /// Decide which `reasoning` items the follow-up request must replay, given
    /// what this model response just produced.
    ///
    /// Reasoning gateways (e.g. DeepSeek thinking mode) reject any tool-result
    /// follow-up whose preceding assistant function calls do not carry the
    /// matching reasoning item. A tool-calling turn may legitimately stream no
    /// new reasoning (an empty `fresh_items`), and if so the follow-up still
    /// needs the reasoning that was replayed into *this* request — the gateway
    /// requires reasoning before every assistant tool call, including turns
    /// where the model chose not to reason out loud. Carry the previously
    /// replayed items forward verbatim so persistence does not drop them
    /// between turns.
    pub(super) fn responses_reasoning_items_for_replay(
        request: &CompletionRequest,
        fresh_items: impl IntoIterator<Item = serde_json::Value>,
    ) -> Vec<serde_json::Value> {
        let fresh = fresh_items.into_iter().collect::<Vec<_>>();
        if !fresh.is_empty() {
            return fresh;
        }
        request
            .turns
            .iter()
            .rev()
            .find(|turn| matches!(turn.role, agena_domain::Role::Assistant))
            .map(|turn| {
                turn.provider_state
                    .openai_reasoning_items
                    .iter()
                    .filter(|item| {
                        item.get("type").and_then(serde_json::Value::as_str) == Some("reasoning")
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn responses_input_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Result<Vec<OpenAiResponsesInputItem>, ProviderError> {
        let mut input = Self::to_responses_input_with_system(request, false)?;
        clear_responses_prompt_cache_hints(input.as_mut_slice());
        Ok(input)
    }

    pub(super) fn responses_instructions(request: &CompletionRequest) -> Option<String> {
        request
            .system
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    pub(super) fn responses_parallel_tool_calls(request: &CompletionRequest) -> bool {
        request.model_supports_parallel_tool_calls.unwrap_or(false)
    }

    pub(super) fn responses_request_max_output_tokens(
        &self,
        request: &CompletionRequest,
    ) -> Option<u32> {
        if matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex) {
            None
        } else {
            request.max_output_tokens
        }
    }

    pub(super) fn responses_service_tier(request: &CompletionRequest) -> Option<String> {
        request
            .request_override
            .body_patch
            .get("service_tier")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    pub(super) fn responses_client_metadata(
        context: RequestHeaderContext<'_>,
    ) -> Option<HashMap<String, String>> {
        context
            .responses_api_metadata
            .map(ResponsesApiRequestMetadata::client_metadata)
            .or_else(|| {
                let mut metadata = HashMap::new();
                if let Some(window_id) = context.window_id_header() {
                    metadata.insert("x-codex-window-id".to_owned(), window_id);
                }
                (!metadata.is_empty()).then_some(metadata)
            })
    }

    pub(super) fn responses_include(
        mut include: Vec<String>,
        reasoning: Option<&OpenAiResponsesReasoningConfig>,
        stateless_encrypted_reasoning: bool,
    ) -> Option<Vec<String>> {
        if (reasoning.is_some() || stateless_encrypted_reasoning)
            && !include
                .iter()
                .any(|value| value == "reasoning.encrypted_content")
        {
            include.push("reasoning.encrypted_content".to_owned());
        }
        (!include.is_empty()).then_some(include)
    }

    pub(super) fn responses_reasoning_config(
        request: &CompletionRequest,
        model: &str,
    ) -> Option<OpenAiResponsesReasoningConfig> {
        let effort = chat_wire::reasoning_effort(request.thinking.as_ref(), model);
        let summary = match request.thinking.as_ref() {
            Some(agena_domain::ThinkingRequest::Adaptive { display, .. }) => match display {
                Some(agena_domain::ThinkingDisplay::Omitted) => None,
                _ => Some("auto".to_owned()),
            },
            Some(agena_domain::ThinkingRequest::Budget { .. })
            | Some(agena_domain::ThinkingRequest::Effort { .. }) => Some("auto".to_owned()),
            None | Some(agena_domain::ThinkingRequest::Disabled) => None,
        };
        (effort.is_some() || summary.is_some())
            .then_some(OpenAiResponsesReasoningConfig { effort, summary })
    }

    pub(super) fn responses_text_config(
        request: &CompletionRequest,
    ) -> Option<OpenAiResponsesTextConfig> {
        let verbosity = request.verbosity.clone();
        let format =
            OpenAiResponsesTextFormat::from_response_format(request.response_format.as_ref());
        (verbosity.is_some() || format.is_some())
            .then_some(OpenAiResponsesTextConfig { verbosity, format })
    }

    pub(super) fn responses_tool_plan_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Result<OpenAiResponsesToolPlan, ProviderError> {
        self.responses_tool_plan(request)
    }

    pub(super) fn to_responses_input_with_system(
        request: &CompletionRequest,
        include_system: bool,
    ) -> Result<Vec<OpenAiResponsesInputItem>, ProviderError> {
        let mut input = Vec::new();

        // Validate the full window together. A newly completed tool result can
        // reference a call retained verbatim in the native compacted prefix.
        if let Some(ProviderCompactionContext::OpenAiResponses { items }) =
            request.provider_compaction.as_ref()
        {
            input.extend(items.iter().cloned().map(OpenAiResponsesInputItem::Raw));
        }

        if include_system
            && let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty())
        {
            Self::push_responses_text_message(&mut input, "system", system.clone());
        }

        for run in &request.turns {
            Self::append_responses_items_for_message(&mut input, run);
        }

        validate_responses_input(input.as_slice())?;
        Ok(input)
    }

    pub(super) fn attachment_upload_name(item: &AttachmentItem) -> String {
        wire_message::filename(item)
            .map(str::to_owned)
            .unwrap_or_else(|| item.summary_label())
    }

    pub(super) fn responses_file_content(item: &AttachmentItem) -> Option<OpenAiInputContent> {
        let filename = Some(Self::attachment_upload_name(item));
        match &item.source {
            AttachmentSource::Base64 { .. }
            | AttachmentSource::ProviderData { .. }
            | AttachmentSource::DataUrl { .. } => {
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
            AttachmentSource::LocalPath { .. } | AttachmentSource::Unavailable { .. } => None,
        }
    }

    pub(super) fn responses_content_from_attachment(item: &AttachmentItem) -> OpenAiInputContent {
        match item.kind {
            AttachmentKind::Image => wire_message::media_url(item)
                .map(|image_url| OpenAiInputContent::Image { image_url })
                .unwrap_or_else(|| OpenAiInputContent::InputText {
                    text: wire_message::hint_text(item),
                }),
            AttachmentKind::Audio | AttachmentKind::Video => OpenAiInputContent::InputText {
                text: format!(
                    "Unsupported native {} input for Responses: {}",
                    item.kind,
                    item.summary_label()
                ),
            },
            AttachmentKind::Pdf | AttachmentKind::File => Self::responses_file_content(item)
                .unwrap_or_else(|| OpenAiInputContent::InputText {
                    text: wire_message::hint_text(item),
                }),
        }
    }

    pub(super) fn push_responses_text_message(
        input: &mut Vec<OpenAiResponsesInputItem>,
        role: &str,
        text: String,
    ) {
        if text.trim().is_empty() {
            return;
        }

        input.push(OpenAiResponsesInputItem::Message(OpenAiInputMessage {
            role: role.to_owned(),
            content: vec![OpenAiInputContent::text_for_role(role, text)],
            copilot_cache_control: None,
        }));
    }

    pub(super) fn flush_assistant_responses_text(
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

    pub(super) fn push_responses_message_from_parts(
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
            copilot_cache_control: None,
        }));
    }

    /// OpenAI-style reasoning items carry the chain-of-thought as opaque
    /// `encrypted_content`. Some Responses-compatible gateways validate that
    /// the plaintext `content` array on such items is empty
    /// (`array_above_max_length` on `input[N].content`), so strip `content`
    /// before replay whenever the item already carries encrypted content.
    /// Chat-style `reasoning_content` items (content only, no
    /// `encrypted_content`) are replayed unchanged so models like deepseek
    /// still receive their prior reasoning.
    pub(super) fn sanitize_responses_reasoning_item(
        mut item: serde_json::Value,
    ) -> serde_json::Value {
        if item
            .get("encrypted_content")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|content| !content.trim().is_empty())
            && let Some(object) = item.as_object_mut()
        {
            object.remove("content");
        }
        item
    }

    /// Drop the plaintext `content` array from every reasoning input item.
    /// Last-resort recovery for gateways that reject reasoning content replay
    /// even for chat-style items; the opaque state (when present) and summary
    /// are preserved, and non-reasoning items are untouched.
    pub(super) fn strip_responses_reasoning_content(input: &mut [OpenAiResponsesInputItem]) {
        for item in input.iter_mut() {
            if let OpenAiResponsesInputItem::Reasoning(value) = item
                && let Some(object) = value.as_object_mut()
            {
                object.remove("content");
            }
        }
    }

    /// True when the provider rejected the request because a reasoning input
    /// item carried a non-empty plaintext `content` array
    /// (`array_above_max_length` on `input[N].content`). Used to decide the
    /// one-shot reasoning-content retry in `complete_stream`.
    pub(super) fn is_reasoning_content_array_error(error: &ProviderError) -> bool {
        let message = match error {
            ProviderError::HttpStatus { body, .. } => body,
            ProviderError::ProviderClassified { message, .. } => message,
            _ => return false,
        };
        message.contains("array_above_max_length")
            && message.contains("input[")
            && message.contains(".content")
    }

    pub(super) fn append_responses_items_for_message(
        input: &mut Vec<OpenAiResponsesInputItem>,
        run: &CompletionInputRun,
    ) {
        let projected_parts = wire_message::project(run);
        match run.role {
            Role::System => Self::push_responses_text_message(
                input,
                "system",
                session_text_lossy(run, projected_parts.as_slice()),
            ),
            Role::User => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "user", run.as_text_lossy());
                } else {
                    Self::push_responses_message_from_parts(
                        input,
                        "user",
                        projected_parts.as_slice(),
                    );
                }
            }
            Role::Assistant => {
                {
                    // Models that declare `assistant_reasoning_field =
                    // "reasoning_content"` must replay their prior reasoning to
                    // the API. Their reasoning items carry plain-text `content`
                    // (chat-style) instead of OpenAI's `encrypted_content`, so
                    // replay either carrier for those models. Other models keep
                    // the encrypted-content-only replay.
                    let replay_content_reasoning = run.provider_state.assistant_reasoning_field
                        == Some(agena_domain::AssistantReasoningField::ReasoningContent);
                    input.extend(
                        run.provider_state
                            .openai_reasoning_items
                            .iter()
                            .filter(|item| {
                                item.get("type").and_then(serde_json::Value::as_str)
                                    == Some("reasoning")
                                    && if replay_content_reasoning {
                                        item.get("encrypted_content")
                                            .and_then(serde_json::Value::as_str)
                                            .is_some_and(|content| !content.trim().is_empty())
                                            || item
                                                .get("content")
                                                .and_then(serde_json::Value::as_array)
                                                .is_some_and(|content| !content.is_empty())
                                    } else {
                                        item.get("encrypted_content")
                                            .and_then(serde_json::Value::as_str)
                                            .is_some_and(|content| !content.trim().is_empty())
                                    }
                            })
                            .cloned()
                            .map(Self::sanitize_responses_reasoning_item)
                            .map(OpenAiResponsesInputItem::Reasoning),
                    );
                }
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "assistant", run.as_text_lossy());
                } else {
                    let mut text_chunks = Vec::new();
                    let mut calls: Vec<OpenAiResponsesInputItem> = Vec::new();
                    let mut outputs: Vec<OpenAiResponsesInputItem> = Vec::new();
                    let mut pending_output: Option<(String, String, Vec<wire_message::WirePart>)> =
                        None;
                    for part in projected_parts {
                        match part {
                            wire_message::WirePart::Text { text } => {
                                if let Some(output) = pending_output.take() {
                                    Self::flush_responses_function_output(
                                        &mut outputs,
                                        &mut Some(output),
                                    );
                                }
                                text_chunks.push(text);
                            }
                            wire_message::WirePart::Reasoning { .. } => {
                                // Reasoning is replayed through the dedicated
                                // `Reasoning` input item (see the openai_reasoning_items
                                // replay above), never as visible output text.
                            }
                            // A system notice (background-operation notification)
                            // is a genuine mid-conversation `system` message —
                            // never the assistant's own reply text. Flush the
                            // accumulated assistant turn (text, calls, outputs)
                            // first so the notice lands after it in order, then
                            // open a fresh assistant grouping.
                            wire_message::WirePart::SystemMessage { text } => {
                                if let Some(output) = pending_output.take() {
                                    Self::flush_responses_function_output(
                                        &mut outputs,
                                        &mut Some(output),
                                    );
                                }
                                Self::flush_assistant_responses_text(input, &mut text_chunks);
                                input.append(&mut calls);
                                input.append(&mut outputs);
                                Self::push_responses_text_message(input, "system", text);
                            }
                            wire_message::WirePart::Attachment { item } => {
                                if let Some((_, _, extra_parts)) = pending_output.as_mut() {
                                    extra_parts.push(wire_message::WirePart::Attachment { item });
                                } else {
                                    text_chunks.push(wire_message::hint_text(&item));
                                }
                            }
                            wire_message::WirePart::ToolCall {
                                id,
                                function,
                                arguments_json,
                            } => {
                                if let Some(output) = pending_output.take() {
                                    Self::flush_responses_function_output(
                                        &mut outputs,
                                        &mut Some(output),
                                    );
                                }
                                if let Some(call_id) = responses_input_call_id(id.as_str()) {
                                    calls.push(OpenAiResponsesInputItem::FunctionCall(
                                        OpenAiFunctionCallItem {
                                            kind: "function_call",
                                            call_id,
                                            namespace: None,
                                            name: function.function_name().to_owned(),
                                            arguments: arguments_json,
                                            copilot_cache_control: None,
                                        },
                                    ));
                                }
                            }
                            wire_message::WirePart::ToolResult {
                                tool_call_id,
                                output_json,
                                attachments,
                                ..
                            } => {
                                if let Some(output) = pending_output.take() {
                                    Self::flush_responses_function_output(
                                        &mut outputs,
                                        &mut Some(output),
                                    );
                                }
                                if let Some(call_id) =
                                    responses_input_call_id(tool_call_id.as_ref())
                                {
                                    pending_output = Some((
                                        call_id,
                                        output_json,
                                        attachments
                                            .into_iter()
                                            .map(|item| wire_message::WirePart::Attachment { item })
                                            .collect(),
                                    ));
                                }
                            }
                        }
                    }
                    if let Some(output) = pending_output.take() {
                        Self::flush_responses_function_output(&mut outputs, &mut Some(output));
                    }
                    // OpenAI-compatible gateways that translate the Responses
                    // input into Chat Completions messages before reaching a
                    // reasoning model (for example the Console Go / codex route)
                    // turn every `function_call` into its own assistant message
                    // that must carry the preceding reasoning item. Emit all
                    // function calls before their outputs — Responses correlates
                    // them by `call_id`, so the reordering is semantically
                    // identical — so a parallel tool turn replays as one
                    // assistant message with the reasoning intact instead of an
                    // interleaved call/output stream that drops the reasoning
                    // on the second call.
                    Self::flush_assistant_responses_text(input, &mut text_chunks);
                    input.extend(calls);
                    input.extend(outputs);
                }
            }
            Role::Tool => {
                for part in projected_parts {
                    if let wire_message::WirePart::ToolResult {
                        tool_call_id,
                        output_json,
                        attachments,
                        ..
                    } = part
                        && let Some(call_id) = responses_input_call_id(tool_call_id.as_ref())
                    {
                        input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                            OpenAiFunctionCallOutputItem {
                                kind: "function_call_output",
                                call_id,
                                output: Self::multimodal_function_output_value(
                                    output_json.as_str(),
                                    &attachments
                                        .into_iter()
                                        .map(|item| wire_message::WirePart::Attachment { item })
                                        .collect::<Vec<_>>(),
                                ),
                                copilot_cache_control: None,
                            },
                        ));
                    }
                }
            }
        }
    }

    pub(super) fn responses_input_contents_from_parts(
        parts: &[wire_message::WirePart],
    ) -> Vec<OpenAiInputContent> {
        parts
            .iter()
            .map(|part| match part {
                wire_message::WirePart::Text { text } => {
                    OpenAiInputContent::InputText { text: text.clone() }
                }
                wire_message::WirePart::Reasoning { text } => {
                    OpenAiInputContent::InputText { text: text.clone() }
                }
                wire_message::WirePart::SystemMessage { text } => {
                    OpenAiInputContent::InputText { text: text.clone() }
                }
                wire_message::WirePart::Attachment { item } => {
                    Self::responses_content_from_attachment(item)
                }
                wire_message::WirePart::ToolCall { function, .. } => {
                    OpenAiInputContent::InputText {
                        text: format!("[tool_call:{}]", function.function_name()),
                    }
                }
                wire_message::WirePart::ToolResult { tool_call_id, .. } => {
                    OpenAiInputContent::InputText {
                        text: format!("[tool_result:{tool_call_id}]"),
                    }
                }
            })
            .collect()
    }

    pub(super) fn multimodal_function_output_value(
        output_json: &str,
        extra_parts: &[wire_message::WirePart],
    ) -> serde_json::Value {
        if extra_parts.is_empty() {
            return serde_json::Value::String(output_json.to_owned());
        }

        let mut content = Vec::new();
        if !output_json.trim().is_empty() {
            content.push(OpenAiInputContent::InputText {
                text: output_json.to_owned(),
            });
        }
        content.extend(Self::responses_input_contents_from_parts(extra_parts));
        serde_json::to_value(content).expect("openai function_call_output content should serialize")
    }

    pub(super) fn flush_responses_function_output(
        input: &mut Vec<OpenAiResponsesInputItem>,
        pending_output: &mut Option<(String, String, Vec<wire_message::WirePart>)>,
    ) {
        let Some((call_id, output_json, extra_parts)) = pending_output.take() else {
            return;
        };
        input.push(OpenAiResponsesInputItem::FunctionCallOutput(
            OpenAiFunctionCallOutputItem {
                kind: "function_call_output",
                call_id,
                output: Self::multimodal_function_output_value(
                    output_json.as_str(),
                    extra_parts.as_slice(),
                ),
                copilot_cache_control: None,
            },
        ));
    }

    pub(super) fn parse_responses_tool_calls(
        items: Option<&Vec<OpenAiOutputItem>>,
    ) -> Result<Vec<CompletionToolCall>, ProviderError> {
        items
            .into_iter()
            .flatten()
            .filter(|item| item.kind.as_deref() == Some("function_call"))
            .map(|item| {
                let id = responses_output_call_id(item.call_id.as_deref()).ok_or_else(|| {
                    ProviderError::Provider(
                        "openai responses payload returned function_call without call_id"
                            .to_owned(),
                    )
                })?;

                let name = utils::optional_non_empty(item.name.clone()).ok_or_else(|| {
                    ProviderError::Provider(
                        "openai responses payload returned function_call without name".to_owned(),
                    )
                })?;
                let name = responses_model_tool_name(item.namespace.as_deref(), name.as_str());

                Ok(CompletionToolCall::Function {
                    id,
                    name,
                    arguments_json: item.arguments.clone().unwrap_or_default(),
                })
            })
            .collect()
    }

    pub(super) async fn send_json<R>(
        &self,
        operation: &str,
        endpoint: String,
        body: Option<&utils::PreparedJsonBody>,
        context: RequestHeaderContext<'_>,
    ) -> Result<R, ProviderError>
    where
        R: for<'de> Deserialize<'de> + Send + 'static,
    {
        let response =
            utils::send_with_credential_refresh(self.id.as_str(), &self.api_key, |api_key| {
                let mut headers = self.auth_headers(context, api_key);
                headers.insert(
                    reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                    "application/json".to_owned(),
                );
                utils::adapter_log_http_request_prepared(
                    self.id.as_str(),
                    RESPONSES_ADAPTER_KIND,
                    operation,
                    "POST",
                    endpoint.as_str(),
                    headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                    body,
                );
                let mut request = utils::apply_resolved_request_headers(
                    self.client.post(endpoint.clone()),
                    &headers,
                );

                if let Some(body) = body {
                    request = request.body(body.bytes());
                }

                request
            })
            .await?;
        utils::parse_json_response_logged(
            self.id.as_str(),
            RESPONSES_ADAPTER_KIND,
            operation,
            response,
        )
        .await
    }

    pub(super) fn resolved_headers(
        &self,
        context: RequestHeaderContext<'_>,
    ) -> BTreeMap<String, String> {
        let mut headers = self.extra_headers.clone();
        utils::ensure_header_case_insensitive(&mut headers, "originator", || {
            CHATGPT_CODEX_ORIGINATOR.to_owned()
        });
        utils::ensure_header_case_insensitive(
            &mut headers,
            reqwest::header::USER_AGENT.as_str(),
            || self.client_identity.codex_user_agent(),
        );

        if self.supports_codex_compat_headers()
            && let Some(metadata) = context.responses_api_metadata
        {
            for (key, value) in metadata.session_headers() {
                utils::insert_header_case_insensitive(&mut headers, key, value);
            }
        }

        if matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex) {
            if let Some(account_id) = self.chatgpt_account_id() {
                utils::insert_header_case_insensitive(
                    &mut headers,
                    "ChatGPT-Account-ID",
                    account_id,
                );
            }
            if self.chatgpt_account_is_fedramp() {
                utils::insert_header_case_insensitive(&mut headers, "X-OpenAI-Fedramp", "true");
            }
        }

        if self.supports_codex_compat_headers() {
            if let Some(metadata) = context.responses_api_metadata {
                for (key, value) in metadata.compatibility_headers() {
                    utils::insert_header_case_insensitive(&mut headers, key, value);
                }
            } else if let Some(window_id) = context.window_id_header() {
                utils::insert_header_case_insensitive(&mut headers, "x-codex-window-id", window_id);
            }
        }

        if matches!(self.profile, OpenAiProfile::GithubCopilot) {
            utils::ensure_header_case_insensitive(
                &mut headers,
                reqwest::header::USER_AGENT.as_str(),
                || self.client_identity.codex_user_agent(),
            );
            utils::ensure_header_case_insensitive(&mut headers, "Openai-Intent", || {
                "conversation-edits".to_owned()
            });
            utils::insert_header_case_insensitive(
                &mut headers,
                "x-initiator",
                context.initiator_header(),
            );
            if context.vision_request {
                utils::insert_header_case_insensitive(
                    &mut headers,
                    "Copilot-Vision-Request",
                    "true",
                );
            }
        }

        if let Some(session_affinity) = context.session_affinity_header() {
            let header = if self.is_xai_endpoint() {
                // xAI documents this header for sticky Chat Completions
                // routing; `prompt_cache_key` belongs to its Responses API.
                "x-grok-conv-id"
            } else {
                "x-session-affinity"
            };
            utils::insert_header_case_insensitive(&mut headers, header, session_affinity);
        }

        if let Some(request_headers) = context.request_headers {
            headers = utils::merged_request_headers(&headers, request_headers);
        }

        utils::configured_request_headers(&headers)
    }

    pub(super) fn auth_headers(
        &self,
        context: RequestHeaderContext<'_>,
        api_key: &str,
    ) -> BTreeMap<String, String> {
        let mut headers = self.resolved_headers(context);
        headers.insert(
            self.auth_header.clone(),
            utils::auth_header_value(self.auth_scheme.as_deref(), api_key),
        );
        headers
    }

    pub(super) fn provider_model_from_listed_model(
        &self,
        model: OpenAiListedModel,
    ) -> Option<Model> {
        match model {
            OpenAiListedModel::Compatible(model) => {
                if self.profile == OpenAiProfile::GithubCopilot
                    && (!model.copilot.visible() || model.copilot.uses_messages_endpoint())
                {
                    return None;
                }

                let metadata = model.metadata();
                let model_id = ModelId::new(model.id);
                let mut capabilities = self.runtime_model_capabilities(&model_id);
                if self.profile == OpenAiProfile::GithubCopilot {
                    capabilities = model
                        .copilot
                        .capabilities()
                        .merged_with_fallbacks_from(&capabilities);
                }

                let display_name = model
                    .display_name
                    .or(model.name)
                    .and_then(|value| utils::normalize_optional_text(Some(value)));
                Some(Model {
                    provider_id: ProviderId::new(self.id.as_str()),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name,
                    native_compaction: true,
                    capabilities,
                    metadata,
                    thinking_modes: Vec::new(),
                    speed_modes: BTreeMap::new(),
                })
            }
            OpenAiListedModel::Recommended(model) => {
                let metadata = model.metadata();
                let display_name = model
                    .display_name
                    .or(model.name)
                    .and_then(|value| utils::normalize_optional_text(Some(value)));
                let model_id = utils::normalize_optional_text(Some(model.id))?;
                let model_id = ModelId::new(model_id);
                let capabilities = self.runtime_model_capabilities(&model_id);
                Some(Model {
                    provider_id: ProviderId::new(self.id.as_str()),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name,
                    native_compaction: true,
                    capabilities,
                    metadata,
                    thinking_modes: Vec::new(),
                    speed_modes: BTreeMap::new(),
                })
            }
            OpenAiListedModel::Codex(model) => {
                let thinking_modes = model.thinking_modes();
                let metadata = model.metadata();
                let capabilities = model.capabilities();
                let display_name =
                    utils::normalize_optional_text(model.display_name.or(model.name));
                let slug = utils::normalize_optional_text(Some(model.slug))?;
                let model_id = ModelId::new(slug);
                let capabilities = capabilities
                    .merged_with_fallbacks_from(&self.runtime_model_capabilities(&model_id));
                Some(Model {
                    provider_id: ProviderId::new(self.id.as_str()),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name,
                    native_compaction: true,
                    capabilities,
                    metadata,
                    thinking_modes,
                    speed_modes: BTreeMap::new(),
                })
            }
        }
    }
}
