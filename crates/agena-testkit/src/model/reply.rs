use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use axum::http::{
    HeaderMap, HeaderName, HeaderValue, StatusCode,
    header::{CACHE_CONTROL, CONTENT_TYPE},
};
use bytes::Bytes;
use serde_json::{Value, json};

use super::{ModelEvent, ModelOptions, ModelProtocol, ModelRequest};

/// Provider-neutral usage. Cached input and reasoning output are subsets
/// of input_tokens and output_tokens, respectively.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
}

impl Usage {
    pub fn tokens(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cached_tokens: 0,
            reasoning_tokens: 0,
        }
    }

    pub fn cached(mut self, tokens: u64) -> Self {
        self.cached_tokens = tokens;
        self
    }

    pub fn reasoning(mut self, tokens: u64) -> Self {
        self.reasoning_tokens = tokens;
        self
    }

    fn json(&self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.input_tokens.saturating_add(self.output_tokens),
            "input_tokens_details": {"cached_tokens": self.cached_tokens},
            "output_tokens_details": {"reasoning_tokens": self.reasoning_tokens}
        })
    }
}

impl Default for Usage {
    fn default() -> Self {
        Self::tokens(5, 3)
    }
}

/// One provider function call. A normal Agena tool uses name tools_call and an
/// arguments object with tool and input; approval calls use their own names.
#[derive(Clone, Debug)]
pub struct FunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments_json: String,
    /// Optional argument deltas. The terminal snapshot remains arguments_json,
    /// which also makes inconsistent delta/snapshot fault scenarios expressible.
    pub argument_chunks: Option<Vec<String>>,
}

impl FunctionCall {
    pub fn new(call_id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self::raw(call_id, name, arguments.to_string())
    }

    pub fn raw(
        call_id: impl Into<String>,
        name: impl Into<String>,
        arguments_json: impl Into<String>,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            arguments_json: arguments_json.into(),
            argument_chunks: None,
        }
    }

    pub fn tool(call_id: impl Into<String>, tool: impl Into<String>, input: Value) -> Self {
        Self::new(
            call_id,
            "tools_call",
            json!({"tool": tool.into(), "input": input}),
        )
    }

    pub fn arguments_chunks<I, S>(mut self, chunks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.argument_chunks = Some(chunks.into_iter().map(Into::into).collect());
        self
    }
}

#[derive(Clone, Debug)]
pub(super) struct Completion {
    pub(super) text: Vec<String>,
    pub(super) thinking: Vec<String>,
    pub(super) functions: Vec<FunctionCall>,
    pub(super) usage: Option<Usage>,
    pub(super) end_turn: Option<bool>,
    pub(super) finish_reason: Option<String>,
    pub(super) incomplete: Option<String>,
}

impl Default for Completion {
    fn default() -> Self {
        Self {
            text: Vec::new(),
            thinking: Vec::new(),
            functions: Vec::new(),
            usage: Some(Usage::default()),
            end_turn: None,
            finish_reason: None,
            incomplete: None,
        }
    }
}

#[derive(Clone, Debug)]
enum ReplyBody {
    Completion(Completion),
    Json(Value),
    Bytes(Bytes),
    Stream(Vec<Bytes>),
    Events(ModelProtocol, Vec<ModelEvent>),
}

/// A model reply plan. Completion replies automatically select JSON or SSE from
/// the incoming stream flag. Raw replies bypass that selection for fault tests.
#[derive(Clone, Debug)]
pub struct Reply {
    body: ReplyBody,
    protocol: Option<ModelProtocol>,
    status: u16,
    headers: BTreeMap<String, String>,
    hold_after: Option<usize>,
    hold_after_text: Option<usize>,
    fragment_bytes: Option<usize>,
    invalid: Option<String>,
    /// This reply is a `tools_call` that needs its tool's help in the current
    /// context. The scripted model answers the gate itself instead of making
    /// every scenario script a preliminary `tools_help` step.
    read_required_help: bool,
}

impl Reply {
    /// The execution tools this reply calls through `tools_call`. The runtime
    /// requires each tool's help in the current context first.
    pub(super) fn called_tools(&self) -> Vec<String> {
        let ReplyBody::Completion(completion) = &self.body else {
            return Vec::new();
        };
        completion
            .functions
            .iter()
            .filter(|call| call.name == "tools_call")
            .filter_map(|call| {
                serde_json::from_str::<Value>(&call.arguments_json)
                    .ok()?
                    .get("tool")?
                    .as_str()
                    .map(str::trim)
                    .filter(|tool| !tool.is_empty())
                    .map(ToOwned::to_owned)
            })
            .collect()
    }

