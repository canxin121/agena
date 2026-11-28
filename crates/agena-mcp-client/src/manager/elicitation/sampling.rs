#![allow(deprecated)]

use std::sync::Arc;

use agena_domain::mcp_app_tools::McpAppTaskInput;
use agena_domain::mcp_sampling::{
    MAX_INPUT_BYTES, MAX_MEDIA_BLOCK_BYTES, MAX_MEDIA_BLOCKS, MAX_MEDIA_TOTAL_BYTES, MAX_TOOLS,
    McpSamplingBlock, McpSamplingIncludeContext, McpSamplingMessage, McpSamplingReplyBlock,
    McpSamplingRequest, McpSamplingTool, McpSamplingToolChoice, McpSamplingToolResultBlock,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rmcp::model::{
    AudioContent, ContentBlock, ContextInclusion, CreateMessageRequestParams, CreateMessageResult,
    ErrorCode, ImageContent, Role, SamplingContent, SamplingMessage, SamplingMessageContentBlock,
    Tool, ToolChoice, ToolChoiceMode, ToolResultContent,
};
use serde_json::{Map, Value, json};

use super::{
    CancellationToken, ConnectedServer, ErrorData, Incoming, McpElicitationHandler, McpError,
    McpResult, RequestContext, RequestError, RoleClient, ServiceError, TaskRegistry,
    associated_incoming, oneshot,
};

fn validate_image(mime_type: &str, data: &str, media_count: &mut usize, media_bytes: &mut usize) -> McpResult<()> {
    let mime = mime_type.trim().to_ascii_lowercase();
    if !matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    ) {
        return Err(McpError::Malformed(format!(
            "unsupported MCP sampling image MIME type {mime_type:?}"
        )));
    }
    let decoded = decode_media_bytes(data, media_count, media_bytes)?;
    let valid_magic = match mime.as_str() {
        "image/png" => decoded.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => decoded.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => decoded.starts_with(b"GIF87a") || decoded.starts_with(b"GIF89a"),
        "image/webp" => decoded.len() >= 12 && &decoded[0..4] == b"RIFF" && &decoded[8..12] == b"WEBP",
        _ => false,
    };
    if !valid_magic {
        return Err(McpError::Malformed(
            "MCP sampling image bytes do not match declared MIME type".into(),
        ));
    }
    Ok(())
}

fn validate_audio(mime_type: &str, data: &str, media_count: &mut usize, media_bytes: &mut usize) -> McpResult<()> {
    let mime = mime_type.trim().to_ascii_lowercase();
    if !matches!(
        mime.as_str(),
        "audio/wav"
            | "audio/x-wav"
            | "audio/mpeg"
            | "audio/mp3"
            | "audio/ogg"
            | "audio/webm"
            | "audio/flac"
            | "audio/mp4"
            | "audio/aac"
    ) {
        return Err(McpError::Malformed(format!(
            "unsupported MCP sampling audio MIME type {mime_type:?}"
        )));
    }
    let _ = decode_media_bytes(data, media_count, media_bytes)?;
    Ok(())
}

fn decode_media_bytes(
    data: &str,
    media_count: &mut usize,
    media_bytes: &mut usize,
) -> McpResult<Vec<u8>> {
    if *media_count >= MAX_MEDIA_BLOCKS {
        return Err(McpError::Malformed(
            "MCP sampling request exceeds maximum media block count".into(),
        ));
    }
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed.len() > (MAX_MEDIA_BLOCK_BYTES / 3 + 2) * 4 {
        return Err(McpError::Malformed(
            "MCP sampling media block is empty or exceeds size budget".into(),
        ));
    }
    let decoded = STANDARD
        .decode(trimmed)
        .map_err(|_| McpError::Malformed("MCP sampling media data is not valid base64".into()))?;
    if decoded.is_empty() || decoded.len() > MAX_MEDIA_BLOCK_BYTES {
        return Err(McpError::Malformed(
            "MCP sampling media block exceeds 4 MiB limit".into(),
        ));
    }
    *media_bytes = media_bytes.saturating_add(decoded.len());
    if *media_bytes > MAX_MEDIA_TOTAL_BYTES {
        return Err(McpError::Malformed(
            "MCP sampling media exceeds 8 MiB total budget".into(),
        ));
    }
    *media_count += 1;
    Ok(decoded)
}

