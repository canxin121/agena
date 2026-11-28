use std::fmt::Write as _;
use std::time::Duration;

use agena_domain::mcp_app_tools::McpAppViewScope;
use agena_domain::mcp_sampling::{
    MAX_INPUT_BYTES, MAX_OUTPUT_TOKENS, McpSamplingBlock, McpSamplingIncludeContext,
    McpSamplingReply, McpSamplingReplyBlock, McpSamplingRequest, McpSamplingTool,
    McpSamplingToolChoice, McpSamplingToolResultBlock,
};
use agena_domain::{Role, ToolApiFunction, UserInputOption, UserInputQuestion};
use agena_provider::{
    CompletionFinishReason, CompletionInputAttachment, CompletionInputAttachmentKind,
    CompletionInputAttachmentSource, CompletionInputPart, CompletionInputRun,
    CompletionInputToolResultStatus, ModelToolFunction, ToolApiDefinition,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};

use super::{AppError, CancellationToken, SessionManager, mcp_apps::app_view_error, session_id};
use crate::part::AskUserToolInput;

const TIMEOUT: Duration = Duration::from_secs(30);
const ALLOW: &str = "Allow";
const MAX_INCLUDED_CONTEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy)]
enum SamplingOwner<'a> {
    Tool { session: i64, call: i64 },
    View(&'a McpAppViewScope),
}