    /// Whether this reply opted into the automatic `tools_help` step.
    pub(super) fn reads_required_help(&self) -> bool {
        self.read_required_help
    }

    fn require_help(mut self) -> Self {
        self.read_required_help = true;
        self
    }

    fn completion(completion: Completion) -> Self {
        Self::new(ReplyBody::Completion(completion), 200)
    }

    fn new(body: ReplyBody, status: u16) -> Self {
        Self {
            body,
            protocol: None,
            status,
            headers: BTreeMap::new(),
            hold_after: None,
            hold_after_text: None,
            fragment_bytes: None,
            invalid: None,
            read_required_help: false,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::text_chunks([text.into()])
    }

    /// Override completion encoding for a custom route. Standard endpoints are
    /// detected automatically; this never changes raw JSON/body/stream bytes.
    pub fn for_protocol(mut self, protocol: ModelProtocol) -> Self {
        self.protocol = Some(protocol);
        self
    }

    /// Structured native events, including arbitrary provider fields. Framing
    /// is added, but no terminal event or implicit completion is inserted.
    pub fn events(protocol: ModelProtocol, events: impl IntoIterator<Item = ModelEvent>) -> Self {
        Self::new(
            ReplyBody::Events(protocol, events.into_iter().collect()),
            200,
        )
    }

    /// Each text chunk becomes a separate SSE delta. JSON replies join them.
    pub fn text_chunks<I, S>(chunks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::completion(Completion {
            text: chunks.into_iter().map(Into::into).collect(),
            ..Completion::default()
        })
    }

    pub fn function_call(
        call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: Value,
    ) -> Self {
        Self::function_calls([FunctionCall::new(call_id, name, arguments)])
    }

    pub fn function_call_raw(
        call_id: impl Into<String>,
        name: impl Into<String>,
        arguments_json: impl Into<String>,
    ) -> Self {
        Self::function_calls([FunctionCall::raw(call_id, name, arguments_json)])
    }

    pub fn function_calls(calls: impl IntoIterator<Item = FunctionCall>) -> Self {
        Self::completion(Completion {
            functions: calls.into_iter().collect(),
            ..Completion::default()
        })
    }

    pub fn tool_call(call_id: impl Into<String>, tool: impl Into<String>, input: Value) -> Self {
        // The runtime rejects a tools_call whose help is not in the current
        // context, so a scripted model reads the contract first, exactly like a
        // real model must. The helper call is answered automatically and needs
        // no scripted expectation.
        let mut reply = Self::function_calls([FunctionCall::tool(call_id, tool, input)]);
        reply.read_required_help = true;
        reply
    }
    /// The automatic approval protocol is a function call, also for stream=false.
    pub fn approve(reason: impl Into<String>) -> Self {
        Self::function_call("", "approve_action", json!({"reason": reason.into()}))
    }

    pub fn block(rule: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::function_call(
            "",
            "block_action",
            json!({"rule": rule.into(), "reason": reason.into()}),
        )
    }

    pub fn with_function_call(
        self,
        call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: Value,
    ) -> Self {
        self.map_completion(|completion| {
            completion
                .functions
                .push(FunctionCall::new(call_id, name, arguments));
        })
    }

    /// Append a `tools_call` to this reply. Like [`Reply::tool_call`], the
    /// scripted model reads the tool's help before the runtime will run it.
    pub fn with_tool_call(
        self,
        call_id: impl Into<String>,
        tool: impl Into<String>,
        input: Value,
    ) -> Self {
        self.map_completion(|completion| {
            completion
                .functions
                .push(FunctionCall::tool(call_id, tool, input));
        })
        .require_help()
    }

    pub fn with_thinking_chunks<I, S>(self, chunks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.map_completion(|completion| {
            completion.thinking = chunks.into_iter().map(Into::into).collect();
        })
    }

    /// Set argument deltas for the first function call.
    pub fn arguments_chunks<I, S>(mut self, chunks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        match &mut self.body {
            ReplyBody::Completion(completion) => {
                if let Some(call) = completion.functions.first_mut() {
                    call.argument_chunks = Some(chunks.into_iter().map(Into::into).collect());
                } else {
                    self.invalid = Some("arguments_chunks needs a function call".into());
                }
            }
            _ => self.invalid = Some("arguments_chunks needs a completion reply".into()),
        }
        self
    }

    pub fn usage(self, usage: Usage) -> Self {
        self.map_completion(|completion| completion.usage = Some(usage))
    }

    pub fn without_usage(self) -> Self {
        self.map_completion(|completion| completion.usage = None)
    }

    pub fn end_turn(self, end_turn: bool) -> Self {
        self.map_completion(|completion| completion.end_turn = Some(end_turn))
    }

    pub fn finish_reason(self, reason: impl Into<String>) -> Self {
        self.map_completion(|completion| completion.finish_reason = Some(reason.into()))
    }

    pub fn incomplete(self, reason: impl Into<String>) -> Self {
        self.map_completion(|completion| {
            let reason = reason.into();
            completion.finish_reason = Some(reason.clone());
            completion.incomplete = Some(reason);
        })
    }

    fn map_completion(mut self, update: impl FnOnce(&mut Completion)) -> Self {
        match &mut self.body {
            ReplyBody::Completion(completion) => update(completion),
            _ => self.invalid = Some("completion option applied to a raw reply".into()),
        }
        self
    }

    /// Hold before HTTP body chunk n (zero-based), or before EOF if n equals the
    /// number of chunks. This counts encoded chunks after byte fragmentation,
    /// including reasoning and function-call events. Use hold_after_text_chunks
    /// to wait for a stable number of text deltas instead.
    ///
    /// StepHandle::release is sticky and may be called before arrival. Combining
    /// an HTTP chunk gate with a text gate is invalid, in either builder order.
    pub fn hold_after_chunks(mut self, chunks: usize) -> Self {
        self.hold_after = Some(chunks);
        self
    }

    pub fn held(self) -> Self {
        self.hold_after_chunks(0)
    }

    /// Emit n complete text delta events, then wait for StepHandle::release.
    /// Thinking events precede this gate and byte fragmentation never moves it
    /// inside a text event. With n=0, hold immediately before the first text
    /// event; with n equal to the text chunk count, hold before later function
    /// events and the terminal response snapshot.
    ///
    /// This requires a streaming completion with at least one text chunk. Raw
    /// replies, nonstream requests, out-of-range counts, and combining this with
    /// hold_after_chunks/held produce an explicit fixture error.
    pub fn hold_after_text_chunks(mut self, chunks: usize) -> Self {
        self.hold_after_text = Some(chunks);
        self
    }

    /// Split encoded frames into HTTP body chunks, including inside UTF-8/SSE
    /// boundaries. TCP may combine these writes; client read sizes are not promised.
    pub fn fragment_bytes(mut self, bytes: usize) -> Self {
        self.fragment_bytes = Some(bytes);
        self
    }

    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    pub fn raw_json(status: u16, body: Value) -> Self {
        Self::new(ReplyBody::Json(body), status)
    }

    pub fn raw_body(status: u16, content_type: impl Into<String>, body: impl Into<Bytes>) -> Self {
        Self::new(ReplyBody::Bytes(body.into()), status).header("content-type", content_type)
    }

    /// Bytes are sent exactly as supplied, without adding SSE framing or a
    /// terminal event. This can model truncated streams and malformed payloads.
    pub fn raw_stream<I, B>(status: u16, chunks: I) -> Self
    where
        I: IntoIterator<Item = B>,
        B: Into<Bytes>,
    {
        Self::new(
            ReplyBody::Stream(chunks.into_iter().map(Into::into).collect()),
            status,
        )
    }

    pub(super) fn prepare(
        &self,
        request: &ModelRequest,
        step_id: u64,
        options: &ModelOptions,
    ) -> Result<PreparedReply> {
        ensure!(
            self.invalid.is_none(),
            "{}",
            self.invalid.as_deref().unwrap_or_default()
        );
        let status = StatusCode::from_u16(self.status).context("invalid response status")?;
        ensure!(
            !status.is_informational(),
            "informational status is not a final mock response"
        );
        if let Some(count) = self.hold_after_text {
            ensure!(
                self.hold_after.is_none(),
                "hold_after_text_chunks cannot be combined with hold_after_chunks or held"
            );
            let ReplyBody::Completion(completion) = &self.body else {
                anyhow::bail!(
                    "hold_after_text_chunks requires a completion reply, not a raw reply"
                );
            };
            ensure!(
                request.streaming(),
                "hold_after_text_chunks requires a streaming request"
            );
            ensure!(
                !completion.text.is_empty(),
                "hold_after_text_chunks requires at least one text chunk"
            );
            ensure!(
                count <= completion.text.len(),
                "hold_after_text_chunks({count}) exceeds the {} text chunks",
                completion.text.len()
            );
            ensure!(
                status != StatusCode::NO_CONTENT && status != StatusCode::NOT_MODIFIED,
                "hold_after_text_chunks cannot gate a bodyless HTTP response"
            );
        }
        let protocol = self
            .protocol
            .or_else(|| request.protocol())
            .unwrap_or(options.protocol);
        let (mut chunks, streaming, mut text_hold_after) = match &self.body {
            ReplyBody::Completion(completion) => {
                let (chunks, text_boundary) = super::wire::encode(
                    completion,
                    request,
                    protocol,
                    step_id,
                    self.hold_after_text,
                )?;
                (chunks, request.streaming(), text_boundary)
            }
            ReplyBody::Json(value) => (vec![Bytes::from(serde_json::to_vec(value)?)], false, None),
            ReplyBody::Bytes(bytes) => (vec![bytes.clone()], false, None),
            ReplyBody::Stream(chunks) => (chunks.clone(), true, None),
            ReplyBody::Events(protocol, events) => (
                super::protocol::encode_events(*protocol, events)?,
                true,
                None,
            ),
        };
        let bytes = chunks
            .iter()
            .try_fold(0_usize, |sum, chunk| sum.checked_add(chunk.len()))
            .context("response size overflow")?;
        ensure!(
            bytes <= options.max_response_bytes,
            "response has {bytes} bytes, exceeding the {} byte limit",
            options.max_response_bytes
        );
        if let Some(size) = self.fragment_bytes {
            ensure!(size > 0, "fragment_bytes must be positive");
            let count = chunks
                .iter()
                .try_fold(0_usize, |sum, chunk| {
                    sum.checked_add(chunk.len().div_ceil(size))
                })
                .context("response chunk count overflow")?;
            ensure!(
                count <= options.max_response_chunks,
                "response fragmentation exceeds chunk limit"
            );
            if let Some(boundary) = &mut text_hold_after {
                *boundary = chunks
                    .iter()
                    .take(*boundary)
                    .try_fold(0_usize, |sum, chunk| {
                        sum.checked_add(chunk.len().div_ceil(size))
                    })
                    .context("text gate fragment count overflow")?;
            }
            chunks = chunks
                .iter()
                .flat_map(|chunk| {
                    (0..chunk.len()).step_by(size).map(move |start| {
                        chunk.slice(start..start.saturating_add(size).min(chunk.len()))
                    })
                })
                .collect();
        }
        ensure!(
            chunks.len() <= options.max_response_chunks,
            "response exceeds chunk limit"
        );
        let hold_after = self.hold_after.or(text_hold_after);
        if let Some(hold_after) = hold_after {
            ensure!(
                hold_after <= chunks.len(),
                "hold_after_chunks({hold_after}) exceeds the {} response chunks",
                chunks.len()
            );
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static(if streaming {
                match &self.body {
                    ReplyBody::Completion(_) => protocol.stream_content_type(),
                    ReplyBody::Events(protocol, _) => protocol.stream_content_type(),
                    _ => "text/event-stream",
                }
            } else {
                "application/json"
            }),
        );
        if streaming {
            headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        }
        if streaming
            && (matches!(&self.body, ReplyBody::Completion(_))
                && protocol == ModelProtocol::BedrockAnthropic
                || matches!(
                    &self.body,
                    ReplyBody::Events(ModelProtocol::BedrockAnthropic, _)
                ))
        {
            headers.insert(
                HeaderName::from_static("x-amzn-bedrock-content-type"),
                HeaderValue::from_static("application/json"),
            );
        }
        for (name, value) in &self.headers {
            headers.insert(
                HeaderName::from_bytes(name.as_bytes())
                    .context("invalid mock response header name")?,
                HeaderValue::from_str(value).context("invalid mock response header value")?,
            );
        }
        Ok(PreparedReply {
            status,
            headers,
            chunks,
            hold_after,
        })
    }
}

pub(super) struct PreparedReply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub chunks: Vec<Bytes>,
    pub hold_after: Option<usize>,
}