pub(super) fn decode(params: CreateMessageRequestParams) -> McpResult<McpSamplingRequest> {
    let invalid = || McpError::Malformed("invalid MCP sampling request".into());
    if params.max_tokens == 0
        || params.messages.is_empty()
        || params.messages.len() > 128
        || params
            .temperature
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(invalid());
    }
    let include_context = match params.include_context {
        None | Some(ContextInclusion::None) => McpSamplingIncludeContext::None,
        Some(ContextInclusion::ThisServer) => McpSamplingIncludeContext::ThisServer,
        Some(ContextInclusion::AllServers) => McpSamplingIncludeContext::AllServers,
        Some(_) => return Err(McpError::SamplingUnsupported),
    };
    let tool_choice = match params.tool_choice.and_then(|choice| choice.mode) {
        None => None,
        Some(ToolChoiceMode::Auto) => Some(McpSamplingToolChoice::Auto),
        Some(ToolChoiceMode::Required) => Some(McpSamplingToolChoice::Required),
        Some(ToolChoiceMode::None) => Some(McpSamplingToolChoice::None),
        Some(_) => return Err(McpError::SamplingUnsupported),
    };
    let stop_sequences = params.stop_sequences.unwrap_or_default();
    if stop_sequences.len() > 16
        || stop_sequences
            .iter()
            .any(|stop| stop.is_empty() || stop.len() > 1024)
    {
        return Err(invalid());
    }
    let mut bytes = params.system_prompt.as_ref().map_or(0, String::len)
        + stop_sequences.iter().map(String::len).sum::<usize>();
    let mut tools = Vec::new();
    if let Some(declared_tools) = params.tools {
        if declared_tools.len() > MAX_TOOLS {
            return Err(invalid());
        }
        let mut seen = std::collections::BTreeSet::new();
        for tool in declared_tools {
            let name = tool.name.to_string();
            if name.trim().is_empty() || name.len() > 128 || !seen.insert(name.clone()) {
                return Err(invalid());
            }
            let description = tool.description.map(|d| d.to_string());
            let schema = Value::Object((*tool.input_schema).clone());
            let schema_len = schema.to_string().len();
            bytes = bytes
                .saturating_add(name.len())
                .saturating_add(description.as_ref().map_or(0, String::len))
                .saturating_add(schema_len);
            if bytes > MAX_INPUT_BYTES {
                return Err(invalid());
            }
            tools.push(McpSamplingTool {
                name,
                description,
                input_schema: schema,
            });
        }
    }
    if tool_choice == Some(McpSamplingToolChoice::Required) && tools.is_empty() {
        return Err(invalid());
    }

    let mut media_count = 0usize;
    let mut media_bytes = 0usize;
    let mut messages = Vec::with_capacity(params.messages.len());
    for message in params.messages {
        let role = match message.role {
            Role::User => agena_domain::Role::User,
            Role::Assistant => agena_domain::Role::Assistant,
        };
        if message.content.is_empty() || message.content.len() > 128 {
            return Err(invalid());
        }
        let mut content = Vec::new();
        for block in message.content.into_vec() {
            match block {
                SamplingMessageContentBlock::Text(text) => {
                    bytes = bytes.saturating_add(text.text.len());
                    if bytes > MAX_INPUT_BYTES {
                        return Err(invalid());
                    }
                    content.push(McpSamplingBlock::Text(text.text));
                }
                SamplingMessageContentBlock::Image(image) => {
                    validate_image(
                        &image.mime_type,
                        &image.data,
                        &mut media_count,
                        &mut media_bytes,
                    )?;
                    content.push(McpSamplingBlock::Image {
                        mime_type: image.mime_type.trim().to_ascii_lowercase(),
                        data: image.data.trim().to_owned(),
                    });
                }
                SamplingMessageContentBlock::Audio(audio) => {
                    validate_audio(
                        &audio.mime_type,
                        &audio.data,
                        &mut media_count,
                        &mut media_bytes,
                    )?;
                    content.push(McpSamplingBlock::Audio {
                        mime_type: audio.mime_type.trim().to_ascii_lowercase(),
                        data: audio.data.trim().to_owned(),
                    });
                }
                SamplingMessageContentBlock::ToolUse(tool_use) => {
                    if role != agena_domain::Role::Assistant
                        || tool_use.id.trim().is_empty()
                        || tool_use.id.len() > 128
                        || tool_use.name.trim().is_empty()
                        || tool_use.name.len() > 128
                    {
                        return Err(invalid());
                    }
                    let input_len = Value::Object(tool_use.input.clone()).to_string().len();
                    bytes = bytes
                        .saturating_add(tool_use.id.len())
                        .saturating_add(tool_use.name.len())
                        .saturating_add(input_len);
                    if bytes > MAX_INPUT_BYTES {
                        return Err(invalid());
                    }
                    content.push(McpSamplingBlock::ToolUse {
                        id: tool_use.id,
                        name: tool_use.name,
                        input: tool_use.input,
                    });
                }
                SamplingMessageContentBlock::ToolResult(tool_result) => {
                    if role != agena_domain::Role::User
                        || tool_result.tool_use_id.trim().is_empty()
                        || tool_result.tool_use_id.len() > 128
                        || tool_result.content.len() > 64
                    {
                        return Err(invalid());
                    }
                    let mut result_blocks = Vec::new();
                    for item in tool_result.content {
                        match item {
                            ContentBlock::Text(text) => {
                                bytes = bytes.saturating_add(text.text.len());
                                if bytes > MAX_INPUT_BYTES {
                                    return Err(invalid());
                                }
                                result_blocks.push(McpSamplingToolResultBlock::Text(text.text));
                            }
                            ContentBlock::Image(image) => {
                                validate_image(
                                    &image.mime_type,
                                    &image.data,
                                    &mut media_count,
                                    &mut media_bytes,
                                )?;
                                result_blocks.push(McpSamplingToolResultBlock::Image {
                                    mime_type: image.mime_type.trim().to_ascii_lowercase(),
                                    data: image.data.trim().to_owned(),
                                });
                            }
                            ContentBlock::Audio(audio) => {
                                validate_audio(
                                    &audio.mime_type,
                                    &audio.data,
                                    &mut media_count,
                                    &mut media_bytes,
                                )?;
                                result_blocks.push(McpSamplingToolResultBlock::Audio {
                                    mime_type: audio.mime_type.trim().to_ascii_lowercase(),
                                    data: audio.data.trim().to_owned(),
                                });
                            }
                            _ => return Err(McpError::SamplingUnsupported),
                        }
                    }
                    if result_blocks.is_empty()
                        && let Some(structured) = tool_result.structured_content
                    {
                        let rendered = structured.to_string();
                        bytes = bytes.saturating_add(rendered.len());
                        if bytes > MAX_INPUT_BYTES {
                            return Err(invalid());
                        }
                        result_blocks.push(McpSamplingToolResultBlock::Text(rendered));
                    }
                    content.push(McpSamplingBlock::ToolResult {
                        tool_use_id: tool_result.tool_use_id,
                        content: result_blocks,
                        is_error: tool_result.is_error.unwrap_or(false),
                    });
                }
                _ => return Err(McpError::SamplingUnsupported),
            }
        }
        messages.push(McpSamplingMessage { role, content });
    }
    Ok(McpSamplingRequest {
        messages,
        system_prompt: params.system_prompt,
        include_context,
        tools,
        tool_choice,
        max_tokens: params.max_tokens,
        temperature: params.temperature,
        stop_sequences,
    })
}

