use anyhow::{Result, ensure};
use bytes::Bytes;
use serde_json::{Value, json};

use super::{arguments, native_stop};
use crate::model::{
    ModelEvent, ModelProtocol, ModelRequest, protocol::encode_event, reply::Completion,
};

pub(super) fn encode(
    completion: &Completion,
    request: &ModelRequest,
    _step: u64,
    hold: Option<usize>,
) -> Result<(Vec<Bytes>, Option<usize>)> {
    let model = request
        .decoded_model()
        .unwrap_or_else(|| "fake-model".into());
    let mut calls = Vec::new();
    for call in &completion.functions {
        ensure!(
            call.argument_chunks.is_none(),
            "Ollama tool arguments are complete JSON objects; use ModelEvent for malformed events"
        );
        calls.push(json!({"function":{"name":call.name,"arguments":arguments(call)?}}));
    }
    let mut terminal = json!({"model":model,"created_at":"1970-01-01T00:00:00Z",
        "message":{"role":"assistant","content":""},
        "done":true,"done_reason":native_stop(completion, "stop", "tool_calls", "length")});
    if let Some(usage) = &completion.usage {
        terminal["prompt_eval_count"] = json!(usage.input_tokens);
        terminal["eval_count"] = json!(usage.output_tokens);
    }
    if !request.streaming() {
        terminal["message"]["content"] = json!(completion.text.concat());
        if !completion.thinking.is_empty() {
            terminal["message"]["thinking"] = json!(completion.thinking.concat());
        }
        if !calls.is_empty() {
            terminal["message"]["tool_calls"] = json!(calls);
        }
        return Ok((vec![Bytes::from(serde_json::to_vec(&terminal)?)], None));
    }
    let chunk = |message: Value| {
        json!({"model":model,"created_at":"1970-01-01T00:00:00Z",
        "message":message,"done":false})
    };
    let mut events = Vec::new();
    for delta in &completion.thinking {
        events.push(chunk(
            json!({"role":"assistant","thinking":delta,"content":""}),
        ));
    }
    let mut boundary = (hold == Some(0)).then_some(events.len());
    for (index, delta) in completion.text.iter().enumerate() {
        events.push(chunk(json!({"role":"assistant","content":delta})));
        if hold == Some(index + 1) {
            boundary = Some(events.len());
        }
    }
    for call in calls {
        events.push(chunk(
            json!({"role":"assistant","content":"","tool_calls":[call]}),
        ));
    }
    events.push(terminal);
    let chunks = events
        .into_iter()
        .map(|event| encode_event(ModelProtocol::Ollama, &ModelEvent::json(event)))
        .collect::<Result<Vec<_>>>()?;
    Ok((chunks, boundary))
}