impl SessionManager {
    pub async fn resolve_mcp_sampling(
        &self,
        session: i64,
        call: i64,
        server: &str,
        request: McpSamplingRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<McpSamplingReply>, AppError> {
        self.run_mcp_sampling(
            SamplingOwner::Tool { session, call },
            server,
            "Return model output to the MCP server",
            request,
            cancellation,
        )
        .await
    }

    pub async fn resolve_mcp_app_sampling(
        &self,
        scope: &McpAppViewScope,
        source: &str,
        release_prompt: &'static str,
        request: McpSamplingRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<McpSamplingReply>, AppError> {
        self.run_mcp_sampling(
            SamplingOwner::View(scope),
            source,
            release_prompt,
            request,
            cancellation,
        )
        .await
    }

    async fn build_sampling_included_context(
        &self,
        session: agena_storage::kernel::SessionId,
        server: &str,
        scope: McpSamplingIncludeContext,
    ) -> Result<Option<String>, AppError> {
        if scope == McpSamplingIncludeContext::None {
            return Ok(None);
        }
        let page = self
            .kernel()
            .reader
            .history(session, None, 24)
            .await
            .map_err(super::kernel_error)?;
        let mut lines = Vec::new();
        let mut total = 0usize;
        for entry in page.entries.iter().rev() {
            let line = match &entry.payload {
                agena_storage::kernel::EntryPayload::User { document, .. } => {
                    if scope != McpSamplingIncludeContext::AllServers {
                        continue;
                    }
                    let text = document.text();
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    format!("User: {trimmed}")
                }
                agena_storage::kernel::EntryPayload::Tool { tool_id } => {
                    let Ok(tool) = self.kernel().reader.tool(session, *tool_id).await else {
                        continue;
                    };
                    let tool_name = &tool.call.name;
                    let input_str = tool.call.input.to_string();
                    if scope == McpSamplingIncludeContext::ThisServer {
                        let matches_server = tool_name.contains(server)
                            || tool
                                .call
                                .input
                                .get("server")
                                .and_then(Value::as_str)
                                .is_some_and(|s| s == server)
                            || tool
                                .call
                                .input
                                .get("input")
                                .and_then(|inner| inner.get("server"))
                                .and_then(Value::as_str)
                                .is_some_and(|s| s == server);
                        if !matches_server {
                            continue;
                        }
                    }
                    let preview = tool
                        .result
                        .as_ref()
                        .and_then(|res| res.get("summary").and_then(Value::as_str))
                        .unwrap_or("(no summary)");
                    format!(
                        "Tool {tool_name} ({}): {preview}",
                        truncate_str(&input_str, 512)
                    )
                }
                _ => continue,
            };
            let bounded = truncate_str(&line, 2048);
            if total.saturating_add(bounded.len() + 1) > MAX_INCLUDED_CONTEXT_BYTES {
                break;
            }
            total = total.saturating_add(bounded.len() + 1);
            lines.push(bounded.to_owned());
        }
        let header = match scope {
            McpSamplingIncludeContext::ThisServer => {
                format!("Included MCP context (thisServer: {server})")
            }
            McpSamplingIncludeContext::AllServers => {
                "Included MCP context (allServers)".to_owned()
            }
            McpSamplingIncludeContext::None => return Ok(None),
        };
        if lines.is_empty() {
            return Ok(Some(format!(
                "{header}\nNo prior matching session entries."
            )));
        }
        Ok(Some(format!("{header}\n{}", lines.join("\n"))))
    }

    async fn run_mcp_sampling(
        &self,
        owner: SamplingOwner<'_>,
        source: &str,
        release_prompt: &str,
        request: McpSamplingRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<McpSamplingReply>, AppError> {
        let (session, state, owner_cancel) = match owner {
            SamplingOwner::Tool { session, call } => {
                let (_, control) = self.live_tool(session, call)?;
                (control.session, control.state.clone(), control.cancel.clone())
            }
            SamplingOwner::View(scope) => {
                let cancel = self
                    .mcp_app_views
                    .view_cancellation(scope)
                    .map_err(app_view_error)?;
                let session = session_id(scope.binding.session_id)?;
                (session, self.state_for_session(session).await?, cancel)
            }
        };
        if cancellation.is_cancelled() || owner_cancel.is_cancelled() {
            return Ok(None);
        }
        let context = self.execution_context_with_state(session, &state).await?;
        let selection = if state.config.auxiliary_selection.is_empty() {
            &context.selection
        } else {
            &state.config.auxiliary_selection
        };
        let mut options = self.options_for_selection_with_state(selection, &state)?;
        self.apply_model_modes_with_state(&mut options, &state)?;
        let metadata = state
            .providers
            .model_metadata(&options.model)
            .unwrap_or_default();
        let max_tokens = crate::session_output_token_budget(
            metadata.limits.context_window_tokens,
            metadata.limits.max_output_tokens,
            Some(request.max_tokens.clamp(1, MAX_OUTPUT_TOKENS)),
        );
        options.max_output_tokens = Some(max_tokens);
        options.temperature = request.temperature;
        crate::prompt_budget::align_output_token_overrides(
            &mut options.request_override,
            max_tokens,
        );

        let included_context = self
            .build_sampling_included_context(session, source, request.include_context)
            .await?;
        let tools_enabled = !request.tools.is_empty()
            && request.tool_choice != Some(McpSamplingToolChoice::None);

        let mut preview = format!(
            "Model {}\nOutput limit {max_tokens} tokens\n",
            options.model
        );
        if let Some(system) = &request.system_prompt {
            let _ = write!(preview, "\nSystem prompt\n{system}\n");
        }
        if let Some(ctx) = &included_context {
            let _ = write!(preview, "\n{ctx}\n");
        }
        if tools_enabled {
            let mode = match request.tool_choice {
                Some(McpSamplingToolChoice::Required) => "required",
                Some(McpSamplingToolChoice::None) => "none",
                Some(McpSamplingToolChoice::Auto) | None => "auto",
            };
            let names = request
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(preview, "\nTools ({mode}): {names}\n");
        }
        for message in &request.messages {
            let role = match message.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                _ => {
                    return Err(AppError::Config(
                        "invalid role in MCP sampling request".into(),
                    ));
                }
            };
            let _ = write!(preview, "\n{role}\n");
            for block in &message.content {
                match block {
                    McpSamplingBlock::Text(text) => {
                        let _ = writeln!(preview, "{text}");
                    }
                    McpSamplingBlock::Image { mime_type, data } => {
                        let bytes = STANDARD.decode(data).map(|b| b.len()).unwrap_or(0);
                        let _ = writeln!(preview, "[Image: {mime_type}, {bytes} bytes]");
                    }
                    McpSamplingBlock::Audio { mime_type, data } => {
                        let bytes = STANDARD.decode(data).map(|b| b.len()).unwrap_or(0);
                        let _ = writeln!(preview, "[Audio: {mime_type}, {bytes} bytes]");
                    }
                    McpSamplingBlock::ToolUse { id, name, input } => {
                        let _ = writeln!(
                            preview,
                            "[ToolUse {name} ({id}): {}]",
                            Value::Object(input.clone())
                        );
                    }
                    McpSamplingBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        let summary = content
                            .iter()
                            .map(|item| match item {
                                McpSamplingToolResultBlock::Text(text) => text.clone(),
                                McpSamplingToolResultBlock::Image { mime_type, data } => {
                                    let bytes =
                                        STANDARD.decode(data).map(|b| b.len()).unwrap_or(0);
                                    format!("[Image: {mime_type}, {bytes} bytes]")
                                }
                                McpSamplingToolResultBlock::Audio { mime_type, data } => {
                                    let bytes =
                                        STANDARD.decode(data).map(|b| b.len()).unwrap_or(0);
                                    format!("[Audio: {mime_type}, {bytes} bytes]")
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let _ = writeln!(
                            preview,
                            "[ToolResult {tool_use_id}{}: {summary}]",
                            if *is_error { " (error)" } else { "" }
                        );
                    }
                }
            }
        }
        if preview.len() > MAX_INPUT_BYTES + MAX_INCLUDED_CONTEXT_BYTES + 4096 {
            return Err(AppError::Config(
                "MCP sampling input exceeds its preview budget".into(),
            ));
        }
        let approve = async |action: &str, preview: String| {
            let dialog = sampling_dialog(source, action, preview);
            match owner {
                SamplingOwner::Tool { session, call } => {
                    self.request_cancellable_host_user_input(session, call, dialog, &cancellation)
                        .await
                }
                SamplingOwner::View(scope) => {
                    self.request_mcp_app_input(scope, dialog, &cancellation)
                        .await
                }
            }
        };
        let allowed = approve("Run a model request", preview).await?;
        if allowed.cancelled || allowed.timed_out || allowed.reply != ALLOW {
            return Ok(None);
        }

        let route = state.providers.media_route_binding(&options.model).ok();
        let tool_registry = SamplingToolRegistry::new(&request.tools);
        let make_attachment = |kind: CompletionInputAttachmentKind, mime: String, data: String| {
            let source = match route.as_ref() {
                Some(route) => CompletionInputAttachmentSource::ProviderData {
                    route: route.clone(),
                    data,
                },
                None => CompletionInputAttachmentSource::Base64 { data },
            };
            CompletionInputAttachment {
                kind,
                mime,
                source,
                filename: None,
                title: None,
                size_bytes: None,
                sha256: None,
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            }
        };

        let mut turns = Vec::with_capacity(request.messages.len());
        let push_part = |turns: &mut Vec<CompletionInputRun>,
                         role: Role,
                         part: CompletionInputPart| {
            if let Some(last) = turns.last_mut()
                && last.role == role
            {
                last.parts.push(part);
            } else {
                turns.push(CompletionInputRun {
                    role,
                    parts: vec![part],
                    provider_state: Default::default(),
                });
            }
        };
        for message in request.messages {
            for block in message.content {
                match block {
                    McpSamplingBlock::Text(text) => {
                        push_part(&mut turns, message.role, CompletionInputPart::Text { text });
                    }
                    McpSamplingBlock::Image { mime_type, data } => {
                        push_part(
                            &mut turns,
                            message.role,
                            CompletionInputPart::Attachment {
                                attachment: make_attachment(
                                    CompletionInputAttachmentKind::Image,
                                    mime_type,
                                    data,
                                ),
                            },
                        );
                    }
                    McpSamplingBlock::Audio { mime_type, data } => {
                        push_part(
                            &mut turns,
                            message.role,
                            CompletionInputPart::Attachment {
                                attachment: make_attachment(
                                    CompletionInputAttachmentKind::Audio,
                                    mime_type,
                                    data,
                                ),
                            },
                        );
                    }
                    McpSamplingBlock::ToolUse { id, name, input } => {
                        let wire_name = tool_registry.wire_name_for(&name);
                        push_part(
                            &mut turns,
                            Role::Assistant,
                            CompletionInputPart::ToolCall {
                                id,
                                function: ModelToolFunction::Gateway(ToolApiFunction::Call),
                                arguments_json: json!({ "tool": wire_name, "input": input })
                                    .to_string(),
                            },
                        );
                    }
                    McpSamplingBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        let mut texts = Vec::new();
                        let mut attachments = Vec::new();
                        for item in content {
                            match item {
                                McpSamplingToolResultBlock::Text(text) => texts.push(text),
                                McpSamplingToolResultBlock::Image { mime_type, data } => {
                                    attachments.push(make_attachment(
                                        CompletionInputAttachmentKind::Image,
                                        mime_type,
                                        data,
                                    ));
                                }
                                McpSamplingToolResultBlock::Audio { mime_type, data } => {
                                    attachments.push(make_attachment(
                                        CompletionInputAttachmentKind::Audio,
                                        mime_type,
                                        data,
                                    ));
                                }
                            }
                        }
                        push_part(
                            &mut turns,
                            Role::Tool,
                            CompletionInputPart::ToolResult {
                                tool_call_id: tool_use_id,
                                function: ModelToolFunction::Gateway(ToolApiFunction::Call),
                                arguments_json: "{}".into(),
                                status: if is_error {
                                    CompletionInputToolResultStatus::Failed
                                } else {
                                    CompletionInputToolResultStatus::Completed
                                },
                                output_json: json!({
                                    "phase": if is_error { "failed" } else { "succeeded" },
                                    "result": { "model": texts.join("\n") },
                                    "sources": []
                                })
                                .to_string(),
                                attachments,
                            },
                        );
                    }
                }
            }
        }

        let mut system_sections = Vec::new();
        if let Some(system) = request.system_prompt
            && !system.is_empty()
        {
            system_sections.push(system);
        }
        if let Some(ctx) = included_context {
            system_sections.push(ctx);
        }
        if tools_enabled {
            system_sections.push(sampling_tools_prompt(
                &request.tools,
                &tool_registry,
                request.tool_choice,
            ));
        }
        let effective_system = (!system_sections.is_empty()).then(|| system_sections.join("\n\n"));

        let tool_defs = if tools_enabled {
            vec![sampling_gateway_tool_definition(&tool_registry)]
        } else {
            Vec::new()
        };
        let mut completion = super::super::execution::completion_request(
            &options,
            effective_system,
            turns,
            tool_defs,
        );
        completion.disable_tools = !tools_enabled;
        completion.stop_sequences = request.stop_sequences;

        let sample_cancel = cancellation.child_token();
        let _cancel_on_drop = sample_cancel.clone().drop_guard();
        let permit = matches!(owner, SamplingOwner::View(_))
            .then(|| {
                self.auxiliary_tasks
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| AppError::Config("auxiliary task capacity exhausted".into()))
            })
            .transpose()?;
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Ok(None),
            _ = owner_cancel.cancelled() => return Ok(None),
            result = tokio::time::timeout(TIMEOUT, crate::provider::with_request_cancellation(
                Some(sample_cancel.clone()),
                state.providers.complete(&options.model, completion),
            )) => result.map_err(|_| AppError::Internal("MCP sampling request timed out".into()))??,
        };
        drop(permit);

        if response.text.len() > MAX_INPUT_BYTES {
            return Err(AppError::Config("MCP sampling response exceeds byte budget".into()));
        }
        if !tools_enabled && !response.tool_calls.is_empty() {
            return Err(AppError::Config("unsupported MCP sampling response".into()));
        }

        let mut reply_blocks = Vec::new();
        if !response.text.is_empty() {
            reply_blocks.push(McpSamplingReplyBlock::Text(response.text.clone()));
        }
        let mut tool_use_count = 0usize;
        if tools_enabled {
            for call in response.tool_calls {
                let (id, name, input) = decode_sampling_tool_call(&call, &tool_registry)?;
                tool_use_count += 1;
                reply_blocks.push(McpSamplingReplyBlock::ToolUse { id, name, input });
            }
            if request.tool_choice == Some(McpSamplingToolChoice::Required) && tool_use_count == 0
            {
                return Err(AppError::Config(
                    "MCP sampling model did not emit a required tool call".into(),
                ));
            }
        }
        if reply_blocks.is_empty() {
            reply_blocks.push(McpSamplingReplyBlock::Text(String::new()));
        }

        let release_preview = format_release_preview(&reply_blocks);
        let allowed = approve(release_prompt, release_preview).await?;
        if allowed.cancelled
            || allowed.timed_out
            || allowed.reply != ALLOW
            || cancellation.is_cancelled()
            || owner_cancel.is_cancelled()
        {
            return Ok(None);
        }
        let stop_reason = if tool_use_count > 0 {
            Some("toolUse".into())
        } else {
            match response.finish_reason {
                Some(CompletionFinishReason::Length) => Some("maxTokens".into()),
                Some(CompletionFinishReason::Stop) => Some("endTurn".into()),
                _ => None,
            }
        };
        Ok(Some(McpSamplingReply {
            model: response.model.to_string(),
            content: reply_blocks,
            stop_reason,
        }))
    }
}

struct SamplingToolRegistry {
    wire_to_original: std::collections::BTreeMap<String, String>,
    original_to_wire: std::collections::BTreeMap<String, String>,
    wire_order: Vec<String>,
}

impl SamplingToolRegistry {
    fn new(tools: &[McpSamplingTool]) -> Self {
        let mut wire_to_original = std::collections::BTreeMap::new();
        let mut original_to_wire = std::collections::BTreeMap::new();
        let mut wire_order = Vec::with_capacity(tools.len());
        for (index, tool) in tools.iter().enumerate() {
            let base = sanitize_sampling_tool_segment(&tool.name, index);
            let mut candidate = format!("mcp_sampling.{base}");
            let mut suffix = 1usize;
            while wire_to_original.contains_key(&candidate) {
                candidate = format!("mcp_sampling.{base}_{suffix}");
                suffix += 1;
            }
            wire_to_original.insert(candidate.clone(), tool.name.clone());
            original_to_wire.insert(tool.name.clone(), candidate.clone());
            wire_order.push(candidate);
        }
        Self {
            wire_to_original,
            original_to_wire,
            wire_order,
        }
    }