pub(in crate::manager) async fn sampling_request(
    params: CreateMessageRequestParams,
    context: RequestContext<RoleClient>,
    tasks: &TaskRegistry,
) -> Result<CreateMessageResult, ErrorData> {
    let legacy = tasks.legacy_route(&context)?;
    let sender = legacy
        .as_ref()
        .map(|task| &task.incoming.sender)
        .or_else(|| associated_incoming(&context))
        .ok_or_else(|| wire_error(McpError::SamplingDeclined))?;
    let request = decode(params).map_err(wire_error)?;
    let (reply, answer) = oneshot::channel();
    sender
        .try_send(Incoming::Sampling {
            request,
            cancellation: context.ct.clone(),
            reply,
        })
        .map_err(|_| wire_error(McpError::SamplingDeclined))?;
    let mut result = tokio::select! {
        biased;
        _ = context.ct.cancelled() => Err(wire_error(McpError::SamplingDeclined)),
        reply = answer => reply.unwrap_or_else(|_| Err(wire_error(McpError::SamplingDeclined))),
    }?;
    if let Some(task) = legacy {
        result.message.meta = Some(task.metadata());
    }
    Ok(result)
}

pub(super) async fn continue_sampling(
    server: &ConnectedServer,
    request: McpSamplingRequest,
    handler: Option<&dyn McpElicitationHandler>,
) -> Result<CreateMessageResult, RequestError> {
    let cancel = server.events.shutdown.child_token();
    let _cancel_on_drop = cancel.clone().drop_guard();
    let work = respond(
        &server.name,
        &server.redactions,
        request,
        handler,
        cancel.clone(),
    );
    tokio::pin!(work);
    tokio::select! {
        biased;
        _ = server.peer.transport_closed() => {
            cancel.cancel();
            let _ = work.await;
            Err(ServiceError::TransportClosed.into())
        }
        result = &mut work => Ok(result?),
    }
}