pub(super) fn encode_completion(
    completion: &Completion,
    request: &ModelRequest,
    step_id: u64,
    hold_after_text: Option<usize>,
) -> Result<(Vec<Bytes>, Option<usize>)> {
    let response_id = format!("resp_testkit_{step_id}");
    let message_id = format!("msg_testkit_{step_id}");
    let reasoning_id = format!("rs_testkit_{step_id}");
    let mut output = Vec::new();
    if !completion.thinking.is_empty() {
        output.push(json!({
            "type": "reasoning",
            "id": reasoning_id,
            "summary": [{"type": "summary_text", "text": completion.thinking.concat()}]
        }));
    }
    let message_index = output.len();
    if !completion.text.is_empty() {
        output.push(json!({
            "id": message_id,
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": completion.text.concat(),
                "annotations": []
            }]
        }));
    }
    let first_function_index = output.len();
    let mut functions = Vec::new();
    for (index, call) in completion.functions.iter().enumerate() {
        ensure!(!call.name.is_empty(), "function name must not be empty");
        let call_id = if call.call_id.is_empty() {
            format!("call_testkit_{step_id}_{index}")
        } else {
            call.call_id.clone()
        };
        let item = json!({
            "type": "function_call",
            "id": format!("fc_testkit_{step_id}_{index}"),
            "call_id": call_id,
            "name": call.name,
            "arguments": call.arguments_json,
            "status": "completed"
        });
        output.push(item.clone());
        functions.push(item);
    }
    let status = if completion.incomplete.is_some() {
        "incomplete"
    } else {
        "completed"
    };
    let finish_reason =
        completion
            .finish_reason
            .as_deref()
            .unwrap_or(if completion.functions.is_empty() {
                "stop"
            } else {
                "tool_calls"
            });
    let mut response = json!({
        "id": response_id,
        "object": "response",
        "created_at": 0,
        "status": status,
        "model": request.decoded_model().unwrap_or_else(|| "fake-model".into()),
        "error": null,
        "incomplete_details": completion.incomplete.as_ref().map(|reason| json!({"reason": reason})),
        "output": output,
        "stop_reason": finish_reason
    });
    if let Some(usage) = &completion.usage {
        response["usage"] = usage.json();
    }
    if let Some(end_turn) = completion.end_turn {
        response["end_turn"] = json!(end_turn);
    }
    if !request.streaming() {
        return Ok((vec![Bytes::from(serde_json::to_vec(&response)?)], None));
    }

    let mut events = Vec::new();
    for delta in &completion.thinking {
        events.push(sse(json!({
            "type": "response.reasoning_summary_text.delta",
            "item_id": reasoning_id,
            "output_index": 0,
            "summary_index": 0,
            "delta": delta
        }))?);
    }
    let mut text_boundary = (hold_after_text == Some(0)).then_some(events.len());
    for (index, delta) in completion.text.iter().enumerate() {
        events.push(sse(json!({
            "type": "response.output_text.delta",
            "item_id": message_id,
            "output_index": message_index,
            "content_index": 0,
            "delta": delta
        }))?);
        if hold_after_text == Some(index + 1) {
            text_boundary = Some(events.len());
        }
    }
    for (index, (call, item)) in completion.functions.iter().zip(functions).enumerate() {
        let output_index = first_function_index + index;
        let item_id = item["id"].as_str().context("generated function item ID")?;
        let mut added = item.clone();
        added["arguments"] = json!("");
        added["status"] = json!("in_progress");
        events.push(sse(json!({
            "type": "response.output_item.added",
            "output_index": output_index,
            "item": added
        }))?);
        let default_chunks = [call.arguments_json.clone()];
        let deltas = call.argument_chunks.as_deref().unwrap_or(&default_chunks);
        for delta in deltas {
            events.push(sse(json!({
                "type": "response.function_call_arguments.delta",
                "item_id": item_id,
                "output_index": output_index,
                "delta": delta
            }))?);
        }
        events.push(sse(json!({
            "type": "response.function_call_arguments.done",
            "item_id": item_id,
            "output_index": output_index,
            "arguments": call.arguments_json
        }))?);
        events.push(sse(json!({
            "type": "response.output_item.done",
            "output_index": output_index,
            "item": item
        }))?);
    }
    events.push(sse(json!({
        "type": if completion.incomplete.is_some() { "response.incomplete" } else { "response.completed" },
        "response": response
    }))?);
    Ok((events, text_boundary))
}

fn sse(event: Value) -> Result<Bytes> {
    let mut bytes = b"data: ".to_vec();
    bytes.extend(serde_json::to_vec(&event)?);
    bytes.extend_from_slice(b"\n\n");
    Ok(Bytes::from(bytes))
}