    fn wire_name_for(&self, original: &str) -> String {
        self.original_to_wire
            .get(original)
            .cloned()
            .unwrap_or_else(|| format!("mcp_sampling.{}", sanitize_sampling_tool_segment(original, 0)))
    }

    fn resolve_original(&self, candidate: &str) -> Option<&str> {
        if let Some(original) = self.wire_to_original.get(candidate) {
            return Some(original.as_str());
        }
        self.original_to_wire
            .get_key_value(candidate)
            .map(|(original, _)| original.as_str())
    }
}

fn sanitize_sampling_tool_segment(name: &str, fallback_index: usize) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-') {
            sanitized.push(byte as char);
        } else {
            sanitized.push('_');
        }
    }
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        format!("tool_{fallback_index}")
    } else {
        trimmed.to_owned()
    }
}

fn sampling_gateway_tool_definition(registry: &SamplingToolRegistry) -> ToolApiDefinition {
    let names: Vec<Value> = registry
        .wire_order
        .iter()
        .cloned()
        .map(Value::String)
        .collect();
    ToolApiDefinition {
        name: ToolApiFunction::Call.function_name().into(),
        description: "Invoke one of the sampling tools declared by the MCP server.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "enum": names,
                    "description": "Exact sampling tool identifier from the available MCP sampling tools list."
                },
                "input": {
                    "type": "object",
                    "description": "Arguments matching the selected sampling tool inputSchema.",
                    "additionalProperties": true
                }
            },
            "required": ["tool", "input"],
            "additionalProperties": false
        }),
        output_schema: json!({}),
    }
}