fn redact_value(redactions: &crate::McpRedactions, value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redactions.text(text)),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(|v| redact_value(redactions, v)).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, redact_value(redactions, v)))
                .collect(),
        ),
        other => other,
    }
}

fn redact_map(redactions: &crate::McpRedactions, map: Map<String, Value>) -> Map<String, Value> {
    map.into_iter()
        .map(|(k, v)| (k, redact_value(redactions, v)))
        .collect()
}

pub(super) fn app_input(
    redactions: &crate::McpRedactions,
    request: &McpSamplingRequest,
) -> McpResult<McpAppTaskInput> {
    let redact = |text: &String| redactions.text(text.clone());
    let messages = request
        .messages
        .iter()
        .map(|message| {
            let role = if message.role == agena_domain::Role::User {
                Role::User
            } else {
                Role::Assistant
            };
            let blocks = message
                .content
                .iter()
                .map(|block| match block {
                    McpSamplingBlock::Text(text) => SamplingMessageContentBlock::text(redact(text)),
                    McpSamplingBlock::Image { mime_type, data } => {
                        SamplingMessageContentBlock::Image(ImageContent::new(
                            data.clone(),
                            mime_type.clone(),
                        ))
                    }
                    McpSamplingBlock::Audio { mime_type, data } => {
                        SamplingMessageContentBlock::Audio(AudioContent::new(
                            data.clone(),
                            mime_type.clone(),
                        ))
                    }
                    McpSamplingBlock::ToolUse { id, name, input } => {
                        SamplingMessageContentBlock::tool_use(
                            id.clone(),
                            name.clone(),
                            redact_map(redactions, input.clone()),
                        )
                    }
                    McpSamplingBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        let items = content
                            .iter()
                            .map(|item| match item {
                                McpSamplingToolResultBlock::Text(text) => {
                                    ContentBlock::text(redact(text))
                                }
                                McpSamplingToolResultBlock::Image { mime_type, data } => {
                                    ContentBlock::image(data.clone(), mime_type.clone())
                                }
                                McpSamplingToolResultBlock::Audio { mime_type, data } => {
                                    ContentBlock::audio(data.clone(), mime_type.clone())
                                }
                            })
                            .collect();
                        let mut res = ToolResultContent::new(tool_use_id.clone(), items);
                        if *is_error {
                            res.is_error = Some(true);
                        }
                        SamplingMessageContentBlock::ToolResult(res)
                    }
                })
                .collect();
            SamplingMessage::new_multiple(role, blocks)
        })
        .collect();
    let mut params = CreateMessageRequestParams::new(messages, request.max_tokens);
    params.temperature = request.temperature;
    params.system_prompt = request.system_prompt.as_ref().map(redact);
    params.include_context = match request.include_context {
        McpSamplingIncludeContext::None => None,
        McpSamplingIncludeContext::ThisServer => Some(ContextInclusion::ThisServer),
        McpSamplingIncludeContext::AllServers => Some(ContextInclusion::AllServers),
    };
    if !request.tools.is_empty() {
        params.tools = Some(
            request
                .tools
                .iter()
                .map(|tool| {
                    let schema = tool
                        .input_schema
                        .as_object()
                        .cloned()
                        .unwrap_or_default();
                    Tool::new(
                        tool.name.clone(),
                        tool.description
                            .as_ref()
                            .map(redact)
                            .unwrap_or_default(),
                        Arc::new(schema),
                    )
                })
                .collect(),
        );
    }
    params.tool_choice = request.tool_choice.map(|choice| match choice {
        McpSamplingToolChoice::Auto => ToolChoice::auto(),
        McpSamplingToolChoice::Required => ToolChoice::required(),
        McpSamplingToolChoice::None => ToolChoice::none(),
    });
    if !request.stop_sequences.is_empty() {
        params.stop_sequences = Some(request.stop_sequences.iter().map(redact).collect());
    }
    Ok(McpAppTaskInput::new(
        json!({"method":"sampling/createMessage", "params":serde_json::to_value(params)?}),
        app_response,
    ))
}

fn app_response(response: Value) -> Option<Value> {
    let response: CreateMessageResult = serde_json::from_value(response).ok()?;
    response.validate().ok()?;
    let sanitize = |block: SamplingMessageContentBlock| -> Option<SamplingMessageContentBlock> {
        match block {
            SamplingMessageContentBlock::Text(text) => {
                Some(SamplingMessageContentBlock::text(text.text))
            }
            SamplingMessageContentBlock::ToolUse(tool_use) => {
                Some(SamplingMessageContentBlock::tool_use(
                    tool_use.id,
                    tool_use.name,
                    tool_use.input,
                ))
            }
            _ => None,
        }
    };
    let content = match response.message.content {
        SamplingContent::Single(block) => sanitize(block)?.into(),
        SamplingContent::Multiple(blocks) if !blocks.is_empty() && blocks.len() <= 128 => blocks
            .into_iter()
            .map(sanitize)
            .collect::<Option<Vec<_>>>()?
            .into(),
        _ => return None,
    };
    let mut message = SamplingMessage::new(Role::Assistant, "");
    message.content = content;
    let mut result = CreateMessageResult::new(message, response.model);
    result.stop_reason = response.stop_reason;
    serde_json::to_value(result).ok()
}

pub(super) async fn respond(
    server: &str,
    redactions: &crate::McpRedactions,
    mut request: McpSamplingRequest,
    handler: Option<&dyn McpElicitationHandler>,
    cancellation: CancellationToken,
) -> McpResult<CreateMessageResult> {
    let handler = handler.ok_or(McpError::SamplingUnsupported)?;
    if cancellation.is_cancelled() {
        return Err(McpError::SamplingDeclined);
    }
    let redact = |value: &mut String| *value = redactions.text(std::mem::take(value));
    if let Some(prompt) = &mut request.system_prompt {
        redact(prompt);
    }
    for message in &mut request.messages {
        for block in &mut message.content {
            match block {
                McpSamplingBlock::Text(text) => redact(text),
                McpSamplingBlock::ToolUse { input, .. } => {
                    *input = redact_map(redactions, std::mem::take(input));
                }
                McpSamplingBlock::ToolResult { content, .. } => {
                    for item in content {
                        if let McpSamplingToolResultBlock::Text(text) = item {
                            redact(text);
                        }
                    }
                }
                McpSamplingBlock::Image { .. } | McpSamplingBlock::Audio { .. } => {}
            }
        }
    }
    request.stop_sequences.iter_mut().for_each(redact);
    let reply = handler.sample(server, request, cancellation).await?;
    let mut blocks = Vec::with_capacity(reply.content.len());
    for block in reply.content {
        match block {
            McpSamplingReplyBlock::Text(text) => {
                blocks.push(SamplingMessageContentBlock::text(redactions.text(text)));
            }
            McpSamplingReplyBlock::ToolUse { id, name, input } => {
                blocks.push(SamplingMessageContentBlock::tool_use(
                    id,
                    name,
                    redact_map(redactions, input),
                ));
            }
        }
    }
    let message = match blocks.len() {
        0 => SamplingMessage::assistant_text(""),
        1 => SamplingMessage::new(Role::Assistant, blocks.remove(0)),
        _ => SamplingMessage::new_multiple(Role::Assistant, blocks),
    };
    let mut result = CreateMessageResult::new(message, reply.model);
    result.stop_reason = reply.stop_reason;
    Ok(result)
}