fn sampling_tools_prompt(
    tools: &[McpSamplingTool],
    registry: &SamplingToolRegistry,
    choice: Option<McpSamplingToolChoice>,
) -> String {
    let requirement = match choice {
        Some(McpSamplingToolChoice::Required) => {
            "You MUST invoke at least one of these tools using `tools_call` with `tool` set to the listed `mcp_sampling.*` name."
        }
        _ => "When calling a tool, invoke `tools_call` with `tool` set to the listed `mcp_sampling.*` name and `input` set to an object matching its inputSchema.",
    };
    let mut out = format!("# Available MCP sampling tools\n{requirement}\n");
    for tool in tools {
        let wire = registry.wire_name_for(&tool.name);
        let _ = write!(out, "\n- `{wire}` (MCP tool `{}`)", tool.name);
        if let Some(desc) = tool.description.as_deref().filter(|d| !d.trim().is_empty()) {
            let _ = write!(out, ": {desc}");
        }
        let _ = writeln!(out, "\n  inputSchema: {}", tool.input_schema);
    }
    out
}

fn decode_sampling_tool_call(
    call: &agena_provider::CompletionToolCall,
    registry: &SamplingToolRegistry,
) -> Result<(String, String, Map<String, Value>), AppError> {
    let agena_provider::CompletionToolCall::Function {
        id,
        name: call_name,
        arguments_json,
    } = call;
    let parsed: Value = serde_json::from_str(arguments_json).map_err(|_| {
        AppError::Config("MCP sampling model emitted malformed tool arguments".into())
    })?;
    let object = parsed.as_object().ok_or_else(|| {
        AppError::Config("MCP sampling tool arguments must be a JSON object".into())
    })?;
    if call_name.as_str() == ToolApiFunction::Call.function_name()
        && let Some(tool_name) = object.get("tool").and_then(Value::as_str)
    {
        let Some(original) = registry.resolve_original(tool_name) else {
            return Err(AppError::Config(format!(
                "MCP sampling model requested undeclared tool {tool_name:?}"
            )));
        };
        let input = match object.get("input") {
            Some(Value::Object(map)) => map.clone(),
            Some(Value::String(raw)) => serde_json::from_str::<Map<String, Value>>(raw)
                .map_err(|_| {
                    AppError::Config("MCP sampling tool input string must decode to an object".into())
                })?,
            None => Map::new(),
            _ => {
                return Err(AppError::Config(
                    "MCP sampling tool input must be a JSON object".into(),
                ));
            }
        };
        return Ok((id.clone(), original.to_owned(), input));
    }
    let direct_name = call_name.as_str();
    if let Some(original) = registry.resolve_original(direct_name) {
        return Ok((id.clone(), original.to_owned(), object.clone()));
    }
    Err(AppError::Config(format!(
        "MCP sampling model requested undeclared tool {direct_name:?}"
    )))
}

fn format_release_preview(blocks: &[McpSamplingReplyBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        if !out.is_empty() {
            out.push('\n');
        }
        match block {
            McpSamplingReplyBlock::Text(text) => out.push_str(text),
            McpSamplingReplyBlock::ToolUse { id, name, input } => {
                let _ = write!(
                    out,
                    "Tool call {name} ({id}): {}",
                    Value::Object(input.clone())
                );
            }
        }
    }
    out
}

fn truncate_str(value: &str, max_bytes: usize) -> &str {
    &value[..value.floor_char_boundary(max_bytes.min(value.len()))]
}

fn sampling_dialog(server: &str, action: &str, preview: String) -> AskUserToolInput {
    AskUserToolInput {
        title: format!("MCP sampling from {server}"),
        kind: "ask_user".into(),
        body_markdown: String::new(),
        auto_resolution_ms: None,
        questions: vec![UserInputQuestion {
            header: "MCP sampling".into(),
            question: format!("{action}?\n\n{preview}"),
            options: [ALLOW, "Decline"]
                .into_iter()
                .map(|label| UserInputOption {
                    label: label.into(),
                    description: String::new(),
                    custom: false,
                })
                .collect(),
            multiple: false,
            allow_custom: false,
        }],
    }
}