pub(super) fn wire_error(error: McpError) -> ErrorData {
    let message = error.user_message();
    match error {
        McpError::SamplingUnsupported => ErrorData::new(ErrorCode::METHOD_NOT_FOUND, message, None),
        McpError::Malformed(_) => ErrorData::invalid_params(message, None),
        McpError::SamplingDeclined => ErrorData::invalid_request(message, None),
        _ => ErrorData::internal_error(message, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_2X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DA8J8BAAf/Af8Bf4mnAAAAAElFTkSuQmCC";

    #[test]
    fn decode_accepts_media_tools_and_include_context_and_rejects_spoofed_media() {
        let mut params = CreateMessageRequestParams::new(
            vec![
                SamplingMessage::new_multiple(
                    Role::User,
                    vec![
                        SamplingMessageContentBlock::text("inspect this image"),
                        SamplingMessageContentBlock::Image(ImageContent::new(
                            PNG_2X1,
                            "image/png",
                        )),
                        SamplingMessageContentBlock::Audio(AudioContent::new(
                            STANDARD.encode(b"RIFF0000WAVE"),
                            "audio/wav",
                        )),
                    ],
                ),
                SamplingMessage::new(
                    Role::Assistant,
                    SamplingMessageContentBlock::tool_use(
                        "call-1",
                        "lookup",
                        json!({"q": "x"}).as_object().unwrap().clone(),
                    ),
                ),
                SamplingMessage::new(
                    Role::User,
                    SamplingMessageContentBlock::tool_result(
                        "call-1",
                        vec![ContentBlock::text("found")],
                    ),
                ),
            ],
            256,
        );
        params.include_context = Some(ContextInclusion::ThisServer);
        params.tools = Some(vec![Tool::new(
            "lookup",
            "Lookup an entry",
            Arc::new(json!({"type": "object", "properties": {"q": {"type": "string"}}}).as_object().unwrap().clone()),
        )]);
        params.tool_choice = Some(ToolChoice::required());

        let decoded = decode(params).expect("valid sampling request with media, tools, and context");
        assert_eq!(decoded.include_context, McpSamplingIncludeContext::ThisServer);
        assert_eq!(decoded.tools.len(), 1);
        assert_eq!(decoded.tool_choice, Some(McpSamplingToolChoice::Required));
        assert_eq!(decoded.messages.len(), 3);

        // Spoofed PNG MIME type must be rejected.
        let spoofed = CreateMessageRequestParams::new(
            vec![SamplingMessage::new(
                Role::User,
                SamplingMessageContentBlock::Image(ImageContent::new(
                    STANDARD.encode(b"not-a-png"),
                    "image/png",
                )),
            )],
            64,
        );
        assert!(matches!(decode(spoofed), Err(McpError::Malformed(_))));

        // ToolChoice::Required without tools must be rejected.
        let mut missing_tools = CreateMessageRequestParams::new(
            vec![SamplingMessage::user_text("hi")],
            64,
        );
        missing_tools.tool_choice = Some(ToolChoice::required());
        assert!(matches!(decode(missing_tools), Err(McpError::Malformed(_))));
    }
}
